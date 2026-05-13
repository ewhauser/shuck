use super::*;

pub(crate) struct CommandFactStore<'a> {
    pub(in crate::facts) commands: Vec<CommandFact<'a>>,
    pub(in crate::facts) command_fact_indices_by_id: Vec<Option<usize>>,
    pub(in crate::facts) structural_command_ids: Vec<CommandId>,
    #[cfg(test)]
    pub(in crate::facts) command_ids_by_span: CommandLookupIndex,
    pub(in crate::facts) command_ids_by_name_word_span: FxHashMap<FactSpan, CommandId>,
    pub(in crate::facts) innermost_command_ids_by_offset: CommandOffsetLookup,
    pub(in crate::facts) innermost_command_ids_by_binding_offset: CommandOffsetLookup,
    pub(in crate::facts) command_dominance_barrier_flags: Vec<bool>,
    pub(in crate::facts) if_condition_command_ids: DenseCommandIdSet,
    pub(in crate::facts) elif_condition_command_ids: DenseCommandIdSet,
    pub(in crate::facts) fact_store: FactStore<'a>,
    pub(in crate::facts) redundant_echo_space_facts: OnceLock<Vec<RedundantEchoSpaceFact>>,
    pub(in crate::facts) completion_registered_function_command_flags: Vec<bool>,
    pub(in crate::facts) completion_registered_function_scopes: FxHashSet<ScopeId>,
    pub(in crate::facts) external_entrypoint_function_scopes: FxHashSet<ScopeId>,
    pub(in crate::facts) function_headers: Vec<FunctionHeaderFact<'a>>,
    pub(in crate::facts) function_doc_content: OnceLock<Vec<FunctionDocContentFact>>,
    pub(in crate::facts) function_definition_command_ids_by_scope: FxHashMap<ScopeId, CommandId>,
    pub(in crate::facts) case_cli_reachable_function_scopes: FxHashSet<ScopeId>,
    pub(in crate::facts) function_in_alias_spans: Vec<Span>,
    pub(in crate::facts) alias_definition_expansion_spans: Vec<Span>,
    pub(in crate::facts) function_body_without_braces_spans: Vec<Span>,
    pub(in crate::facts) function_parameter_fallback_spans: Vec<Span>,
    pub(in crate::facts) redundant_return_status_spans: Vec<Span>,
    pub(in crate::facts) for_headers: Vec<ForHeaderFact<'a>>,
    pub(in crate::facts) select_headers: Vec<SelectHeaderFact<'a>>,
    pub(in crate::facts) case_items: Vec<CaseItemFact<'a>>,
    pub(in crate::facts) case_pattern_shadows: Vec<CasePatternShadowFact>,
    pub(in crate::facts) case_pattern_impossible_spans: Vec<Span>,
    pub(in crate::facts) case_pattern_expansions: Vec<CasePatternExpansionFact>,
    pub(in crate::facts) getopts_cases: Vec<GetoptsCaseFact>,
    pub(in crate::facts) pipelines: Vec<PipelineFact<'a>>,
    pub(in crate::facts) lists: Vec<ListFact<'a>>,
    pub(in crate::facts) tautology_chain_operator_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) statement_facts: Vec<StatementFact>,
    pub(in crate::facts) background_semicolon_spans: Vec<Span>,
    pub(in crate::facts) single_test_subshell_spans: Vec<Span>,
    pub(in crate::facts) subshell_test_group_spans: Vec<Span>,
    pub(in crate::facts) function_positional_parameter_facts:
        FxHashMap<ScopeId, FunctionPositionalParameterFacts>,
    pub(in crate::facts) function_cli_dispatch_facts: FxHashMap<ScopeId, FunctionCliDispatchFacts>,
    pub(in crate::facts) condition_status_capture_spans: Vec<Span>,
    pub(in crate::facts) command_substitution_command_spans: Vec<Span>,
    pub(in crate::facts) backtick_command_name_spans: Vec<Span>,
    pub(in crate::facts) extra_masked_return_declaration_facts:
        OnceLock<Vec<ExtraMaskedReturnDeclarationFact>>,
    pub(in crate::facts) assignment_spacing_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) missing_space_before_bracket_close_facts: OnceLock<Vec<(Span, usize)>>,
    pub(in crate::facts) jammed_test_bracket_facts: OnceLock<Vec<(Span, usize)>>,
    pub(in crate::facts) assignment_like_command_name_spans: Vec<Span>,
    pub(in crate::facts) assign_special_zero_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) spacey_assignment_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) bare_command_name_assignment_spans: Vec<Span>,
}

pub(crate) struct WordFactStore<'a> {
    pub(in crate::facts) plain_unindexed_array_references:
        OnceLock<Vec<PlainUnindexedArrayReferenceFact>>,
    pub(in crate::facts) suppressed_subscript_reference_spans: FxHashSet<FactSpan>,
    #[cfg(test)]
    pub(in crate::facts) subscript_later_suppression_reference_spans: FxHashSet<FactSpan>,
    pub(in crate::facts) compound_assignment_value_word_flags: Box<[bool]>,
    pub(in crate::facts) word_nodes: Vec<WordNode<'a>>,
    pub(in crate::facts) word_occurrences: Vec<WordOccurrence>,
    pub(in crate::facts) word_index: FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    pub(in crate::facts) array_assignment_split_word_ids: Vec<WordOccurrenceId>,
    pub(in crate::facts) brace_variable_before_bracket_spans: Vec<Span>,
    pub(in crate::facts) bare_done_word_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) array_index_arithmetic_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_score_line_spans: Vec<Span>,
    pub(in crate::facts) dollar_in_arithmetic_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_expansion_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_index_subscript_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_command_substitution_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_only_suppressed_subscript_spans: Vec<Span>,
    pub(in crate::facts) single_quoted_fragments: Vec<SingleQuotedFragmentFact>,
    pub(in crate::facts) dollar_double_quoted_fragments: Vec<DollarDoubleQuotedFragmentFact>,
    pub(in crate::facts) open_double_quote_fragments: Vec<OpenDoubleQuoteFragmentFact>,
    pub(in crate::facts) suspect_closing_quote_fragments: Vec<SuspectClosingQuoteFragmentFact>,
    pub(in crate::facts) literal_brace_spans: Vec<Span>,
    pub(in crate::facts) backtick_fragments: Vec<BacktickFragmentFact>,
    pub(in crate::facts) legacy_arithmetic_fragments: Vec<LegacyArithmeticFragmentFact>,
    pub(in crate::facts) positional_parameter_fragments: Vec<PositionalParameterFragmentFact>,
    pub(in crate::facts) positional_parameter_operator_spans: Vec<Span>,
    pub(in crate::facts) double_paren_grouping_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_update_operator_spans: Vec<Span>,
    pub(in crate::facts) arithmetic_update_operator_fix_facts: Vec<ArithmeticUpdateOperatorFixFact>,
    pub(in crate::facts) arithmetic_literal_facts: Vec<ArithmeticLiteralFact>,
    pub(in crate::facts) escape_scan_matches: Vec<EscapeScanMatch>,
    pub(in crate::facts) echo_backslash_escape_word_spans: Vec<Span>,
    pub(in crate::facts) echo_to_sed_substitution_spans: Vec<Span>,
    pub(in crate::facts) unicode_smart_quote_spans: Vec<Span>,
    #[cfg(test)]
    pub(in crate::facts) pattern_literal_spans: Vec<Span>,
    pub(in crate::facts) pattern_charclass_spans: Vec<Span>,
    pub(in crate::facts) nested_parameter_expansion_fragments:
        Vec<NestedParameterExpansionFragmentFact>,
    pub(in crate::facts) indirect_expansion_fragments: Vec<IndirectExpansionFragmentFact>,
    pub(in crate::facts) indexed_array_reference_fragments: Vec<IndexedArrayReferenceFragmentFact>,
    pub(in crate::facts) plain_unindexed_reference_spans: Vec<Span>,
    pub(in crate::facts) parameter_pattern_special_target_fragments:
        Vec<ParameterPatternSpecialTargetFragmentFact>,
    pub(in crate::facts) zsh_parameter_index_flag_fragments: Vec<ZshParameterIndexFlagFragmentFact>,
    pub(in crate::facts) substring_expansion_fragments: Vec<SubstringExpansionFragmentFact>,
    pub(in crate::facts) case_modification_fragments: Vec<CaseModificationFragmentFact>,
    pub(in crate::facts) replacement_expansion_fragments: Vec<ReplacementExpansionFragmentFact>,
    pub(in crate::facts) positional_parameter_trim_fragments:
        Vec<PositionalParameterTrimFragmentFact>,
}

