use std::cell::RefCell;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    BinaryOp, BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand, FunctionDef,
    Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Position, RedirectKind, Span,
    Stmt, StmtSeq, StmtTerminator, VarRef, Word, WordPart, WordPartNode, static_word_text,
    word_is_standalone_status_capture, word_is_standalone_variable_like,
};
use shuck_semantic::{
    AssignmentValueOrigin, BindingAttributes, BindingKind, BindingOrigin, CallSite,
    LoopValueOrigin, ScopeId, ScopeKind, SemanticAnalysis, SemanticModel, SemanticValueFlow,
    UninitializedCertainty,
};
use shuck_semantic::{BindingId, BlockId, ReferenceId};

use crate::facts::words::analyze_literal_runtime;
use crate::{ExpansionContext, FactSpan, LinterFacts};

type S001FunctionEventKey = Vec<usize>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafeValueQuery {
    Argv,
    RedirectTarget,
    NumericTestOperand,
    Pattern,
    Regex,
    Quoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldSafeBindingClass {
    Empty,
    NonEmpty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S001QuoteExposure {
    Unsafe,
    Empty,
    QuoteInertNonEmpty,
}

impl SafeValueQuery {
    pub fn from_context(context: ExpansionContext) -> Option<Self> {
        match context {
            ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::HereString
            | ExpansionContext::DeclarationAssignmentValue => Some(Self::Argv),
            ExpansionContext::RedirectTarget(_) | ExpansionContext::DescriptorDupTarget(_) => {
                Some(Self::RedirectTarget)
            }
            ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern => Some(Self::Pattern),
            ExpansionContext::RegexOperand => Some(Self::Regex),
            ExpansionContext::AssignmentValue
            | ExpansionContext::ForList
            | ExpansionContext::SelectList
            | ExpansionContext::StringTestOperand
            | ExpansionContext::ConditionalVarRefSubscript
            | ExpansionContext::TrapAction => None,
        }
    }

    fn operand_context(self) -> Option<ExpansionContext> {
        match self {
            Self::Argv => Some(ExpansionContext::CommandArgument),
            Self::NumericTestOperand => Some(ExpansionContext::CommandArgument),
            Self::RedirectTarget => Some(ExpansionContext::RedirectTarget(RedirectKind::Output)),
            Self::Pattern => Some(ExpansionContext::CasePattern),
            Self::Regex => Some(ExpansionContext::RegexOperand),
            Self::Quoted => None,
        }
    }

    fn is_field_context(self) -> bool {
        matches!(
            self,
            Self::Argv | Self::RedirectTarget | Self::NumericTestOperand
        )
    }

    fn literal_is_safe(self, text: &str) -> bool {
        match self {
            Self::Argv | Self::RedirectTarget | Self::NumericTestOperand => {
                literal_is_field_safe(text)
            }
            Self::Pattern => literal_is_pattern_safe(text),
            Self::Regex => literal_is_regex_safe(text),
            Self::Quoted => true,
        }
    }
}

pub struct SafeValueIndex<'a> {
    semantic: &'a SemanticModel,
    analysis: &'a SemanticAnalysis<'a>,
    value_flow: RefCell<SemanticValueFlow<'a, 'a>>,
    facts: &'a LinterFacts<'a>,
    source: &'a str,
    command_cover_memo: RefCell<FxHashMap<(crate::facts::CommandId, Name, FactSpan), bool>>,
    memo: FxHashMap<(FactSpan, FactSpan, SafeValueQuery, Option<ScopeId>), bool>,
    visiting: FxHashSet<(FactSpan, FactSpan, SafeValueQuery, Option<ScopeId>)>,
    binding_value_stack: Vec<BindingId>,
    s001_unset_before_call_memo: FxHashMap<ScopeId, bool>,
    #[cfg(test)]
    uninitialized_reference_overrides: FxHashMap<FactSpan, UninitializedCertainty>,
}

impl<'a> SafeValueIndex<'a> {
    pub fn build(
        semantic: &'a SemanticModel,
        analysis: &'a SemanticAnalysis<'a>,
        facts: &'a LinterFacts<'a>,
        source: &'a str,
    ) -> Self {
        Self {
            semantic,
            analysis,
            value_flow: RefCell::new(analysis.value_flow()),
            facts,
            source,
            command_cover_memo: RefCell::new(FxHashMap::default()),
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
            binding_value_stack: Vec::new(),
            s001_unset_before_call_memo: FxHashMap::default(),
            #[cfg(test)]
            uninitialized_reference_overrides: FxHashMap::default(),
        }
    }

    pub fn part_is_safe(&mut self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        if part_is_safe_special_parameter_access(part) {
            return true;
        }
        if self.span_is_after_unconditional_inline_terminator(span) {
            return false;
        }
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
                operand_word_ast,
                ..
            } => {
                self.indirect_name_is_safe(reference, span, query)
                    && operator.as_ref().is_none_or(|operator| {
                        self.parameter_operator_is_safe(
                            &reference.name,
                            operator,
                            operand_word_ast.as_ref(),
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
                operand_word_ast,
                ..
            } => self.parameter_expansion_is_safe(
                reference,
                operator,
                operand_word_ast.as_ref(),
                span,
                query,
            ),
        }
    }

    pub fn word_is_safe(&mut self, word: &Word, query: SafeValueQuery) -> bool {
        if self.span_is_after_unconditional_inline_terminator(word.span) {
            return false;
        }
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

    fn uninitialized_reference_certainty_at(&self, span: Span) -> Option<UninitializedCertainty> {
        #[cfg(test)]
        if let Some(certainty) = self
            .uninitialized_reference_overrides
            .get(&FactSpan::new(span))
            .copied()
        {
            return Some(certainty);
        }

        self.analysis.uninitialized_reference_certainty_at(span)
    }

    #[cfg(test)]
    fn override_uninitialized_reference_certainty(
        &mut self,
        span: Span,
        certainty: UninitializedCertainty,
    ) {
        self.uninitialized_reference_overrides
            .insert(FactSpan::new(span), certainty);
    }

    #[cfg(test)]
    pub fn word_occurrence_is_safe(
        &mut self,
        fact: crate::WordOccurrenceRef<'_, 'a>,
        query: SafeValueQuery,
    ) -> bool {
        if self.span_is_after_unconditional_inline_terminator(fact.span()) {
            return false;
        }
        let analysis = fact.analysis();
        if query != SafeValueQuery::Quoted
            && (analysis.array_valued || analysis.hazards.command_or_process_substitution)
        {
            return false;
        }

        fact.parts_with_spans()
            .all(|(part, span)| self.part_is_safe(part, span, query))
    }

    pub fn name_reference_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        self.name_is_safe(name, at, query)
    }

    pub fn part_s001_quote_exposure(
        &mut self,
        part: &WordPart,
        span: Span,
        query: SafeValueQuery,
    ) -> S001QuoteExposure {
        if !query.is_field_context() {
            return if self.part_is_safe(part, span, query) {
                S001QuoteExposure::QuoteInertNonEmpty
            } else {
                S001QuoteExposure::Unsafe
            };
        }
        if self.span_is_after_unconditional_inline_terminator(span) {
            return S001QuoteExposure::Unsafe;
        }

        match part {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                self.literal_part_s001_quote_exposure(part, span, query)
            }
            WordPart::DoubleQuoted { parts, .. } => {
                let mut saw_empty = false;
                for part in parts {
                    match self.part_s001_quote_exposure(&part.kind, part.span, query) {
                        S001QuoteExposure::QuoteInertNonEmpty => {
                            return S001QuoteExposure::QuoteInertNonEmpty;
                        }
                        S001QuoteExposure::Empty => saw_empty = true,
                        S001QuoteExposure::Unsafe => return S001QuoteExposure::Unsafe,
                    }
                }
                if saw_empty {
                    S001QuoteExposure::Empty
                } else {
                    S001QuoteExposure::QuoteInertNonEmpty
                }
            }
            WordPart::Variable(name) => self.name_s001_quote_exposure(name, span, query),
            WordPart::Parameter(parameter) => {
                self.parameter_part_s001_quote_exposure(parameter, span, query)
            }
            WordPart::ArrayAccess(reference) => {
                if reference.has_array_selector() {
                    S001QuoteExposure::Unsafe
                } else {
                    self.name_s001_quote_exposure(&reference.name, span, query)
                }
            }
            WordPart::Substring { reference, .. } => {
                self.name_s001_quote_exposure(&reference.name, span, query)
            }
            WordPart::Transformation { reference, .. } => {
                self.name_s001_quote_exposure(&reference.name, span, query)
            }
            WordPart::IndirectExpansion { reference, .. } => {
                self.name_s001_quote_exposure(&reference.name, span, query)
            }
            WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayLength(_) => S001QuoteExposure::QuoteInertNonEmpty,
            WordPart::ZshQualifiedGlob(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ParameterExpansion { .. } => {
                if self.part_is_safe(part, span, query) {
                    S001QuoteExposure::QuoteInertNonEmpty
                } else {
                    S001QuoteExposure::Unsafe
                }
            }
        }
    }

    pub fn span_has_s001_function_unset_exposure(
        &mut self,
        span: Span,
        query: SafeValueQuery,
    ) -> bool {
        query.is_field_context() && self.s001_reference_function_unset_before_first_call(span)
    }

    pub fn part_has_s001_standalone_numeric_argv_exposure(
        &mut self,
        part: &WordPart,
        span: Span,
    ) -> bool {
        if self.span_is_return_argument(span) {
            return false;
        }
        let Some(name) = plain_scalar_reference_name_from_part(part) else {
            return false;
        };

        self.s001_name_has_only_numeric_value_bindings(&name, span)
    }

    pub fn part_has_s001_arithmetic_numeric_operand_exposure(
        &mut self,
        part: &WordPart,
        span: Span,
    ) -> bool {
        let Some(name) = plain_scalar_reference_name_from_part(part) else {
            return false;
        };

        self.s001_name_has_only_arithmetic_numeric_bindings(&name, span)
    }

    pub fn part_is_safe_initializer_command_substitution_self_reference(
        &mut self,
        part: &WordPart,
        span: Span,
        query: SafeValueQuery,
    ) -> bool {
        let Some(name) = plain_scalar_reference_name_from_part(part) else {
            return false;
        };

        self.semantic
            .bindings_for(&name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                if binding.span.start.offset > span.start.offset {
                    return false;
                }
                let Some(word) = self
                    .facts
                    .binding_value(binding_id)
                    .and_then(|value| value.scalar_word())
                else {
                    return false;
                };
                if !span_contains(word.span, span) {
                    return false;
                }

                self.facts.any_word_fact(word.span).is_some_and(|fact| {
                    fact.command_substitution_spans()
                        .iter()
                        .copied()
                        .any(|command_substitution| span_contains(command_substitution, span))
                }) && !self.span_is_inside_loop_context(span)
                    && !self.span_is_inside_if_condition(span)
                    && {
                        self.binding_value_stack.push(binding_id);
                        let result = self.name_is_safe(&name, span, query);
                        self.binding_value_stack.pop();
                        result
                    }
            })
    }

    pub fn part_is_safe_initializer_command_substitution_static_setup_reference(
        &mut self,
        part: &WordPart,
        span: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Argv {
            return false;
        }
        let Some(name) = plain_scalar_reference_name_from_part(part) else {
            return false;
        };
        if !shell_name_is_uppercase_setup_value(name.as_str())
            || !self.span_is_inside_initializer_command_substitution(span)
            || self.span_is_inside_loop_context(span)
            || self.span_is_inside_if_condition(span)
        {
            return false;
        }

        let mut setup_bindings = Vec::new();
        for binding_id in self.semantic.bindings_for(&name).iter().copied() {
            let binding = self.semantic.binding(binding_id);
            if binding.span.start.offset >= span.start.offset {
                continue;
            }
            if binding.attributes.contains(BindingAttributes::LOCAL) {
                return false;
            }
            if !matches!(
                self.semantic.scope(binding.scope).kind,
                ScopeKind::File | ScopeKind::Function(_)
            ) {
                return false;
            }
            let setup_atom = self
                .facts
                .binding_value(binding_id)
                .and_then(|value| value.scalar_word())
                .and_then(|word| static_word_text(word, self.source))
                .is_some_and(|text| literal_is_setup_atom(&text));
            if !setup_atom {
                return false;
            }
            setup_bindings.push(binding_id);
        }

        !setup_bindings.is_empty()
            && (self.bindings_cover_all_paths_to_reference(&setup_bindings, &name, span)
                || setup_bindings.iter().copied().any(|binding_id| {
                    let binding = self.semantic.binding(binding_id);
                    binding.span.end.offset <= span.start.offset
                        && self.binding_dominates_reference(binding_id, &name, span)
                }))
    }

    fn span_is_inside_initializer_command_substitution(&self, span: Span) -> bool {
        self.semantic.bindings().iter().any(|binding| {
            let Some(word) = self
                .facts
                .binding_value(binding.id)
                .and_then(|value| value.scalar_word())
            else {
                return false;
            };
            span_contains(word.span, span)
                && self.facts.any_word_fact(word.span).is_some_and(|fact| {
                    fact.command_substitution_spans()
                        .iter()
                        .copied()
                        .any(|command_substitution| span_contains(command_substitution, span))
                })
        })
    }

    fn span_is_inside_loop_context(&self, span: Span) -> bool {
        let mut current = self
            .facts
            .innermost_command_id_at(span.start.offset)
            .or_else(|| {
                self.facts
                    .innermost_command_id_containing_offset(span.start.offset)
            });
        while let Some(command_id) = current {
            if matches!(
                self.facts.command(command_id).command(),
                Command::Compound(
                    CompoundCommand::For(_)
                        | CompoundCommand::Repeat(_)
                        | CompoundCommand::Foreach(_)
                        | CompoundCommand::ArithmeticFor(_)
                        | CompoundCommand::While(_)
                        | CompoundCommand::Until(_)
                        | CompoundCommand::Select(_)
                )
            ) {
                return true;
            }
            current = self.facts.command_parent_id(command_id);
        }

        false
    }

    fn span_is_inside_if_condition(&self, span: Span) -> bool {
        let mut current = self
            .facts
            .innermost_command_id_at(span.start.offset)
            .or_else(|| {
                self.facts
                    .innermost_command_id_containing_offset(span.start.offset)
            });
        while let Some(command_id) = current {
            if let Command::Compound(CompoundCommand::If(command)) =
                self.facts.command(command_id).command()
                && (span_contains(command.condition.span, span)
                    || command
                        .elif_branches
                        .iter()
                        .any(|(condition, _)| span_contains(condition.span, span)))
            {
                return true;
            }
            current = self.facts.command_parent_id(command_id);
        }

        false
    }

    fn literal_part_is_safe(&self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
        let word = Word {
            parts: vec![WordPartNode::new(part.clone(), span)],
            span,
            brace_syntax: Vec::new(),
        };
        if let Some(context) = query.operand_context()
            && self.literal_runtime_is_unsafe_for_safe_value(&word, context)
        {
            return false;
        }

        static_word_text(&word, self.source).is_some_and(|text| query.literal_is_safe(&text))
    }

    fn literal_part_s001_quote_exposure(
        &self,
        part: &WordPart,
        span: Span,
        query: SafeValueQuery,
    ) -> S001QuoteExposure {
        let word = Word {
            parts: vec![WordPartNode::new(part.clone(), span)],
            span,
            brace_syntax: Vec::new(),
        };
        if let Some(context) = query.operand_context()
            && self.literal_runtime_is_unsafe_for_safe_value(&word, context)
        {
            return S001QuoteExposure::Unsafe;
        }

        static_word_text(&word, self.source).map_or(S001QuoteExposure::Unsafe, |text| {
            if !query.literal_is_safe(&text) {
                S001QuoteExposure::Unsafe
            } else if text.is_empty() {
                S001QuoteExposure::Empty
            } else {
                S001QuoteExposure::QuoteInertNonEmpty
            }
        })
    }

    fn literal_runtime_is_unsafe_for_safe_value(
        &self,
        word: &Word,
        context: ExpansionContext,
    ) -> bool {
        let runtime = analyze_literal_runtime(word, self.source, context, None);
        if !runtime.is_runtime_sensitive() {
            return false;
        }

        !self.in_binding_value()
            || !runtime.hazards.tilde_expansion
            || runtime.hazards.pathname_matching
            || runtime.hazards.brace_fanout
    }

    fn in_binding_value(&self) -> bool {
        !self.binding_value_stack.is_empty()
    }

    fn name_is_safe(&mut self, name: &Name, at: Span, query: SafeValueQuery) -> bool {
        if safe_special_parameter(name) {
            return true;
        }
        if self.case_cli_reachable_call_path_keeps_argument_bindings_unsafe(at, query) {
            let mut bindings = self.safe_bindings_for_name(name, at);
            self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
            self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
            bindings.retain(|binding_id| {
                !self.binding_is_cleared_by_dominating_unset(*binding_id, name, at)
            });
            bindings.retain(|binding_id| {
                !self.binding_is_blocked_by_exit_like_function_call(*binding_id, at)
            });
            if self.local_declaration_status_capture_bindings_cover_reference(
                &bindings, name, at, query,
            ) {
                return true;
            }
            return safe_numeric_shell_variable(name);
        }
        if query == SafeValueQuery::NumericTestOperand
            && self.s001_function_reference_has_file_scope_integer_bindings(name, at)
        {
            return true;
        }

        let mut bindings = self.safe_bindings_for_name(name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
        bindings.retain(|binding_id| {
            !self.binding_is_cleared_by_dominating_unset(*binding_id, name, at)
        });
        if query.is_field_context() {
            bindings.retain(|binding_id| {
                !self.binding_is_blocked_by_exit_like_function_call(*binding_id, at)
            });
        }
        let case_cli_scope = query
            .is_field_context()
            .then(|| self.case_cli_dispatch_scope_at(at.start.offset))
            .flatten();
        if bindings.is_empty()
            && self.status_capture_declaration_probe_covers_reference(
                name,
                at,
                query,
                case_cli_scope,
            )
        {
            return true;
        }
        if bindings.is_empty() {
            return safe_numeric_shell_variable(name);
        }
        if self
            .local_declaration_status_capture_bindings_cover_reference(&bindings, name, at, query)
        {
            return true;
        }
        let binding_belongs_to_case_cli_scope = case_cli_scope.is_some_and(|scope| {
            bindings
                .iter()
                .copied()
                .any(|binding_id| self.binding_is_in_scope_or_descendant(binding_id, scope))
        });
        if query.is_field_context()
            && case_cli_scope.is_some()
            && !self.case_cli_dispatch_outer_bindings_can_stay_safe(&bindings, at, query)
            && !binding_belongs_to_case_cli_scope
        {
            return safe_numeric_shell_variable(name);
        }
        if query.is_field_context() && self.bindings_are_all_plain_empty_static_literals(&bindings)
        {
            return false;
        }
        if self.covering_optional_field_safe_bindings_can_stay_safe(&bindings, name, at, query) {
            return true;
        }
        if self.local_conditional_literal_bindings_can_stay_safe(&bindings, name, at, query) {
            return true;
        }
        if self.optional_field_safe_bindings_can_stay_safe(&bindings, query) {
            return true;
        }
        if self.status_capture_bindings_cover_reference(&bindings, name, at, query, case_cli_scope)
        {
            return true;
        }
        if self.status_capture_subset_covers_reference(&bindings, name, at, query, case_cli_scope) {
            return true;
        }
        let helper_bindings = self
            .called_helper_bindings_for_name(name, at)
            .into_iter()
            .collect::<FxHashSet<_>>();
        let needs_arg_path_coverage = query.is_field_context();
        let bindings_cover_all_paths = helper_bindings.is_empty()
            && needs_arg_path_coverage
            && self.value_sources_cover_all_paths_to_reference(&bindings, name, at);
        let unset_covers_reference = needs_arg_path_coverage
            && !bindings.is_empty()
            && self.unset_command_covers_reference(name, at);
        let direct_bindings = if helper_bindings.is_empty() {
            Vec::new()
        } else {
            bindings
                .iter()
                .copied()
                .filter(|binding_id| !helper_bindings.contains(binding_id))
                .collect::<Vec<_>>()
        };
        let direct_bindings_cover_all_paths = needs_arg_path_coverage
            && !direct_bindings.is_empty()
            && self.bindings_cover_all_paths_to_reference(&direct_bindings, name, at);
        let unsafe_helper_binding_writes_visible_local = self
            .enclosing_function_scope_at(at.start.offset)
            .is_some_and(|scope| {
                helper_bindings.iter().copied().any(|binding_id| {
                    self.binding_writes_visible_local_in_scope_before(binding_id, scope, at)
                        && !self.binding_is_safe(binding_id, at, query, case_cli_scope)
                })
            });
        if unsafe_helper_binding_writes_visible_local {
            return false;
        }
        if direct_bindings_cover_all_paths
            && self.enclosing_function_scope_at(at.start.offset).is_some()
            && !self.span_is_exit_or_return_argument(at)
            && self.covering_direct_field_safe_bindings_can_stay_safe(&direct_bindings, query)
        {
            return true;
        }
        if needs_arg_path_coverage
            && !bindings_cover_all_paths
            && !direct_bindings_cover_all_paths
            && self.one_sided_bindings_preserve_safe_base(
                &bindings,
                name,
                at,
                query,
                case_cli_scope,
            )
        {
            return true;
        }
        if needs_arg_path_coverage
            && !direct_bindings_cover_all_paths
            && !bindings.is_empty()
            && bindings
                .iter()
                .copied()
                .all(|binding_id| helper_bindings.contains(&binding_id))
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_one_sided_short_circuit_assignment(binding_id))
        {
            return false;
        }
        if needs_arg_path_coverage
            && !direct_bindings_cover_all_paths
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_one_sided_append_assignment(binding_id))
        {
            return bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_safe(binding_id, at, query, case_cli_scope));
        }
        let direct_bindings_are_status_captures =
            direct_bindings.iter().copied().all(|binding_id| {
                self.binding_is_standalone_status_capture(binding_id, at, case_cli_scope)
            });
        if direct_bindings_cover_all_paths && direct_bindings_are_status_captures {
            bindings.retain(|binding_id| !helper_bindings.contains(binding_id));
        }
        let outer_bindings_cover_callers = !needs_arg_path_coverage
            || self.helper_outer_bindings_cover_all_caller_paths(name, at, &bindings);
        let reference_is_inside_function =
            self.enclosing_function_scope_at(at.start.offset).is_some();
        if helper_bindings.is_empty()
            && needs_arg_path_coverage
            && !bindings_cover_all_paths
            && !unset_covers_reference
            && (!outer_bindings_cover_callers || !reference_is_inside_function)
        {
            return false;
        }
        if !outer_bindings_cover_callers && !direct_bindings_cover_all_paths {
            return false;
        }
        match self.uninitialized_reference_certainty_at(at) {
            Some(UninitializedCertainty::Definite) => {
                if bindings.iter().copied().any(|binding_id| {
                    !helper_bindings.contains(&binding_id)
                        && self.binding_is_guarded_before_reference(binding_id, at)
                }) {
                    return false;
                }
            }
            Some(UninitializedCertainty::Possible) => {
                let has_dominating_binding = bindings
                    .iter()
                    .copied()
                    .any(|binding_id| self.binding_dominates_reference(binding_id, name, at));
                if !has_dominating_binding
                    && !bindings_cover_all_paths
                    && !unset_covers_reference
                    && !bindings
                        .iter()
                        .copied()
                        .all(|binding_id| helper_bindings.contains(&binding_id))
                {
                    return false;
                }
            }
            None => {}
        }

        bindings
            .into_iter()
            .all(|binding_id| self.binding_is_safe(binding_id, at, query, case_cli_scope))
    }

    fn name_s001_quote_exposure(
        &mut self,
        name: &Name,
        at: Span,
        query: SafeValueQuery,
    ) -> S001QuoteExposure {
        if safe_special_parameter(name) {
            return S001QuoteExposure::QuoteInertNonEmpty;
        }
        if query.is_field_context() && self.s001_reference_function_unset_before_first_call(at) {
            return S001QuoteExposure::Unsafe;
        }
        if query == SafeValueQuery::NumericTestOperand
            && self.s001_function_reference_has_file_scope_integer_bindings(name, at)
        {
            return S001QuoteExposure::QuoteInertNonEmpty;
        }
        if self.name_is_safe(name, at, query) {
            return S001QuoteExposure::QuoteInertNonEmpty;
        }
        if !query.is_field_context() {
            return S001QuoteExposure::Unsafe;
        }

        let mut bindings = self.safe_bindings_for_name(name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
        bindings.retain(|binding_id| {
            !self.binding_is_cleared_by_dominating_unset(*binding_id, name, at)
        });
        bindings.retain(|binding_id| {
            !self.binding_is_blocked_by_exit_like_function_call(*binding_id, at)
        });
        if !bindings.is_empty() {
            let dispatch_bindings = self.s001_top_level_dispatch_helper_bindings_before(name, at);
            if !dispatch_bindings.is_empty() {
                let mut combined = bindings.clone();
                combined.extend(dispatch_bindings);
                combined
                    .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
                combined.dedup();
                if let Some(exposure) =
                    self.s001_field_safe_binding_group_exposure(&combined, at, query)
                {
                    return exposure;
                }
            }
            return S001QuoteExposure::Unsafe;
        }

        let dispatch_bindings = self.s001_top_level_dispatch_helper_bindings_before(name, at);
        self.s001_field_safe_binding_group_exposure(&dispatch_bindings, at, query)
            .unwrap_or(S001QuoteExposure::Unsafe)
    }

    fn case_cli_dispatch_scope_at(&self, offset: usize) -> Option<ScopeId> {
        self.semantic
            .ancestor_scopes(self.semantic.scope_at(offset))
            .find(|scope| {
                self.facts
                    .function_cli_dispatch_facts(*scope)
                    .exported_from_case_cli()
            })
    }

    fn case_cli_reachable_function_scope_at(&self, offset: usize) -> Option<ScopeId> {
        let scope = self.analysis.enclosing_function_scope_at(offset)?;
        self.facts
            .is_case_cli_reachable_function_scope(scope)
            .then_some(scope)
    }

    fn case_cli_reachable_call_path_keeps_argument_bindings_unsafe(
        &self,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context() || self.span_is_within_command_name(at) {
            return false;
        }
        let Some(scope) = self.case_cli_reachable_function_scope_at(at.start.offset) else {
            return false;
        };

        self.facts
            .function_cli_dispatch_facts(scope)
            .exported_from_case_cli()
            || self.static_caller_is_case_cli_exported(scope, &mut FxHashSet::default())
            || self.named_function_call_sites(scope).is_empty()
    }

    fn static_caller_is_case_cli_exported(
        &self,
        scope: ScopeId,
        seen: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if !seen.insert(scope) {
            return false;
        }

        self.named_function_call_sites(scope)
            .into_iter()
            .any(|(caller_scope, _)| {
                self.facts
                    .function_cli_dispatch_facts(caller_scope)
                    .exported_from_case_cli()
                    || self.static_caller_is_case_cli_exported(caller_scope, seen)
            })
    }

    fn case_cli_dispatch_outer_bindings_can_stay_safe(
        &self,
        bindings: &[BindingId],
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Argv || !self.is_argument_of_dynamic_command(at) {
            return false;
        }

        bindings
            .iter()
            .copied()
            .all(|binding_id| self.binding_is_quoted_static_literal(binding_id))
    }

    fn binding_is_in_scope_or_descendant(
        &self,
        binding_id: BindingId,
        ancestor_scope: ScopeId,
    ) -> bool {
        self.analysis
            .binding_is_in_scope_or_descendant(binding_id, ancestor_scope)
    }

    fn is_argument_of_dynamic_command(&self, at: Span) -> bool {
        self.facts.commands().iter().any(|command| {
            command.body_args().iter().any(|word| word.span == at)
                && command
                    .body_name_word()
                    .is_some_and(word_is_standalone_variable_like)
        })
    }

    fn span_is_exit_or_return_argument(&self, at: Span) -> bool {
        self.facts
            .innermost_command_id_at(at.start.offset)
            .or_else(|| {
                self.facts
                    .innermost_command_id_containing_offset(at.start.offset)
            })
            .is_some_and(|command_id| {
                let command = self.facts.command(command_id);
                match command.command() {
                    Command::Builtin(BuiltinCommand::Exit(command)) => {
                        command
                            .code
                            .as_ref()
                            .is_some_and(|word| span_contains(word.span, at))
                            || command
                                .extra_args
                                .iter()
                                .any(|word| span_contains(word.span, at))
                    }
                    Command::Builtin(BuiltinCommand::Return(command)) => {
                        command
                            .code
                            .as_ref()
                            .is_some_and(|word| span_contains(word.span, at))
                            || command
                                .extra_args
                                .iter()
                                .any(|word| span_contains(word.span, at))
                    }
                    Command::Simple(_)
                    | Command::Builtin(_)
                    | Command::Decl(_)
                    | Command::Binary(_)
                    | Command::Compound(_)
                    | Command::Function(_)
                    | Command::AnonymousFunction(_) => false,
                }
            })
    }

    fn span_is_return_argument(&self, at: Span) -> bool {
        self.facts
            .innermost_command_id_at(at.start.offset)
            .or_else(|| {
                self.facts
                    .innermost_command_id_containing_offset(at.start.offset)
            })
            .is_some_and(|command_id| {
                let command = self.facts.command(command_id);
                match command.command() {
                    Command::Builtin(BuiltinCommand::Return(command)) => {
                        command
                            .code
                            .as_ref()
                            .is_some_and(|word| span_contains(word.span, at))
                            || command
                                .extra_args
                                .iter()
                                .any(|word| span_contains(word.span, at))
                    }
                    Command::Simple(_)
                    | Command::Builtin(_)
                    | Command::Decl(_)
                    | Command::Binary(_)
                    | Command::Compound(_)
                    | Command::Function(_)
                    | Command::AnonymousFunction(_) => false,
                }
            })
    }

    fn span_is_within_command_name(&self, at: Span) -> bool {
        self.facts.commands().iter().any(|command| {
            command
                .body_name_word()
                .is_some_and(|word| span_contains(word.span, at))
        })
    }

    fn binding_is_blocked_by_exit_like_function_call(
        &self,
        binding_id: BindingId,
        at: Span,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        self.facts.function_headers().iter().any(|header| {
            function_has_terminal_exit(header.function())
                && header
                    .call_arity()
                    .zero_arg_call_spans()
                    .iter()
                    .filter_map(|call_span| self.command_for_name_word_span(*call_span))
                    .any(|command| {
                        !command.is_nested_word_command()
                            && command.body_args().is_empty()
                            && self.command_runs_in_unconditional_flow(command.id(), at)
                            && {
                                let call_span = command.span_in_source(self.source);
                                self.definition_command_resolves_at_call(
                                    header.command_id(),
                                    call_span,
                                ) && call_span.end.offset <= at.start.offset
                                    && (call_span.start.offset >= binding.span.end.offset
                                        || call_span.end.offset <= binding.span.start.offset)
                            }
                    })
        })
    }

    fn span_is_after_unconditional_inline_terminator(&self, at: Span) -> bool {
        self.facts.commands().iter().any(|command| {
            command.span().end.offset <= at.start.offset
                && !command.is_nested_word_command()
                && self.command_runs_in_unconditional_flow(command.id(), at)
                && matches!(
                    stmt_terminal_flow_kind(command.stmt()),
                    TerminalFlowKind::Exit | TerminalFlowKind::Stop
                )
        })
    }

    fn definition_command_is_visible_at_call(
        &self,
        command_id: crate::facts::CommandId,
        call_span: Span,
    ) -> bool {
        let command = self.facts.command(command_id);
        let command_scope = self.enclosing_function_scope_at(command.span().start.offset);
        let call_scope = self.enclosing_function_scope_at(call_span.start.offset);
        if command_scope.is_some() && command_scope != call_scope {
            return false;
        }
        if self.command_is_in_background_context(command_id) {
            return false;
        }

        let mut parent_id = self.facts.command_parent_id(command_id);
        while let Some(id) = parent_id {
            if self.facts.command_is_dominance_barrier(id) {
                return false;
            }
            parent_id = self.facts.command_parent_id(id);
        }
        true
    }

    fn definition_command_resolves_at_call(
        &self,
        command_id: crate::facts::CommandId,
        call_span: Span,
    ) -> bool {
        if !self.definition_command_is_visible_at_call(command_id, call_span) {
            return false;
        }

        let command = self.facts.command(command_id);
        let definition_scope = self.enclosing_function_scope_at(command.span().start.offset);
        let call_scope = self.enclosing_function_scope_at(call_span.start.offset);

        if definition_scope.is_none() && call_scope.is_some() {
            return true;
        }

        command.span_in_source(self.source).end.offset <= call_span.start.offset
    }

    fn command_for_name_word_span(
        &self,
        span: Span,
    ) -> Option<crate::facts::CommandFactRef<'a, 'a>> {
        self.facts.command_for_name_word_span(span)
    }

    fn function_definition_command_for_scope(
        &self,
        scope: ScopeId,
    ) -> Option<crate::facts::CommandFactRef<'a, 'a>> {
        self.facts.function_definition_command(scope)
    }

    fn function_scope_resolves_at_call_site(&self, callee_scope: ScopeId, site: &CallSite) -> bool {
        if let Some(binding_id) = self
            .analysis
            .visible_function_binding_at_call(&site.callee, site.name_span)
        {
            return self.analysis.function_scope_for_binding(binding_id) == Some(callee_scope);
        }

        self.function_definition_command_for_scope(callee_scope)
            .is_some_and(|definition_command| {
                self.definition_command_resolves_at_call(definition_command.id(), site.span)
            })
    }

    fn s001_reference_function_unset_before_first_call(&mut self, at: Span) -> bool {
        let Some(scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return false;
        };
        if let Some(result) = self.s001_unset_before_call_memo.get(&scope) {
            return *result;
        }

        let result = self.s001_function_unset_before_first_call(scope);
        self.s001_unset_before_call_memo.insert(scope, result);
        result
    }

    fn s001_function_unset_before_first_call(&self, scope: ScopeId) -> bool {
        let Some(definition_command) = self.function_definition_command_for_scope(scope) else {
            return false;
        };
        let after_offset = definition_command.span_in_source(self.source).end.offset;
        let Some(first_unset) = self.s001_first_function_unset_event_after(scope, after_offset)
        else {
            return false;
        };

        let first_call = self.s001_first_function_call_event_after(scope, after_offset);
        first_call.is_none_or(|first_call| first_unset < first_call)
    }

    fn s001_first_function_call_event_after(
        &self,
        scope: ScopeId,
        after_offset: usize,
    ) -> Option<S001FunctionEventKey> {
        self.s001_function_call_event_keys_after_inner(
            scope,
            after_offset,
            &mut FxHashSet::default(),
        )
        .into_iter()
        .min()
    }

    fn s001_function_call_event_keys_after_inner(
        &self,
        scope: ScopeId,
        after_offset: usize,
        seen_scopes: &mut FxHashSet<ScopeId>,
    ) -> Vec<S001FunctionEventKey> {
        if !seen_scopes.insert(scope) {
            return Vec::new();
        }

        let result = (|| {
            let definition_command = self.function_definition_command_for_scope(scope)?;
            let function_kind = self.named_function_kind(scope)?;
            let mut event_keys = Vec::new();
            for function_name in function_kind.static_names() {
                for site in self.semantic.call_sites_for(function_name) {
                    if site.scope == scope {
                        continue;
                    }
                    let call_span = self
                        .command_for_name_word_span(site.span)
                        .map_or(site.span, |command| command.span_in_source(self.source));
                    if !self.definition_command_resolves_at_call(definition_command.id(), call_span)
                    {
                        continue;
                    }
                    match self.enclosing_function_scope_at(call_span.start.offset) {
                        Some(caller_scope) => {
                            for mut event_key in self.s001_function_call_event_keys_after_inner(
                                caller_scope,
                                after_offset,
                                seen_scopes,
                            ) {
                                event_key.push(call_span.start.offset);
                                event_keys.push(event_key);
                            }
                        }
                        None => {
                            if call_span.start.offset > after_offset {
                                event_keys.push(vec![call_span.start.offset]);
                            }
                        }
                    }
                }
            }

            Some(event_keys)
        })()
        .unwrap_or_default();

        seen_scopes.remove(&scope);
        result
    }

    fn s001_first_function_unset_event_after(
        &self,
        target_scope: ScopeId,
        after_offset: usize,
    ) -> Option<S001FunctionEventKey> {
        let target_function = self.named_function_kind(target_scope)?;
        let mut first_key: Option<S001FunctionEventKey> = None;

        for command in self.facts.structural_commands() {
            if !command.options().unset().is_some_and(|unset| {
                target_function
                    .static_names()
                    .iter()
                    .any(|name| unset.targets_function_name(self.source, name.as_str()))
            }) {
                continue;
            }
            if !self.command_runs_in_persistent_shell_context(command.id())
                || self.command_is_in_background_context(command.id())
                || self.command_has_dominance_barrier_ancestor(command.id())
            {
                continue;
            }

            let command_span = command.span_in_source(self.source);
            let event_key = match self.enclosing_function_scope_at(command_span.start.offset) {
                Some(unsetter_scope) => {
                    if unsetter_scope == target_scope {
                        None
                    } else {
                        self.s001_first_function_call_event_after(unsetter_scope, after_offset)
                            .map(|mut event_key| {
                                event_key.push(command_span.start.offset);
                                event_key
                            })
                    }
                }
                None => (command_span.start.offset > after_offset)
                    .then(|| vec![command_span.start.offset]),
            };

            if let Some(event_key) = event_key {
                first_key = Some(match first_key {
                    Some(current) => current.min(event_key),
                    None => event_key,
                });
            }
        }

        first_key
    }

    fn command_has_dominance_barrier_ancestor(&self, command_id: crate::facts::CommandId) -> bool {
        let mut current = self.facts.command_parent_id(command_id);
        while let Some(id) = current {
            if self.facts.command_is_dominance_barrier(id) {
                return true;
            }
            current = self.facts.command_parent_id(id);
        }
        false
    }

    fn command_runs_in_unconditional_flow(
        &self,
        command_id: crate::facts::CommandId,
        reference_at: Span,
    ) -> bool {
        let command = self.facts.command(command_id);
        if self.enclosing_function_scope_at(command.span().start.offset)
            != self.enclosing_function_scope_at(reference_at.start.offset)
        {
            return false;
        }
        if self.command_is_in_background_context(command_id) {
            return false;
        }

        let mut parent_id = self.facts.command_parent_id(command_id);
        while let Some(id) = parent_id {
            if self.facts.command_is_dominance_barrier(id) {
                return false;
            }
            parent_id = self.facts.command_parent_id(id);
        }
        true
    }

    fn command_is_in_background_context(&self, command_id: crate::facts::CommandId) -> bool {
        let mut current = Some(command_id);
        while let Some(id) = current {
            if matches!(
                self.facts.command(id).stmt().terminator,
                Some(StmtTerminator::Background(_))
            ) {
                return true;
            }
            current = self.facts.command_parent_id(id);
        }
        false
    }

    fn enclosing_function_scope_at(&self, offset: usize) -> Option<ScopeId> {
        self.analysis.enclosing_function_scope_at(offset)
    }

    fn binding_is_quoted_static_literal(&self, binding_id: BindingId) -> bool {
        self.facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .is_some_and(|word| {
                word.is_fully_quoted() && static_word_text(word, self.source).is_some()
            })
    }

    fn binding_is_plain_empty_static_literal(&self, binding_id: BindingId) -> bool {
        if self.binding_is_name_only_declaration(binding_id) {
            return true;
        }

        matches!(
            self.semantic.binding(binding_id).origin,
            BindingOrigin::Assignment {
                value: AssignmentValueOrigin::StaticLiteral,
                ..
            }
        ) && self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .and_then(|word| static_word_text(word, self.source))
            .is_some_and(|text| text.is_empty())
    }

    fn binding_is_name_only_declaration(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);
        matches!(binding.origin, BindingOrigin::Declaration { .. })
            && binding.attributes.contains(BindingAttributes::LOCAL)
            && !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
    }

    fn bindings_are_all_plain_empty_static_literals(&self, bindings: &[BindingId]) -> bool {
        !bindings.is_empty()
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_plain_empty_static_literal(binding_id))
    }

    fn optional_field_safe_bindings_can_stay_safe(
        &self,
        bindings: &[BindingId],
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context() {
            return false;
        }

        let mut saw_name_only_declaration = false;
        let mut saw_field_safe_value = false;
        for binding_id in bindings.iter().copied() {
            if self.binding_is_name_only_declaration(binding_id) {
                saw_name_only_declaration = true;
                continue;
            }

            let binding = self.semantic.binding(binding_id);
            if !matches!(
                binding.origin,
                BindingOrigin::Assignment {
                    value: AssignmentValueOrigin::StaticLiteral,
                    ..
                } | BindingOrigin::Declaration { .. }
            ) {
                return false;
            }
            let Some(text) = self
                .facts
                .binding_value(binding_id)
                .and_then(|value| value.scalar_word())
                .and_then(|word| static_word_text(word, self.source))
            else {
                return false;
            };
            if text.is_empty() || !query.literal_is_safe(&text) {
                return false;
            }
            saw_field_safe_value = true;
        }

        saw_name_only_declaration && saw_field_safe_value
    }

    fn covering_direct_field_safe_bindings_can_stay_safe(
        &mut self,
        bindings: &[BindingId],
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context() || bindings.is_empty() {
            return false;
        }

        let mut visiting = FxHashSet::default();
        self.field_safe_binding_group_class(bindings, query, &mut visiting)
            == Some(FieldSafeBindingClass::NonEmpty)
    }

    fn covering_optional_field_safe_bindings_can_stay_safe(
        &mut self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context()
            || bindings.is_empty()
            || bindings.iter().copied().any(|binding_id| {
                self.semantic.binding(binding_id).span.end.offset > at.start.offset
            })
            || !self.bindings_cover_all_paths_to_reference(bindings, name, at)
        {
            return false;
        }

        let mut saw_empty = false;
        let mut saw_non_empty = false;
        let mut visiting = FxHashSet::default();
        for binding_id in bindings.iter().copied() {
            match self.field_safe_binding_class(binding_id, query, &mut visiting) {
                Some(FieldSafeBindingClass::Empty) => saw_empty = true,
                Some(FieldSafeBindingClass::NonEmpty) => saw_non_empty = true,
                None => return false,
            }
        }

        saw_empty && saw_non_empty
    }

    fn local_conditional_literal_bindings_can_stay_safe(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context() || bindings.is_empty() {
            return false;
        }
        let Some(function_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return false;
        };

        let mut saw_conditional_literal = false;
        let mut first_binding_start = at.start.offset;
        for binding_id in bindings.iter().copied() {
            let binding = self.semantic.binding(binding_id);
            if binding.span.end.offset > at.start.offset
                || !self.binding_is_in_scope_or_descendant(binding_id, function_scope)
            {
                return false;
            }
            first_binding_start = first_binding_start.min(binding.span.start.offset);

            let Some(value) = self.facts.binding_value(binding_id) else {
                return false;
            };
            if !value.conditional_assignment_shortcut() {
                return false;
            }
            let Some(text) = value
                .scalar_word()
                .and_then(|word| static_word_text(word, self.source))
            else {
                return false;
            };
            if text.is_empty() || !query.literal_is_safe(&text) {
                return false;
            }
            saw_conditional_literal = true;
        }

        saw_conditional_literal
            && self
                .semantic
                .bindings_for(name)
                .iter()
                .copied()
                .any(|binding_id| {
                    let binding = self.semantic.binding(binding_id);
                    binding.span.end.offset <= first_binding_start
                        && self.semantic.binding_visible_at(binding_id, at)
                        && self.binding_is_in_scope_or_descendant(binding_id, function_scope)
                        && self.binding_is_name_only_declaration(binding_id)
                        && binding.attributes.contains(BindingAttributes::LOCAL)
                })
    }

    fn field_safe_binding_group_class(
        &mut self,
        bindings: &[BindingId],
        query: SafeValueQuery,
        visiting: &mut FxHashSet<BindingId>,
    ) -> Option<FieldSafeBindingClass> {
        let mut saw_non_empty = false;
        for binding_id in bindings.iter().copied() {
            match self.field_safe_binding_class(binding_id, query, visiting)? {
                FieldSafeBindingClass::Empty => {}
                FieldSafeBindingClass::NonEmpty => saw_non_empty = true,
            }
        }

        Some(if saw_non_empty {
            FieldSafeBindingClass::NonEmpty
        } else {
            FieldSafeBindingClass::Empty
        })
    }

    fn field_safe_binding_class(
        &mut self,
        binding_id: BindingId,
        query: SafeValueQuery,
        visiting: &mut FxHashSet<BindingId>,
    ) -> Option<FieldSafeBindingClass> {
        if !visiting.insert(binding_id) {
            return None;
        }

        let result = self.field_safe_binding_class_uncached(binding_id, query, visiting);
        visiting.remove(&binding_id);
        result
    }

    fn field_safe_binding_class_uncached(
        &mut self,
        binding_id: BindingId,
        query: SafeValueQuery,
        visiting: &mut FxHashSet<BindingId>,
    ) -> Option<FieldSafeBindingClass> {
        if self.binding_is_name_only_declaration(binding_id) {
            return Some(FieldSafeBindingClass::Empty);
        }

        let binding = self.semantic.binding(binding_id);
        if !matches!(
            binding.kind,
            BindingKind::Assignment | BindingKind::Declaration(_)
        ) || self.facts.binding_value(binding_id).is_some_and(|value| {
            value.one_sided_short_circuit_assignment() || value.conditional_assignment_shortcut()
        }) {
            return None;
        }

        match binding.origin {
            BindingOrigin::Assignment {
                value: AssignmentValueOrigin::StaticLiteral,
                ..
            }
            | BindingOrigin::Declaration { .. } => {
                self.static_field_safe_binding_class(binding_id, query)
            }
            BindingOrigin::Assignment {
                value: AssignmentValueOrigin::PlainScalarAccess,
                ..
            } => self.plain_scalar_field_safe_binding_class(binding_id, query, visiting),
            BindingOrigin::Assignment { .. }
            | BindingOrigin::LoopVariable { .. }
            | BindingOrigin::ParameterDefaultAssignment { .. }
            | BindingOrigin::Imported { .. }
            | BindingOrigin::FunctionDefinition { .. }
            | BindingOrigin::BuiltinTarget { .. }
            | BindingOrigin::ArithmeticAssignment { .. }
            | BindingOrigin::Nameref { .. } => None,
        }
    }

    fn s001_field_safe_binding_group_exposure(
        &mut self,
        bindings: &[BindingId],
        at: Span,
        query: SafeValueQuery,
    ) -> Option<S001QuoteExposure> {
        if bindings.is_empty() || !query.is_field_context() {
            return None;
        }

        let mut saw_empty = false;
        let mut saw_non_empty = false;
        let mut saw_value = false;
        let mut visiting = FxHashSet::default();
        for binding_id in bindings.iter().copied() {
            match self.field_safe_binding_class(binding_id, query, &mut visiting) {
                Some(FieldSafeBindingClass::Empty) => {
                    saw_empty = true;
                    saw_value = true;
                }
                Some(FieldSafeBindingClass::NonEmpty) => {
                    saw_non_empty = true;
                    saw_value = true;
                }
                None if self
                    .s001_transient_guard_binding_is_overwritten_by_field_safe_bindings(
                        binding_id, bindings, at, query,
                    ) => {}
                None => return None,
            }
        }

        if !saw_value {
            return None;
        }
        Some(if saw_non_empty {
            S001QuoteExposure::QuoteInertNonEmpty
        } else if saw_empty {
            S001QuoteExposure::Empty
        } else {
            return None;
        })
    }

    fn s001_transient_guard_binding_is_overwritten_by_field_safe_bindings(
        &mut self,
        binding_id: BindingId,
        bindings: &[BindingId],
        _at: Span,
        query: SafeValueQuery,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        if !matches!(
            binding.origin,
            BindingOrigin::Assignment { .. } | BindingOrigin::Declaration { .. }
        ) {
            return false;
        }
        if self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .and_then(|word| static_word_text(word, self.source))
            .is_some_and(|text| query.literal_is_safe(&text))
        {
            return false;
        }
        let Some(function_end_span) = self.function_scope_end_span(binding.scope) else {
            return false;
        };

        let mut later_field_safe_bindings = Vec::new();
        let mut visiting = FxHashSet::default();
        for candidate_id in bindings.iter().copied() {
            let candidate = self.semantic.binding(candidate_id);
            if candidate_id == binding_id
                || candidate.scope != binding.scope
                || candidate.name != binding.name
                || candidate.span.start.offset <= binding.span.start.offset
            {
                continue;
            }
            if self
                .field_safe_binding_class(candidate_id, query, &mut visiting)
                .is_some()
            {
                later_field_safe_bindings.push(candidate_id);
            } else {
                return false;
            }
        }
        if later_field_safe_bindings.is_empty() {
            return false;
        }

        self.bindings_cover_all_paths_to_reference(
            &later_field_safe_bindings,
            &binding.name,
            function_end_span,
        ) || (later_field_safe_bindings.len() >= 2
            && self.span_is_inside_if_condition(binding.span))
    }

    fn static_field_safe_binding_class(
        &self,
        binding_id: BindingId,
        query: SafeValueQuery,
    ) -> Option<FieldSafeBindingClass> {
        let text = self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .and_then(|word| static_word_text(word, self.source))?;
        if text.is_empty() {
            Some(FieldSafeBindingClass::Empty)
        } else {
            query
                .literal_is_safe(&text)
                .then_some(FieldSafeBindingClass::NonEmpty)
        }
    }

    fn plain_scalar_field_safe_binding_class(
        &mut self,
        binding_id: BindingId,
        query: SafeValueQuery,
        visiting: &mut FxHashSet<BindingId>,
    ) -> Option<FieldSafeBindingClass> {
        if let Some(class) = self.static_field_safe_binding_class(binding_id, query) {
            return Some(class);
        }

        let word = self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())?;
        let target_name = plain_scalar_reference_name(word)?;
        let binding_span = self.semantic.binding(binding_id).span;
        self.binding_value_stack.push(binding_id);
        let prior_bindings = self.safe_bindings_for_name(&target_name, binding_span);
        self.binding_value_stack.pop();
        if prior_bindings.is_empty()
            || !self.bindings_cover_all_paths_to_reference(
                &prior_bindings,
                &target_name,
                binding_span,
            )
        {
            return None;
        }

        self.field_safe_binding_group_class(&prior_bindings, query, visiting)
    }

    fn binding_is_safe(
        &mut self,
        binding_id: BindingId,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        let binding_key = FactSpan::new(binding.span);
        if self.binding_has_effective_integer_attribute(binding_id) {
            return true;
        }
        if self.binding_value_is_standalone_status_capture(binding_id)
            && self.status_capture_binding_conflicts_with_caller_local(binding_id, at)
        {
            return false;
        }
        if matches!(
            query,
            SafeValueQuery::Argv
                | SafeValueQuery::RedirectTarget
                | SafeValueQuery::NumericTestOperand
        ) && self.binding_is_standalone_status_capture(binding_id, at, case_cli_scope)
        {
            return true;
        }

        let key = (binding_key, FactSpan::new(at), query, case_cli_scope);
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
            }
            | BindingOrigin::Declaration { .. } => {
                if matches!(binding.kind, BindingKind::AppendAssignment) {
                    self.append_assignment_preserves_safe_value(binding_id, query, case_cli_scope)
                } else if self.binding_is_name_only_declaration(binding_id) {
                    true
                } else {
                    let binding_value = self.facts.binding_value(binding_id);
                    let scalar_word = binding_value.and_then(|value| value.scalar_word());
                    let case_cli_status_capture_stays_unsafe = case_cli_scope
                        == Some(binding.scope)
                        && query.is_field_context()
                        && scalar_word.is_some_and(word_is_standalone_status_capture);
                    let conditional_assignment_shortcut_stays_unsafe =
                        binding_value.is_some_and(|value| {
                            value.conditional_assignment_shortcut()
                                && !self.conditional_assignment_shortcut_value_can_stay_safe(
                                    binding_id,
                                    scalar_word,
                                    query,
                                )
                        });
                    if case_cli_status_capture_stays_unsafe
                        || conditional_assignment_shortcut_stays_unsafe
                    {
                        false
                    } else {
                        scalar_word.is_some_and(|word| {
                            self.word_is_safe_for_binding_value(binding_id, word, query)
                        })
                    }
                }
            }
            BindingOrigin::LoopVariable {
                definition_span,
                items: LoopValueOrigin::StaticWords,
            } => {
                let words = self
                    .facts
                    .binding_value(binding_id)
                    .and_then(|value| value.loop_words())
                    .map(|words| words.to_vec());
                words.is_some_and(|words| {
                    !words.is_empty()
                        && (self.loop_variable_reference_stays_within_body(*definition_span, at)
                            || self.loop_variable_reference_stays_within_static_callers(
                                *definition_span,
                                at,
                            ))
                        && words.into_iter().all(|word| {
                            !word_contains_special_parameter_slice(word)
                                && self.word_is_safe(word, query)
                        })
                })
            }
            BindingOrigin::Assignment { .. }
            | BindingOrigin::LoopVariable { .. }
            | BindingOrigin::Imported { .. }
            | BindingOrigin::FunctionDefinition { .. }
            | BindingOrigin::BuiltinTarget { .. }
            | BindingOrigin::ArithmeticAssignment { .. }
            | BindingOrigin::Nameref { .. } => false,
            BindingOrigin::ParameterDefaultAssignment { .. } => self
                .parameter_default_assignment_preserves_safe_value(
                    binding_id,
                    query,
                    case_cli_scope,
                ),
        };

        self.visiting.remove(&key);
        self.memo.insert(key, result);
        result
    }

    fn binding_has_effective_integer_attribute(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }
        if !matches!(
            binding.kind,
            BindingKind::Assignment
                | BindingKind::AppendAssignment
                | BindingKind::Declaration(_)
                | BindingKind::ParameterDefaultAssignment
        ) {
            return false;
        }

        self.semantic
            .previous_visible_binding(&binding.name, binding.span, Some(binding.span))
            .is_some_and(|previous| {
                previous.attributes.contains(BindingAttributes::INTEGER)
                    && !self
                        .semantic
                        .binding_cleared_before(previous.id, binding.span)
            })
    }

    fn conditional_assignment_shortcut_value_can_stay_safe(
        &mut self,
        binding_id: BindingId,
        scalar_word: Option<&Word>,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::NumericTestOperand {
            return false;
        }
        let Some(word) = scalar_word else {
            return false;
        };
        if word_static_text_is_shell_integer(word, self.source) {
            return true;
        }
        word_has_arithmetic_expansion(word)
            && self.word_is_safe_for_binding_value(binding_id, word, query)
    }

    fn append_assignment_preserves_safe_value(
        &mut self,
        binding_id: BindingId,
        query: SafeValueQuery,
        _case_cli_scope: Option<ScopeId>,
    ) -> bool {
        let (name, binding_span) = {
            let binding = self.semantic.binding(binding_id);
            (binding.name.clone(), binding.span)
        };
        let Some(word) = self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
        else {
            return false;
        };
        if !self.word_is_safe_for_binding_value(binding_id, word, query) {
            return false;
        }

        self.binding_value_stack.push(binding_id);
        let prior_value_is_safe = self.name_is_safe(&name, binding_span, query)
            || self.append_prior_bindings_are_empty_safe(&name, binding_span, query);
        self.binding_value_stack.pop();
        prior_value_is_safe
    }

    fn append_prior_bindings_are_empty_safe(
        &self,
        name: &Name,
        binding_span: Span,
        query: SafeValueQuery,
    ) -> bool {
        if !query.is_field_context() {
            return false;
        }

        let mut prior_bindings = self.analysis.reaching_bindings_for_name(name, binding_span);
        self.retain_value_bindings(&mut prior_bindings);
        if let Some(current_binding) = self.current_binding_value_for_name(name) {
            prior_bindings.retain(|binding_id| *binding_id != current_binding);
        }
        if prior_bindings.is_empty()
            && let Some(previous) =
                self.semantic
                    .previous_visible_binding(name, binding_span, Some(binding_span))
            && self.binding_can_supply_parameter_value(previous.id)
        {
            prior_bindings.push(previous.id);
        }
        prior_bindings
            .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        prior_bindings.dedup();

        self.bindings_are_all_plain_empty_static_literals(&prior_bindings)
            && self.bindings_cover_all_paths_to_reference(&prior_bindings, name, binding_span)
    }

    fn status_capture_bindings_cover_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        if !query.is_field_context() {
            return false;
        }

        let mut status_bindings = Vec::new();
        for binding_id in bindings.iter().copied() {
            let binding = self.semantic.binding(binding_id);
            match &binding.origin {
                BindingOrigin::Assignment {
                    value:
                        AssignmentValueOrigin::PlainScalarAccess
                        | AssignmentValueOrigin::StaticLiteral
                        | AssignmentValueOrigin::Unknown,
                    ..
                }
                | BindingOrigin::Declaration { .. }
                    if self.binding_is_standalone_status_capture(
                        binding_id,
                        at,
                        case_cli_scope,
                    ) =>
                {
                    status_bindings.push(binding_id);
                }
                BindingOrigin::Assignment { .. }
                | BindingOrigin::LoopVariable { .. }
                | BindingOrigin::ParameterDefaultAssignment { .. }
                | BindingOrigin::Imported { .. }
                | BindingOrigin::FunctionDefinition { .. }
                | BindingOrigin::BuiltinTarget { .. }
                | BindingOrigin::ArithmeticAssignment { .. }
                | BindingOrigin::Nameref { .. } => return false,
                BindingOrigin::Declaration { .. } => {}
            }
        }

        !status_bindings.is_empty()
            && self.bindings_cover_all_paths_to_reference(&status_bindings, name, at)
    }

    fn status_capture_subset_covers_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        if !query.is_field_context() {
            return false;
        }

        let status_bindings = bindings
            .iter()
            .copied()
            .filter(|binding_id| {
                self.semantic.binding(*binding_id).span.end.offset <= at.start.offset
            })
            .filter(|binding_id| {
                self.binding_is_standalone_status_capture(*binding_id, at, case_cli_scope)
            })
            .collect::<Vec<_>>();
        if status_bindings.is_empty()
            || !self.bindings_cover_all_paths_to_reference(&status_bindings, name, at)
        {
            return false;
        }

        let Some(reference_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return true;
        };
        let first_status_offset = status_bindings
            .iter()
            .map(|binding_id| self.semantic.binding(*binding_id).span.start.offset)
            .min()
            .unwrap_or(at.start.offset);

        !bindings.iter().copied().any(|binding_id| {
            let binding = self.semantic.binding(binding_id);
            binding.scope == reference_scope
                && binding.span.start.offset > first_status_offset
                && binding.span.start.offset < at.start.offset
                && !self.binding_is_standalone_status_capture(binding_id, at, case_cli_scope)
        })
    }

    fn binding_is_standalone_status_capture(
        &self,
        binding_id: BindingId,
        at: Span,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        matches!(
            binding.origin,
            BindingOrigin::Assignment {
                value: AssignmentValueOrigin::PlainScalarAccess
                    | AssignmentValueOrigin::StaticLiteral
                    | AssignmentValueOrigin::Unknown,
                ..
            } | BindingOrigin::Declaration { .. }
        ) && case_cli_scope != Some(binding.scope)
            && self.binding_value_is_standalone_status_capture(binding_id)
            && !self.status_capture_binding_conflicts_with_caller_local(binding_id, at)
    }

    fn binding_is_local_declaration_status_capture(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);

        matches!(binding.kind, BindingKind::Declaration(_))
            && binding.attributes.contains(BindingAttributes::LOCAL)
            && binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
            && self.binding_value_is_standalone_status_capture(binding_id)
    }

    fn local_declaration_status_capture_bindings_cover_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        query.is_field_context()
            && self.span_is_return_argument(at)
            && !bindings.is_empty()
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_local_declaration_status_capture(binding_id))
            && self.bindings_cover_all_paths_to_reference(bindings, name, at)
    }

    fn binding_value_is_standalone_status_capture(&self, binding_id: BindingId) -> bool {
        self.facts
            .binding_value(binding_id)
            .is_some_and(|value| value.standalone_status_or_pid_capture())
    }

    fn status_capture_binding_conflicts_with_caller_local(
        &self,
        binding_id: BindingId,
        at: Span,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        if binding.attributes.contains(BindingAttributes::LOCAL) {
            return false;
        }
        if !self.status_capture_binding_is_in_short_circuit_branch(binding.span) {
            return false;
        }
        let Some(binding_scope) = self.enclosing_function_scope_at(binding.span.start.offset)
        else {
            return false;
        };
        if self.semantic.ancestor_scopes(binding_scope).any(|scope| {
            self.scope_has_visible_initialized_local_binding_before(
                &binding.name,
                scope,
                binding.span,
            )
        }) {
            return false;
        }

        let at_scope = self.semantic.scope_at(at.start.offset);
        let reference_is_in_binding_scope = self
            .semantic
            .scope_is_in_scope_or_descendant(at_scope, binding_scope);
        let relevant_call_sites = self
            .named_function_call_sites(binding_scope)
            .into_iter()
            .filter(|(_, span)| {
                reference_is_in_binding_scope
                    || self.call_site_dominates_use(*span, &binding.name, at)
            })
            .collect::<Vec<_>>();

        !relevant_call_sites.is_empty()
            && relevant_call_sites.into_iter().all(|(scope, span)| {
                let caller_scope = self
                    .enclosing_function_scope_at(span.start.offset)
                    .unwrap_or(scope);
                self.scope_has_visible_initialized_local_binding_before(
                    &binding.name,
                    caller_scope,
                    span,
                )
            })
    }

    fn status_capture_binding_is_in_short_circuit_branch(&self, span: Span) -> bool {
        self.facts.lists().iter().any(|list| {
            list.segments()
                .iter()
                .skip(1)
                .any(|segment| span_contains(segment.span(), span))
        })
    }

    fn status_capture_declaration_probe_covers_reference(
        &self,
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        if !query.is_field_context() {
            return false;
        }

        self.facts.structural_commands().any(|command| {
            command.span().end.offset <= at.start.offset
                && self.command_blocks_cover_all_paths_to_reference(command, name, at)
                && !case_cli_scope.is_some_and(|scope| {
                    self.offset_is_in_scope_or_descendant(command.span().start.offset, scope)
                })
                && command
                    .declaration_assignment_probes()
                    .iter()
                    .any(|probe| probe.status_capture() && probe.target_name() == name.as_str())
        })
    }

    fn offset_is_in_scope_or_descendant(&self, offset: usize, ancestor_scope: ScopeId) -> bool {
        self.semantic
            .ancestor_scopes(self.semantic.scope_at(offset))
            .any(|scope| scope == ancestor_scope)
    }

    fn command_blocks_cover_all_paths_to_reference(
        &self,
        command: crate::facts::CommandFactRef<'_, 'a>,
        name: &Name,
        at: Span,
    ) -> bool {
        let key = (command.id(), name.clone(), FactSpan::new(at));
        if let Some(result) = self.command_cover_memo.borrow().get(&key) {
            return *result;
        }

        let result = self.command_blocks_cover_all_paths_to_reference_uncached(command, name, at);
        self.command_cover_memo.borrow_mut().insert(key, result);
        result
    }

    fn command_blocks_cover_all_paths_to_reference_uncached(
        &self,
        command: crate::facts::CommandFactRef<'_, 'a>,
        name: &Name,
        at: Span,
    ) -> bool {
        if self.enclosing_function_scope_at(command.span().start.offset)
            != self.enclosing_function_scope_at(at.start.offset)
        {
            return false;
        }
        if self.command_is_in_background_context(command.id()) {
            return false;
        }

        let Some(reference_id) = self.reference_id_for_name_at(name, at) else {
            return false;
        };
        let Some(reference_block) = self.block_for_reference(reference_id) else {
            return false;
        };

        let cover_blocks = self.analysis.block_ids_for_span(command.span());
        if cover_blocks.is_empty() {
            return false;
        }
        if cover_blocks.contains(&reference_block) {
            return true;
        }

        let entry = self
            .enclosing_function_scope_at(at.start.offset)
            .and_then(|scope| self.analysis.cfg().scope_entry(scope))
            .unwrap_or_else(|| self.analysis.cfg().entry());
        let cover_blocks = cover_blocks.iter().copied().collect::<FxHashSet<_>>();
        self.analysis
            .blocks_cover_all_paths_to_block(entry, reference_block, &cover_blocks)
    }

    fn unset_command_covers_reference(&self, name: &Name, at: Span) -> bool {
        self.facts.unset_commands_for_name(name).any(|command| {
            command.span().end.offset <= at.start.offset
                && self.command_runs_in_persistent_shell_context(command.id())
                && self.command_runs_in_unconditional_flow(command.id(), at)
                && self.command_blocks_cover_all_paths_to_reference(command, name, at)
        })
    }

    fn binding_is_cleared_by_dominating_unset(
        &self,
        binding_id: BindingId,
        name: &Name,
        at: Span,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        self.facts.unset_commands_for_name(name).any(|command| {
            command.span().start.offset >= binding.span.end.offset
                && command.span().end.offset <= at.start.offset
                && self.command_runs_in_persistent_shell_context(command.id())
                && !self.command_is_in_background_context(command.id())
                && self.command_blocks_cover_all_paths_to_reference(command, name, at)
        })
    }

    fn command_runs_in_persistent_shell_context(
        &self,
        command_id: crate::facts::CommandId,
    ) -> bool {
        let command = self.facts.command(command_id);

        self.semantic
            .ancestor_scopes(command.scope())
            .next()
            .is_none_or(|scope| {
                matches!(
                    self.semantic.scope(scope).kind,
                    ScopeKind::Function(_) | ScopeKind::File
                )
            })
    }

    fn reference_is_safe(&mut self, reference: &VarRef, at: Span, query: SafeValueQuery) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        self.name_is_safe(&reference.name, at, query)
    }

    fn word_is_safe_for_binding_value(
        &mut self,
        binding_id: BindingId,
        word: &Word,
        query: SafeValueQuery,
    ) -> bool {
        self.binding_value_stack.push(binding_id);
        let result = self.word_is_safe(word, query);
        self.binding_value_stack.pop();
        result
    }

    fn parameter_default_assignment_preserves_safe_value(
        &mut self,
        binding_id: BindingId,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        let (name, binding_span) = {
            let binding = self.semantic.binding(binding_id);
            (binding.name.clone(), binding.span)
        };
        let mut prior_bindings = self
            .analysis
            .reaching_bindings_for_name(&name, binding_span);
        self.retain_value_bindings(&mut prior_bindings);
        prior_bindings.retain(|prior_id| *prior_id != binding_id);
        if prior_bindings.is_empty()
            && let Some(previous) =
                self.semantic
                    .previous_visible_binding(&name, binding_span, Some(binding_span))
            && self.binding_can_supply_parameter_value(previous.id)
        {
            prior_bindings.push(previous.id);
        }
        prior_bindings.sort_by_key(|prior_id| self.semantic.binding(*prior_id).span.start.offset);
        prior_bindings.dedup();

        if prior_bindings.is_empty() {
            return safe_numeric_shell_variable(&name)
                && !self.unset_command_covers_reference(&name, binding_span);
        }
        if query.is_field_context() {
            if self.bindings_are_all_plain_empty_static_literals(&prior_bindings) {
                return false;
            }
            if !self.bindings_cover_all_paths_to_reference(&prior_bindings, &name, binding_span) {
                return false;
            }
        }

        prior_bindings
            .into_iter()
            .all(|prior_id| self.binding_is_safe(prior_id, binding_span, query, case_cli_scope))
    }

    fn loop_variable_reference_stays_within_body(&self, definition_span: Span, at: Span) -> bool {
        self.facts.for_headers().iter().any(|header| {
            header
                .command()
                .targets
                .iter()
                .any(|target| target.span == definition_span)
                && span_contains(header.command().body.span, at)
        }) || self.facts.select_headers().iter().any(|header| {
            header.command().variable_span == definition_span
                && span_contains(header.command().body.span, at)
        })
    }

    fn loop_variable_reference_stays_within_static_callers(
        &self,
        definition_span: Span,
        at: Span,
    ) -> bool {
        let Some(helper_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return false;
        };
        self.loop_variable_scope_callers_stay_within_body(
            definition_span,
            helper_scope,
            &mut FxHashSet::default(),
        )
    }

    fn loop_variable_scope_callers_stay_within_body(
        &self,
        definition_span: Span,
        helper_scope: ScopeId,
        seen_scopes: &mut FxHashSet<ScopeId>,
    ) -> bool {
        if !seen_scopes.insert(helper_scope) {
            return false;
        }

        let caller_sites = self.named_function_call_sites(helper_scope);
        !caller_sites.is_empty()
            && caller_sites.into_iter().all(|(_, call_span)| {
                if self.loop_variable_reference_stays_within_body(definition_span, call_span) {
                    return true;
                }

                let caller_scope = self.semantic.scope_at(call_span.start.offset);
                let mut caller_seen = seen_scopes.clone();
                self.loop_variable_scope_callers_stay_within_body(
                    definition_span,
                    caller_scope,
                    &mut caller_seen,
                )
            })
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
        self.reference_is_safe(reference, at, query)
    }

    fn safe_bindings_for_name(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self.visible_bindings_for_name_without_helpers(name, at);
        let mut helper_bindings = self.called_helper_bindings_for_name(name, at);
        self.retain_value_bindings(&mut helper_bindings);
        helper_bindings.extend(self.top_level_transitive_helper_bindings_before(name, at));
        self.retain_value_bindings(&mut helper_bindings);
        helper_bindings
            .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        helper_bindings.dedup();
        let mut caller_bindings = self.caller_bindings_covering_all_static_call_sites(name, at);
        self.retain_value_bindings(&mut caller_bindings);
        let mut uncalled_function_bindings = self.uncalled_function_outer_bindings_at_end(name, at);
        self.retain_value_bindings(&mut uncalled_function_bindings);
        let function_scope = self.enclosing_function_scope_at(at.start.offset);
        let function_local_binding = function_scope.is_some_and(|scope| {
            bindings
                .iter()
                .copied()
                .any(|binding_id| self.binding_is_in_scope_or_descendant(binding_id, scope))
        });
        let caller_bindings_refine_outer_bindings =
            function_scope.is_some() && !function_local_binding && !caller_bindings.is_empty();
        if !uncalled_function_bindings.is_empty() && !function_local_binding {
            bindings = uncalled_function_bindings;
        }
        if caller_bindings_refine_outer_bindings
            && function_scope.is_some()
            && !self.bindings_are_static_loop_variables(&caller_bindings)
        {
            bindings = caller_bindings;
            bindings.extend(helper_bindings);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
        } else if !caller_bindings.is_empty()
            && function_scope.is_some()
            && !function_local_binding
            && self.bindings_are_static_loop_variables(&caller_bindings)
        {
            bindings = caller_bindings;
            bindings.extend(helper_bindings);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
        } else if bindings.is_empty() {
            bindings = caller_bindings;
            bindings.extend(helper_bindings);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
        } else if !helper_bindings.is_empty() {
            bindings.extend(helper_bindings);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
        }
        if let Some(scope) = function_scope
            && bindings.iter().copied().any(|binding_id| {
                self.binding_is_in_scope_or_descendant(binding_id, scope)
                    && self.binding_shadows_outer_scope_values(binding_id)
            })
        {
            bindings
                .retain(|binding_id| self.binding_is_in_scope_or_descendant(*binding_id, scope));
        }
        if let Some(scope) = function_scope {
            bindings.retain(|binding_id| {
                !self.binding_writes_visible_local_in_scope_before(*binding_id, scope, at)
            });
        }
        let reference_scope = self.semantic.scope_at(at.start.offset);
        bindings.retain(|binding_id| {
            self.semantic.binding(*binding_id).span.start.offset <= at.start.offset
                || !self.binding_is_in_scope_or_descendant(*binding_id, reference_scope)
                || self.future_binding_can_reach_reference(*binding_id, name, at)
        });

        self.retain_value_bindings(&mut bindings);
        bindings
    }

    fn bindings_are_static_loop_variables(&self, bindings: &[BindingId]) -> bool {
        !bindings.is_empty()
            && bindings.iter().copied().all(|binding_id| {
                matches!(
                    self.semantic.binding(binding_id).origin,
                    BindingOrigin::LoopVariable {
                        items: LoopValueOrigin::StaticWords,
                        ..
                    }
                )
            })
    }

    fn binding_shadows_outer_scope_values(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);
        binding.attributes.contains(BindingAttributes::LOCAL)
            || matches!(binding.origin, BindingOrigin::LoopVariable { .. })
    }

    fn uncalled_function_outer_bindings_at_end(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        let Some(helper_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return Vec::new();
        };
        if self
            .facts
            .is_case_cli_reachable_function_scope(helper_scope)
            || !self.named_function_call_sites(helper_scope).is_empty()
        {
            return Vec::new();
        }
        let Some(file_scope) = self
            .semantic
            .ancestor_scopes(helper_scope)
            .find(|scope| matches!(self.semantic.scope(*scope).kind, ScopeKind::File))
        else {
            return Vec::new();
        };

        let eof = Position::new().advanced_by(self.source);
        let eof_span = Span::from_positions(eof, eof);
        let mut bindings = self.caller_branch_bindings_before(name, file_scope, eof_span);
        bindings.retain(|binding_id| {
            let binding = self.semantic.binding(*binding_id);
            binding.scope != helper_scope
                && !self.binding_is_in_scope_or_descendant(*binding_id, helper_scope)
        });
        let latest_unguarded = bindings
            .iter()
            .copied()
            .filter(|binding_id| !self.binding_is_guarded_before_reference(*binding_id, eof_span))
            .max_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        let Some(latest_unguarded) = latest_unguarded else {
            return Vec::new();
        };
        let latest_unguarded_offset = self.semantic.binding(latest_unguarded).span.start.offset;
        bindings.retain(|binding_id| {
            *binding_id == latest_unguarded
                || (self.semantic.binding(*binding_id).span.start.offset > latest_unguarded_offset
                    && self.binding_is_guarded_before_reference(*binding_id, eof_span))
        });
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn caller_bindings_covering_all_static_call_sites(
        &mut self,
        name: &Name,
        at: Span,
    ) -> Vec<BindingId> {
        let Some(helper_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return Vec::new();
        };
        self.caller_bindings_covering_static_scope_call_sites(
            name,
            helper_scope,
            &mut FxHashSet::default(),
        )
        .unwrap_or_default()
    }

    fn caller_bindings_covering_static_scope_call_sites(
        &mut self,
        name: &Name,
        helper_scope: ScopeId,
        seen_scopes: &mut FxHashSet<ScopeId>,
    ) -> Option<Vec<BindingId>> {
        if !seen_scopes.insert(helper_scope) {
            return None;
        }

        let caller_sites = self.named_function_call_sites(helper_scope);
        if caller_sites.is_empty() {
            return None;
        }

        let mut bindings = Vec::new();
        for (scope, span) in caller_sites {
            let caller_scope = self
                .enclosing_function_scope_at(span.start.offset)
                .unwrap_or(scope);
            let mut branch = self.caller_branch_bindings_before(name, caller_scope, span);
            self.drop_declarations_shadowed_by_covering_loop_bindings(&mut branch, span);
            if branch
                .iter()
                .copied()
                .any(|binding_id| self.binding_is_in_scope_or_descendant(binding_id, caller_scope))
            {
                branch.retain(|binding_id| {
                    self.binding_is_in_scope_or_descendant(*binding_id, caller_scope)
                });
            }
            let call_span = self
                .command_for_name_word_span(span)
                .map_or(span, |command| command.span());
            let loop_branch = self.loop_bindings_covering_callsite(&branch, call_span);
            if !loop_branch.is_empty() {
                bindings.extend(loop_branch);
                continue;
            }
            if !branch.is_empty() && self.bindings_cover_all_paths_to_callsite(&branch, call_span) {
                bindings.extend(branch);
                continue;
            }

            let mut caller_seen = seen_scopes.clone();
            let transitive = self.caller_bindings_covering_static_scope_call_sites(
                name,
                caller_scope,
                &mut caller_seen,
            )?;
            if transitive.is_empty() {
                return None;
            }
            bindings.extend(transitive);
        }

        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        Some(bindings)
    }

    fn loop_bindings_covering_callsite(
        &self,
        bindings: &[BindingId],
        call_span: Span,
    ) -> Vec<BindingId> {
        bindings
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding = self.semantic.binding(*binding_id);
                let BindingOrigin::LoopVariable {
                    definition_span,
                    items: LoopValueOrigin::StaticWords,
                } = &binding.origin
                else {
                    return false;
                };
                self.loop_variable_reference_stays_within_body(*definition_span, call_span)
            })
            .collect()
    }

    fn visible_bindings_for_name_without_helpers(&self, name: &Name, at: Span) -> Vec<BindingId> {
        let synthetic_use_block = self
            .reference_id_for_name_at(name, at)
            .is_none()
            .then(|| self.block_for_name_reference_or_virtual_offset(name, at))
            .flatten();
        let mut bindings = self
            .value_flow
            .borrow()
            .reaching_value_bindings_for_name_with_synthetic_use_block(
                name,
                at,
                synthetic_use_block,
            );
        if let Some(current_binding) = self.current_binding_value_for_name(name) {
            if bindings.contains(&current_binding) {
                bindings = self.value_flow.borrow().reaching_value_bindings_bypassing(
                    name,
                    current_binding,
                    at,
                );
                if bindings.is_empty()
                    && let Some(previous) = self.semantic.previous_visible_binding(
                        name,
                        self.semantic.binding(current_binding).span,
                        Some(self.semantic.binding(current_binding).span),
                    )
                {
                    bindings.push(previous.id);
                }
            }
            bindings.retain(|binding_id| *binding_id != current_binding);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
            return bindings;
        }
        if bindings.len() == 1 {
            let mut expanded =
                self.value_flow
                    .borrow()
                    .reaching_value_bindings_bypassing(name, bindings[0], at);
            if !expanded.is_empty() {
                expanded.push(bindings[0]);
                expanded
                    .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
                expanded.dedup();
                bindings = expanded;
            }
        }

        bindings
    }

    fn retain_value_bindings(&self, bindings: &mut Vec<BindingId>) {
        bindings.retain(|binding_id| self.binding_can_supply_parameter_value(*binding_id));
    }

    fn binding_can_supply_parameter_value(&self, binding_id: BindingId) -> bool {
        self.value_flow
            .borrow()
            .binding_can_supply_parameter_value(binding_id)
    }

    fn future_binding_can_reach_reference(
        &self,
        binding_id: BindingId,
        name: &Name,
        at: Span,
    ) -> bool {
        let Some(binding_block) = self.block_for_binding(binding_id) else {
            return false;
        };
        let Some(reference_block) = self.block_for_name_reference_or_virtual_offset(name, at)
        else {
            return false;
        };
        if binding_block == reference_block {
            return true;
        }

        let cfg = self.analysis.cfg();
        let mut stack = vec![binding_block];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if self.analysis.block_is_unreachable(block_id) || !seen.insert(block_id) {
                continue;
            }
            for (successor, _) in cfg.successors(block_id) {
                if *successor == reference_block {
                    return true;
                }
                stack.push(*successor);
            }
        }

        false
    }

    fn current_binding_value_for_name(&self, name: &Name) -> Option<BindingId> {
        self.binding_value_stack
            .iter()
            .rev()
            .copied()
            .find(|binding_id| &self.semantic.binding(*binding_id).name == name)
    }

    fn drop_declarations_shadowed_by_covering_loop_bindings(
        &self,
        bindings: &mut Vec<BindingId>,
        at: Span,
    ) {
        let covering_loop_bindings = bindings
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                let BindingOrigin::LoopVariable {
                    definition_span,
                    items: LoopValueOrigin::StaticWords,
                } = &binding.origin
                else {
                    return None;
                };
                (self.loop_variable_reference_stays_within_body(*definition_span, at)
                    || self
                        .loop_variable_reference_stays_within_static_callers(*definition_span, at))
                .then_some((binding.scope, definition_span.start.offset))
            })
            .collect::<FxHashSet<_>>();
        if covering_loop_bindings.is_empty() {
            return;
        }

        bindings.retain(|binding_id| {
            let binding = self.semantic.binding(*binding_id);
            if !matches!(binding.origin, BindingOrigin::Declaration { .. }) {
                return true;
            }

            !covering_loop_bindings.iter().any(|(scope, loop_start)| {
                binding.scope == *scope && binding.span.start.offset <= *loop_start
            })
        });
    }

    fn drop_outer_bindings_shadowed_by_covering_loop_bindings(
        &self,
        bindings: &mut Vec<BindingId>,
        at: Span,
    ) {
        let covering_loop_scopes = bindings
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                let BindingOrigin::LoopVariable {
                    definition_span,
                    items: LoopValueOrigin::StaticWords,
                } = &binding.origin
                else {
                    return None;
                };
                (self.loop_variable_reference_stays_within_body(*definition_span, at)
                    || self
                        .loop_variable_reference_stays_within_static_callers(*definition_span, at))
                .then_some((binding_id, binding.scope))
            })
            .collect::<Vec<_>>();
        if covering_loop_scopes.is_empty() {
            return;
        }

        bindings.retain(|binding_id| {
            if covering_loop_scopes
                .iter()
                .any(|(covering_id, _)| covering_id == binding_id)
            {
                return true;
            }

            let binding = self.semantic.binding(*binding_id);
            if matches!(
                binding.origin,
                BindingOrigin::LoopVariable {
                    items: LoopValueOrigin::StaticWords,
                    ..
                }
            ) {
                return false;
            }
            !covering_loop_scopes.iter().any(|(_, loop_scope)| {
                self.semantic
                    .scope_is_descendant_of(*loop_scope, binding.scope)
            })
        });
    }

    fn called_helper_bindings_for_name(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        self.value_flow
            .borrow_mut()
            .helper_value_bindings_before(name, at)
    }

    fn s001_top_level_dispatch_helper_bindings_before(
        &mut self,
        name: &Name,
        at: Span,
    ) -> Vec<BindingId> {
        if self.enclosing_function_scope_at(at.start.offset).is_some() {
            return Vec::new();
        }

        let mut bindings = Vec::new();
        let scope = self.semantic.scope_at(at.start.offset);
        let dispatcher_scopes = self
            .value_flow
            .borrow()
            .called_function_scopes_before(scope, at.start.offset);
        for dispatcher_scope in dispatcher_scopes {
            bindings.extend(self.s001_branch_helper_bindings_in_scope(name, dispatcher_scope));
        }
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn s001_branch_helper_bindings_in_scope(
        &mut self,
        name: &Name,
        scope: ScopeId,
    ) -> Vec<BindingId> {
        let helper_scopes = self.helper_scopes_providing_name(name);
        if helper_scopes.is_empty() {
            return Vec::new();
        }

        let helper_scope_set = helper_scopes.iter().copied().collect::<FxHashSet<_>>();
        let mut call_sites = Vec::new();
        for callee_scope in helper_scopes {
            let Some(function_kind) = self.named_function_kind(callee_scope) else {
                continue;
            };
            for function_name in function_kind.static_names() {
                for site in self.semantic.call_sites_for(function_name) {
                    if site.scope == scope {
                        call_sites.push((callee_scope, site.span));
                    }
                }
            }
        }
        call_sites.sort_by_key(|(_, span)| (span.start.offset, span.end.offset));
        call_sites.dedup_by_key(|(callee_scope, span)| {
            (*callee_scope, span.start.offset, span.end.offset)
        });
        if call_sites.len() < 2 {
            return Vec::new();
        }

        let mut bindings = Vec::new();
        for (callee_scope, _) in call_sites {
            bindings.extend(self.semantic.bindings_for(name).iter().copied().filter(
                |binding_id| {
                    let binding = self.semantic.binding(*binding_id);
                    binding.scope == callee_scope
                        && helper_scope_set.contains(&binding.scope)
                        && !binding.attributes.contains(BindingAttributes::LOCAL)
                },
            ));
        }
        bindings
    }

    fn top_level_transitive_helper_bindings_before(&self, name: &Name, at: Span) -> Vec<BindingId> {
        if self.enclosing_function_scope_at(at.start.offset).is_some() {
            return Vec::new();
        }

        self.transitive_helper_bindings_before(name, at, !self.span_is_exit_or_return_argument(at))
    }

    fn called_helper_bindings_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        self.value_flow
            .borrow_mut()
            .nonlocal_value_bindings_from_called_functions_before(name, scope, at)
    }

    fn helper_outer_bindings_cover_all_caller_paths(
        &mut self,
        name: &Name,
        at: Span,
        bindings: &[BindingId],
    ) -> bool {
        let Some(helper_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return true;
        };
        if !bindings
            .iter()
            .copied()
            .any(|binding_id| !self.binding_is_in_scope_or_descendant(binding_id, helper_scope))
        {
            return true;
        }

        let caller_sites = self.named_function_call_sites(helper_scope);
        if caller_sites.is_empty() {
            return true;
        }

        caller_sites.into_iter().all(|(scope, span)| {
            let caller_scope = self
                .enclosing_function_scope_at(span.start.offset)
                .unwrap_or(scope);
            let mut branch = self.caller_branch_bindings_before(name, caller_scope, span);
            self.drop_declarations_shadowed_by_covering_loop_bindings(&mut branch, span);
            if branch
                .iter()
                .copied()
                .any(|binding_id| self.binding_is_in_scope_or_descendant(binding_id, caller_scope))
            {
                branch.retain(|binding_id| {
                    self.binding_is_in_scope_or_descendant(*binding_id, caller_scope)
                });
            }
            if branch.is_empty() {
                return false;
            }

            let helper_branch = self
                .called_helper_bindings_before(name, scope, span)
                .into_iter()
                .collect::<FxHashSet<_>>();
            let direct_branch = branch
                .into_iter()
                .filter(|binding_id| !helper_branch.contains(binding_id))
                .collect::<Vec<_>>();
            let call_span = self
                .command_for_name_word_span(span)
                .map_or(span, |command| command.span());

            direct_branch.is_empty()
                || !self
                    .loop_bindings_covering_callsite(&direct_branch, call_span)
                    .is_empty()
                || self.bindings_cover_all_paths_to_callsite(&direct_branch, call_span)
        })
    }

    fn named_function_call_sites(&self, scope: ScopeId) -> Vec<(ScopeId, Span)> {
        self.value_flow
            .borrow()
            .named_function_call_sites(scope)
            .into_iter()
            .map(|site| (site.scope, site.span))
            .collect()
    }

    fn caller_branch_bindings_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut branch = self.visible_bindings_for_name_without_helpers(name, at);
        branch.extend(
            self.value_flow
                .borrow()
                .ancestor_value_bindings_before(name, scope, at),
        );
        branch.extend(self.called_helper_bindings_before(name, scope, at));
        if matches!(self.semantic.scope(scope).kind, ScopeKind::File)
            && self.callsite_is_within_guarded_branch(at)
        {
            branch.extend(self.top_level_transitive_helper_bindings_before(name, at));
            self.drop_outer_bindings_shadowed_by_covering_top_level_helper_bindings(
                &mut branch,
                scope,
                at,
            );
        }
        if self.scope_has_visible_local_binding_before(name, scope, at) {
            branch.retain(|binding_id| self.semantic.binding(*binding_id).scope == scope);
        }
        branch.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        branch.dedup();
        branch
    }

    fn callsite_is_within_guarded_branch(&self, at: Span) -> bool {
        let call_span = self
            .command_for_name_word_span(at)
            .map_or(at, |command| command.span());
        let Some(mut command_id) = self.facts.innermost_command_id_at(call_span.start.offset)
        else {
            return false;
        };
        while self.facts.command(command_id).span() != call_span {
            let Some(parent_id) = self.facts.command_parent_id(command_id) else {
                return false;
            };
            command_id = parent_id;
        }

        let mut current = self.facts.command_parent_id(command_id);
        while let Some(command_id) = current {
            let command = self.facts.command(command_id);
            if command.span().start.offset < call_span.start.offset
                && call_span.end.offset <= command.span().end.offset
                && self.facts.command_is_dominance_barrier(command_id)
            {
                return true;
            }
            current = self.facts.command_parent_id(command_id);
        }

        false
    }

    fn drop_outer_bindings_shadowed_by_covering_top_level_helper_bindings(
        &self,
        bindings: &mut Vec<BindingId>,
        scope: ScopeId,
        at: Span,
    ) {
        let helper_bindings = bindings
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding_scope = self.semantic.binding(*binding_id).scope;
                self.semantic.scope_is_descendant_of(binding_scope, scope)
            })
            .collect::<Vec<_>>();
        if helper_bindings.is_empty() {
            return;
        }

        let call_span = self
            .command_for_name_word_span(at)
            .map_or(at, |command| command.span());
        if !self.bindings_cover_all_paths_to_callsite(&helper_bindings, call_span) {
            return;
        }

        bindings.retain(|binding_id| {
            let binding_scope = self.semantic.binding(*binding_id).scope;
            binding_scope != scope || helper_bindings.contains(binding_id)
        });
    }

    fn scope_has_visible_local_binding_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> bool {
        self.semantic
            .bindings_for(name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                binding.scope == scope
                    && binding.span.end.offset <= at.start.offset
                    && binding.attributes.contains(BindingAttributes::LOCAL)
            })
    }

    fn scope_has_visible_initialized_local_binding_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> bool {
        let latest_local_start = self
            .semantic
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding = self.semantic.binding(*binding_id);
                binding.scope == scope
                    && binding.span.end.offset <= at.start.offset
                    && binding.attributes.contains(BindingAttributes::LOCAL)
            })
            .map(|binding_id| self.semantic.binding(binding_id).span.start.offset)
            .max();
        let Some(latest_local_start) = latest_local_start else {
            return false;
        };

        self.semantic
            .bindings_for(name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                let initializes_local_value = match binding.origin {
                    BindingOrigin::Declaration { .. } => binding.attributes.intersects(
                        BindingAttributes::DECLARATION_INITIALIZED | BindingAttributes::INTEGER,
                    ),
                    BindingOrigin::Assignment { .. }
                    | BindingOrigin::LoopVariable { .. }
                    | BindingOrigin::ParameterDefaultAssignment { .. }
                    | BindingOrigin::Imported { .. }
                    | BindingOrigin::FunctionDefinition { .. }
                    | BindingOrigin::BuiltinTarget { .. }
                    | BindingOrigin::ArithmeticAssignment { .. }
                    | BindingOrigin::Nameref { .. } => {
                        self.binding_can_supply_parameter_value(binding_id)
                    }
                };
                binding.scope == scope
                    && binding.span.end.offset <= at.start.offset
                    && binding.span.start.offset >= latest_local_start
                    && initializes_local_value
            })
    }

    fn binding_writes_visible_local_in_scope_before(
        &self,
        binding_id: BindingId,
        scope: ScopeId,
        at: Span,
    ) -> bool {
        let binding = self.semantic.binding(binding_id);
        if binding.scope == scope
            || binding.attributes.contains(BindingAttributes::LOCAL)
            || !self.scope_has_visible_local_binding_before(&binding.name, scope, at)
            || self.scope_has_visible_local_binding_before(
                &binding.name,
                binding.scope,
                binding.span,
            )
        {
            return false;
        }
        let Some(definition_command) = self.function_definition_command_for_scope(binding.scope)
        else {
            return false;
        };

        self.value_flow
            .borrow()
            .named_function_call_sites(binding.scope)
            .into_iter()
            .any(|site| {
                site.scope == scope
                    && self.function_scope_resolves_at_call_site(binding.scope, &site)
                    && self.scope_has_visible_local_binding_before(&binding.name, scope, site.span)
                    && self.definition_command_resolves_at_call(definition_command.id(), site.span)
                    && self.call_site_dominates_use(site.span, &binding.name, at)
            })
    }

    fn call_site_dominates_use(&self, call_span: Span, name: &Name, at: Span) -> bool {
        let _ = name;
        self.call_site_dominates_offset(call_span, at.start.offset)
    }

    fn function_scope_end_span(&self, scope: ScopeId) -> Option<Span> {
        self.facts
            .function_headers()
            .iter()
            .find(|header| header.function_scope() == Some(scope))
            .map(|header| Span::at(header.function().span.end))
    }

    fn transitive_helper_bindings_before(
        &self,
        name: &Name,
        at: Span,
        relaxed: bool,
    ) -> Vec<BindingId> {
        let scope = self.semantic.scope_at(at.start.offset);
        let callee_scopes = if relaxed {
            self.value_flow
                .borrow()
                .transitively_called_function_scopes_before_relaxed(scope, at.start.offset)
        } else {
            self.value_flow
                .borrow()
                .transitively_called_function_scopes_before(scope, at.start.offset)
        };
        let mut bindings = callee_scopes
            .into_iter()
            .flat_map(|callee_scope| {
                self.semantic
                    .bindings_for(name)
                    .iter()
                    .copied()
                    .filter(move |binding_id| {
                        let binding = self.semantic.binding(*binding_id);
                        binding.scope == callee_scope
                            && !binding.attributes.contains(BindingAttributes::LOCAL)
                    })
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn call_site_dominates_offset(&self, call_span: Span, limit_offset: usize) -> bool {
        if call_span.start.offset >= limit_offset {
            return false;
        }

        let Some(mut command_id) = self.facts.innermost_command_id_at(call_span.start.offset)
        else {
            return true;
        };
        while self.facts.command(command_id).span() != call_span {
            let Some(parent_id) = self.facts.command_parent_id(command_id) else {
                return true;
            };
            command_id = parent_id;
        }

        let mut current = self.facts.command_parent_id(command_id);
        while let Some(command_id) = current {
            let command = self.facts.command(command_id);
            if command.span().end.offset > limit_offset {
                break;
            }
            if command.span().start.offset < call_span.start.offset
                && self.facts.command_is_dominance_barrier(command_id)
            {
                return false;
            }
            current = self.facts.command_parent_id(command_id);
        }

        true
    }

    fn binding_is_guarded_before_reference(&self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.semantic.binding(binding_id);
        let Some(mut current) = self
            .facts
            .innermost_command_id_at(binding.span.start.offset)
            .and_then(|id| self.facts.command_parent_id(id))
        else {
            return false;
        };

        loop {
            let command = self.facts.command(current);
            if self.facts.command_is_dominance_barrier(current)
                && command.span().end.offset <= at.start.offset
            {
                return true;
            }

            let Some(parent_id) = self.facts.command_parent_id(current) else {
                return false;
            };
            current = parent_id;
        }
    }

    fn binding_is_one_sided_short_circuit_assignment(&self, binding_id: BindingId) -> bool {
        self.facts
            .binding_value(binding_id)
            .is_some_and(|value| value.one_sided_short_circuit_assignment())
    }

    fn binding_is_one_sided_append_assignment(&self, binding_id: BindingId) -> bool {
        matches!(
            self.semantic.binding(binding_id).kind,
            BindingKind::AppendAssignment
        ) && self.binding_is_one_sided_short_circuit_assignment(binding_id)
    }

    fn one_sided_bindings_preserve_safe_base(
        &mut self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        let mut base_bindings = Vec::new();
        let mut saw_one_sided_binding = false;
        for binding_id in bindings.iter().copied() {
            if self.binding_is_one_sided_short_circuit_assignment(binding_id) {
                saw_one_sided_binding = true;
            } else {
                base_bindings.push(binding_id);
            }
        }

        saw_one_sided_binding
            && !base_bindings.is_empty()
            && self.bindings_cover_all_paths_to_reference(&base_bindings, name, at)
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_safe(binding_id, at, query, case_cli_scope))
    }

    fn helper_scopes_providing_name(&self, name: &Name) -> Vec<ScopeId> {
        self.semantic
            .bindings_for(name)
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                (!binding.attributes.contains(BindingAttributes::LOCAL)
                    && matches!(
                        self.semantic.scope(binding.scope).kind,
                        ScopeKind::Function(_)
                    ))
                .then_some(binding.scope)
            })
            .collect::<FxHashSet<_>>()
            .into_iter()
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
        self.analysis.binding_dominates_reference_from_flow_entry(
            binding_id,
            name,
            at,
            !self.binding_is_guarded_before_reference(binding_id, at),
        )
    }

    fn bindings_cover_all_paths_to_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
    ) -> bool {
        self.value_source_blocks_cover_all_paths_to_reference(
            bindings,
            name,
            at,
            FxHashSet::default(),
        )
    }

    fn value_sources_cover_all_paths_to_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
    ) -> bool {
        let unset_blocks = if bindings.is_empty() {
            Default::default()
        } else {
            self.unset_value_blocks_for_name_before_reference(name, at)
        };
        self.value_source_blocks_cover_all_paths_to_reference(bindings, name, at, unset_blocks)
    }

    fn value_source_blocks_cover_all_paths_to_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        mut cover_blocks: FxHashSet<BlockId>,
    ) -> bool {
        let Some(reference_block) = self.block_for_name_reference_or_virtual_offset(name, at)
        else {
            return true;
        };

        cover_blocks.extend(bindings.iter().copied().filter_map(|binding_id| {
            let binding_block = self.block_for_binding(binding_id)?;
            if (binding_block == reference_block
                && self.binding_is_guarded_before_reference(binding_id, at))
                || self.binding_is_one_sided_short_circuit_assignment(binding_id)
            {
                None
            } else {
                Some(binding_block)
            }
        }));
        if cover_blocks.contains(&reference_block) {
            return true;
        }

        let binding_scopes = bindings
            .iter()
            .copied()
            .map(|binding_id| self.semantic.binding(binding_id).scope)
            .collect::<Vec<_>>();
        let entry = self
            .analysis
            .flow_entry_block_for_binding_scopes(&binding_scopes, at.start.offset);
        self.analysis
            .blocks_cover_all_paths_to_block(entry, reference_block, &cover_blocks)
    }

    fn unset_value_blocks_for_name_before_reference(
        &self,
        name: &Name,
        at: Span,
    ) -> FxHashSet<BlockId> {
        self.facts
            .unset_commands_for_name(name)
            .filter(|command| {
                command.span().end.offset <= at.start.offset
                    && self.enclosing_function_scope_at(command.span().start.offset)
                        == self.enclosing_function_scope_at(at.start.offset)
                    && self.command_runs_in_persistent_shell_context(command.id())
                    && !self.command_is_in_background_context(command.id())
                    && !self.command_is_in_boolean_list(command.id())
            })
            .flat_map(|command| {
                self.analysis
                    .block_ids_for_span(command.span())
                    .iter()
                    .copied()
            })
            .collect()
    }

    fn command_is_in_boolean_list(&self, command_id: crate::facts::CommandId) -> bool {
        let mut current = self.facts.command_parent_id(command_id);
        while let Some(id) = current {
            if let Command::Binary(binary) = self.facts.command(id).command()
                && matches!(binary.op, BinaryOp::And | BinaryOp::Or)
            {
                return true;
            }
            current = self.facts.command_parent_id(id);
        }
        false
    }

    fn bindings_cover_all_paths_to_callsite(
        &self,
        bindings: &[BindingId],
        call_span: Span,
    ) -> bool {
        let call_blocks = self
            .analysis
            .cfg()
            .blocks()
            .iter()
            .filter(|block| block.commands.contains(&call_span))
            .map(|block| block.id)
            .collect::<FxHashSet<_>>();
        let cover_bindings = bindings
            .iter()
            .copied()
            .filter(|binding_id| {
                let Some(binding_block) = self.block_for_binding(*binding_id) else {
                    return false;
                };
                !((call_blocks.contains(&binding_block)
                    && self.binding_is_guarded_before_reference(*binding_id, call_span))
                    || self.binding_is_one_sided_short_circuit_assignment(*binding_id))
            })
            .collect::<Vec<_>>();
        self.value_flow
            .borrow()
            .value_bindings_cover_all_paths_to_span(&cover_bindings, call_span)
    }

    fn reference_id_for_name_at(&self, name: &Name, at: Span) -> Option<ReferenceId> {
        self.analysis.reference_id_for_name_at(name, at)
    }

    fn block_for_name_reference_or_virtual_offset(&self, name: &Name, at: Span) -> Option<BlockId> {
        if let Some(reference_id) = self.reference_id_for_name_at(name, at) {
            return self.block_for_reference(reference_id);
        }

        let command_id = self
            .facts
            .innermost_command_id_at(at.start.offset)
            .or_else(|| {
                self.facts
                    .innermost_command_id_containing_offset(at.start.offset)
            })?;
        self.analysis
            .block_ids_for_span(self.facts.command(command_id).span())
            .first()
            .copied()
    }

    fn block_for_binding(&self, binding_id: BindingId) -> Option<BlockId> {
        self.analysis.block_for_binding(binding_id)
    }

    fn block_for_reference(&self, reference_id: ReferenceId) -> Option<BlockId> {
        self.analysis.block_for_reference_id(reference_id)
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
                    operand_word_ast,
                    ..
                } => {
                    self.indirect_name_is_safe(reference, at, query)
                        && operator.as_ref().is_none_or(|operator| {
                            self.parameter_operator_is_safe(
                                &reference.name,
                                operator,
                                operand_word_ast.as_ref(),
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
                            || self.slice_reference_static_literals_stay_safe(
                                reference, syntax, at, query,
                            )
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand_word_ast,
                    ..
                } => self.parameter_expansion_is_safe(
                    reference,
                    operator,
                    operand_word_ast.as_ref(),
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

    fn parameter_part_s001_quote_exposure(
        &mut self,
        parameter: &ParameterExpansion,
        at: Span,
        query: SafeValueQuery,
    ) -> S001QuoteExposure {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
                if !reference.has_array_selector() =>
            {
                self.name_s001_quote_exposure(&reference.name, at, query)
            }
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                reference, ..
            }) if !reference.has_array_selector() => {
                self.name_s001_quote_exposure(&reference.name, at, query)
            }
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { .. }) => {
                S001QuoteExposure::QuoteInertNonEmpty
            }
            ParameterExpansionSyntax::Bourne(_) | ParameterExpansionSyntax::Zsh(_) => {
                if self.parameter_part_is_safe(parameter, at, query) {
                    S001QuoteExposure::QuoteInertNonEmpty
                } else {
                    S001QuoteExposure::Unsafe
                }
            }
        }
    }

    fn parameter_expansion_is_safe(
        &mut self,
        reference: &VarRef,
        operator: &ParameterOp,
        operand_word: Option<&Word>,
        _at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }

        self.parameter_operator_is_safe(
            &reference.name,
            operator,
            operand_word,
            reference.name_span,
            query,
        )
    }

    fn parameter_operator_is_safe(
        &mut self,
        name: &Name,
        operator: &ParameterOp,
        operand_word: Option<&Word>,
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
            ParameterOp::UseDefault | ParameterOp::AssignDefault => {
                self.name_is_safe(name, at, query)
                    || (query == SafeValueQuery::NumericTestOperand
                        && operand_word
                            .is_some_and(|word| self.word_is_safe_numeric_operand(word, at))
                        && self.name_has_numeric_loop_body_assignment(name, at))
            }
            ParameterOp::Error => self.name_is_safe(name, at, query),
            ParameterOp::UseReplacement => {
                operand_word.is_some_and(|word| self.word_is_static_safe_literal(word, query))
            }
            ParameterOp::ReplaceFirst {
                replacement_word_ast,
                ..
            }
            | ParameterOp::ReplaceAll {
                replacement_word_ast,
                ..
            } => {
                self.name_is_safe(name, at, query)
                    && self.word_is_static_safe_literal(replacement_word_ast, query)
            }
        }
    }

    fn slice_reference_static_literals_stay_safe(
        &mut self,
        reference: &VarRef,
        slice: &BourneParameterExpansion,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        let BourneParameterExpansion::Slice {
            offset_word_ast,
            length_word_ast,
            ..
        } = slice
        else {
            return false;
        };
        let Some(offset_text) = static_word_text(offset_word_ast, self.source) else {
            return false;
        };
        let Ok(offset) = offset_text.parse::<isize>() else {
            return false;
        };
        let length = match length_word_ast {
            Some(word) => {
                let Some(length_text) = static_word_text(word, self.source) else {
                    return false;
                };
                let Ok(length) = length_text.parse::<usize>() else {
                    return false;
                };
                Some(length)
            }
            None => None,
        };

        let mut bindings = self.safe_bindings_for_name(&reference.name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.retain_value_bindings(&mut bindings);
        bindings.retain(|binding_id| {
            !self.binding_is_cleared_by_dominating_unset(*binding_id, &reference.name, at)
        });
        if query.is_field_context() {
            bindings.retain(|binding_id| {
                !self.binding_is_blocked_by_exit_like_function_call(*binding_id, at)
            });
        }
        if bindings.is_empty() {
            return false;
        }

        bindings.into_iter().all(|binding_id| {
            let Some(word) = self
                .facts
                .binding_value(binding_id)
                .and_then(|value| value.scalar_word())
            else {
                return false;
            };
            let Some(text) = static_word_text(word, self.source) else {
                return false;
            };
            static_slice_result_is_safe(&text, offset, length, query)
        })
    }

    fn word_is_static_safe_literal(&self, word: &Word, query: SafeValueQuery) -> bool {
        let Some(text) = static_word_text(word, self.source) else {
            return false;
        };

        if word.is_fully_quoted() {
            match query {
                SafeValueQuery::Argv
                | SafeValueQuery::RedirectTarget
                | SafeValueQuery::NumericTestOperand
                | SafeValueQuery::Quoted => true,
                SafeValueQuery::Pattern | SafeValueQuery::Regex => query.literal_is_safe(&text),
            }
        } else {
            query.literal_is_safe(&text)
        }
    }

    fn word_is_safe_numeric_operand(&mut self, word: &Word, at: Span) -> bool {
        if word_static_text_is_shell_integer(word, self.source) {
            return true;
        }

        plain_scalar_reference_name(word)
            .is_some_and(|name| self.name_is_safe(&name, at, SafeValueQuery::NumericTestOperand))
    }

    fn name_has_numeric_loop_body_assignment(&mut self, name: &Name, at: Span) -> bool {
        let Some(body_span) = self.enclosing_while_body_for_condition_span(at) else {
            return false;
        };
        let bindings = self.semantic.bindings_for(name).to_vec();

        bindings.into_iter().any(|binding_id| {
            let binding = self.semantic.binding(binding_id);
            span_contains(body_span, binding.span)
                && self.binding_assigns_numeric_operand_value(binding_id, at)
        })
    }

    fn s001_function_reference_has_file_scope_integer_bindings(
        &mut self,
        name: &Name,
        at: Span,
    ) -> bool {
        let Some(function_scope) = self.enclosing_function_scope_at(at.start.offset) else {
            return false;
        };
        if self
            .semantic
            .bindings_for(name)
            .iter()
            .copied()
            .any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                binding.scope == function_scope
                    && self.binding_can_supply_parameter_value(binding_id)
            })
        {
            return false;
        }

        let Some(file_scope) = self
            .semantic
            .ancestor_scopes(self.semantic.scope_at(at.start.offset))
            .find(|scope| matches!(self.semantic.scope(*scope).kind, ScopeKind::File))
        else {
            return false;
        };

        let bindings = self
            .semantic
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding = self.semantic.binding(*binding_id);
                binding.scope == file_scope && self.binding_can_supply_parameter_value(*binding_id)
            })
            .collect::<Vec<_>>();

        if bindings.is_empty()
            || !bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_assigns_numeric_operand_value(binding_id, at))
        {
            return false;
        }

        let call_sites = self.named_function_call_sites(function_scope);
        call_sites.is_empty()
            || call_sites.into_iter().all(|(_, call_span)| {
                self.bindings_cover_all_paths_to_callsite(&bindings, call_span)
            })
    }

    fn s001_name_has_only_numeric_value_bindings(&mut self, name: &Name, at: Span) -> bool {
        let mut bindings = self.safe_bindings_for_name(name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.retain_value_bindings(&mut bindings);
        let mut transitive_helper_bindings = self.transitive_helper_bindings_before(
            name,
            at,
            !self.span_is_exit_or_return_argument(at),
        );
        self.retain_value_bindings(&mut transitive_helper_bindings);
        bindings.extend(transitive_helper_bindings.iter().copied());
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        if bindings.is_empty() {
            return false;
        }
        if bindings.iter().copied().any(|binding_id| {
            (self.binding_is_one_sided_short_circuit_assignment(binding_id)
                && !self.binding_assigns_static_integer_literal(binding_id))
                || self.facts.binding_value(binding_id).is_some_and(|value| {
                    value.conditional_assignment_shortcut()
                        && !self.binding_assigns_static_integer_literal(binding_id)
                        || value.scalar_word().is_some_and(|word| {
                            static_word_text(word, self.source).is_some_and(|text| text.is_empty())
                        })
                })
        }) {
            return false;
        }

        let has_covering_binding = self.bindings_cover_all_paths_to_reference(&bindings, name, at)
            || bindings.iter().copied().any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                binding.span.end.offset <= at.start.offset
                    && self.binding_dominates_reference(binding_id, name, at)
            });
        has_covering_binding
            && bindings.into_iter().all(|binding_id| {
                self.binding_assigns_s001_standalone_numeric_argv_value(binding_id)
            })
    }

    fn binding_assigns_s001_standalone_numeric_argv_value(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);
        if self.binding_has_effective_integer_attribute(binding_id)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let Some(word) = self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
        else {
            return false;
        };

        word_static_text_is_shell_integer(word, self.source) || word_is_arithmetic_expansion(word)
    }

    fn binding_assigns_static_integer_literal(&self, binding_id: BindingId) -> bool {
        self.facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .is_some_and(|word| word_static_text_is_shell_integer(word, self.source))
    }

    fn s001_name_has_only_arithmetic_numeric_bindings(&mut self, name: &Name, at: Span) -> bool {
        let mut bindings = self.safe_bindings_for_name(name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.drop_outer_bindings_shadowed_by_covering_loop_bindings(&mut bindings, at);
        self.retain_value_bindings(&mut bindings);
        if bindings.is_empty()
            || bindings.iter().copied().any(|binding_id| {
                self.binding_is_one_sided_short_circuit_assignment(binding_id)
                    || self.facts.binding_value(binding_id).is_some_and(|value| {
                        value.conditional_assignment_shortcut()
                            || value.scalar_word().is_some_and(|word| {
                                static_word_text(word, self.source)
                                    .is_some_and(|text| text.is_empty())
                            })
                    })
            })
        {
            return false;
        }

        let has_covering_binding = self.bindings_cover_all_paths_to_reference(&bindings, name, at)
            || bindings.iter().copied().any(|binding_id| {
                let binding = self.semantic.binding(binding_id);
                binding.span.end.offset <= at.start.offset
                    && self.binding_dominates_reference(binding_id, name, at)
            });
        has_covering_binding
            && bindings
                .into_iter()
                .all(|binding_id| self.binding_assigns_s001_arithmetic_numeric_value(binding_id))
    }

    fn binding_assigns_s001_arithmetic_numeric_value(&self, binding_id: BindingId) -> bool {
        let binding = self.semantic.binding(binding_id);
        if self.binding_has_effective_integer_attribute(binding_id)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        self.facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
            .is_some_and(word_is_arithmetic_expansion)
    }

    fn enclosing_while_body_for_condition_span(&self, at: Span) -> Option<Span> {
        self.facts
            .commands()
            .iter()
            .filter_map(|command| match command.command() {
                Command::Compound(CompoundCommand::While(command))
                    if span_contains(command.condition.span, at) =>
                {
                    Some(command.body.span)
                }
                Command::Simple(_)
                | Command::Builtin(_)
                | Command::Decl(_)
                | Command::Binary(_)
                | Command::Compound(_)
                | Command::Function(_)
                | Command::AnonymousFunction(_) => None,
            })
            .min_by_key(|span| span.end.offset - span.start.offset)
    }

    fn binding_assigns_numeric_operand_value(&mut self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.semantic.binding(binding_id);
        if self.binding_has_effective_integer_attribute(binding_id)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
        {
            return true;
        }

        let Some(word) = self
            .facts
            .binding_value(binding_id)
            .and_then(|value| value.scalar_word())
        else {
            return false;
        };

        if word_static_text_is_shell_integer(word, self.source)
            || word_has_arithmetic_expansion(word)
        {
            return true;
        }

        plain_scalar_reference_name(word)
            .is_some_and(|name| self.name_is_safe(&name, at, SafeValueQuery::NumericTestOperand))
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

fn plain_scalar_reference_name(word: &Word) -> Option<Name> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    plain_scalar_reference_name_from_part(&part.kind)
}

fn plain_scalar_reference_name_from_part(part: &WordPart) -> Option<Name> {
    match part {
        WordPart::Variable(name) if !matches!(name.as_str(), "@" | "*") => Some(name.clone()),
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return None;
            };
            plain_scalar_reference_name_from_part(&part.kind)
        }
        WordPart::Parameter(parameter) => match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none()
                    && !matches!(reference.name.as_str(), "@" | "*") =>
            {
                Some(reference.name.clone())
            }
            ParameterExpansionSyntax::Bourne(_) | ParameterExpansionSyntax::Zsh(_) => None,
        },
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => None,
    }
}

