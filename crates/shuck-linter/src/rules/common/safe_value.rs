use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    BourneParameterExpansion, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp,
    RedirectKind, SourceText, Span, VarRef, Word, WordPart, WordPartNode,
};
use shuck_semantic::{
    AssignmentValueOrigin, BindingAttributes, BindingKind, BindingOrigin, ContractCertainty,
    LoopValueOrigin, ScopeId, ScopeKind, SemanticAnalysis, SemanticModel, UninitializedCertainty,
};
use shuck_semantic::{BindingId, BlockId, ReferenceId, ReferenceKind};

use crate::{FactSpan, LinterFacts};

use super::{
    expansion::{ExpansionContext, analyze_literal_runtime},
    word::static_word_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafeValueQuery {
    Argv,
    RedirectTarget,
    Pattern,
    Regex,
    Quoted,
}

enum SourceTextLiteral<'a> {
    Bare(&'a str),
    Quoted(&'a str),
}

impl SafeValueQuery {
    pub fn from_context(context: ExpansionContext) -> Option<Self> {
        match context {
            ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::HereString
            | ExpansionContext::DeclarationAssignmentValue => Some(Self::Argv),
            ExpansionContext::RedirectTarget(_) => Some(Self::RedirectTarget),
            ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern => Some(Self::Pattern),
            ExpansionContext::RegexOperand => Some(Self::Regex),
            _ => None,
        }
    }

    fn operand_context(self) -> Option<ExpansionContext> {
        match self {
            Self::Argv => Some(ExpansionContext::CommandArgument),
            Self::RedirectTarget => Some(ExpansionContext::RedirectTarget(RedirectKind::Output)),
            Self::Pattern => Some(ExpansionContext::CasePattern),
            Self::Regex => Some(ExpansionContext::RegexOperand),
            Self::Quoted => None,
        }
    }

    fn literal_is_safe(self, text: &str) -> bool {
        match self {
            Self::Argv | Self::RedirectTarget => literal_is_field_safe(text),
            Self::Pattern => literal_is_pattern_safe(text),
            Self::Regex => literal_is_regex_safe(text),
            Self::Quoted => true,
        }
    }
}

pub struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    analysis: &'a SemanticAnalysis<'a>,
    facts: &'a LinterFacts<'a>,
    source: &'a str,
    maybe_uninitialized_refs: FxHashSet<FactSpan>,
    memo: FxHashMap<(FactSpan, SafeValueQuery), bool>,
    visiting: FxHashSet<(FactSpan, SafeValueQuery)>,
}

impl<'a> SafeValueIndex<'a> {
    pub fn build(
        semantic: &'a SemanticModel,
        analysis: &'a SemanticAnalysis<'a>,
        facts: &'a LinterFacts<'a>,
        source: &'a str,
    ) -> Self {
        let maybe_uninitialized_refs = analysis
            .uninitialized_references()
            .iter()
            .filter(|uninitialized| uninitialized.certainty == UninitializedCertainty::Possible)
            .map(|uninitialized| FactSpan::new(semantic.reference(uninitialized.reference).span))
            .collect();

        Self {
            semantic,
            analysis,
            facts,
            source,
            maybe_uninitialized_refs,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    pub fn part_is_safe(&mut self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        match part {
            WordPart::ZshQualifiedGlob(_) => query == SafeValueQuery::Quoted,
            WordPart::Parameter(parameter) => self.parameter_part_is_safe(parameter, span, query),
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                self.literal_part_is_safe(part, span, query)
            }
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter()
                .all(|part| self.part_is_safe(&part.kind, part.span, query)),
            WordPart::Variable(name) => self.name_is_safe(name, span, query),
            WordPart::ArithmeticExpansion { .. } => true,
            WordPart::Length(_) | WordPart::ArrayLength(_) => true,
            WordPart::ArrayAccess(reference) => {
                (query == SafeValueQuery::Quoted || !reference.has_array_selector())
                    && self.reference_is_safe(reference, span, query)
            }
            WordPart::Substring { reference, .. } => self.reference_is_safe(reference, span, query),
            WordPart::Transformation {
                reference,
                operator,
            } => self.transformation_is_safe(reference, *operator, span, query),
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                self.indirect_name_is_safe(reference, span, query)
                    && operator.as_ref().is_none_or(|operator| {
                        self.parameter_operator_is_safe(
                            &reference.name,
                            operator,
                            operand.as_ref(),
                            span,
                            query,
                        )
                    })
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. } => query == SafeValueQuery::Quoted,
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                self.parameter_expansion_is_safe(reference, operator, operand.as_ref(), span, query)
            }
        }
    }

    pub fn word_is_safe(&mut self, word: &Word, query: SafeValueQuery) -> bool {
        let Some(analysis) = self
            .facts
            .any_word_fact(word.span)
            .map(|fact| fact.analysis())
        else {
            return false;
        };
        if query != SafeValueQuery::Quoted
            && (analysis.array_valued || analysis.hazards.command_or_process_substitution)
        {
            return false;
        }

        word.parts_with_spans()
            .all(|(part, span)| self.part_is_safe(part, span, query))
    }

    fn literal_part_is_safe(&self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        let word = Word {
            parts: vec![WordPartNode::new(part.clone(), span)],
            span,
            brace_syntax: Vec::new(),
        };
        if let Some(context) = query.operand_context()
            && analyze_literal_runtime(&word, self.source, context, None).is_runtime_sensitive()
        {
            return false;
        }

        static_word_text(&word, self.source).is_some_and(|text| query.literal_is_safe(&text))
    }

    fn name_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        if safe_special_parameter(name) {
            return true;
        }

        let bindings = self.safe_bindings_for_name(name, at);
        if bindings.is_empty() {
            return safe_numeric_shell_variable(name);
        }
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at))
            && !bindings
                .iter()
                .copied()
                .any(|binding_id| self.binding_dominates_reference(binding_id, name, at))
        {
            return false;
        }

        bindings
            .into_iter()
            .all(|binding_id| self.binding_is_safe(binding_id, query))
    }

    fn binding_is_safe(&mut self, binding_id: BindingId, query: SafeValueQuery) -> bool {
        let binding = self.semantic.binding(binding_id);
        let binding_key = FactSpan::new(binding.span);
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let key = (binding_key, query);
        if let Some(result) = self.memo.get(&key) {
            return *result;
        }
        if !self.visiting.insert(key) {
            return false;
        }

        let result = match &binding.origin {
            BindingOrigin::Assignment {
                value:
                    AssignmentValueOrigin::PlainScalarAccess | AssignmentValueOrigin::StaticLiteral,
                ..
            } => {
                let scalar_word = self
                    .facts
                    .binding_value(binding_id)
                    .filter(|value| !value.conditional_assignment_shortcut())
                    .and_then(|value| value.scalar_word());
                scalar_word.is_some_and(|word| self.word_is_safe(word, query))
            }
            BindingOrigin::LoopVariable {
                items: LoopValueOrigin::StaticWords,
                ..
            } => {
                let words = self
                    .facts
                    .binding_value(binding_id)
                    .and_then(|value| value.loop_words())
                    .map(|words| words.to_vec());
                words.is_some_and(|words| {
                    !words.is_empty()
                        && words.into_iter().all(|word| {
                            !word_contains_special_parameter_slice(word)
                                && self.word_is_safe(word, query)
                        })
                })
            }
            BindingOrigin::Assignment { .. }
            | BindingOrigin::LoopVariable { .. }
            | BindingOrigin::ParameterDefaultAssignment { .. }
            | BindingOrigin::Imported { .. }
            | BindingOrigin::FunctionDefinition { .. }
            | BindingOrigin::BuiltinTarget { .. }
            | BindingOrigin::ArithmeticAssignment { .. }
            | BindingOrigin::Declaration { .. }
            | BindingOrigin::Nameref { .. } => false,
        };

        self.visiting.remove(&key);
        self.memo.insert(key, result);
        result
    }

    fn reference_is_safe(&mut self, reference: &VarRef, at: Span, query: SafeValueQuery) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        self.name_is_safe(&reference.name, at, query)
    }

    fn indirect_name_is_safe(
        &mut self,
        reference: &VarRef,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            return false;
        }

        let bindings = self.safe_bindings_for_name(&reference.name, at);
        if bindings.is_empty() {
            return false;
        }

        bindings.into_iter().all(|binding_id| {
            let targets = self.semantic.indirect_targets_for_binding(binding_id);
            !targets.is_empty()
                && targets
                    .iter()
                    .copied()
                    .all(|target| self.binding_is_safe(target, query))
        })
    }

    fn safe_bindings_for_name(&self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self.analysis.reaching_bindings_for_name(name, at);
        if bindings.len() == 1 {
            let mut expanded = self
                .analysis
                .visible_bindings_bypassing(name, bindings[0], at);
            if !expanded.is_empty() {
                expanded.push(bindings[0]);
                expanded
                    .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
                expanded.dedup();
                bindings = expanded;
            }
        }
        if bindings.is_empty() {
            bindings = self.called_helper_bindings_for_name(name, at);
        }

        bindings
    }

    fn called_helper_bindings_for_name(&self, name: &Name, at: Span) -> Vec<BindingId> {
        let scope = self.semantic.scope_at(at.start.offset);
        let mut seen = FxHashSet::default();
        let mut bindings = self.called_helper_bindings_before(name, scope, at, &mut seen);
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn called_helper_bindings_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
        seen: &mut FxHashSet<(ScopeId, usize, usize)>,
    ) -> Vec<BindingId> {
        if !seen.insert((scope, at.start.offset, at.end.offset)) {
            return Vec::new();
        }

        let mut bindings = self
            .helper_bindings_called_in_scope_before(name, scope, at)
            .into_iter()
            .collect::<FxHashSet<_>>();

        if let Some(caller_bindings) = self.helper_bindings_reaching_all_callers(name, scope, seen)
        {
            bindings.extend(caller_bindings);
        }

        seen.remove(&(scope, at.start.offset, at.end.offset));
        bindings.into_iter().collect()
    }

    fn helper_bindings_reaching_all_callers(
        &self,
        name: &Name,
        scope: ScopeId,
        seen: &mut FxHashSet<(ScopeId, usize, usize)>,
    ) -> Option<FxHashSet<BindingId>> {
        let function_kind = self.named_function_kind(scope)?;
        let mut caller_sites = Vec::new();
        let mut seen_sites = FxHashSet::default();

        for function_name in function_kind.static_names() {
            for site in self.semantic.call_sites_for(function_name) {
                if site.scope == scope {
                    continue;
                }
                if seen_sites.insert((site.scope, site.span.start.offset, site.span.end.offset)) {
                    caller_sites.push(site.clone());
                }
            }
        }

        let mut saw_caller = false;
        let mut union = FxHashSet::default();
        for site in caller_sites {
            saw_caller = true;
            let branch = self
                .called_helper_bindings_before(name, site.scope, site.span, seen)
                .into_iter()
                .collect::<FxHashSet<_>>();
            if branch.is_empty() {
                return Some(FxHashSet::default());
            }
            union.extend(branch);
        }

        saw_caller.then_some(union)
    }

    fn helper_bindings_called_in_scope_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut bindings = Vec::new();

        for callee_scope in self.helper_scopes_definitely_providing_name(name) {
            let Some(function_kind) = self.named_function_kind(callee_scope) else {
                continue;
            };

            let called_before = function_kind.static_names().iter().any(|function_name| {
                self.semantic
                    .call_sites_for(function_name)
                    .iter()
                    .any(|site| {
                        site.scope == scope && self.call_site_dominates_use(site.span, name, at)
                    })
            });
            if !called_before {
                continue;
            }

            bindings.extend(self.semantic.bindings_for(name).iter().copied().filter(
                |binding_id| {
                    let binding = self.semantic.binding(*binding_id);
                    binding.scope == callee_scope
                        && !binding.attributes.contains(BindingAttributes::LOCAL)
                },
            ));
        }

        bindings
    }

    fn call_site_dominates_use(&self, call_span: Span, name: &Name, at: Span) -> bool {
        if call_span.start.offset >= at.start.offset {
            return false;
        }
        let _ = name;

        !self.facts.commands().iter().any(|fact| {
            let outer = fact.span();
            outer.start.offset < call_span.start.offset
                && call_span.end.offset <= outer.end.offset
                && outer.end.offset <= at.start.offset
                && match fact.command() {
                    shuck_ast::Command::Binary(_) => true,
                    shuck_ast::Command::Compound(compound) => !matches!(
                        compound,
                        shuck_ast::CompoundCommand::BraceGroup(_)
                            | shuck_ast::CompoundCommand::Arithmetic(_)
                            | shuck_ast::CompoundCommand::Time(_)
                    ),
                    shuck_ast::Command::Simple(_)
                    | shuck_ast::Command::Builtin(_)
                    | shuck_ast::Command::Decl(_)
                    | shuck_ast::Command::Function(_)
                    | shuck_ast::Command::AnonymousFunction(_) => false,
                }
        })
    }

    fn helper_scopes_definitely_providing_name(&self, name: &Name) -> Vec<ScopeId> {
        self.semantic
            .scopes()
            .iter()
            .filter_map(|scope| matches!(scope.kind, ScopeKind::Function(_)).then_some(scope.id))
            .filter(|scope| {
                self.analysis
                    .summarize_scope_provided_bindings(*scope)
                    .iter()
                    .any(|binding| {
                        binding.name == *name && binding.certainty == ContractCertainty::Definite
                    })
            })
            .collect()
    }

    fn named_function_kind(&self, scope: ScopeId) -> Option<&shuck_semantic::FunctionScopeKind> {
        match &self.semantic.scope(scope).kind {
            ScopeKind::Function(function) if !function.static_names().is_empty() => Some(function),
            ScopeKind::File
            | ScopeKind::Function(_)
            | ScopeKind::Subshell
            | ScopeKind::CommandSubstitution
            | ScopeKind::Pipeline => None,
        }
    }

    fn binding_dominates_reference(&self, binding_id: BindingId, name: &Name, at: Span) -> bool {
        let Some(reference_id) = self.reference_id_for_name_at(name, at) else {
            return false;
        };
        let Some(reference_block) = self.block_for_reference(reference_id) else {
            return false;
        };
        let Some(binding_block) = self.block_for_binding(binding_id) else {
            return false;
        };
        if binding_block == reference_block {
            return true;
        }

        let cfg = self.analysis.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let mut stack = vec![cfg.entry()];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if block_id == binding_block
                || unreachable.contains(&block_id)
                || !seen.insert(block_id)
            {
                continue;
            }
            if block_id == reference_block {
                return false;
            }
            for (successor, _) in cfg.successors(block_id) {
                stack.push(*successor);
            }
        }

        true
    }
    fn reference_id_for_name_at(&self, name: &Name, at: Span) -> Option<ReferenceId> {
        self.semantic
            .references()
            .iter()
            .find(|reference| {
                reference.span == at
                    && &reference.name == name
                    && !matches!(
                        reference.kind,
                        ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
                    )
            })
            .map(|reference| reference.id)
    }

    fn block_for_binding(&self, binding_id: BindingId) -> Option<BlockId> {
        self.analysis
            .cfg()
            .blocks()
            .iter()
            .find(|block| block.bindings.contains(&binding_id))
            .map(|block| block.id)
    }

    fn block_for_reference(&self, reference_id: ReferenceId) -> Option<BlockId> {
        self.analysis
            .cfg()
            .blocks()
            .iter()
            .find(|block| block.references.contains(&reference_id))
            .map(|block| block.id)
    }

    fn transformation_is_safe(
        &mut self,
        reference: &VarRef,
        operator: char,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }

        match operator {
            'Q' | 'K' | 'k' => true,
            _ => self.reference_is_safe(reference, at, query),
        }
    }

    fn parameter_part_is_safe(
        &mut self,
        parameter: &ParameterExpansion,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference } => {
                    (query == SafeValueQuery::Quoted || !reference.has_array_selector())
                        && self.reference_is_safe(reference, at, query)
                }
                BourneParameterExpansion::Length { .. } => true,
                BourneParameterExpansion::Indices { .. }
                | BourneParameterExpansion::PrefixMatch { .. } => query == SafeValueQuery::Quoted,
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    self.indirect_name_is_safe(reference, at, query)
                        && operator.as_ref().is_none_or(|operator| {
                            self.parameter_operator_is_safe(
                                &reference.name,
                                operator,
                                operand.as_ref(),
                                at,
                                query,
                            )
                        })
                }
                BourneParameterExpansion::Slice { reference, .. } => {
                    if reference.has_array_selector() {
                        query == SafeValueQuery::Quoted
                    } else {
                        self.reference_is_safe(reference, at, query)
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    ..
                } => self.parameter_expansion_is_safe(
                    reference,
                    operator,
                    operand.as_ref(),
                    at,
                    query,
                ),
                BourneParameterExpansion::Transformation {
                    reference,
                    operator,
                } => self.transformation_is_safe(reference, *operator, at, query),
            },
            ParameterExpansionSyntax::Zsh(_) => false,
        }
    }

    fn parameter_expansion_is_safe(
        &mut self,
        reference: &VarRef,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }

        self.parameter_operator_is_safe(&reference.name, operator, operand, at, query)
    }

    fn parameter_operator_is_safe(
        &mut self,
        name: &Name,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        match operator {
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll
            | ParameterOp::RemovePrefixShort { .. }
            | ParameterOp::RemovePrefixLong { .. }
            | ParameterOp::RemoveSuffixShort { .. }
            | ParameterOp::RemoveSuffixLong { .. } => self.name_is_safe(name, at, query),
            ParameterOp::UseDefault | ParameterOp::AssignDefault | ParameterOp::Error => {
                self.name_is_safe(name, at, query)
                    && operand
                        .is_some_and(|operand| self.source_text_is_safe_literal(operand, query))
            }
            ParameterOp::UseReplacement => {
                operand.is_some_and(|operand| self.source_text_is_safe_literal(operand, query))
            }
            ParameterOp::ReplaceFirst { replacement, .. }
            | ParameterOp::ReplaceAll { replacement, .. } => {
                self.name_is_safe(name, at, query)
                    && self.source_text_is_safe_literal(replacement, query)
            }
        }
    }

    fn source_text_is_safe_literal(&self, text: &SourceText, query: SafeValueQuery) -> bool {
        source_text_literal_value(text.slice(self.source)).is_some_and(|literal| match literal {
            SourceTextLiteral::Bare(text) => query.literal_is_safe(text),
            SourceTextLiteral::Quoted(text) => match query {
                SafeValueQuery::Argv | SafeValueQuery::RedirectTarget | SafeValueQuery::Quoted => {
                    true
                }
                SafeValueQuery::Pattern | SafeValueQuery::Regex => query.literal_is_safe(text),
            },
        })
    }
}