pub(crate) struct AssignmentFactStore<'a> {
    pub(in crate::facts) assignment_value_target_index: AssignmentValueTargetIndex,
    pub(in crate::facts) binding_values: FxHashMap<BindingId, BindingValueFact<'a>>,
    pub(in crate::facts) broken_assoc_key_spans: Vec<Span>,
    pub(in crate::facts) comma_array_assignment_spans: Vec<Span>,
    pub(in crate::facts) ifs_literal_backslash_assignment_value_spans: Vec<Span>,
    pub(in crate::facts) env_prefix_assignment_scope_spans: Vec<Span>,
    pub(in crate::facts) env_prefix_expansion_scope_spans: Vec<Span>,
    pub(in crate::facts) env_prefix_expansion_fix_facts: Vec<EnvPrefixExpansionFixFact>,
    pub(in crate::facts) unset_command_ids_by_target_name: FxHashMap<Name, Vec<CommandId>>,
    pub(in crate::facts) function_unset_command_ids_by_target_name: FxHashMap<Name, Vec<CommandId>>,
    pub(in crate::facts) presence_tested_names: FxHashSet<Name>,
    pub(in crate::facts) nested_presence_test_spans: FxHashMap<Name, Vec<Span>>,
    pub(in crate::facts) c006_presence_tested_names: FxHashSet<Name>,
    pub(in crate::facts) c006_nested_presence_test_spans: FxHashMap<Name, Vec<Span>>,
    pub(in crate::facts) c006_suppressing_reference_offsets_by_name: FxHashMap<Name, Vec<usize>>,
    pub(in crate::facts) presence_test_references_by_name:
        FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    pub(in crate::facts) presence_test_names_by_name: FxHashMap<Name, Vec<PresenceTestNameFact>>,
    pub(in crate::facts) possible_variable_misspelling_use_scan: OnceLock<bool>,
    pub(in crate::facts) possible_variable_misspelling_index:
        OnceLock<PossibleVariableMisspellingIndex>,
    pub(in crate::facts) subshell_assignment_sites: Vec<NamedSpan>,
    pub(in crate::facts) subshell_later_use_sites: Vec<NamedSpan>,
    pub(in crate::facts) plus_equals_assignment_spans: Vec<Span>,
}

pub(crate) struct SourceFactStore<'a> {
    pub(in crate::facts) source: &'a str,
    pub(in crate::facts) line_index: &'a LineIndex,
    pub(in crate::facts) comment_index: &'a CommentIndex,
    pub(in crate::facts) shell: ShellDialect,
    pub(in crate::facts) script_line_count: OnceLock<ScriptLineCountFact>,
    pub(in crate::facts) indented_shebang_span: Option<Span>,
    pub(in crate::facts) indented_shebang_indent_span: Option<Span>,
    pub(in crate::facts) space_after_hash_bang_span: Option<Span>,
    pub(in crate::facts) space_after_hash_bang_whitespace_span: Option<Span>,
    pub(in crate::facts) shebang_not_on_first_line_span: Option<Span>,
    pub(in crate::facts) shebang_not_on_first_line_fix_span: Option<Span>,
    pub(in crate::facts) shebang_not_on_first_line_preferred_newline: Option<&'static str>,
    pub(in crate::facts) missing_shebang_line_span: Option<Span>,
    pub(in crate::facts) duplicate_shebang_flag_span: Option<Span>,
    pub(in crate::facts) non_absolute_shebang_span: Option<Span>,
    pub(in crate::facts) shebang_interpreter: OnceLock<Option<ShebangInterpreterFact>>,
    pub(in crate::facts) shebang_invocation: OnceLock<Option<ShebangInvocationFact>>,
    pub(in crate::facts) missing_file_description_comment:
        OnceLock<Option<FileDescriptionCommentFact>>,
    pub(in crate::facts) errexit_enabled_anywhere: bool,
    pub(in crate::facts) region_index: &'a RegionIndex,
    pub(in crate::facts) commented_continuation_comment_spans: Vec<Span>,
    pub(in crate::facts) comment_double_quote_nesting_spans: Vec<Span>,
    pub(in crate::facts) escaped_dash_command_name_spans: OnceLock<Vec<Span>>,
    pub(in crate::facts) trailing_directive_comment_spans: Vec<Span>,
    pub(in crate::facts) todo_comment_facts: OnceLock<Vec<TodoCommentFact>>,
    pub(in crate::facts) backtick_substitution_spans: Vec<Span>,
    pub(in crate::facts) backtick_escaped_parameters: Vec<BacktickEscapedParameter>,
    pub(in crate::facts) backtick_escaped_parameter_reference_spans: Vec<Span>,
    pub(in crate::facts) backtick_double_escaped_parameter_spans: Vec<Span>,
    pub(in crate::facts) dollar_question_after_command_spans: Vec<Span>,
    pub(in crate::facts) unused_heredoc_spans: Vec<Span>,
    pub(in crate::facts) heredoc_missing_end_spans: Vec<Span>,
    pub(in crate::facts) heredoc_closer_not_alone_spans: Vec<Span>,
    pub(in crate::facts) misquoted_heredoc_close_spans: Vec<Span>,
    pub(in crate::facts) heredoc_end_space_spans: Vec<Span>,
    pub(in crate::facts) indented_heredoc_close_facts: OnceLock<Vec<(Span, Span)>>,
    pub(in crate::facts) echo_here_doc_spans: Vec<Span>,
    pub(in crate::facts) spaced_tabstrip_close_spans: Vec<Span>,
}

pub(crate) struct CompatFactStore {
    pub(in crate::facts) possible_variable_misspelling_scope_compat_name_uses:
        OnceLock<Vec<ComparableNameUse>>,
    pub(in crate::facts) conditional_portability: ConditionalPortabilityFacts,
}

pub struct LinterFacts<'a> {
    pub(in crate::facts) semantic: &'a SemanticModel,
    pub(in crate::facts) semantic_artifacts: &'a LinterSemanticArtifacts<'a>,
    pub(in crate::facts) command: CommandFactStore<'a>,
    pub(in crate::facts) words: WordFactStore<'a>,
    pub(in crate::facts) assignments: AssignmentFactStore<'a>,
    pub(in crate::facts) source_facts: SourceFactStore<'a>,
    pub(in crate::facts) compat: CompatFactStore,
}

#[derive(Clone, Copy)]
pub struct CommandFactQueries<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
}

#[derive(Clone, Copy)]
pub struct WordFacts<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
}

#[derive(Clone, Copy)]
pub struct AssignmentFacts<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
}

#[derive(Clone, Copy)]
pub struct SourceFacts<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
}

#[derive(Clone, Copy)]
pub struct CompatFacts<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptLineCountFact {
    pub(in crate::facts) physical_lines: usize,
    pub(in crate::facts) non_comment_non_blank_lines: usize,
    pub(in crate::facts) report_span: Span,
}

impl ScriptLineCountFact {
    pub fn physical_lines(self) -> usize {
        self.physical_lines
    }

    pub fn non_comment_non_blank_lines(self) -> usize {
        self.non_comment_non_blank_lines
    }

    pub fn report_span(self) -> Span {
        self.report_span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDescriptionCommentFact {
    pub(in crate::facts) span: Span,
    pub(in crate::facts) shebang_only_file: bool,
}

impl FileDescriptionCommentFact {
    pub fn span(self) -> Span {
        self.span
    }

    pub fn is_shebang_only_file(self) -> bool {
        self.shebang_only_file
    }
}

impl<'a> LinterFacts<'a> {
    pub fn build(
        file: &'a File,
        source: &'a str,
        semantic: &'a LinterSemanticArtifacts<'a>,
        indexer: &'a Indexer,
    ) -> Self {
        Self::build_with_ambient_shell_options(
            file,
            source,
            semantic,
            indexer,
            AmbientShellOptions::default(),
        )
    }

    pub fn build_with_ambient_shell_options(
        file: &'a File,
        source: &'a str,
        semantic: &'a LinterSemanticArtifacts<'a>,
        indexer: &'a Indexer,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        Self::build_with_shell_and_ambient_shell_options(
            file,
            source,
            semantic,
            indexer,
            ShellDialect::Unknown,
            ambient_shell_options,
        )
    }

    pub fn build_with_shell_and_ambient_shell_options(
        file: &'a File,
        source: &'a str,
        semantic: &'a LinterSemanticArtifacts<'a>,
        indexer: &'a Indexer,
        shell: ShellDialect,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        let semantic_analysis = semantic.semantic().analysis();
        Self::build_with_semantic_analysis_shell_and_ambient_shell_options(
            file,
            source,
            semantic,
            &semantic_analysis,
            indexer,
            shell,
            ambient_shell_options,
        )
    }

    pub(crate) fn build_with_semantic_analysis_shell_and_ambient_shell_options(
        file: &'a File,
        source: &'a str,
        semantic: &'a LinterSemanticArtifacts<'a>,
        semantic_analysis: &SemanticAnalysis<'a>,
        indexer: &'a Indexer,
        shell: ShellDialect,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        LinterFactsBuilder::new(
            file,
            source,
            semantic,
            semantic_analysis,
            indexer,
            shell,
            ambient_shell_options,
        )
        .build()
    }

    pub fn commands(&self) -> CommandFacts<'_, 'a> {
        CommandFacts::new(
            &self.command.commands,
            &self.command.fact_store,
            &self.command.command_fact_indices_by_id,
        )
    }

    pub fn command_facts(&self) -> CommandFactQueries<'_, 'a> {
        CommandFactQueries { facts: self }
    }

    pub fn words(&self) -> WordFacts<'_, 'a> {
        WordFacts { facts: self }
    }

    pub fn assignments(&self) -> AssignmentFacts<'_, 'a> {
        AssignmentFacts { facts: self }
    }

    pub fn source_facts(&self) -> SourceFacts<'_, 'a> {
        SourceFacts { facts: self }
    }

    pub fn compat(&self) -> CompatFacts<'_, 'a> {
        CompatFacts { facts: self }
    }
}

impl<'facts, 'a> CommandFactQueries<'facts, 'a> {
    pub(crate) fn function_positional_parameter_facts(
        self,
        scope: ScopeId,
    ) -> FunctionPositionalParameterFacts {
        self.facts
            .command
            .function_positional_parameter_facts
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn function_cli_dispatch_facts(self, scope: ScopeId) -> FunctionCliDispatchFacts {
        self.facts
            .command
            .function_cli_dispatch_facts
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub fn structural_commands(self) -> impl Iterator<Item = CommandFactRef<'facts, 'a>> + 'facts {
        self.facts
            .command
            .structural_command_ids
            .iter()
            .copied()
            .map(move |id| self.command(id))
    }

    pub(crate) fn extra_masked_return_declaration_facts(
        self,
    ) -> &'facts [ExtraMaskedReturnDeclarationFact] {
        self.facts
            .command
            .extra_masked_return_declaration_facts
            .get_or_init(|| build_extra_masked_return_declaration_facts(self.facts))
    }