fn span_contains(container: Span, inner: Span) -> bool {
    container.start.offset <= inner.start.offset && inner.end.offset <= container.end.offset
}

fn shell_name_is_uppercase_setup_value(text: &str) -> bool {
    let mut saw_uppercase = false;
    for character in text.chars() {
        if character.is_ascii_uppercase() {
            saw_uppercase = true;
            continue;
        }
        if character == '_' || character.is_ascii_digit() {
            continue;
        }
        return false;
    }

    saw_uppercase
}

fn literal_is_setup_atom(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn safe_special_parameter(name: &Name) -> bool {
    matches!(name.as_str(), "@" | "#" | "?" | "$" | "!" | "-")
}

fn part_is_safe_special_parameter_access(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => safe_special_parameter(name),
        WordPart::DoubleQuoted { parts, .. } => {
            matches!(
                parts.as_slice(),
                [part] if part_is_safe_special_parameter_access(&part.kind)
            )
        }
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if safe_special_parameter(&reference.name) && reference.subscript.is_none()
        ),
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

fn safe_numeric_shell_variable(name: &Name) -> bool {
    matches!(
        name.as_str(),
        "BASHPID"
            | "COLUMNS"
            | "EUID"
            | "LINENO"
            | "OPTIND"
            | "PPID"
            | "RANDOM"
            | "SECONDS"
            | "UID"
    )
}

fn word_static_text_is_shell_integer(word: &Word, source: &str) -> bool {
    static_word_text(word, source).is_some_and(|text| shell_integer_text_is_safe(&text))
}

fn shell_integer_text_is_safe(text: &str) -> bool {
    let digits = text
        .strip_prefix(['+', '-'])
        .filter(|rest| !rest.is_empty())
        .unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn word_has_arithmetic_expansion(word: &Word) -> bool {
    parts_have_arithmetic_expansion(&word.parts)
}

fn word_is_arithmetic_expansion(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if matches!(part.kind, WordPart::ArithmeticExpansion { .. })
    )
}

fn parts_have_arithmetic_expansion(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::ArithmeticExpansion { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts_have_arithmetic_expansion(parts),
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    })
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
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
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

fn function_has_terminal_exit(function: &FunctionDef) -> bool {
    matches!(
        stmt_terminal_flow_kind(&function.body),
        TerminalFlowKind::Exit
    )
}

fn static_slice_result_is_safe(
    text: &str,
    offset: isize,
    length: Option<usize>,
    query: SafeValueQuery,
) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    let start = if offset < 0 {
        let distance = offset.unsigned_abs();
        if distance > chars.len() {
            return false;
        }
        chars.len().saturating_sub(distance)
    } else {
        offset as usize
    };
    if start > chars.len() {
        return false;
    }
    let end = length
        .map(|length| start.saturating_add(length).min(chars.len()))
        .unwrap_or(chars.len());
    let sliced = chars[start..end].iter().collect::<String>();
    !sliced.is_empty() && query.literal_is_safe(&sliced)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalFlowKind {
    None,
    MaybeExit,
    MaybeStop,
    Exit,
    Stop,
}

fn stmt_seq_terminal_flow_kind(commands: &StmtSeq) -> TerminalFlowKind {
    let mut saw_maybe_exit = false;
    let mut saw_maybe_stop = false;

    for stmt in commands.as_slice() {
        match stmt_terminal_flow_kind(stmt) {
            TerminalFlowKind::None => {}
            TerminalFlowKind::MaybeExit => saw_maybe_exit = true,
            TerminalFlowKind::MaybeStop => saw_maybe_stop = true,
            TerminalFlowKind::Exit => {
                return if saw_maybe_stop {
                    TerminalFlowKind::Stop
                } else {
                    TerminalFlowKind::Exit
                };
            }
            TerminalFlowKind::Stop => return TerminalFlowKind::Stop,
        }
    }

    if saw_maybe_stop {
        TerminalFlowKind::MaybeStop
    } else if saw_maybe_exit {
        TerminalFlowKind::MaybeExit
    } else {
        TerminalFlowKind::None
    }
}

fn stmt_terminal_flow_kind(stmt: &Stmt) -> TerminalFlowKind {
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
        return TerminalFlowKind::None;
    }

    command_terminal_flow_kind(&stmt.command)
}