fn literal_is_field_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '*' | '?' | '['))
}

fn literal_is_pattern_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']' | '|' | '(' | ')'))
}

fn literal_is_regex_safe(text: &str) -> bool {
    let mut escaped = false;

    for character in text.chars() {
        if escaped {
            return false;
        }

        if character == '\\' {
            escaped = true;
            continue;
        }

        if matches!(
            character,
            '.' | '[' | ']' | '(' | ')' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$'
        ) {
            return false;
        }
    }

    !escaped
}

fn source_text_needs_parse(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '$' | '`' | '\\' | '\'' | '"'))
}

fn source_text_literal_value(text: &str) -> Option<SourceTextLiteral<'_>> {
    if !source_text_needs_parse(text) {
        return Some(SourceTextLiteral::Bare(text));
    }

    if let Some(inner) = text.strip_prefix('"').and_then(|text| text.strip_suffix('"'))
        && !inner
            .chars()
            .any(|character| matches!(character, '$' | '`' | '\\' | '"'))
    {
        return Some(SourceTextLiteral::Quoted(inner));
    }

    if let Some(inner) = text.strip_prefix('\'').and_then(|text| text.strip_suffix('\''))
        && !inner.contains('\'')
    {
        return Some(SourceTextLiteral::Quoted(inner));
    }

    None
}