    pub(crate) fn command(self, id: CommandId) -> CommandFactRef<'facts, 'a> {
        let index = self
            .facts
            .command
            .command_fact_indices_by_id
            .get(id.index())
            .and_then(|index| *index)
            .unwrap_or_else(|| panic!("command id {} must exist", id.index()));

        CommandFactRef::new(
            &self.facts.command.commands[index],
            &self.facts.command.fact_store,
        )
    }

    pub(crate) fn command_for_name_word_span(
        self,
        span: Span,
    ) -> Option<CommandFactRef<'facts, 'a>> {
        self.facts
            .command
            .command_ids_by_name_word_span
            .get(&FactSpan::new(span))
            .copied()
            .or_else(|| self.command_id_for_exact_span(span))
            .map(|id| self.command(id))
    }

    pub(crate) fn innermost_command_at(self, offset: usize) -> Option<CommandFactRef<'facts, 'a>> {
        self.innermost_command_id_at(offset)
            .map(|id| self.command(id))
    }

    pub(crate) fn expansion_behavior_at(self, offset: usize) -> ShellBehaviorAt<'a> {
        self.innermost_command_at(offset).map_or_else(
            || self.facts.semantic.shell_behavior_at(offset),
            |command| command.shell_behavior().clone(),
        )
    }

    pub(crate) fn innermost_command_id_at(self, offset: usize) -> Option<CommandId> {
        precomputed_command_id_for_offset(
            &self.facts.command.innermost_command_ids_by_offset,
            offset,
        )
    }

    pub(crate) fn innermost_command_id_containing_offset(self, offset: usize) -> Option<CommandId> {
        self.facts
            .semantic
            .innermost_command_id_containing_offset(offset)
            .filter(|id| {
                self.facts
                    .command
                    .command_fact_indices_by_id
                    .get(id.index())
                    .is_some_and(Option::is_some)
            })
    }

    pub(crate) fn innermost_command_at_binding_offset(
        self,
        offset: usize,
    ) -> Option<CommandFactRef<'facts, 'a>> {
        precomputed_command_id_for_offset(
            &self.facts.command.innermost_command_ids_by_binding_offset,
            offset,
        )
        .map(|id| self.command(id))
    }

    pub(crate) fn command_parent_id(self, id: CommandId) -> Option<CommandId> {
        self.facts
            .semantic
            .syntax_backed_command_parent_id(id)
            .filter(|parent_id| {
                self.facts
                    .command
                    .command_fact_indices_by_id
                    .get(parent_id.index())
                    .is_some_and(Option::is_some)
            })
    }

    #[cfg(test)]
    pub(crate) fn command_parent(self, id: CommandId) -> Option<CommandFactRef<'facts, 'a>> {
        self.command_parent_id(id)
            .map(|parent_id| self.command(parent_id))
    }

    pub(crate) fn redundant_echo_space_facts(self) -> &'facts [RedundantEchoSpaceFact] {
        self.facts
            .command
            .redundant_echo_space_facts
            .get_or_init(|| build_redundant_echo_space_facts(self.facts))
    }

    pub(crate) fn function_definition_command(
        self,
        scope: ScopeId,
    ) -> Option<CommandFactRef<'facts, 'a>> {
        self.facts
            .command
            .function_definition_command_ids_by_scope
            .get(&scope)
            .copied()
            .map(|id| self.command(id))
    }

    pub(crate) fn is_case_cli_reachable_function_scope(self, scope: ScopeId) -> bool {
        self.facts
            .command
            .case_cli_reachable_function_scopes
            .contains(&scope)
    }

    pub(crate) fn command_is_dominance_barrier(self, id: CommandId) -> bool {
        self.facts
            .command
            .command_dominance_barrier_flags
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn command_id_for_stmt(self, stmt: &Stmt) -> Option<CommandId> {
        self.command_id_for_command(&stmt.command)
    }

    #[cfg(test)]
    pub(crate) fn command_id_for_command(self, command: &Command) -> Option<CommandId> {
        command_id_for_command(command, &self.facts.command.command_ids_by_span)
    }

    fn command_id_for_exact_span(self, span: Span) -> Option<CommandId> {
        let mut current = self.innermost_command_id_at(span.start.offset)?;
        loop {
            if self.command(current).span() == span {
                return Some(current);
            }
            current = self.command_parent_id(current)?;
        }
    }

    pub(crate) fn is_if_condition_command(self, id: CommandId) -> bool {
        self.facts.command.if_condition_command_ids.contains(id)
    }

    pub(crate) fn is_elif_condition_command(self, id: CommandId) -> bool {
        self.facts.command.elif_condition_command_ids.contains(id)
    }

    pub(crate) fn command_is_in_completion_registered_function(self, id: CommandId) -> bool {
        self.facts
            .command
            .completion_registered_function_command_flags
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn function_is_completion_registered(self, scope: ScopeId) -> bool {
        self.facts
            .command
            .completion_registered_function_scopes
            .contains(&scope)
    }

    pub(crate) fn function_is_external_entrypoint(self, scope: ScopeId) -> bool {
        self.facts
            .command
            .external_entrypoint_function_scopes
            .contains(&scope)
    }

    pub(crate) fn function_headers(self) -> &'facts [FunctionHeaderFact<'a>] {
        &self.facts.command.function_headers
    }

    pub(crate) fn function_doc_content(self) -> &'facts [FunctionDocContentFact] {
        self.facts.command.function_doc_content.get_or_init(|| {
            build_function_doc_content_facts(
                self.facts.semantic,
                &self.facts.command.function_headers,
                &self.facts.command.commands,
                &self.facts.command.function_positional_parameter_facts,
                self.facts.source_facts.source,
                self.facts.source_facts.line_index,
                self.facts.source_facts.comment_index,
            )
        })
    }

    pub(crate) fn fork_bomb_pattern_spans(self) -> Vec<Span> {
        self.facts
            .command
            .function_headers
            .iter()
            .filter_map(|function| {
                let (name, _) = function.static_name_entry()?;
                let scope = function.function_scope()?;
                self.function_has_background_self_pipe(name.as_str(), scope)
                    .then(|| function.span_in_source(self.facts.source_facts.source))
            })
            .collect()
    }

    fn function_has_background_self_pipe(self, name: &str, scope: ScopeId) -> bool {
        self.facts.command.commands.iter().any(|command| {
            command.enclosing_function_scope() == Some(scope)
                && stmt_is_plain_background(command.stmt())
                && match command.command() {
                    Command::Binary(binary) => binary_is_two_segment_self_pipe(
                        binary,
                        name,
                        self.facts.source_facts.source,
                    ),
                    _ => false,
                }
        })
    }

    pub(crate) fn function_in_alias_spans(self) -> &'facts [Span] {
        &self.facts.command.function_in_alias_spans
    }

    pub(crate) fn alias_definition_expansion_spans(self) -> &'facts [Span] {
        &self.facts.command.alias_definition_expansion_spans
    }

    pub(crate) fn function_body_without_braces_spans(self) -> &'facts [Span] {
        &self.facts.command.function_body_without_braces_spans
    }

    pub(crate) fn function_parameter_fallback_spans(self) -> &'facts [Span] {
        &self.facts.command.function_parameter_fallback_spans
    }

    pub(crate) fn redundant_return_status_spans(self) -> &'facts [Span] {
        &self.facts.command.redundant_return_status_spans
    }

    pub(crate) fn for_headers(self) -> &'facts [ForHeaderFact<'a>] {
        &self.facts.command.for_headers
    }

    pub(crate) fn select_headers(self) -> &'facts [SelectHeaderFact<'a>] {
        &self.facts.command.select_headers
    }

    pub(crate) fn case_items(self) -> &'facts [CaseItemFact<'a>] {
        &self.facts.command.case_items
    }

    pub(crate) fn case_pattern_shadows(self) -> &'facts [CasePatternShadowFact] {
        &self.facts.command.case_pattern_shadows
    }

    pub(crate) fn case_pattern_impossible_spans(self) -> &'facts [Span] {
        &self.facts.command.case_pattern_impossible_spans
    }

    pub(crate) fn case_pattern_expansions(self) -> &'facts [CasePatternExpansionFact] {
        &self.facts.command.case_pattern_expansions
    }

    pub(crate) fn getopts_cases(self) -> &'facts [GetoptsCaseFact] {
        &self.facts.command.getopts_cases
    }

    pub(crate) fn pipelines(self) -> &'facts [PipelineFact<'a>] {
        &self.facts.command.pipelines
    }

    pub(crate) fn lists(self) -> &'facts [ListFact<'a>] {
        &self.facts.command.lists
    }

    pub(crate) fn tautology_chain_operator_spans(self) -> &'facts [Span] {
        self.facts
            .command
            .tautology_chain_operator_spans
            .get_or_init(|| {
                build_tautology_chain_operator_spans(
                    &self.facts.command.commands,
                    &self.facts.command.command_fact_indices_by_id,
                    &self.facts.command.lists,
                    self.facts.source_facts.source,
                )
            })
    }

    pub(crate) fn duplicate_redirect_spans(self) -> Vec<Span> {
        let source = self.facts.source_facts.source;
        self.facts
            .commands()
            .iter()
            .flat_map(|command| duplicate_redirect_spans(command.redirect_facts(), source))
            .collect()
    }

    pub(crate) fn statement_facts(self) -> &'facts [StatementFact] {
        &self.facts.command.statement_facts
    }

    pub(crate) fn background_semicolon_spans(self) -> &'facts [Span] {
        &self.facts.command.background_semicolon_spans
    }

    pub(crate) fn single_test_subshell_spans(self) -> &'facts [Span] {
        &self.facts.command.single_test_subshell_spans
    }

    pub(crate) fn subshell_test_group_spans(self) -> &'facts [Span] {
        &self.facts.command.subshell_test_group_spans
    }

    pub(crate) fn condition_status_capture_spans(self) -> &'facts [Span] {
        &self.facts.command.condition_status_capture_spans
    }

    pub(crate) fn command_substitution_command_spans(self) -> &'facts [Span] {
        &self.facts.command.command_substitution_command_spans
    }

    pub(crate) fn backtick_command_name_spans(self) -> &'facts [Span] {
        &self.facts.command.backtick_command_name_spans
    }

    pub(crate) fn assignment_spacing_spans(self) -> &'facts [Span] {
        self.facts.command.assignment_spacing_spans.get_or_init(|| {
            let source = self.facts.source_facts.source;
            if source_may_have_assignment_spacing_candidate(source)
                && self.facts.command.commands.iter().any(|command| {
                    command_may_have_assignment_spacing_candidate(command.command(), source)
                })
            {
                build_assignment_spacing_spans(&self.facts.command.commands, source)
            } else {
                Vec::new()
            }
        })
    }

    pub(crate) fn missing_space_before_bracket_close_facts(self) -> &'facts [(Span, usize)] {
        self.facts
            .command
            .missing_space_before_bracket_close_facts
            .get_or_init(|| {
                let locator = Locator::new(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                );
                self.facts
                    .command
                    .commands
                    .iter()
                    .filter_map(|command| {
                        build_missing_space_before_bracket_close_fact(
                            command.command(),
                            self.facts.source_facts.source,
                            locator,
                        )
                    })
                    .collect()
            })
    }

    pub(crate) fn jammed_test_bracket_facts(self) -> &'facts [(Span, usize)] {
        self.facts
            .command
            .jammed_test_bracket_facts
            .get_or_init(|| {
                self.facts
                    .command
                    .commands
                    .iter()
                    .filter_map(|command| {
                        build_jammed_test_bracket_fact(
                            command.command(),
                            self.facts.source_facts.source,
                        )
                    })
                    .collect()
            })
    }

    pub(crate) fn assignment_like_command_name_spans(self) -> &'facts [Span] {
        &self.facts.command.assignment_like_command_name_spans
    }

    pub(crate) fn assign_special_zero_spans(self) -> &'facts [Span] {
        self.facts
            .command
            .assign_special_zero_spans
            .get_or_init(|| {
                build_assign_special_zero_spans(
                    &self.facts.command.commands,
                    self.facts.source_facts.source,
                )
            })
    }

    pub(crate) fn spacey_assignment_spans(self) -> &'facts [Span] {
        self.facts.command.spacey_assignment_spans.get_or_init(|| {
            build_spacey_assignment_spans(
                &self.facts.command.commands,
                self.facts.source_facts.source,
            )
        })
    }

    pub(crate) fn bare_command_name_assignment_spans(self) -> &'facts [Span] {
        &self.facts.command.bare_command_name_assignment_spans
    }
}