fn command_terminal_flow_kind(command: &Command) -> TerminalFlowKind {
    match command {
        Command::Builtin(BuiltinCommand::Exit(_)) => TerminalFlowKind::Exit,
        Command::Builtin(BuiltinCommand::Return(_)) => TerminalFlowKind::Stop,
        Command::Compound(CompoundCommand::If(command)) => alternative_terminal_flow_kind(
            std::iter::once(stmt_seq_terminal_flow_kind(&command.then_branch))
                .chain(
                    command
                        .elif_branches
                        .iter()
                        .map(|(_, body)| stmt_seq_terminal_flow_kind(body)),
                )
                .chain(command.else_branch.iter().map(stmt_seq_terminal_flow_kind)),
            command.else_branch.is_none(),
        ),
        Command::Compound(CompoundCommand::For(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::Repeat(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::Foreach(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::ArithmeticFor(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::While(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::Until(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            maybe_stop_terminal_flow_kind(stmt_seq_terminal_flow_kind(&command.body))
        }
        Command::Compound(CompoundCommand::Case(command)) => alternative_terminal_flow_kind(
            command
                .cases
                .iter()
                .map(|case| stmt_seq_terminal_flow_kind(&case.body)),
            true,
        ),
        Command::Compound(CompoundCommand::BraceGroup(body)) => stmt_seq_terminal_flow_kind(body),
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_deref()
            .map_or(TerminalFlowKind::None, stmt_terminal_flow_kind),
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => TerminalFlowKind::None,
    }
}

fn maybe_stop_terminal_flow_kind(flow: TerminalFlowKind) -> TerminalFlowKind {
    match flow {
        TerminalFlowKind::None => TerminalFlowKind::None,
        TerminalFlowKind::MaybeExit | TerminalFlowKind::Exit => TerminalFlowKind::MaybeExit,
        TerminalFlowKind::MaybeStop | TerminalFlowKind::Stop => TerminalFlowKind::MaybeStop,
    }
}

fn alternative_terminal_flow_kind(
    branches: impl IntoIterator<Item = TerminalFlowKind>,
    can_skip_all: bool,
) -> TerminalFlowKind {
    let mut saw_none = can_skip_all;
    let mut saw_maybe_exit = false;
    let mut saw_maybe_stop = false;
    let mut saw_exit = false;
    let mut saw_stop = false;

    for flow in branches {
        match flow {
            TerminalFlowKind::None => saw_none = true,
            TerminalFlowKind::MaybeExit => saw_maybe_exit = true,
            TerminalFlowKind::MaybeStop => saw_maybe_stop = true,
            TerminalFlowKind::Exit => saw_exit = true,
            TerminalFlowKind::Stop => saw_stop = true,
        }
    }

    if saw_maybe_stop || ((saw_none || saw_maybe_exit) && saw_stop) {
        return TerminalFlowKind::MaybeStop;
    }
    if saw_maybe_exit || (saw_none && saw_exit) {
        return TerminalFlowKind::MaybeExit;
    }
    if saw_exit && !saw_stop {
        return TerminalFlowKind::Exit;
    }
    if saw_exit || saw_stop {
        return TerminalFlowKind::Stop;
    }

    TerminalFlowKind::None
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, Name, RedirectKind};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::{
        BindingOrigin, ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind,
        SemanticBuildOptions, UninitializedCertainty,
    };

    use super::{
        SafeValueIndex, SafeValueQuery, function_has_terminal_exit, static_slice_result_is_safe,
    };
    use crate::{ExpansionContext, LinterFacts, LinterSemanticArtifacts};

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
            SafeValueQuery::from_context(ExpansionContext::DescriptorDupTarget(
                RedirectKind::DupOutput
            )),
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$v"
            })
            .expect("expected loop-body command argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${@:2}"
            })
            .expect("expected direct slice command argument");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
                .all(|fact| !safe_values.word_occurrence_is_safe(*fact, SafeValueQuery::Argv))
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
                .all(|fact| safe_values.word_occurrence_is_safe(*fact, SafeValueQuery::Argv))
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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

        assert!(!safe_values.word_occurrence_is_safe(short_circuit_word, SafeValueQuery::Argv));
        assert!(safe_values.word_occurrence_is_safe(if_else_word, SafeValueQuery::Argv));
    }

    #[test]
    fn numeric_assignment_ternary_bindings_stay_safe() {
        let source = "\
#!/bin/bash
I=1
while [ $I -le 3 ]; do
  [[ -z $SPEED ]] && I=$(( I + 1 )) || I=11
done
J=1
while [ $J -le 3 ]; do
  [[ -z $SPEED ]] && J=+11 || J=-1
done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let loop_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && matches!(fact.span().slice(source), "$I" | "$J")
            })
            .collect::<Vec<_>>();

        assert_eq!(loop_words.len(), 2);
        for fact in loop_words {
            assert!(!safe_values.word_occurrence_is_safe(fact, SafeValueQuery::Argv));
            assert!(safe_values.word_occurrence_is_safe(fact, SafeValueQuery::NumericTestOperand));
        }
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$w"
            })
            .expect("expected guarded function command argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn conditional_same_block_initializers_do_not_dominate_later_uses() {
        let source = "\
#!/bin/bash
counter=0
while [ \"$counter\" -eq 0 ]; do
  if [ \"$1\" = validate ] || [ \"$1\" = install ]; then
    validate=validate
  fi
  steamcmd ${validate} +quit
  break
done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${validate}"
            })
            .expect("expected conditional initializer command argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn branch_ladder_pipeline_uses_stay_unsafe_after_guarded_initializers() {
        let source = "\
#!/bin/bash
if [ \"$1\" = validate ] || [ \"$1\" = install ]; then
  validate=validate
fi

if [ \"$mode\" = a ]; then
  steamcmd ${validate} +quit | tee /dev/null
elif [ \"$mode\" = b ]; then
  steamcmd ${validate} +runscript foo | tee /dev/null
else
  steamcmd ${validate} +app_update 1 | tee /dev/null
fi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let validate_uses = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${validate}"
            })
            .collect::<Vec<_>>();

        assert_eq!(validate_uses.len(), 3, "expected all sibling pipeline uses");
        assert!(
            validate_uses
                .into_iter()
                .all(|fact| !safe_values.word_occurrence_is_safe(fact, SafeValueQuery::Argv))
        );
    }

    #[test]
    fn function_local_guarded_initializers_do_not_dominate_later_uses() {
        let source = "\
#!/bin/bash
f() {
  if [ \"$1\" = validate ]; then
    validate=validate
  fi
  echo ${validate}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${validate}"
            })
            .expect("expected function-local validate command argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn function_local_pipeline_uses_stay_unsafe_after_guarded_initializers() {
        let source = "\
#!/bin/bash
f() {
  if [ \"$1\" = validate ]; then
    validate=validate
  fi
  while :; do
    if [ \"$appid\" = 90 ]; then
      if [ -n \"$branch\" ] && [ -n \"$beta\" ]; then
        echo ${validate} | tee /dev/null
      elif [ -n \"$branch\" ]; then
        echo ${validate} | tee /dev/null
      else
        echo ${validate} | tee /dev/null
      fi
    fi
    break
  done
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let validate_uses = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${validate}"
            })
            .collect::<Vec<_>>();

        assert_eq!(
            validate_uses.len(),
            3,
            "expected all function-local pipeline uses"
        );
        assert!(
            validate_uses
                .into_iter()
                .all(|fact| !safe_values.word_occurrence_is_safe(fact, SafeValueQuery::Argv))
        );
    }

    #[test]
    fn case_defaults_distinguish_maybe_uninitialized_from_explicit_empty_bindings() {
        let source = "\
#!/bin/bash
case $1 in
  nfs|dir)
    disk_ext=.qcow2
    ;;
  btrfs)
    disk_ext=.raw
    ;;
esac
printf '%s\\n' vm-${disk_ext:-}

case $2 in
  nfs|dir)
    disk_ext_with_default=.qcow2
    ;;
  btrfs)
    disk_ext_with_default=.raw
    ;;
  *)
    disk_ext_with_default=
    ;;
esac
printf '%s\\n' vm-${disk_ext_with_default:-}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let maybe_uninitialized = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "vm-${disk_ext:-}"
            })
            .expect("expected maybe-uninitialized command argument");
        let explicit_default = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "vm-${disk_ext_with_default:-}"
            })
            .expect("expected explicit-default command argument");

        assert!(!safe_values.word_occurrence_is_safe(maybe_uninitialized, SafeValueQuery::Argv));
        assert!(safe_values.word_occurrence_is_safe(explicit_default, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.is_nested_word_command() && fact.span().slice(source) == "${humanreadable}"
            })
            .expect("expected nested command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn static_loop_variables_after_multiline_loops_stay_unsafe() {
        let source = "\
#!/bin/sh
for i in castool chdman; do
  [ -e $i ] && install -s -m0755 -oroot -groot $i $PKG/usr/games/
done
[ -e split ] && install -s -m0755 -oroot -groot $i $PKG/usr/games/$PRGNAM-split
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let after_loop_use = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$i"
                    && fact.span().start.line == 5
            })
            .expect("expected post-loop $i word fact");
        let binding_id = safe_values
            .safe_bindings_for_name(&Name::new("i"), after_loop_use.span())
            .into_iter()
            .next()
            .expect("expected reaching loop binding");
        let definition_span = match &semantic.binding(binding_id).origin {
            BindingOrigin::LoopVariable {
                definition_span, ..
            } => *definition_span,
            other @ BindingOrigin::Assignment { .. }
            | other @ BindingOrigin::ParameterDefaultAssignment { .. }
            | other @ BindingOrigin::Imported { .. }
            | other @ BindingOrigin::FunctionDefinition { .. }
            | other @ BindingOrigin::BuiltinTarget { .. }
            | other @ BindingOrigin::ArithmeticAssignment { .. }
            | other @ BindingOrigin::Declaration { .. }
            | other @ BindingOrigin::Nameref { .. } => {
                panic!("expected loop-variable binding, got {other:?}")
            }
        };

        assert!(
            !safe_values
                .loop_variable_reference_stays_within_body(definition_span, after_loop_use.span())
        );
        let (part, part_span) = after_loop_use
            .parts_with_spans()
            .next()
            .expect("expected single-part loop variable word");
        assert!(!safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv));
        assert!(!safe_values.word_occurrence_is_safe(after_loop_use, SafeValueQuery::Argv));
    }

    #[test]
    fn nested_command_substitution_arguments_with_dynamic_values_stay_unsafe() {
        let source = "\
#!/bin/sh
PRGNAM=cproc
GIT_SHA=$( git rev-parse --short HEAD )
DATE=$( git log --date=format:%Y%m%d --format=%cd | head -1 )
VERSION=${DATE}_${GIT_SHA}
echo \"MD5SUM=\\\"$( md5sum $PRGNAM-$VERSION.tar.xz | cut -d' ' -f1 )\\\"\"
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let version_use = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.is_nested_word_command()
                    && fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source).contains("VERSION")
            })
            .expect("expected nested command argument fact for VERSION");

        assert!(!safe_values.word_occurrence_is_safe(version_use, SafeValueQuery::Argv));
    }

    #[test]
    fn definite_uninitialized_bindings_stay_unsafe() {
        let source = "\
#!/bin/bash
if [ \"${PULSE:-yes}\" != \"yes\" ]; then
  pulseopt=\"--without-pulse\"
fi

config() {
  ./configure \\
    $pulseopt \\
    --prefix=/usr
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$pulseopt"
            })
            .expect("expected pulseopt command-argument fact");

        assert!(
            analysis
                .uninitialized_references()
                .iter()
                .any(|uninitialized| {
                    semantic.reference(uninitialized.reference).span == word_fact.span()
                }),
            "expected pulseopt to be marked uninitialized"
        );
        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn all_empty_static_literal_bindings_stay_unsafe_but_mixed_option_bindings_can_stay_safe() {
        let source = "\
#!/bin/bash
gl2ps=
if true; then
  libdirsuffix=
else
  libdirsuffix=
fi

if true; then
  mixedsuffix=64
else
  mixedsuffix=
fi

cmake $gl2ps -DOPT=1
mkdir -p /tmp/usr/lib${libdirsuffix}/ladspa
mkdir -p /tmp/usr/lib${mixedsuffix}/ladspa

if true; then
  opt=-n
else
  opt=
fi
printf '%s\\n' $opt hi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let unsafe_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && (fact.span().slice(source) == "$gl2ps"
                        || fact
                            .span()
                            .slice(source)
                            .contains("lib${libdirsuffix}/ladspa"))
            })
            .collect::<Vec<_>>();
        let safe_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && (fact.span().slice(source) == "$opt"
                        || fact
                            .span()
                            .slice(source)
                            .contains("lib${mixedsuffix}/ladspa"))
            })
            .collect::<Vec<_>>();

        assert_eq!(unsafe_words.len(), 2, "expected both empty-only uses");
        assert!(
            unsafe_words
                .into_iter()
                .all(|fact| !safe_values.word_occurrence_is_safe(fact, SafeValueQuery::Argv))
        );
        assert_eq!(safe_words.len(), 2, "expected both mixed-value uses");
        assert!(
            safe_words
                .into_iter()
                .all(|fact| safe_values.word_occurrence_is_safe(fact, SafeValueQuery::Argv))
        );
    }

    #[test]
    fn helper_call_sequences_can_make_outer_safe_globals_unsafe() {
        let source = "\
#!/bin/bash
Region=default

GetRegion() {
  Region=\"$(printf '%s' \"$1\")\"
}

GetAMI() {
  aws ec2 describe-images --region $Region
}

GetRegion \"$1\"
GetAMI
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$Region"
            })
            .expect("expected Region command-argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn conditional_helper_globals_stay_unsafe_inside_nested_command_substitution_consumers() {
        let source = "\
#!/bin/bash
Region=default

GetRegion() {
  if [ \"$Region\" = default ]; then
    Region=\"$(printf '%s' \"$1\")\"
  fi
}

GetAMI() {
  AMI=$(aws ssm get-parameters --region $Region)
}

GetRegion \"$1\"
GetAMI
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$Region"
            })
            .expect("expected nested command-substitution helper-derived argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${compressflag}"
            })
            .expect("expected helper-provided command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_entry_functions_do_not_inherit_outer_safe_globals() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid
configfile=/etc/collectd.conf

start() {
  [ -x $exec ] || exit 5
  $exec -P $pidfile -C $configfile
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_name = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandName)
                    && fact.span().slice(source) == "$exec"
            })
            .expect("expected dynamic command-name fact");
        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$pidfile"
            })
            .expect("expected dynamic command-argument fact");

        assert!(!safe_values.word_occurrence_is_safe(command_name, SafeValueQuery::Argv));
        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_without_top_level_exit_keeps_outer_safe_globals() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid
configfile=/etc/collectd.conf

start() {
  [ -x $exec ] || exit 5
  $exec -P $pidfile -C $configfile
}

case \"$1\" in
  start) $1 ;;
esac
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_name = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandName)
                    && fact.span().slice(source) == "$exec"
            })
            .expect("expected dynamic command-name fact");
        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$pidfile"
            })
            .expect("expected dynamic command-argument fact");

        assert!(safe_values.word_occurrence_is_safe(command_name, SafeValueQuery::Argv));
        assert!(safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_scope_memoization_does_not_leak_status_capture_unsafety_into_helpers() {
        let source = "\
#!/bin/sh
start() {
  status=$?
  printf '%s\\n' ${status}
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let dispatched_use = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${status}"
            })
            .expect("expected case-cli entrypoint status use");
        let binding_id = safe_values
            .safe_bindings_for_name(&Name::new("status"), dispatched_use.span())
            .into_iter()
            .next()
            .expect("expected reaching status binding");
        let case_cli_scope =
            safe_values.case_cli_dispatch_scope_at(dispatched_use.span().start.offset);

        assert_eq!(
            case_cli_scope,
            Some(semantic.binding(binding_id).scope),
            "expected the status binding to belong to the case-cli scope"
        );

        assert!(!safe_values.binding_is_safe(
            binding_id,
            dispatched_use.span(),
            SafeValueQuery::Argv,
            case_cli_scope
        ));
        assert!(safe_values.binding_is_safe(
            binding_id,
            dispatched_use.span(),
            SafeValueQuery::Argv,
            None
        ));
    }

    #[test]
    fn case_cli_dispatch_entry_functions_keep_local_command_names_but_not_arguments_safe() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$n"
            })
            .expect("expected case-cli local command argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_entry_functions_with_literal_exit_keep_local_arguments_unsafe() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit 0
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$n"
            })
            .expect("expected case-cli local command argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_reachable_helpers_keep_local_arguments_unsafe() {
        let source = "\
#!/bin/sh
foo() {
  local n=0
  echo $n
}

bound() { foo; }
renew() { foo; }
deconfig() { :; }

case \"$1\" in
  deconfig|renew|bound) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$n"
            })
            .expect("expected case-cli helper command argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_broadens_into_pre_dispatch_functions_even_when_patterns_skip_them() {
        let source = "\
#!/bin/sh
foo() {
  local n=0
  echo $n
}

bar() { :; }

case \"$1\" in
  bar) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$n"
            })
            .expect("expected pre-dispatch function command argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_entry_functions_keep_nested_command_names_but_not_arguments_safe() {
        let source = "\
#!/bin/sh
start() {
  printf '%s\n' \"$(
    cmd=/bin/echo
    arg=hello
    $cmd $arg
  )\"
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_name = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandName)
                    && fact.span().slice(source) == "$cmd"
            })
            .expect("expected nested command-substitution command name");
        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$arg"
            })
            .expect("expected nested command-substitution command argument");

        assert!(command_name.is_nested_word_command());
        assert!(command_arg.is_nested_word_command());
        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn top_level_exit_broadens_function_arguments_without_dispatch() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}