fn safe_special_parameter(name: &Name) -> bool {
    matches!(name.as_str(), "@" | "#" | "?" | "$" | "!" | "-")
}

fn safe_numeric_shell_variable(name: &Name) -> bool {
    matches!(name.as_str(), "PPID")
}

fn word_contains_special_parameter_slice(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        part_contains_special_parameter_slice(&part.kind)
            && !matches!(part.kind, WordPart::DoubleQuoted { .. })
    })
}

fn part_contains_special_parameter_slice(part: &WordPart) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_special_parameter_slice(&part.kind)),
        WordPart::Substring { reference, .. } => special_parameter_slice_reference(reference),
        WordPart::Parameter(parameter) => parameter_contains_special_parameter_slice(parameter),
        _ => false,
    }
}

fn parameter_contains_special_parameter_slice(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice { reference, .. }) => {
            special_parameter_slice_reference(reference)
        }
        ParameterExpansionSyntax::Bourne(_) | ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn special_parameter_slice_reference(reference: &VarRef) -> bool {
    matches!(reference.name.as_str(), "@" | "*")
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, Name};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::{
        ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind,
        SemanticBuildOptions, SemanticModel,
    };

    use super::{SafeValueIndex, SafeValueQuery};
    use crate::LinterFacts;
    use crate::rules::common::expansion::ExpansionContext;
    use crate::{ShellDialect, classify_file_context};

    #[test]
    fn maps_pattern_and_regex_contexts_into_safe_value_queries() {
        use shuck_ast::RedirectKind;

        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CommandArgument),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::HereString),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CommandName),
            Some(SafeValueQuery::Argv)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::RedirectTarget(RedirectKind::Output)),
            Some(SafeValueQuery::RedirectTarget)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::CasePattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::ConditionalPattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::ParameterPattern),
            Some(SafeValueQuery::Pattern)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::RegexOperand),
            Some(SafeValueQuery::Regex)
        );
        assert_eq!(
            SafeValueQuery::from_context(ExpansionContext::StringTestOperand),
            None
        );
    }

    #[test]
    fn quoted_query_treats_prefix_matches_as_safe_only_when_quoted() {
        let source = "#!/bin/bash\nprintf '%s\\n' \"${!HOME@}\" ${!HOME@}\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(safe_values.word_is_safe(&command.args[1], SafeValueQuery::Quoted));
        assert!(!safe_values.word_is_safe(&command.args[2], SafeValueQuery::Argv));
    }

    #[test]
    fn treats_zsh_parameter_modifiers_as_dynamic_unknown_values() {
        let source = "print ${(m)foo}\n";
        let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Zsh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Quoted));
    }

    #[test]
    fn keeps_typed_zsh_parameter_operations_conservative() {
        let source = "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}}\n";
        let output = Parser::with_dialect(source, shuck_parser::parser::ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Zsh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert!(
            command
                .args
                .iter()
                .all(|word| !safe_values.word_is_safe(word, SafeValueQuery::Argv))
        );
        assert!(
            command
                .args
                .iter()
                .all(|word| !safe_values.word_is_safe(word, SafeValueQuery::Quoted))
        );
    }

    #[test]
    fn conditional_safe_fallbacks_do_not_hide_unsafe_bindings() {
        let source = "\
#!/bin/bash
foo=$(printf '%s' \"$1\")
if [ \"$foo\" = \"\" ]; then foo=0; fi
[ $foo -eq 1 ]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[2].command else {
            panic!("expected simple test command");
        };

        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn conditionally_initialized_names_stay_unsafe() {
        let source = "\
#!/bin/bash
if [ \"$1\" = yes ]; then
  foo=0
fi
[ $foo -eq 1 ]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[1].command else {
            panic!("expected simple test command");
        };

        assert!(!safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn reassigned_ppid_stops_being_treated_as_runtime_safe() {
        let source = "\
#!/bin/sh
PPID='a b'
printf '%s\\n' $PPID
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[1].command else {
            panic!("expected simple command");
        };

        assert!(!safe_values.word_is_safe(&command.args[1], SafeValueQuery::Argv));
    }

    #[test]
    fn loop_bindings_derived_from_at_slices_stay_unsafe() {
        let source = "\
#!/bin/bash
f() {
  for v in ${@:2}; do
    del $v
  done
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$v"
            })
            .expect("expected loop-body command argument");

        assert!(!safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn direct_at_slices_do_not_become_safe_value_failures() {
        let source = "\
#!/bin/bash
f() {
  dns_set ${@:2}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${@:2}"
            })
            .expect("expected direct slice command argument");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn plain_access_bindings_stay_safe_but_parameter_operations_do_not_propagate() {
        let source = "\
#!/bin/bash
base=Foobar
copy=$base
mixed=$base-1.0
lower=${base,}
trimmed=${base#Foo}
count=${#base}
printf '%s\\n' $copy $mixed $lower $trimmed $count
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[6].command else {
            panic!("expected simple command");
        };

        assert!(safe_values.word_is_safe(&command.args[1], SafeValueQuery::Argv));
        assert!(safe_values.word_is_safe(&command.args[2], SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(&command.args[3], SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(&command.args[4], SafeValueQuery::Argv));
        assert!(!safe_values.word_is_safe(&command.args[5], SafeValueQuery::Argv));
    }

    #[test]
    fn loop_bindings_from_expanded_words_do_not_propagate() {
        let source = "\
#!/bin/bash
name=neverball
for i in $name prefix$name; do
  printf '%s\\n' $i $i.png
done
for size in 16 32; do
  printf '%s\\n' $size ${size}x${size}!
done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let unsafe_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && matches!(fact.span().slice(source), "$i" | "$i.png")
            })
            .collect::<Vec<_>>();
        assert_eq!(unsafe_words.len(), 2, "expected unsafe loop-body words");
        assert!(
            unsafe_words
                .iter()
                .all(|fact| !safe_values.word_is_safe(fact.word(), SafeValueQuery::Argv))
        );

        let safe_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && matches!(fact.span().slice(source), "$size" | "${size}x${size}!")
            })
            .collect::<Vec<_>>();
        assert_eq!(safe_words.len(), 2, "expected safe literal-loop words");
        assert!(
            safe_words
                .iter()
                .all(|fact| safe_values.word_is_safe(fact.word(), SafeValueQuery::Argv))
        );
    }

    #[test]
    fn assignment_ternary_bindings_do_not_propagate_safe_values() {
        let source = "\
#!/bin/bash
true && w='-w' || w=''
if true; then
  flag='-w'
else
  flag=''
fi
iptables $w -t nat -N chain
iptables $flag -t nat -N chain
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let short_circuit_word = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$w"
            })
            .expect("expected short-circuit command argument");
        let if_else_word = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$flag"
            })
            .expect("expected if/else command argument");

        assert!(!safe_values.word_is_safe(short_circuit_word.word(), SafeValueQuery::Argv));
        assert!(safe_values.word_is_safe(if_else_word.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn nested_guarded_assignment_ternaries_stay_unsafe() {
        let source = "\
#!/bin/bash
f() {
  [ \"$1\" = iptables ] && {
    true && w='-w' || w=''
  }
  [ \"$1\" = ip6tables ] && {
    true && w='-w' || w=''
  }
  iptables $w -t nat -N chain
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$w"
            })
            .expect("expected guarded function command argument");

        assert!(!safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn unconditional_safe_overwrites_stay_safe() {
        let source = "\
#!/bin/bash
foo=$(printf '%s' \"$1\")
foo=0
[ $foo -eq 1 ]
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[2].command else {
            panic!("expected simple test command");
        };

        assert!(safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn case_arm_safe_overwrites_stay_safe() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
    settings)
        foo=0
        [ $foo -eq 1 ]
        ;;
esac
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Compound(shuck_ast::CompoundCommand::Case(case_command)) =
            &output.file.body[1].command
        else {
            panic!("expected case command");
        };
        let Command::Simple(command) = &case_command.cases[0].body[1].command else {
            panic!("expected simple test command");
        };

        assert!(safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn if_else_safe_literal_bindings_stay_safe() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
free ${humanreadable}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let Command::Simple(command) = &output.file.body[1].command else {
            panic!("expected simple command");
        };

        assert!(safe_values.word_is_safe(&command.args[0], SafeValueQuery::Argv));
    }

    #[test]
    fn if_else_safe_literal_bindings_stay_safe_inside_command_substitutions() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
value=\"$(free ${humanreadable} | awk '{print $2}')\"
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.is_nested_word_command() && fact.span().slice(source) == "${humanreadable}"
            })
            .expect("expected nested command argument fact");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn helper_initialized_option_flags_stay_safe_across_top_level_call_sequences() {
        let source = "\
#!/bin/bash
fn_select_compression() {
  if command -v zstd >/dev/null 2>&1; then
    compressflag=--zstd
  elif command -v pigz >/dev/null 2>&1; then
    compressflag=--use-compress-program=pigz
  elif command -v gzip >/dev/null 2>&1; then
    compressflag=--gzip
  else
    compressflag=
  fi
}

fn_backup_check_lockfile() { :; }
fn_backup_create_lockfile() { :; }
fn_backup_init() { :; }
fn_backup_stop_server() { :; }
fn_backup_dir() { :; }

fn_backup_compression() {
  if [ -n \"${compressflag}\" ]; then
    tar ${compressflag} -hcf out.tar ./.
  else
    tar -hcf out.tar ./.
  fi
}

fn_select_compression
fn_backup_check_lockfile
fn_backup_create_lockfile
fn_backup_init
fn_backup_stop_server
fn_backup_dir
fn_backup_compression
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${compressflag}"
            })
            .expect("expected helper-provided command argument fact");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn helper_initialized_bindings_do_not_leak_across_distinct_callers() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  printf '%s\\n' ${flag}
}

safe_path() {
  init_flag
  render
}

unsafe_path() {
  render
}

safe_path
unsafe_path
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected shared helper-derived command argument fact");

        assert!(!safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn helper_calls_inside_conditionals_do_not_count_as_definite_initializers() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  if [ \"$1\" = yes ]; then
    init_flag
  fi
  printf '%s\\n' ${flag}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected conditionally helper-initialized argument fact");

        assert!(!safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn helper_initialized_bindings_stay_safe_when_all_callers_provide_distinct_values() {
        let source = "\
#!/bin/bash
init_flag_a() {
  flag=-a
}

init_flag_b() {
  flag=-b
}

render() {
  printf '%s\\n' ${flag}
}

safe_path_a() {
  init_flag_a
  render
}

safe_path_b() {
  init_flag_b
  render
}

safe_path_a
safe_path_b
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected multi-caller helper-derived argument fact");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn imported_bindings_stay_unsafe_without_known_values() {
        let source = "\
#!/bin/bash
printf '%s\\n' $pkgname
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                file_entry_contract: Some(FileContract {
                    required_reads: Vec::new(),
                    provided_bindings: vec![ProvidedBinding::new(
                        Name::from("pkgname"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    )],
                    provided_functions: Vec::new(),
                }),
                ..SemanticBuildOptions::default()
            },
        );
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$pkgname"
            })
            .expect("expected imported command argument fact");

        assert!(!safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn quoted_replacement_operands_stay_safe_in_argv_context() {
        let source = "\
#!/bin/bash
bash ${debug:+\"-x\"} script
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${debug:+\"-x\"}"
            })
            .expect("expected replacement-operator command argument fact");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }

    #[test]
    fn quoted_replacement_operands_with_spaces_stay_safe_in_argv_context() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${debug:+\"a b\"}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${debug:+\"a b\"}"
            })
            .expect("expected replacement-operator command argument fact");

        assert!(safe_values.word_is_safe(word_fact.word(), SafeValueQuery::Argv));
    }
}