impl<'facts, 'a> AssignmentFacts<'facts, 'a> {
    pub(crate) fn binding_value(
        self,
        binding_id: BindingId,
    ) -> Option<&'facts BindingValueFact<'a>> {
        self.facts.assignments.binding_values.get(&binding_id)
    }

    pub(crate) fn broken_assoc_key_spans(self) -> &'facts [Span] {
        &self.facts.assignments.broken_assoc_key_spans
    }

    pub(crate) fn comma_array_assignment_spans(self) -> &'facts [Span] {
        &self.facts.assignments.comma_array_assignment_spans
    }

    pub(crate) fn ifs_literal_backslash_assignment_value_spans(self) -> &'facts [Span] {
        &self
            .facts
            .assignments
            .ifs_literal_backslash_assignment_value_spans
    }

    pub(crate) fn env_prefix_assignment_scope_spans(self) -> &'facts [Span] {
        &self.facts.assignments.env_prefix_assignment_scope_spans
    }

    pub(crate) fn env_prefix_expansion_scope_spans(self) -> &'facts [Span] {
        &self.facts.assignments.env_prefix_expansion_scope_spans
    }

    pub(crate) fn env_prefix_expansion_fix_facts(self) -> &'facts [EnvPrefixExpansionFixFact] {
        &self.facts.assignments.env_prefix_expansion_fix_facts
    }

    pub(crate) fn unset_commands_for_name(
        self,
        name: &Name,
    ) -> impl Iterator<Item = CommandFactRef<'facts, 'a>> + 'facts {
        self.facts
            .assignments
            .unset_command_ids_by_target_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .map(move |id| self.facts.command_facts().command(id))
    }

    pub(crate) fn function_unset_commands_for_name(
        self,
        name: &Name,
    ) -> impl Iterator<Item = CommandFactRef<'facts, 'a>> + 'facts {
        self.facts
            .assignments
            .function_unset_command_ids_by_target_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .map(move |id| self.facts.command_facts().command(id))
    }

    pub(crate) fn possible_variable_misspelling_candidate(
        self,
        semantic: &SemanticModel,
        target_name: &str,
    ) -> Option<String> {
        if *self
            .facts
            .assignments
            .possible_variable_misspelling_use_scan
            .get_or_init(|| {
                should_scan_possible_variable_misspelling_candidates(
                    semantic,
                    &self.facts.assignments.presence_test_references_by_name,
                    &self.facts.assignments.presence_test_names_by_name,
                )
            })
        {
            return scan_possible_variable_misspelling_candidate(
                semantic,
                &self.facts.assignments.presence_test_references_by_name,
                &self.facts.assignments.presence_test_names_by_name,
                target_name,
            );
        }

        self.facts
            .assignments
            .possible_variable_misspelling_index
            .get_or_init(|| {
                build_possible_variable_misspelling_index(
                    semantic,
                    &self.facts.assignments.presence_test_references_by_name,
                    &self.facts.assignments.presence_test_names_by_name,
                )
            })
            .candidate_name(target_name)
            .map(ToOwned::to_owned)
    }

    pub(crate) fn is_presence_tested_name(self, name: &Name, span: Span) -> bool {
        self.facts.assignments.presence_tested_names.contains(name)
            || self
                .facts
                .assignments
                .nested_presence_test_spans
                .get(name)
                .is_some_and(|spans| {
                    spans
                        .iter()
                        .copied()
                        .any(|outer| contains_span(outer, span))
                })
    }

    pub(crate) fn is_c006_presence_tested_name(self, name: &Name, _span: Span) -> bool {
        self.facts
            .assignments
            .c006_presence_tested_names
            .contains(name)
            || self
                .facts
                .assignments
                .c006_nested_presence_test_spans
                .contains_key(name)
    }

    pub(crate) fn has_prior_c006_suppressing_reference(self, name: &Name, span: Span) -> bool {
        self.facts
            .assignments
            .c006_suppressing_reference_offsets_by_name
            .get(name)
            .is_some_and(|offsets| {
                offsets.partition_point(|offset| *offset < span.start.offset) > 0
            })
    }

    pub(crate) fn assignment_value_target_name_for_span(self, span: Span) -> Option<&'facts Name> {
        let query_start = span.start.offset;
        let query_end = span.end.offset;
        let upper = self
            .facts
            .assignments
            .assignment_value_target_index
            .partition_point(|entry| entry.value_start <= query_start);
        self.facts.assignments.assignment_value_target_index[..upper]
            .iter()
            .rev()
            .find(|entry| entry.value_end >= query_end)
            .map(|entry| &entry.target_name)
    }

    pub(crate) fn presence_test_references(
        self,
        name: &Name,
    ) -> &'facts [PresenceTestReferenceFact] {
        self.facts
            .assignments
            .presence_test_references_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn presence_test_names(self, name: &Name) -> &'facts [PresenceTestNameFact] {
        self.facts
            .assignments
            .presence_test_names_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn presence_test_candidate_spans(
        self,
        semantic: &SemanticModel,
    ) -> Vec<(Name, Span)> {
        let mut names = FxHashSet::<Name>::default();
        names.extend(
            self.facts
                .assignments
                .presence_test_references_by_name
                .keys()
                .cloned(),
        );
        names.extend(
            self.facts
                .assignments
                .presence_test_names_by_name
                .keys()
                .cloned(),
        );

        names
            .into_iter()
            .filter_map(|name| {
                let span = self.first_presence_test_candidate_span(semantic, &name)?;
                Some((name, span))
            })
            .collect()
    }

    fn first_presence_test_candidate_span(
        self,
        semantic: &SemanticModel,
        candidate_name: &Name,
    ) -> Option<Span> {
        self.facts
            .assignments
            .presence_test_references_by_name
            .get(candidate_name)
            .into_iter()
            .flatten()
            .map(|presence| semantic.reference(presence.reference_id()).span)
            .chain(
                self.facts
                    .assignments
                    .presence_test_names_by_name
                    .get(candidate_name)
                    .into_iter()
                    .flatten()
                    .map(|presence| presence.tested_span()),
            )
            .min_by_key(|span| (span.start.offset, span.end.offset))
    }

    pub(crate) fn subshell_assignment_sites(self) -> &'facts [NamedSpan] {
        &self.facts.assignments.subshell_assignment_sites
    }

    pub(crate) fn subshell_later_use_sites(self) -> &'facts [NamedSpan] {
        &self.facts.assignments.subshell_later_use_sites
    }

    pub(crate) fn plus_equals_assignment_spans(self) -> &'facts [Span] {
        &self.facts.assignments.plus_equals_assignment_spans
    }
}