exit 0
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$n"
            })
            .expect("expected function-local argument before top-level exit");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn nested_function_arguments_stay_unsafe_without_top_level_exit() {
        let source = "\
#!/bin/bash
outer() {
  inner() {
    local good=0
    return $good
  }
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$good"
            })
            .expect("expected nested-function return argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn transitive_helper_calls_make_top_level_unsafe_bindings_visible() {
        let source = "\
#!/bin/bash
DISK_SIZE=\"32G\"

advanced_settings() {
  DISK_SIZE=\"$(get_size)\"
}

start_script() {
  advanced_settings
}

start_script
if [ -n \"$DISK_SIZE\" ]; then
  qm resize 100 scsi0 ${DISK_SIZE} >/dev/null
fi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let command_arg = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${DISK_SIZE}"
            })
            .expect("expected top-level helper-derived argument");

        assert!(!safe_values.word_occurrence_is_safe(command_arg, SafeValueQuery::Argv));
    }

    #[test]
    fn case_cli_dispatch_indirect_status_targets_stay_unsafe() {
        let source = "\
#!/bin/bash
start() {
  status=$?
  ref=status
  printf '%s\n' ${!ref}
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let indirect_use = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${!ref}"
            })
            .expect("expected indirect case-cli status use");

        assert!(!safe_values.word_occurrence_is_safe(indirect_use, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected shared helper-derived command argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn helper_globals_inherit_caller_visible_bindings() {
        let source = "\
#!/bin/sh
SERVERNUM=99
find_free_servernum() {
  i=$SERVERNUM
  while [ -f /tmp/.X$i-lock ]; do
    i=$(($i + 1))
  done
  echo $i
}
set -- -n '1 2' -a --
while :; do
  case \"$1\" in
    -a|--auto-servernum) SERVERNUM=$(find_free_servernum) ;;
    -n|--server-num) SERVERNUM=\"$2\"; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
  shift
done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let unsafe_words = facts
            .word_facts()
            .iter()
            .filter(|fact| {
                fact.span().slice(source).contains("$i") && matches!(fact.span().start.line, 5 | 8)
            })
            .collect::<Vec<_>>();

        assert_eq!(unsafe_words.len(), 2, "expected both helper-body $i uses");
        assert!(
            unsafe_words
                .iter()
                .all(|fact| !safe_values.word_occurrence_is_safe(*fact, SafeValueQuery::Argv))
        );
    }

    #[test]
    fn helper_globals_with_mixed_safe_and_unsafe_caller_branches_stay_unsafe() {
        let source = "\
#!/bin/sh
if [ -n \"$2\" ]; then
  UIPORT=\"$2\"
else
  UIPORT=\"8080\"
fi
do_start() {
  grep $UIPORT /dev/null
}
do_start
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$UIPORT"
            })
            .expect("expected helper command argument fact");
        let (part, part_span) = word_fact
            .parts_with_spans()
            .find(|(_, span)| span.slice(source) == "$UIPORT")
            .expect("expected UIPORT expansion part");
        let name = Name::from("UIPORT");
        let bindings = safe_values.safe_bindings_for_name(&name, part_span);

        assert_eq!(
            bindings.len(),
            2,
            "expected both caller-visible UIPORT bindings"
        );
        assert!(safe_values.bindings_cover_all_paths_to_reference(&bindings, &name, part_span));
        assert!(!safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv));
        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn helper_globals_with_conditionally_initialized_caller_bindings_stay_unsafe() {
        let source = "\
#!/bin/bash
[ \"$1\" = 64 ] && extra=ENABLE_LIB64=1
run_make() {
  make $extra
}
run_make
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$extra"
            })
            .expect("expected helper command argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected conditionally helper-initialized argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn guarded_numeric_helper_bindings_do_not_count_as_definite_values() {
        let source = "\
#!/bin/bash
init_count() {
  if [ \"$1\" = yes ]; then
    count=0
  fi
}
init_count
move_up $count
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$count"
            })
            .expect("expected numeric helper argument fact");
        let (part, part_span) = word_fact
            .parts_with_spans()
            .find(|(_, span)| span.slice(source) == "$count")
            .expect("expected count expansion part");

        assert!(!safe_values.part_has_s001_standalone_numeric_argv_exposure(part, part_span));
    }

    #[test]
    fn helper_writes_to_caller_local_keep_arguments_unsafe() {
        let source = "\
#!/bin/bash
helper() {
  value=$1
}

render() {
  local value=SAFE
  helper \"$1\"
  printf '%s\\n' ${value}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${value}"
            })
            .expect("expected helper-mutated local argument fact");
        let (part, part_span) = word_fact
            .parts_with_spans()
            .find(|(_, span)| span.slice(source) == "${value}")
            .expect("expected value expansion part");
        let name = Name::from("value");
        let helper_bindings = safe_values.called_helper_bindings_for_name(&name, part_span);

        assert_eq!(
            helper_bindings.len(),
            1,
            "expected helper assignment to remain visible through caller local"
        );
        assert!(!safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv));

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn safe_setup_helper_writes_do_not_suppress_sibling_helper_warnings() {
        let source = "\
#!/bin/bash
setup() {
  value=SAFE
}

use_value() {
  printf '%s\\n' ${value}
}

main() {
  local value
  setup
  use_value
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${value}"
            })
            .expect("expected sibling helper argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn shadowed_helper_definition_does_not_make_safe_caller_local_unsafe() {
        let source = "\
#!/bin/bash
helper() {
  value=$1
}

render() {
  local value=SAFE
  helper() {
    :
  }
  helper
  printf '%s\\n' ${value}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${value}"
            })
            .expect("expected shadowed helper argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn shadowed_helper_call_site_does_not_count_as_outer_helper_write() {
        let source = "\
#!/bin/bash
helper() {
  value=$1
}

render() {
  local value=SAFE
  helper() {
    :
  }
  helper
  printf '%s\\n' ${value}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${value}"
            })
            .expect("expected local argument fact");
        let render_scope = analysis
            .enclosing_function_scope_at(word_fact.span().start.offset)
            .expect("expected render scope");
        let outer_helper_binding = semantic
            .semantic()
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "value"
                    && binding.scope != render_scope
                    && matches!(binding.kind, shuck_semantic::BindingKind::Assignment)
            })
            .expect("expected outer helper binding");

        assert!(!safe_values.binding_writes_visible_local_in_scope_before(
            outer_helper_binding.id,
            render_scope,
            word_fact.span(),
        ));
    }

    #[test]
    fn helper_call_before_local_shadow_does_not_make_local_unsafe() {
        let source = "\
#!/bin/bash
helper() {
  value=$1
}

render() {
  helper \"$1\"
  local value=SAFE
  printf '%s\\n' ${value}
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${value}"
            })
            .expect("expected local argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn unrelated_caller_local_does_not_poison_status_capture_helper_use() {
        let source = "\
#!/bin/bash
helper() {
  false || ret=$?
  printf '%s\\n' $ret
}

shadowed_caller() {
  local ret=SAFE
  helper
}

clean_caller() {
  helper
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$ret"
            })
            .expect("expected helper status argument fact");
        let status_binding = semantic
            .bindings_for(&Name::from("ret"))
            .iter()
            .copied()
            .find(|binding_id| safe_values.binding_value_is_standalone_status_capture(*binding_id))
            .expect("expected status capture binding");

        assert!(
            !safe_values.status_capture_binding_conflicts_with_caller_local(
                status_binding,
                word_fact.span()
            )
        );
    }

    #[test]
    fn unset_static_slice_binding_does_not_stay_safe() {
        let source = "\
#!/bin/bash
sig=RS256
unset sig
openssl dgst -sha${sig:2} file
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "-sha${sig:2}"
            })
            .expect("expected sliced digest argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn escaped_declaration_status_capture_return_stays_safe() {
        let source = "\
#!/bin/bash
run() {
  make install ||
  {
    \\typeset ret=$?
    warn failed
    return $ret
  }
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$ret"
            })
            .expect("expected return status argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${flag}"
            })
            .expect("expected multi-caller helper-derived argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn top_level_helper_initializers_refine_later_helper_calls() {
        let source = "\
#!/bin/bash
start_reader() {
  sleep 1 &
  tarpid=$!
}

read_disc() {
  grep $tarpid /dev/null
}

tarpid=\"\"
mode=$1
if [[ $mode == read ]]; then
  start_reader
fi
if [[ $mode == read ]]; then
  read_disc
fi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$tarpid"
            })
            .expect("expected helper command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn conditional_top_level_helper_initializers_do_not_refine_unconditional_helper_calls() {
        let source = "\
#!/bin/bash
start_reader() {
  sleep 1 &
  tarpid=$!
}

read_disc() {
  grep $tarpid /dev/null
}

tarpid=\"\"
mode=$1
if [[ $mode == read ]]; then
  start_reader
fi
read_disc
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$tarpid"
            })
            .expect("expected helper command argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn imported_bindings_stay_unsafe_without_known_values() {
        let source = "\
#!/bin/bash
printf '%s\\n' $pkgname
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build_with_options(
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
                    externally_consumed_bindings: false,
                }),
                ..SemanticBuildOptions::default()
            },
        );
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "$pkgname"
            })
            .expect("expected imported command argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn quoted_replacement_operands_stay_safe_in_argv_context() {
        let source = "\
#!/bin/bash
bash ${debug:+\"-x\"} script
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${debug:+\"-x\"}"
            })
            .expect("expected replacement-operator command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn quoted_replacement_operands_with_spaces_stay_safe_in_argv_context() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${debug:+\"a b\"}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${debug:+\"a b\"}"
            })
            .expect("expected replacement-operator command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn static_literal_slice_bindings_stay_safe_in_argv_context() {
        let source = "\
#!/bin/bash
if true; then
  sig=RS256
else
  sig=ES512
fi
openssl dgst -sha${sig:2} payload
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "-sha${sig:2}"
            })
            .expect("expected sliced command argument fact");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn overly_negative_static_slice_offset_stays_unsafe() {
        assert!(!static_slice_result_is_safe(
            "RS256",
            -20,
            None,
            SafeValueQuery::Argv
        ));
    }

    #[test]
    fn dynamic_slice_bindings_stay_unsafe_in_argv_context() {
        let source = "\
#!/bin/bash
sig=$1
openssl dgst -sha${sig:2} payload
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "-sha${sig:2}"
            })
            .expect("expected sliced command argument fact");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn backgrounded_exit_like_definitions_do_not_block_safe_bindings() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; } &
