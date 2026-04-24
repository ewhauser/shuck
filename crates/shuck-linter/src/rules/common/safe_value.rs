use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand, FunctionDef, Name,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, RedirectKind, SourceText, Span,
    Stmt, StmtSeq, StmtTerminator, VarRef, Word, WordPart, WordPartNode, static_word_text,
    word_is_standalone_status_capture, word_is_standalone_variable_like,
};
use shuck_semantic::{
    AssignmentValueOrigin, BindingAttributes, BindingKind, BindingOrigin, LoopValueOrigin, ScopeId,
    ScopeKind, SemanticAnalysis, SemanticModel, UninitializedCertainty,
};
use shuck_semantic::{BindingId, BlockId, ReferenceId, ReferenceKind};

use crate::facts::analyze_literal_runtime;
use crate::{ExpansionContext, FactSpan, LinterFacts};

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
    case_cli_reachable_function_scopes: FxHashSet<ScopeId>,
    definite_uninitialized_refs: FxHashSet<FactSpan>,
    maybe_uninitialized_refs: FxHashSet<FactSpan>,
    memo: FxHashMap<(FactSpan, FactSpan, SafeValueQuery, Option<ScopeId>), bool>,
    visiting: FxHashSet<(FactSpan, FactSpan, SafeValueQuery, Option<ScopeId>)>,
    helper_binding_memo: FxHashMap<(Name, ScopeId, FactSpan), Box<[BindingId]>>,
    helper_binding_visiting: FxHashSet<(Name, ScopeId, FactSpan)>,
}

impl<'a> SafeValueIndex<'a> {
    pub fn build(
        semantic: &'a SemanticModel,
        analysis: &'a SemanticAnalysis<'a>,
        facts: &'a LinterFacts<'a>,
        source: &'a str,
    ) -> Self {
        let definite_uninitialized_refs = analysis
            .uninitialized_references()
            .iter()
            .filter(|uninitialized| uninitialized.certainty == UninitializedCertainty::Definite)
            .map(|uninitialized| FactSpan::new(semantic.reference(uninitialized.reference).span))
            .collect();
        let maybe_uninitialized_refs = analysis
            .uninitialized_references()
            .iter()
            .filter(|uninitialized| uninitialized.certainty == UninitializedCertainty::Possible)
            .map(|uninitialized| FactSpan::new(semantic.reference(uninitialized.reference).span))
            .collect();
        let case_cli_reachable_function_scopes =
            build_case_cli_reachable_function_scopes(semantic, facts);

        Self {
            semantic,
            analysis,
            facts,
            source,
            case_cli_reachable_function_scopes,
            definite_uninitialized_refs,
            maybe_uninitialized_refs,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
            helper_binding_memo: FxHashMap::default(),
            helper_binding_visiting: FxHashSet::default(),
        }
    }