impl<'facts, 'a> SourceFacts<'facts, 'a> {
    pub(crate) fn source(self) -> &'facts str {
        self.facts.source_facts.source
    }

    pub(crate) fn line_index(self) -> &'facts LineIndex {
        self.facts.source_facts.line_index
    }

    pub(crate) fn shell(self) -> ShellDialect {
        self.facts.source_facts.shell
    }

    pub(crate) fn script_line_count(self) -> ScriptLineCountFact {
        *self.facts.source_facts.script_line_count.get_or_init(|| {
            build_script_line_count_fact(
                self.facts.source_facts.source,
                self.facts.source_facts.line_index,
                self.facts.source_facts.comment_index,
            )
        })
    }

    pub(crate) fn indented_shebang_span(self) -> Option<Span> {
        self.facts.source_facts.indented_shebang_span
    }

    pub(crate) fn indented_shebang_indent_span(self) -> Option<Span> {
        self.facts.source_facts.indented_shebang_indent_span
    }

    pub(crate) fn space_after_hash_bang_span(self) -> Option<Span> {
        self.facts.source_facts.space_after_hash_bang_span
    }

    pub(crate) fn space_after_hash_bang_whitespace_span(self) -> Option<Span> {
        self.facts
            .source_facts
            .space_after_hash_bang_whitespace_span
    }

    pub(crate) fn shebang_not_on_first_line_span(self) -> Option<Span> {
        self.facts.source_facts.shebang_not_on_first_line_span
    }

    pub(crate) fn shebang_not_on_first_line_fix_span(self) -> Option<Span> {
        self.facts.source_facts.shebang_not_on_first_line_fix_span
    }

    pub(crate) fn shebang_not_on_first_line_preferred_newline(self) -> Option<&'static str> {
        self.facts
            .source_facts
            .shebang_not_on_first_line_preferred_newline
    }

    pub(crate) fn missing_shebang_line_span(self) -> Option<Span> {
        self.facts.source_facts.missing_shebang_line_span
    }

    pub(crate) fn duplicate_shebang_flag_span(self) -> Option<Span> {
        self.facts.source_facts.duplicate_shebang_flag_span
    }

    pub(crate) fn non_absolute_shebang_span(self) -> Option<Span> {
        self.facts.source_facts.non_absolute_shebang_span
    }

    pub(crate) fn shebang_interpreter(self) -> Option<&'facts ShebangInterpreterFact> {
        self.facts
            .source_facts
            .shebang_interpreter
            .get_or_init(|| {
                build_shebang_interpreter_fact(Locator::new(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                ))
            })
            .as_ref()
    }

    pub(crate) fn shebang_invocation(self) -> Option<&'facts ShebangInvocationFact> {
        self.facts
            .source_facts
            .shebang_invocation
            .get_or_init(|| {
                build_shebang_invocation_fact(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                )
            })
            .as_ref()
    }

    pub(crate) fn missing_file_description_comment(self) -> Option<FileDescriptionCommentFact> {
        *self
            .facts
            .source_facts
            .missing_file_description_comment
            .get_or_init(|| {
                build_missing_file_description_comment_fact(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                )
            })
    }

    pub(crate) fn errexit_enabled_anywhere(self) -> bool {
        self.facts.source_facts.errexit_enabled_anywhere
    }

    pub(crate) fn commented_continuation_comment_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.commented_continuation_comment_spans
    }

    pub(crate) fn comment_double_quote_nesting_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.comment_double_quote_nesting_spans
    }

    pub(crate) fn escaped_dash_command_name_spans(self) -> &'facts [Span] {
        self.facts
            .source_facts
            .escaped_dash_command_name_spans
            .get_or_init(|| {
                let mut command_name_offsets = self
                    .facts
                    .command_facts()
                    .structural_commands()
                    .filter_map(|command| command.command_name_word())
                    .map(|word| word.span.start.offset)
                    .collect::<Vec<_>>();
                command_name_offsets.sort_unstable();
                command_name_offsets.dedup();

                build_escaped_dash_command_name_spans(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                    self.facts.source_facts.region_index,
                    &command_name_offsets,
                )
            })
    }

    pub(crate) fn trailing_directive_comment_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.trailing_directive_comment_spans
    }

    pub(crate) fn todo_comment_facts(self) -> &'facts [TodoCommentFact] {
        self.facts.source_facts.todo_comment_facts.get_or_init(|| {
            build_todo_comment_facts(
                self.facts.source_facts.source,
                self.facts.source_facts.line_index,
                self.facts.source_facts.comment_index,
            )
        })
    }

    pub(crate) fn backtick_substitution_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.backtick_substitution_spans
    }

    pub(crate) fn backtick_escaped_parameters(self) -> &'facts [BacktickEscapedParameter] {
        &self.facts.source_facts.backtick_escaped_parameters
    }

    pub(crate) fn backtick_escaped_parameter_reference_spans(self) -> &'facts [Span] {
        &self
            .facts
            .source_facts
            .backtick_escaped_parameter_reference_spans
    }

    pub(crate) fn is_backtick_double_escaped_parameter_reference(self, span: Span) -> bool {
        self.facts
            .source_facts
            .backtick_double_escaped_parameter_spans
            .contains(&span)
    }

    pub(crate) fn dollar_question_after_command_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.dollar_question_after_command_spans
    }

    pub(crate) fn unused_heredoc_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.unused_heredoc_spans
    }

    pub(crate) fn heredoc_missing_end_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.heredoc_missing_end_spans
    }

    pub(crate) fn heredoc_closer_not_alone_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.heredoc_closer_not_alone_spans
    }

    pub(crate) fn misquoted_heredoc_close_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.misquoted_heredoc_close_spans
    }

    pub(crate) fn heredoc_end_space_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.heredoc_end_space_spans
    }

    pub(crate) fn indented_heredoc_close_facts(self) -> &'facts [(Span, Span)] {
        self.facts
            .source_facts
            .indented_heredoc_close_facts
            .get_or_init(|| {
                let locator = Locator::new(
                    self.facts.source_facts.source,
                    self.facts.source_facts.line_index,
                );
                build_indented_heredoc_close_facts(&self.facts.command.commands, locator)
            })
    }

    pub(crate) fn echo_here_doc_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.echo_here_doc_spans
    }

    pub(crate) fn spaced_tabstrip_close_spans(self) -> &'facts [Span] {
        &self.facts.source_facts.spaced_tabstrip_close_spans
    }
}

impl<'facts, 'a> WordFacts<'facts, 'a> {
    pub(crate) fn is_suppressed_subscript_reference(self, span: Span) -> bool {
        self.facts
            .words
            .suppressed_subscript_reference_spans
            .contains(&FactSpan::new(span))
    }

    #[cfg(test)]
    pub(crate) fn is_subscript_later_suppression_reference(self, span: Span) -> bool {
        self.facts
            .words
            .subscript_later_suppression_reference_spans
            .contains(&FactSpan::new(span))
    }