Exit
echo /tmp/$SAFE
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "/tmp/$SAFE"
            })
            .expect("expected mixed path command argument");

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn exhaustive_safe_bindings_override_conservative_maybe_uninitialized_refs() {
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i386\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi

TARGET=$ARCH-linux
VERSION=${TARGET}_$(date +%s)
if [ ! -r /pkg/usr/lib${LIBDIRSUFFIX}/gcc/${TARGET}/${VERSION}/specs ]; then
  :
fi
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source)
                        == "/pkg/usr/lib${LIBDIRSUFFIX}/gcc/${TARGET}/${VERSION}/specs"
            })
            .expect("expected mixed path command argument");
        let (part, part_span) = word_fact
            .parts_with_spans()
            .find(|(_, span)| span.slice(source) == "${LIBDIRSUFFIX}")
            .expect("expected LIBDIRSUFFIX part");
        let name = Name::from("LIBDIRSUFFIX");
        let bindings = safe_values.safe_bindings_for_name(&name, part_span);

        assert!(
            safe_values.bindings_cover_all_paths_to_reference(&bindings, &name, part_span),
            "expected exhaustive branch ladder to cover all paths"
        );

        safe_values.override_uninitialized_reference_certainty(
            part_span,
            UninitializedCertainty::Possible,
        );

        assert!(safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv));
    }

    #[test]
    fn exit_like_function_calls_invalidate_prior_and_later_safe_bindings() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);
        let exit_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "Exit")
            })
            .expect("expected Exit function header");

        let nested_argument = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.is_nested_word_command()
                    && fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "${OPTION_BINARY_FILE}"
            })
            .expect("expected nested command argument fact");
        let redirect_target = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context()
                    == Some(ExpansionContext::RedirectTarget(RedirectKind::Append))
                    && fact.span().slice(source) == "${OPENBSD_CONTENTS}"
            })
            .expect("expected redirect target fact");

        assert!(function_has_terminal_exit(exit_header.function()));
        assert_eq!(exit_header.call_arity().zero_arg_call_spans().len(), 1);

        assert!(!safe_values.word_occurrence_is_safe(nested_argument, SafeValueQuery::Argv));
        assert!(
            !safe_values.word_occurrence_is_safe(redirect_target, SafeValueQuery::RedirectTarget)
        );
    }

    #[test]
    fn inline_returns_make_later_safe_bindings_unsafe() {
        let source = "\
#!/bin/sh
helper() {
  safe=foo
  return 0
  echo /tmp/$safe
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);

        let word_fact = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact.span().slice(source) == "/tmp/$safe"
            })
            .expect("expected mixed path command argument");

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
    }

    #[test]
    fn subshell_exit_does_not_make_function_terminal() {
        let source = "\
#!/bin/sh
helper() (
  exit 1
)
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(!function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn early_unconditional_exit_makes_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  exit 1
  :
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn assigned_exit_makes_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  FOO=1 exit 1
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn negated_exit_makes_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  ! exit 1
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn extra_arg_exit_makes_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  exit 1 2
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn all_if_branches_exiting_make_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  if [ \"$SKIP\" ]; then
    exit 0
  else
    exit 1
  fi
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn conditional_return_before_exit_does_not_make_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  if [ \"$SKIP\" ]; then
    return 0
  fi
  exit 1
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(!function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn conditional_exit_before_exit_makes_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  if [ \"$SKIP\" ]; then
    exit 1
  fi
  exit 0
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn return_before_exit_does_not_make_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  return 0
  exit 1
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(!function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn all_if_branches_returning_do_not_make_function_terminal() {
        let source = "\
#!/bin/sh
helper() {
  if [ \"$SKIP\" ]; then
    return 0
  else
    return 1
  fi
  exit 0
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let helper_header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name.as_str() == "helper")
            })
            .expect("expected helper function header");

        assert!(!function_has_terminal_exit(helper_header.function()));
    }

    #[test]
    fn later_top_level_exit_helpers_block_same_function_bindings() {
        let source = "\
#!/bin/sh
SAFE=foo
wrapper() {
  Exit
  echo /tmp/$SAFE
}
Exit() { exit 0; }
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        let analysis = semantic.semantic().analysis();
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer);
        let mut safe_values = SafeValueIndex::build(semantic.semantic(), &analysis, &facts, source);
        let target = facts
            .word_facts()
            .iter()
            .find(|fact| {
                fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                    && fact
                        .parts_with_spans()
                        .any(|(_, span)| span.slice(source) == "$SAFE")
            })
            .expect("expected same-function argument fact");

        assert!(!safe_values.word_occurrence_is_safe(target, SafeValueQuery::Argv));
    }
}