    pub fn part_is_safe(&mut self, part: &WordPart, span: Span, query: SafeValueQuery) -> bool {
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
        if self.case_cli_reachable_call_path_keeps_argument_bindings_unsafe(at, query) {
            return safe_numeric_shell_variable(name);
        }

        let mut bindings = self.safe_bindings_for_name(name, at);
        self.drop_declarations_shadowed_by_covering_loop_bindings(&mut bindings, at);
        if matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget) {
            bindings.retain(|binding_id| {
                !self.binding_is_blocked_by_exit_like_function_call(*binding_id, at)
            });
        }
        let case_cli_scope = matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
            .then(|| self.case_cli_dispatch_scope_at(at.start.offset))
            .flatten();
        if self.status_capture_declaration_probe_covers_reference(name, at, query, case_cli_scope) {
            return true;
        }
        if bindings.is_empty() {
            return safe_numeric_shell_variable(name);
        }
        let binding_belongs_to_case_cli_scope = case_cli_scope.is_some_and(|scope| {
            bindings
                .iter()
                .copied()
                .any(|binding_id| self.binding_is_in_scope_or_descendant(binding_id, scope))
        });
        if matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
            && case_cli_scope.is_some()
            && !self.case_cli_dispatch_outer_bindings_can_stay_safe(&bindings, at, query)
            && !binding_belongs_to_case_cli_scope
        {
            return safe_numeric_shell_variable(name);
        }
        if matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
            && self.bindings_are_all_plain_empty_static_literals(&bindings)
        {
            return false;
        }
        if self.status_capture_bindings_cover_reference(&bindings, name, at, query, case_cli_scope)
        {
            return true;
        }
        let helper_bindings = self
            .called_helper_bindings_for_name(name, at)
            .into_iter()
            .collect::<FxHashSet<_>>();
        let needs_arg_path_coverage =
            matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget);
        let bindings_cover_all_paths = helper_bindings.is_empty()
            && needs_arg_path_coverage
            && self.bindings_cover_all_paths_to_reference(&bindings, name, at);
        let outer_bindings_cover_callers = !needs_arg_path_coverage
            || self.helper_outer_bindings_cover_all_caller_paths(name, at, &bindings);
        let reference_is_inside_function =
            self.enclosing_function_scope_at(at.start.offset).is_some();
        if helper_bindings.is_empty()
            && needs_arg_path_coverage
            && !bindings_cover_all_paths
            && (!outer_bindings_cover_callers || !reference_is_inside_function)
        {
            return false;
        }
        if !outer_bindings_cover_callers {
            return false;
        }
        if self
            .definite_uninitialized_refs
            .contains(&FactSpan::new(at))
        {
            if bindings.iter().copied().any(|binding_id| {
                !helper_bindings.contains(&binding_id)
                    && self.binding_is_guarded_before_reference(binding_id, at)
            }) {
                return false;
            }
        } else if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            let has_dominating_binding = bindings
                .iter()
                .copied()
                .any(|binding_id| self.binding_dominates_reference(binding_id, name, at));
            if !has_dominating_binding
                && !bindings_cover_all_paths
                && !bindings
                    .iter()
                    .copied()
                    .all(|binding_id| helper_bindings.contains(&binding_id))
            {
                return false;
            }
        }

        bindings
            .into_iter()
            .all(|binding_id| self.binding_is_safe(binding_id, at, query, case_cli_scope))
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

    pub fn in_case_cli_dispatch_entrypoint(&self, offset: usize) -> bool {
        self.case_cli_dispatch_scope_at(offset).is_some()
    }

    fn case_cli_reachable_function_scope_at(&self, offset: usize) -> Option<ScopeId> {
        let scope = self.enclosing_function_scope_at(offset)?;
        self.case_cli_reachable_function_scopes
            .contains(&scope)
            .then_some(scope)
    }

    fn case_cli_reachable_call_path_keeps_argument_bindings_unsafe(
        &self,
        at: Span,
        query: SafeValueQuery,
    ) -> bool {
        if !matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
            || self.span_is_within_command_name(at)
        {
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
        self.semantic
            .ancestor_scopes(self.semantic.binding(binding_id).scope)
            .any(|scope| scope == ancestor_scope)
    }

    fn is_argument_of_dynamic_command(&self, at: Span) -> bool {
        self.facts.commands().iter().any(|command| {
            command.body_args().iter().any(|word| word.span == at)
                && command
                    .body_name_word()
                    .is_some_and(word_is_standalone_variable_like)
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
            let Some(function_definition_command) =
                self.function_definition_command(header.function())
            else {
                return false;
            };
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
                                    function_definition_command.id(),
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
                && self.command_runs_in_unconditional_flow(command.id(), at)
                && matches!(
                    command.command(),
                    Command::Builtin(BuiltinCommand::Exit(_) | BuiltinCommand::Return(_))
                )
        })
    }

    fn function_definition_command(
        &self,
        function: &FunctionDef,
    ) -> Option<&crate::facts::CommandFact<'a>> {
        self.facts.commands().iter().find(|command| {
            matches!(
                command.command(),
                Command::Function(candidate) if candidate.span == function.span
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

    fn command_for_name_word_span(&self, span: Span) -> Option<&crate::facts::CommandFact<'a>> {
        self.facts.commands().iter().find(|command| {
            command
                .body_name_word()
                .is_some_and(|name_word| name_word.span == span)
        })
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
        self.semantic
            .ancestor_scopes(self.semantic.scope_at(offset))
            .find(|scope| matches!(self.semantic.scope(*scope).kind, ScopeKind::Function(_)))
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

    fn bindings_are_all_plain_empty_static_literals(&self, bindings: &[BindingId]) -> bool {
        !bindings.is_empty()
            && bindings
                .iter()
                .copied()
                .all(|binding_id| self.binding_is_plain_empty_static_literal(binding_id))
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
        if binding.attributes.contains(BindingAttributes::INTEGER)
            || matches!(binding.kind, BindingKind::ArithmeticAssignment)
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
            } => {
                let scalar_word = self
                    .facts
                    .binding_value(binding_id)
                    .filter(|value| !value.conditional_assignment_shortcut())
                    .and_then(|value| value.scalar_word());
                if case_cli_scope == Some(binding.scope)
                    && matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
                    && scalar_word.is_some_and(word_is_standalone_status_capture)
                {
                    false
                } else {
                    scalar_word.is_some_and(|word| self.word_is_safe(word, query))
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

    fn status_capture_bindings_cover_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        if !matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget) {
            return false;
        }

        let mut status_bindings = Vec::new();
        for binding_id in bindings.iter().copied() {
            let binding = self.semantic.binding(binding_id);
            match &binding.origin {
                BindingOrigin::Assignment {
                    value:
                        AssignmentValueOrigin::PlainScalarAccess | AssignmentValueOrigin::StaticLiteral,
                    ..
                }
                | BindingOrigin::Declaration { .. }
                    if case_cli_scope != Some(binding.scope)
                        && self
                            .facts
                            .binding_value(binding_id)
                            .filter(|value| !value.conditional_assignment_shortcut())
                            .and_then(|value| value.scalar_word())
                            .is_some_and(word_is_standalone_status_capture) =>
                {
                    status_bindings.push(binding_id);
                }
                BindingOrigin::Declaration { .. } => {}
                _ => return false,
            }
        }

        !status_bindings.is_empty()
            && self.bindings_cover_all_paths_to_reference(&status_bindings, name, at)
    }

    fn status_capture_declaration_probe_covers_reference(
        &self,
        name: &Name,
        at: Span,
        query: SafeValueQuery,
        case_cli_scope: Option<ScopeId>,
    ) -> bool {
        if !matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget) {
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
        command: &crate::facts::CommandFact<'a>,
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

        let cover_blocks = self
            .analysis
            .block_ids_for_span(command.span())
            .iter()
            .copied()
            .collect::<FxHashSet<_>>();
        if cover_blocks.is_empty() {
            return false;
        }
        if cover_blocks.contains(&reference_block) {
            return true;
        }

        let cfg = self.analysis.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let entry = self
            .enclosing_function_scope_at(at.start.offset)
            .and_then(|scope| cfg.scope_entry(scope))
            .unwrap_or_else(|| cfg.entry());
        let mut stack = vec![entry];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if cover_blocks.contains(&block_id)
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

    fn reference_is_safe(&mut self, reference: &VarRef, at: Span, query: SafeValueQuery) -> bool {
        if query != SafeValueQuery::Quoted && reference.has_array_selector() {
            return false;
        }
        self.name_is_safe(&reference.name, at, query)
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
        let caller_sites = self.named_function_call_sites(helper_scope);
        !caller_sites.is_empty()
            && caller_sites.into_iter().all(|(_, call_span)| {
                self.loop_variable_reference_stays_within_body(definition_span, call_span)
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
        if self.maybe_uninitialized_refs.contains(&FactSpan::new(at)) {
            return false;
        }

        let bindings = self.safe_bindings_for_name(&reference.name, at);
        if bindings.is_empty() {
            return false;
        }
        let case_cli_scope = matches!(query, SafeValueQuery::Argv | SafeValueQuery::RedirectTarget)
            .then(|| self.case_cli_dispatch_scope_at(at.start.offset))
            .flatten();

        bindings.into_iter().all(|binding_id| {
            let targets = self.semantic.indirect_targets_for_binding(binding_id);
            !targets.is_empty()
                && targets
                    .iter()
                    .copied()
                    .all(|target| self.binding_is_safe(target, at, query, case_cli_scope))
        })
    }

    fn safe_bindings_for_name(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self.visible_bindings_for_name_without_helpers(name, at);
        let mut helper_bindings = self.called_helper_bindings_for_name(name, at);
        helper_bindings.extend(self.top_level_transitive_helper_bindings_before(name, at));
        helper_bindings
            .sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        helper_bindings.dedup();
        let caller_bindings = self.caller_bindings_covering_all_static_call_sites(name, at);
        if bindings.is_empty() {
            bindings = if caller_bindings.is_empty() {
                helper_bindings
            } else {
                caller_bindings
            };
        } else if !helper_bindings.is_empty() {
            bindings.extend(helper_bindings);
            bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
            bindings.dedup();
        }

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
        let caller_sites = self.named_function_call_sites(helper_scope);
        if caller_sites.is_empty() {
            return Vec::new();
        }

        let mut bindings = Vec::new();
        for (scope, span) in caller_sites {
            let branch = self.caller_branch_bindings_before(name, scope, span);
            if branch.is_empty()
                || !self.bindings_cover_all_paths_to_callsite(
                    &branch,
                    self.command_for_name_word_span(span)
                        .map_or(span, |command| command.span()),
                )
            {
                return Vec::new();
            }
            bindings.extend(branch);
        }

        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn visible_bindings_for_name_without_helpers(&self, name: &Name, at: Span) -> Vec<BindingId> {
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

        bindings
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

    fn called_helper_bindings_for_name(&mut self, name: &Name, at: Span) -> Vec<BindingId> {
        let mut bindings = self
            .semantic
            .ancestor_scopes(self.semantic.scope_at(at.start.offset))
            .flat_map(|scope| self.called_helper_bindings_before(name, scope, at))
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn top_level_transitive_helper_bindings_before(&self, name: &Name, at: Span) -> Vec<BindingId> {
        if self.enclosing_function_scope_at(at.start.offset).is_some() {
            return Vec::new();
        }

        let mut bindings = Vec::new();
        let mut seen_scopes = FxHashSet::default();
        self.collect_transitive_helper_bindings_before(
            name,
            self.semantic.scope_at(at.start.offset),
            at.start.offset,
            &mut seen_scopes,
            &mut bindings,
        );
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn called_helper_bindings_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let key = (name.clone(), scope, FactSpan::new(at));
        if let Some(cached) = self.helper_binding_memo.get(&key) {
            return cached.to_vec();
        }
        if !self.helper_binding_visiting.insert(key.clone()) {
            return Vec::new();
        }

        let mut bindings = self
            .helper_bindings_called_in_scope_before(name, scope, at)
            .into_iter()
            .collect::<FxHashSet<_>>();

        if let Some(caller_bindings) = self.helper_bindings_reaching_all_callers(name, scope) {
            bindings.extend(caller_bindings);
        }

        self.helper_binding_visiting.remove(&key);
        let mut bindings = bindings.into_iter().collect::<Vec<_>>();
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        self.helper_binding_memo
            .insert(key, bindings.clone().into_boxed_slice());
        bindings
    }

    fn helper_bindings_reaching_all_callers(
        &mut self,
        name: &Name,
        scope: ScopeId,
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
                .caller_branch_bindings_before(name, site.scope, site.span)
                .into_iter()
                .collect::<FxHashSet<_>>();
            if branch.is_empty() {
                return Some(FxHashSet::default());
            }
            union.extend(branch);
        }

        saw_caller.then_some(union)
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
            .any(|binding_id| self.semantic.binding(binding_id).scope != helper_scope)
        {
            return true;
        }

        let caller_sites = self.named_function_call_sites(helper_scope);
        if caller_sites.is_empty() {
            return true;
        }

        caller_sites.into_iter().all(|(scope, span)| {
            let branch = self.caller_branch_bindings_before(name, scope, span);
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

            direct_branch.is_empty()
                || self.bindings_cover_all_paths_to_callsite(
                    &direct_branch,
                    self.command_for_name_word_span(span)
                        .map_or(span, |command| command.span()),
                )
        })
    }

    fn named_function_call_sites(&self, scope: ScopeId) -> Vec<(ScopeId, Span)> {
        let Some(function_kind) = self.named_function_kind(scope) else {
            return Vec::new();
        };

        let mut caller_sites = Vec::new();
        let mut seen_sites = FxHashSet::default();
        for function_name in function_kind.static_names() {
            for site in self.semantic.call_sites_for(function_name) {
                if site.scope == scope {
                    continue;
                }
                if seen_sites.insert((site.scope, site.span.start.offset, site.span.end.offset)) {
                    caller_sites.push((site.scope, site.span));
                }
            }
        }

        caller_sites
    }

    fn caller_branch_bindings_before(
        &mut self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut branch = self.visible_bindings_for_name_without_helpers(name, at);
        branch.extend(self.caller_visible_bindings_before(name, scope, at));
        branch.extend(self.called_helper_bindings_before(name, scope, at));
        branch.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        branch.dedup();
        branch
    }

    fn caller_visible_bindings_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let visible_scopes = self
            .semantic
            .ancestor_scopes(scope)
            .collect::<FxHashSet<_>>();
        let mut bindings = self
            .semantic
            .bindings_for(name)
            .iter()
            .copied()
            .filter(|binding_id| {
                let binding = self.semantic.binding(*binding_id);
                visible_scopes.contains(&binding.scope)
                    && binding.span.end.offset <= at.start.offset
            })
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding_id| self.semantic.binding(*binding_id).span.start.offset);
        bindings.dedup();
        bindings
    }

    fn helper_bindings_called_in_scope_before(
        &self,
        name: &Name,
        scope: ScopeId,
        at: Span,
    ) -> Vec<BindingId> {
        let mut bindings = Vec::new();

        for callee_scope in self.helper_scopes_providing_name(name) {
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

    fn collect_transitive_helper_bindings_before(
        &self,
        name: &Name,
        scope: ScopeId,
        limit_offset: usize,
        seen_scopes: &mut FxHashSet<ScopeId>,
        bindings: &mut Vec<BindingId>,
    ) {
        for callee_scope in self.direct_called_function_scopes_before(scope, limit_offset) {
            if !seen_scopes.insert(callee_scope) {
                continue;
            }

            bindings.extend(self.semantic.bindings_for(name).iter().copied().filter(
                |binding_id| {
                    let binding = self.semantic.binding(*binding_id);
                    binding.scope == callee_scope
                        && !binding.attributes.contains(BindingAttributes::LOCAL)
                },
            ));

            if let Some(limit_offset) = self.function_scope_end_offset(callee_scope) {
                self.collect_transitive_helper_bindings_before(
                    name,
                    callee_scope,
                    limit_offset,
                    seen_scopes,
                    bindings,
                );
            }
        }
    }

    fn direct_called_function_scopes_before(
        &self,
        scope: ScopeId,
        limit_offset: usize,
    ) -> Vec<ScopeId> {
        let mut scopes = Vec::new();
        let mut seen_scopes = FxHashSet::default();

        for header in self.facts.function_headers() {
            let Some(callee_scope) = header.function_scope() else {
                continue;
            };
            let Some(function_kind) = self.named_function_kind(callee_scope) else {
                continue;
            };
            let Some(definition_command) = self.function_definition_command(header.function())
            else {
                continue;
            };

            let called_before = function_kind.static_names().iter().any(|function_name| {
                self.semantic
                    .call_sites_for(function_name)
                    .iter()
                    .any(|site| {
                        site.scope == scope
                            && self.call_site_dominates_offset(site.span, limit_offset)
                            && self.definition_command_resolves_at_call(
                                definition_command.id(),
                                site.span,
                            )
                    })
            });
            if called_before && seen_scopes.insert(callee_scope) {
                scopes.push(callee_scope);
            }
        }

        scopes
    }

    fn function_scope_end_offset(&self, scope: ScopeId) -> Option<usize> {
        self.facts
            .function_headers()
            .iter()
            .find(|header| header.function_scope() == Some(scope))
            .map(|header| header.function().span.end.offset)
    }

    fn call_site_dominates_use(&self, call_span: Span, name: &Name, at: Span) -> bool {
        let _ = name;
        self.call_site_dominates_offset(call_span, at.start.offset)
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
            return !self.binding_is_guarded_before_reference(binding_id, at);
        }

        let cfg = self.analysis.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let mut stack = vec![self.flow_entry_block_for_binding_scopes(
            &[self.semantic.binding(binding_id).scope],
            at.start.offset,
        )];
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

    fn bindings_cover_all_paths_to_reference(
        &self,
        bindings: &[BindingId],
        name: &Name,
        at: Span,
    ) -> bool {
        let Some(reference_id) = self.reference_id_for_name_at(name, at) else {
            return true;
        };
        let Some(reference_block) = self.block_for_reference(reference_id) else {
            return true;
        };

        let cover_blocks = bindings
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding_block = self.block_for_binding(binding_id)?;
                if binding_block == reference_block
                    && self.binding_is_guarded_before_reference(binding_id, at)
                {
                    None
                } else {
                    Some(binding_block)
                }
            })
            .collect::<FxHashSet<_>>();
        if cover_blocks.contains(&reference_block) {
            return true;
        }

        let cfg = self.analysis.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let binding_scopes = bindings
            .iter()
            .copied()
            .map(|binding_id| self.semantic.binding(binding_id).scope)
            .collect::<Vec<_>>();
        let mut stack =
            vec![self.flow_entry_block_for_binding_scopes(&binding_scopes, at.start.offset)];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if cover_blocks.contains(&block_id)
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

    fn bindings_cover_all_paths_to_callsite(
        &self,
        bindings: &[BindingId],
        call_span: Span,
    ) -> bool {
        let cfg = self.analysis.cfg();
        let call_blocks = cfg
            .blocks()
            .iter()
            .filter(|block| block.commands.contains(&call_span))
            .map(|block| block.id)
            .collect::<FxHashSet<_>>();
        if call_blocks.is_empty() {
            return true;
        }

        let cover_blocks = bindings
            .iter()
            .copied()
            .filter_map(|binding_id| {
                let binding_block = self.block_for_binding(binding_id)?;
                if call_blocks.contains(&binding_block)
                    && self.binding_is_guarded_before_reference(binding_id, call_span)
                {
                    None
                } else {
                    Some(binding_block)
                }
            })
            .collect::<FxHashSet<_>>();
        if !cover_blocks.is_disjoint(&call_blocks) {
            return true;
        }

        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let binding_scopes = bindings
            .iter()
            .copied()
            .map(|binding_id| self.semantic.binding(binding_id).scope)
            .collect::<Vec<_>>();
        let mut stack =
            vec![self.flow_entry_block_for_binding_scopes(&binding_scopes, call_span.start.offset)];
        let mut seen = FxHashSet::default();
        while let Some(block_id) = stack.pop() {
            if cover_blocks.contains(&block_id)
                || unreachable.contains(&block_id)
                || !seen.insert(block_id)
            {
                continue;
            }
            if call_blocks.contains(&block_id) {
                return false;
            }
            for (successor, _) in cfg.successors(block_id) {
                stack.push(*successor);
            }
        }

        true
    }

    fn flow_entry_block_for_binding_scopes(
        &self,
        binding_scopes: &[ScopeId],
        reference_offset: usize,
    ) -> BlockId {
        let cfg = self.analysis.cfg();
        self.semantic
            .ancestor_scopes(self.semantic.scope_at(reference_offset))
            .find_map(|scope| {
                if !matches!(
                    self.semantic.scope(scope).kind,
                    ScopeKind::Function(_) | ScopeKind::File
                ) {
                    return None;
                }
                binding_scopes
                    .iter()
                    .copied()
                    .all(|binding_scope| {
                        self.semantic
                            .ancestor_scopes(binding_scope)
                            .any(|ancestor| ancestor == scope)
                    })
                    .then(|| cfg.scope_entry(scope))
                    .flatten()
            })
            .unwrap_or_else(|| cfg.entry())
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

fn span_contains(container: Span, inner: Span) -> bool {
    container.start.offset <= inner.start.offset && inner.end.offset <= container.end.offset
}

fn source_text_needs_parse(text: &str) -> bool {
    text.chars()
        .any(|character| matches!(character, '$' | '`' | '\\' | '\'' | '"'))
}

fn source_text_literal_value(text: &str) -> Option<SourceTextLiteral<'_>> {
    if !source_text_needs_parse(text) {
        return Some(SourceTextLiteral::Bare(text));
    }

    if let Some(inner) = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        && !inner
            .chars()
            .any(|character| matches!(character, '$' | '`' | '\\' | '"'))
    {
        return Some(SourceTextLiteral::Quoted(inner));
    }

    if let Some(inner) = text
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''))
        && !inner.contains('\'')
    {
        return Some(SourceTextLiteral::Quoted(inner));
    }

    None
}

fn build_case_cli_reachable_function_scopes(
    semantic: &SemanticModel,
    facts: &LinterFacts<'_>,
) -> FxHashSet<ScopeId> {
    let dispatcher_offset = facts
        .function_headers()
        .iter()
        .filter_map(|header| {
            let scope = header.function_scope()?;
            facts
                .function_cli_dispatch_facts(scope)
                .dispatcher_span()
                .map(|span| span.start.offset)
        })
        .min();
    let top_level_exit_offset = facts
        .commands()
        .iter()
        .filter(|command| {
            facts.command_parent_id(command.id()).is_none()
                && semantic
                    .ancestor_scopes(semantic.scope_at(command.span().start.offset))
                    .all(|scope| !matches!(semantic.scope(scope).kind, ScopeKind::Function(_)))
                && command_fact_is_standalone_exit(command)
        })
        .map(|command| command.span().start.offset)
        .min();

    facts
        .function_headers()
        .iter()
        .filter_map(|header| {
            let scope = header.function_scope()?;
            let nested = semantic
                .ancestor_scopes(scope)
                .skip(1)
                .any(|ancestor| matches!(semantic.scope(ancestor).kind, ScopeKind::Function(_)));
            (nested
                || dispatcher_offset
                    .is_some_and(|offset| header.function().span.start.offset < offset)
                || top_level_exit_offset
                    .is_some_and(|offset| header.function().span.start.offset < offset))
            .then_some(scope)
        })
        .collect()
}

fn command_fact_is_standalone_exit(command: &crate::facts::CommandFact<'_>) -> bool {
    if command.stmt().negated
        || matches!(
            command.stmt().terminator,
            Some(StmtTerminator::Background(_))
        )
    {
        return false;
    }

    let Command::Builtin(BuiltinCommand::Exit(exit)) = command.command() else {
        return false;
    };
    exit.extra_args.is_empty() && exit.assignments.is_empty() && command.stmt().redirects.is_empty()
}

fn safe_special_parameter(name: &Name) -> bool {
    matches!(name.as_str(), "@" | "#" | "?" | "$" | "!" | "-")
}

fn safe_numeric_shell_variable(name: &Name) -> bool {
    matches!(
        name.as_str(),
        "BASHPID" | "EUID" | "LINENO" | "OPTIND" | "PPID" | "RANDOM" | "SECONDS" | "UID"
    )
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

fn function_has_terminal_exit(function: &FunctionDef) -> bool {
    matches!(
        stmt_terminal_flow_kind(&function.body),
        TerminalFlowKind::Exit
    )
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

fn span_strictly_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset
        && outer.end.offset >= inner.end.offset
        && outer != inner
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, Name, RedirectKind};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use shuck_semantic::{
        BindingOrigin, ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind,
        SemanticBuildOptions, SemanticModel,
    };

    use super::{SafeValueIndex, SafeValueQuery, function_has_terminal_exit};
    use crate::ExpansionContext;
    use crate::LinterFacts;
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

        assert!(!safe_values.word_occurrence_is_safe(short_circuit_word, SafeValueQuery::Argv));
        assert!(safe_values.word_occurrence_is_safe(if_else_word, SafeValueQuery::Argv));
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let definition_span = match semantic.binding(binding_id).origin {
            BindingOrigin::LoopVariable {
                definition_span, ..
            } => definition_span,
            ref other => panic!("expected loop-variable binding, got {other:?}"),
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Bash);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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

        assert!(!safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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

        assert!(safe_values.word_occurrence_is_safe(word_fact, SafeValueQuery::Argv));
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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

        safe_values
            .maybe_uninitialized_refs
            .insert(crate::FactSpan::new(part_span));

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);

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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
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
        let semantic = SemanticModel::build(&output.file, source, &indexer);
        let analysis = semantic.analysis();
        let file_context = classify_file_context(source, None, ShellDialect::Sh);
        let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
        let mut safe_values = SafeValueIndex::build(&semantic, &analysis, &facts, source);
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