    pub fn word_facts(self) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::all(self.facts, WordOccurrenceFilter::NonArithmetic)
    }

    pub(crate) fn arithmetic_command_word_facts(self) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::all(self.facts, WordOccurrenceFilter::ArithmeticCommand)
    }

    #[cfg(test)]
    pub(crate) fn parameter_operand_word_facts(self) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::all(self.facts, WordOccurrenceFilter::ParameterOperand)
    }

    pub(crate) fn is_compound_assignment_value_word(self, fact: WordOccurrenceRef<'_, '_>) -> bool {
        self.facts
            .words
            .compound_assignment_value_word_flags
            .get(fact.occurrence_id().index())
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn expansion_word_facts(
        self,
        context: ExpansionContext,
    ) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::all(self.facts, WordOccurrenceFilter::Expansion(context))
    }

    pub(crate) fn case_subject_facts(self) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::all(self.facts, WordOccurrenceFilter::CaseSubject)
    }

    pub fn word_fact(
        self,
        span: Span,
        context: WordFactContext,
    ) -> Option<WordOccurrenceRef<'facts, 'a>> {
        self.facts
            .words
            .word_index
            .get(&FactSpan::new(span))
            .into_iter()
            .flat_map(|indices| indices.iter())
            .copied()
            .map(|id| self.word_occurrence_ref(id))
            .find(|fact| fact.context() == context)
    }

    pub(crate) fn any_word_fact(self, span: Span) -> Option<WordOccurrenceRef<'facts, 'a>> {
        self.facts
            .words
            .word_index
            .get(&FactSpan::new(span))
            .and_then(|indices| indices.first().copied())
            .map(|id| self.word_occurrence_ref(id))
    }

    pub(crate) fn array_assignment_split_word_facts(self) -> WordOccurrenceIter<'facts, 'a> {
        WordOccurrenceIter::ids(
            self.facts,
            &self.facts.words.array_assignment_split_word_ids,
            WordOccurrenceFilter::Any,
        )
    }

    pub(crate) fn word_occurrence_ref(self, id: WordOccurrenceId) -> WordOccurrenceRef<'facts, 'a> {
        WordOccurrenceRef {
            facts: self.facts,
            id,
        }
    }

    pub(crate) fn word_occurrence(self, id: WordOccurrenceId) -> &'facts WordOccurrence {
        &self.facts.words.word_occurrences[id.index()]
    }

    pub(crate) fn word_node(self, id: WordNodeId) -> &'facts WordNode<'a> {
        &self.facts.words.word_nodes[id.index()]
    }

    pub(crate) fn word_node_derived(self, id: WordNodeId) -> &'facts WordNodeDerived<'a> {
        word_node_derived(self.word_node(id))
    }

    pub(crate) fn brace_variable_before_bracket_spans(self) -> &'facts [Span] {
        &self.facts.words.brace_variable_before_bracket_spans
    }

    pub(crate) fn bare_done_word_spans(self) -> &'facts [Span] {
        self.facts.words.bare_done_word_spans.get_or_init(|| {
            build_bare_done_word_spans(
                &self.facts.command.commands,
                &self.facts.words.word_nodes,
                &self.facts.words.word_occurrences,
                self.facts.source_facts.source,
            )
        })
    }

    pub(crate) fn array_index_arithmetic_spans(self) -> &'facts [Span] {
        &self.facts.words.array_index_arithmetic_spans
    }

    pub(crate) fn arithmetic_score_line_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_score_line_spans
    }

    pub(crate) fn dollar_in_arithmetic_spans(self) -> &'facts [Span] {
        &self.facts.words.dollar_in_arithmetic_spans
    }

    pub(crate) fn arithmetic_expansion_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_expansion_spans
    }

    pub(crate) fn arithmetic_index_subscript_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_index_subscript_spans
    }

    pub fn single_quoted_fragments(self) -> &'facts [SingleQuotedFragmentFact] {
        &self.facts.words.single_quoted_fragments
    }

    pub(crate) fn dollar_double_quoted_fragments(self) -> &'facts [DollarDoubleQuotedFragmentFact] {
        &self.facts.words.dollar_double_quoted_fragments
    }

    pub(crate) fn open_double_quote_fragments(self) -> &'facts [OpenDoubleQuoteFragmentFact] {
        &self.facts.words.open_double_quote_fragments
    }

    pub(crate) fn suspect_closing_quote_fragments(
        self,
    ) -> &'facts [SuspectClosingQuoteFragmentFact] {
        &self.facts.words.suspect_closing_quote_fragments
    }

    pub(crate) fn literal_brace_spans(self) -> &'facts [Span] {
        &self.facts.words.literal_brace_spans
    }

    pub fn backtick_fragments(self) -> &'facts [BacktickFragmentFact] {
        &self.facts.words.backtick_fragments
    }

    pub(crate) fn legacy_arithmetic_fragments(self) -> &'facts [LegacyArithmeticFragmentFact] {
        &self.facts.words.legacy_arithmetic_fragments
    }

    pub(crate) fn positional_parameter_fragments(
        self,
    ) -> &'facts [PositionalParameterFragmentFact] {
        &self.facts.words.positional_parameter_fragments
    }

    pub(crate) fn positional_parameter_operator_spans(self) -> &'facts [Span] {
        &self.facts.words.positional_parameter_operator_spans
    }

    pub(crate) fn double_paren_grouping_spans(self) -> &'facts [Span] {
        &self.facts.words.double_paren_grouping_spans
    }

    pub(crate) fn arithmetic_update_operator_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_update_operator_spans
    }

    pub(crate) fn arithmetic_update_operator_fix_facts(
        self,
    ) -> &'facts [ArithmeticUpdateOperatorFixFact] {
        &self.facts.words.arithmetic_update_operator_fix_facts
    }

    pub(crate) fn arithmetic_literal_facts(self) -> &'facts [ArithmeticLiteralFact] {
        &self.facts.words.arithmetic_literal_facts
    }

    pub(crate) fn escape_scan_matches(self) -> &'facts [EscapeScanMatch] {
        &self.facts.words.escape_scan_matches
    }

    pub(crate) fn echo_backslash_escape_word_spans(self) -> &'facts [Span] {
        &self.facts.words.echo_backslash_escape_word_spans
    }

    pub(crate) fn echo_to_sed_substitution_spans(self) -> &'facts [Span] {
        &self.facts.words.echo_to_sed_substitution_spans
    }

    pub(crate) fn arithmetic_command_substitution_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_command_substitution_spans
    }

    pub(crate) fn arithmetic_only_suppressed_subscript_spans(self) -> &'facts [Span] {
        &self.facts.words.arithmetic_only_suppressed_subscript_spans
    }

    pub(crate) fn unicode_smart_quote_spans(self) -> &'facts [Span] {
        &self.facts.words.unicode_smart_quote_spans
    }

    #[cfg(test)]
    pub(crate) fn pattern_literal_spans(self) -> &'facts [Span] {
        &self.facts.words.pattern_literal_spans
    }

    pub fn pattern_charclass_spans(self) -> &'facts [Span] {
        &self.facts.words.pattern_charclass_spans
    }

    pub(crate) fn nested_parameter_expansion_fragments(
        self,
    ) -> &'facts [NestedParameterExpansionFragmentFact] {
        &self.facts.words.nested_parameter_expansion_fragments
    }

    pub(crate) fn indirect_expansion_fragments(self) -> &'facts [IndirectExpansionFragmentFact] {
        &self.facts.words.indirect_expansion_fragments
    }

    pub(crate) fn indexed_array_reference_fragments(
        self,
    ) -> &'facts [IndexedArrayReferenceFragmentFact] {
        &self.facts.words.indexed_array_reference_fragments
    }

    pub(crate) fn plain_unindexed_array_references(
        self,
    ) -> impl Iterator<Item = PlainUnindexedArrayReferenceFact> + 'facts {
        self.facts
            .words
            .plain_unindexed_array_references
            .get_or_init(|| build_plain_unindexed_array_reference_facts(self.facts))
            .iter()
            .copied()
    }

    pub(crate) fn parameter_pattern_special_target_fragments(
        self,
    ) -> &'facts [ParameterPatternSpecialTargetFragmentFact] {
        &self.facts.words.parameter_pattern_special_target_fragments
    }

    pub(crate) fn zsh_parameter_index_flag_fragments(
        self,
    ) -> &'facts [ZshParameterIndexFlagFragmentFact] {
        &self.facts.words.zsh_parameter_index_flag_fragments
    }

    pub fn substring_expansion_fragments(self) -> &'facts [SubstringExpansionFragmentFact] {
        &self.facts.words.substring_expansion_fragments
    }

    pub fn case_modification_fragments(self) -> &'facts [CaseModificationFragmentFact] {
        &self.facts.words.case_modification_fragments
    }

    pub fn replacement_expansion_fragments(self) -> &'facts [ReplacementExpansionFragmentFact] {
        &self.facts.words.replacement_expansion_fragments
    }

    pub(crate) fn positional_parameter_trim_fragments(
        self,
    ) -> &'facts [PositionalParameterTrimFragmentFact] {
        &self.facts.words.positional_parameter_trim_fragments
    }
}

impl<'facts, 'a> CompatFacts<'facts, 'a> {
    pub(crate) fn conditional_portability(self) -> &'facts ConditionalPortabilityFacts {
        &self.facts.compat.conditional_portability
    }

    pub(crate) fn possible_variable_misspelling_scope_compat_name_uses(
        self,
    ) -> &'facts [ComparableNameUse] {
        self.facts
            .compat
            .possible_variable_misspelling_scope_compat_name_uses
            .get_or_init(|| build_possible_variable_misspelling_scope_compat_name_uses(self.facts))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FunctionDocSections {
    globals: bool,
    arguments: bool,
    outputs: bool,
    returns: bool,
}

impl FunctionDocSections {
    pub(crate) fn record_comment_body(&mut self, body: &str) {
        let lower = body.trim().to_ascii_lowercase();
        if lower.starts_with("globals:") {
            self.globals = true;
        } else if lower.starts_with("arguments:") {
            self.arguments = true;
        } else if lower.starts_with("outputs:") {
            self.outputs = true;
        } else if lower.starts_with("returns:") {
            self.returns = true;
        }
    }

    pub(crate) fn has_globals(self) -> bool {
        self.globals
    }

    pub(crate) fn has_arguments(self) -> bool {
        self.arguments
    }

    pub(crate) fn has_outputs(self) -> bool {
        self.outputs
    }

    pub(crate) fn has_returns(self) -> bool {
        self.returns
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionDocContentFact {
    name: Name,
    name_span: Span,
    leading_comment_span: Option<Span>,
    documented_sections: FunctionDocSections,
    body_behavior: FunctionDocBodyBehavior,
}

impl FunctionDocContentFact {
    pub(crate) fn new(
        name: &Name,
        name_span: Span,
        leading_comment_span: Option<Span>,
        documented_sections: FunctionDocSections,
        body_behavior: FunctionDocBodyBehavior,
    ) -> Self {
        Self {
            name: name.clone(),
            name_span,
            leading_comment_span,
            documented_sections,
            body_behavior,
        }
    }

    pub(crate) fn name(&self) -> &Name {
        &self.name
    }

    pub(crate) fn name_span(&self) -> Span {
        self.name_span
    }

    pub(crate) fn has_leading_comment(&self) -> bool {
        self.leading_comment_span.is_some()
    }

    pub(crate) fn documented_sections(&self) -> FunctionDocSections {
        self.documented_sections
    }

    pub(crate) fn uses_global_variables(&self) -> bool {
        self.body_behavior.uses_global_variables()
    }

    pub(crate) fn uses_positional_parameters(&self) -> bool {
        self.body_behavior.uses_positional_parameters()
    }

    pub(crate) fn writes_stdout(&self) -> bool {
        self.body_behavior.writes_stdout()
    }

    pub(crate) fn has_explicit_return(&self) -> bool {
        self.body_behavior.has_explicit_return()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FunctionDocBodyBehavior {
    uses_global_variables: bool,
    uses_positional_parameters: bool,
    writes_stdout: bool,
    has_explicit_return: bool,
}

impl FunctionDocBodyBehavior {
    pub(crate) fn new(
        uses_global_variables: bool,
        uses_positional_parameters: bool,
        writes_stdout: bool,
        has_explicit_return: bool,
    ) -> Self {
        Self {
            uses_global_variables,
            uses_positional_parameters,
            writes_stdout,
            has_explicit_return,
        }
    }

    pub(crate) fn uses_global_variables(self) -> bool {
        self.uses_global_variables
    }

    pub(crate) fn uses_positional_parameters(self) -> bool {
        self.uses_positional_parameters
    }

    pub(crate) fn writes_stdout(self) -> bool {
        self.writes_stdout
    }

    pub(crate) fn has_explicit_return(self) -> bool {
        self.has_explicit_return
    }
}

fn stmt_is_plain_background(stmt: &Stmt) -> bool {
    matches!(
        stmt.terminator,
        Some(StmtTerminator::Background(BackgroundOperator::Plain))
    )
}

fn binary_is_two_segment_self_pipe(binary: &BinaryCommand, name: &str, source: &str) -> bool {
    let Some(chain) = BinaryCommandChain::pipeline(binary) else {
        return false;
    };

    let mut segments = Vec::new();
    let mut operators = Vec::new();
    chain.visit_parts(
        |segment| segments.push(segment),
        |operator| operators.push(operator),
    );

    matches!(operators.as_slice(), [operator] if operator.op == BinaryOp::Pipe)
        && matches!(segments.as_slice(), [left, right] if {
            stmt_is_zero_arg_call_to(left, name, source)
                && stmt_is_zero_arg_call_to(right, name, source)
        })
}

fn stmt_is_zero_arg_call_to(stmt: &Stmt, name: &str, source: &str) -> bool {
    let Command::Simple(command) = &stmt.command else {
        return false;
    };

    command.assignments.is_empty()
        && command.args.is_empty()
        && static_command_name_text(&command.name, source).as_deref() == Some(name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TodoCommentFact {
    content_span: Span,
    content: String,
}

impl TodoCommentFact {
    pub(crate) fn new(content_span: Span, content: String) -> Self {
        Self {
            content_span,
            content,
        }
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn marker_span(&self, marker: &str) -> Span {
        Span::from_positions(
            self.content_span.start,
            self.content_span.start.advanced_by(marker),
        )
    }
}

pub(crate) fn build_possible_variable_misspelling_scope_compat_name_uses(
    facts: &LinterFacts<'_>,
) -> Vec<ComparableNameUse> {
    let source_facts = facts.source_facts();
    let source = source_facts.source();
    if !source_may_have_scope_compat_misspelling(source) {
        return Vec::new();
    }

    let mut uses = Vec::new();
    for word_fact in facts
        .words()
        .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
        .chain(
            facts
                .words()
                .expansion_word_facts(ExpansionContext::AssignmentValue),
        )
        .chain(facts.words().case_subject_facts())
    {
        if let Some(name_use) = scope_compat_standalone_parameter_name_use(word_fact.word()) {
            uses.push(name_use);
        }
    }
    for command in facts.commands() {
        visit_command_words_for_substitutions(
            command.command(),
            command.redirects(),
            source,
            &mut |word| {
                collect_scope_compat_derived_name_uses(
                    word,
                    facts.semantic_artifacts,
                    source,
                    &mut uses,
                );
            },
        );
    }
    for word in facts
        .command_facts()
        .for_headers()
        .iter()
        .flat_map(|header| header.words())
        .chain(
            facts
                .command_facts()
                .select_headers()
                .iter()
                .flat_map(|header| header.words()),
        )
    {
        if let Some(mut name_use) = scope_compat_standalone_parameter_name_use(word.word()) {
            name_use.mark_derived();
            if is_interesting_scope_compat_name_use(
                source,
                name_use.key().as_str(),
                name_use.kind(),
                name_use.span(),
            ) {
                uses.push(name_use);
            }
        }
    }
    uses.extend(
        build_flag_for_loop_source_name_uses(Locator::new(source, source_facts.line_index()))
            .into_iter()
            .filter(|name_use| {
                is_interesting_scope_compat_name_use(
                    source,
                    name_use.key().as_str(),
                    name_use.kind(),
                    name_use.span(),
                )
            }),
    );
    dedup_comparable_name_uses(&mut uses);
    uses
}

pub(crate) fn source_may_have_scope_compat_misspelling(source: &str) -> bool {
    source.contains("SHELLSPEC_EXECDIR")
        || source.contains("CFLAGS")
        || source.contains("CPPFLAGS")
        || source.contains("CXXFLAGS")
}

pub(crate) fn scope_compat_standalone_parameter_name_use(word: &Word) -> Option<ComparableNameUse> {
    let name = standalone_comparable_parameter_name(&word.parts)?;
    Some(ComparableNameUse {
        span: word.span,
        key: ComparableNameKey(name.into()),
        kind: ComparableNameUseKind::Parameter,
    })
}

pub(crate) fn collect_scope_compat_derived_name_uses(
    word: &Word,
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    let allow_quoted_derived_words =
        analyze_word(word, source, None).quote == WordQuote::FullyQuoted;
    collect_scope_compat_command_substitution_name_uses_in_parts(
        &word.parts,
        semantic,
        source,
        allow_quoted_derived_words,
        uses,
    );
}

pub(crate) fn collect_scope_compat_command_substitution_name_uses_in_parts(
    parts: &[WordPartNode],
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_scope_compat_command_substitution_name_uses_in_parts(
                    parts,
                    semantic,
                    source,
                    allow_quoted_derived_words,
                    uses,
                );
            }
            WordPart::CommandSubstitution { body, .. } => {
                collect_scope_compat_command_substitution_name_uses(
                    body,
                    semantic,
                    source,
                    allow_quoted_derived_words,
                    uses,
                );
            }
            WordPart::ParameterExpansion {
                operand_word_ast, ..
            }
            | WordPart::IndirectExpansion {
                operand_word_ast, ..
            } => {
                if let Some(word) = operand_word_ast {
                    collect_scope_compat_command_substitution_name_uses_in_parts(
                        &word.parts,
                        semantic,
                        source,
                        allow_quoted_derived_words,
                        uses,
                    );
                }
            }
            WordPart::Substring {
                offset_word_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                collect_scope_compat_arithmetic_name_use(offset_word_ast, source, uses);
                if let Some(word) = length_word_ast {
                    collect_scope_compat_arithmetic_name_use(word, source, uses);
                }
            }
            WordPart::ArithmeticExpansion {
                expression_word_ast,
                ..
            } => {
                collect_scope_compat_arithmetic_name_use(expression_word_ast, source, uses);
            }
            WordPart::Literal(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
        }
    }
}

pub(crate) fn collect_scope_compat_command_substitution_name_uses(
    body: &StmtSeq,
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    visit_command_substitution_candidate_words(body, semantic, source, &mut |word| {
        push_scope_compat_command_substitution_word_use(
            word,
            source,
            allow_quoted_derived_words,
            uses,
        );
    });
}

pub(crate) fn push_scope_compat_command_substitution_word_use(
    word: &Word,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    if !allow_quoted_derived_words
        && analyze_word(word, source, None).quote == WordQuote::FullyQuoted
    {
        return;
    }
    if let Some(name_use) = scope_compat_standalone_derived_name_use(word, source) {
        uses.push(name_use);
    }
}

pub(crate) fn collect_scope_compat_arithmetic_name_use(
    word: &Word,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    if let Some(name_use) = scope_compat_standalone_derived_name_use(word, source) {
        uses.push(name_use);
    }
}

pub(crate) fn scope_compat_standalone_derived_name_use(
    word: &Word,
    source: &str,
) -> Option<ComparableNameUse> {
    let mut name_use = scope_compat_standalone_parameter_name_use(word)?;
    name_use.mark_derived();
    is_interesting_scope_compat_name_use(
        source,
        name_use.key().as_str(),
        name_use.kind(),
        name_use.span(),
    )
    .then_some(name_use)
}

pub(crate) fn is_interesting_scope_compat_name_use(
    _source: &str,
    name: &str,
    kind: ComparableNameUseKind,
    _span: Span,
) -> bool {
    name == "SHELLSPEC_EXECDIR"
        || name == "SHELLSPEC_SPECDIR"
        || kind == ComparableNameUseKind::Derived && is_reportable_build_flag_family_name(name)
}

pub(crate) fn is_reportable_build_flag_family_name(name: &str) -> bool {
    let Some((_, suffix)) = split_scope_compat_build_flag_family_name(name) else {
        return false;
    };
    matches!(suffix, "CFLAGS" | "CPPFLAGS" | "CXXFLAGS")
}

pub(crate) fn split_scope_compat_build_flag_family_name(
    name: &str,
) -> Option<(&str, &'static str)> {
    ["CXXFLAGS", "CPPFLAGS", "CFLAGS"]
        .into_iter()
        .find_map(|suffix| {
            if name == suffix {
                Some(("", suffix))
            } else {
                name.strip_suffix(suffix)
                    .filter(|prefix| prefix.ends_with('_'))
                    .map(|prefix| (prefix, suffix))
            }
        })
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn populate_array_assignment_split_scalar_expansion_spans(
    shell: ShellDialect,
    commands: &[CommandFact<'_>],
    word_nodes: &[WordNode<'_>],
    word_occurrences: &mut [WordOccurrence],
    fact_store: &mut FactStore<'_>,
    word_ids: &[WordOccurrenceId],
) {
    if word_ids.is_empty() {
        return;
    }
    let has_brace_expansion = shell_has_brace_expansion(shell);
    let mut split_sensitive_spans = Vec::new();
    let mut use_replacement_spans = Vec::new();
    let mut brace_expansion_spans = Vec::new();
    for id in word_ids.iter().copied() {
        collect_array_assignment_split_scalar_expansion_spans(
            id,
            commands,
            word_nodes,
            word_occurrences,
            fact_store,
            has_brace_expansion,
            &mut split_sensitive_spans,
            &mut use_replacement_spans,
            &mut brace_expansion_spans,
        );
        word_occurrences[id.index()].array_assignment_split_scalar_expansion_spans = fact_store
            .word_spans
            .push_many(split_sensitive_spans.drain(..));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_array_assignment_split_scalar_expansion_spans(
    id: WordOccurrenceId,
    commands: &[CommandFact<'_>],
    word_nodes: &[WordNode<'_>],
    word_occurrences: &[WordOccurrence],
    fact_store: &FactStore<'_>,
    has_brace_expansion: bool,
    split_sensitive_spans: &mut Vec<Span>,
    use_replacement_spans: &mut Vec<Span>,
    brace_expansion_spans: &mut Vec<Span>,
) {
    split_sensitive_spans.clear();
    let fact = &word_occurrences[id.index()];
    let word = occurrence_word(word_nodes, fact);
    let derived = word_node_derived(&word_nodes[fact.node_id.index()]);
    if word_nodes[fact.node_id.index()]
        .analysis
        .can_expand_to_multiple_fields
    {
        split_sensitive_spans.extend(
            fact_store
                .word_spans(derived.unquoted_scalar_expansion_spans)
                .iter()
                .copied(),
        );
    }

    let fact_span = occurrence_span(word_nodes, fact);
    let unquoted_command_substitution_spans =
        fact_store.word_spans(fact.split_sensitive_unquoted_command_substitution_spans);

    if !unquoted_command_substitution_spans.is_empty() {
        // commands is sorted by start offset (compare_command_facts_by_offset),
        // so binary-search the subrange whose start lies inside fact_span.
        let start = commands.partition_point(|c| c.span().start.offset < fact_span.start.offset);
        for command in &commands[start..] {
            let command_span = command.span();
            if command_span.start.offset > fact_span.end.offset {
                break;
            }
            if !contains_span_strictly(fact_span, command_span) {
                continue;
            }
            if !unquoted_command_substitution_spans
                .iter()
                .any(|span| contains_span_strictly(*span, command_span))
            {
                continue;
            }
            for nested_id in fact_store.word_occurrence_ids_for_command(command.id()) {
                let nested = &word_occurrences[nested_id.index()];
                let nested_derived = word_node_derived(&word_nodes[nested.node_id.index()]);
                split_sensitive_spans.extend(
                    fact_store
                        .word_spans(nested_derived.scalar_expansion_spans)
                        .iter()
                        .copied(),
                );
            }
        }
    }

    if split_sensitive_spans.is_empty() {
        return;
    }

    use_replacement_spans.clear();
    collect_use_replacement_expansion_spans(&word.parts, use_replacement_spans);

    brace_expansion_spans.clear();
    if has_brace_expansion {
        brace_expansion_spans.extend(
            word.brace_syntax()
                .iter()
                .filter(|brace| brace.expands())
                .map(|brace| brace.span),
        );
    }

    split_sensitive_spans.retain(|span| {
        !use_replacement_spans
            .iter()
            .any(|replacement_span| contains_span(*replacement_span, *span))
            && !brace_expansion_spans
                .iter()
                .any(|brace_span| contains_span(*brace_span, *span))
    });
    sort_and_dedup_spans(split_sensitive_spans);
}

pub(crate) fn shell_has_brace_expansion(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Bash | ShellDialect::Ksh | ShellDialect::Mksh | ShellDialect::Zsh
    )
}

pub(crate) fn build_flag_for_loop_source_name_uses(locator: Locator<'_>) -> Vec<ComparableNameUse> {
    let source = locator.source();
    let mut uses = Vec::new();
    for line_number in 1..=locator.line_index().line_count() {
        let Some(line_range) = locator.line_range(line_number) else {
            continue;
        };
        let line_start = usize::from(line_range.start());
        let line = line_range.slice(source).trim_end_matches('\r');
        let trimmed = line.trim_start();
        let leading_whitespace = line.len() - trimmed.len();
        if let Some(after_for) = trimmed.strip_prefix("for ")
            && let Some(in_offset) = after_for.find(" in ")
        {
            let list = &after_for[in_offset + 4..];
            if let Some(name_start_in_list) = list.find("${") {
                let name_start = line_start
                    + leading_whitespace
                    + "for ".len()
                    + in_offset
                    + 4
                    + name_start_in_list
                    + 2;
                let name_text = &source[name_start..];
                let name_len = name_text
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                let name = &source[name_start..name_start + name_len];
                if is_build_flag_source_name(name)
                    && source
                        .as_bytes()
                        .get(name_start + name_len)
                        .is_some_and(|byte| *byte == b'}')
                    && let (Some(start), Some(end)) = (
                        locator.position_at_offset(name_start - 2),
                        locator.position_at_offset(name_start + name_len + 1),
                    )
                {
                    uses.push(ComparableNameUse {
                        span: Span::from_positions(start, end),
                        key: ComparableNameKey(name.into()),
                        kind: ComparableNameUseKind::Derived,
                    });
                }
            }
        }
    }
    uses
}

pub(crate) fn is_build_flag_source_name(name: &str) -> bool {
    matches!(
        name,
        "CFLAGS" | "CXXFLAGS" | "CPPFLAGS" | "LDFLAGS" | "GOFLAGS"
    ) || name.ends_with("_CFLAGS")
        || name.ends_with("_CXXFLAGS")
        || name.ends_with("_CPPFLAGS")
        || name.ends_with("_LDFLAGS")
}

pub(crate) fn assignment_value_span(value: &AssignmentValue) -> Option<Span> {
    match value {
        AssignmentValue::Scalar(word) => Some(word.span),
        AssignmentValue::Compound(_) => None,
    }
}

pub(crate) struct AssignmentValueTargetEntry {
    value_start: usize,
    value_end: usize,
    target_name: Name,
}

// Sorted ascending by `value_start`. Scalar assignment value spans are word-level
// and do not overlap, so a query span is contained by at most one entry.
pub(crate) type AssignmentValueTargetIndex = Vec<AssignmentValueTargetEntry>;

pub(crate) fn build_assignment_value_target_index(
    commands: &[CommandFact<'_>],
) -> AssignmentValueTargetIndex {
    let mut entries = Vec::<AssignmentValueTargetEntry>::new();
    for command in commands {
        let cmd = command.command();
        for assignment in command_assignments(cmd) {
            push_assignment_value_target_entry(&mut entries, assignment);
        }
        for operand in declaration_operands(cmd) {
            if let DeclOperand::Assignment(assignment) = operand {
                push_assignment_value_target_entry(&mut entries, assignment);
            }
        }
    }
    entries.sort_by_key(|entry| (entry.value_start, entry.value_end));
    entries.dedup_by(|a, b| {
        a.value_start == b.value_start
            && a.value_end == b.value_end
            && a.target_name == b.target_name
    });
    entries
}

pub(crate) fn push_assignment_value_target_entry(
    entries: &mut Vec<AssignmentValueTargetEntry>,
    assignment: &Assignment,
) {
    if let Some(value_span) = assignment_value_span(&assignment.value) {
        entries.push(AssignmentValueTargetEntry {
            value_start: value_span.start.offset,
            value_end: value_span.end.offset,
            target_name: assignment.target.name.clone(),
        });
    }
}
