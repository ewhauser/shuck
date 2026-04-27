pub struct LinterFacts<'a> {
    source: &'a str,
    commands: Vec<CommandFact<'a>>,
    structural_command_ids: Vec<CommandId>,
    #[cfg_attr(not(test), allow(dead_code))]
    command_ids_by_span: CommandLookupIndex,
    innermost_command_ids_by_offset: CommandOffsetLookup,
    innermost_command_ids_by_binding_offset: CommandOffsetLookup,
    command_parent_ids: Vec<Option<CommandId>>,
    command_dominance_barrier_flags: Vec<bool>,
    if_condition_command_ids: FxHashSet<CommandId>,
    elif_condition_command_ids: FxHashSet<CommandId>,
    binding_values: FxHashMap<BindingId, BindingValueFact<'a>>,
    broken_assoc_key_spans: Vec<Span>,
    comma_array_assignment_spans: Vec<Span>,
    ifs_literal_backslash_assignment_value_spans: Vec<Span>,
    env_prefix_assignment_scope_spans: Vec<Span>,
    env_prefix_expansion_scope_spans: Vec<Span>,
    unset_command_ids_by_target_name: FxHashMap<Name, Vec<CommandId>>,
    function_unset_command_ids_by_target_name: FxHashMap<Name, Vec<CommandId>>,
    presence_tested_names: FxHashSet<Name>,
    nested_presence_test_spans: FxHashMap<Name, Vec<Span>>,
    c006_presence_tested_names: FxHashSet<Name>,
    c006_nested_presence_test_spans: FxHashMap<Name, Vec<Span>>,
    c006_suppressing_reference_offsets_by_name: FxHashMap<Name, Vec<usize>>,
    presence_test_references_by_name: FxHashMap<Name, Vec<PresenceTestReferenceFact>>,
    presence_test_names_by_name: FxHashMap<Name, Vec<PresenceTestNameFact>>,
    possible_variable_misspelling_use_scan: OnceLock<bool>,
    possible_variable_misspelling_index: OnceLock<PossibleVariableMisspellingIndex>,
    possible_variable_misspelling_scope_compat_name_uses: OnceLock<Vec<ComparableNameUse>>,
    suppressed_subscript_reference_spans: FxHashSet<FactSpan>,
    subscript_later_suppression_reference_spans: FxHashSet<FactSpan>,
    compound_assignment_value_word_spans: FxHashSet<FactSpan>,
    word_nodes: Vec<WordNode<'a>>,
    word_occurrences: Vec<WordOccurrence>,
    word_index: FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    fact_store: FactStore<'a>,
    unquoted_command_argument_use_offsets: FxHashMap<Name, Vec<usize>>,
    array_assignment_split_word_ids: Vec<WordOccurrenceId>,
    brace_variable_before_bracket_spans: Vec<Span>,
    completion_registered_function_command_flags: Vec<bool>,
    function_headers: Vec<FunctionHeaderFact<'a>>,
    function_in_alias_spans: Vec<Span>,
    alias_definition_expansion_spans: Vec<Span>,
    function_body_without_braces_spans: Vec<Span>,
    function_parameter_fallback_spans: Vec<Span>,
    redundant_return_status_spans: Vec<Span>,
    for_headers: Vec<ForHeaderFact<'a>>,
    select_headers: Vec<SelectHeaderFact<'a>>,
    case_items: Vec<CaseItemFact<'a>>,
    case_pattern_shadows: Vec<CasePatternShadowFact>,
    case_pattern_impossible_spans: Vec<Span>,
    case_pattern_expansions: Vec<CasePatternExpansionFact>,
    getopts_cases: Vec<GetoptsCaseFact>,
    pipelines: Vec<PipelineFact<'a>>,
    lists: Vec<ListFact<'a>>,
    statement_facts: Vec<StatementFact>,
    background_semicolon_spans: Vec<Span>,
    single_test_subshell_spans: Vec<Span>,
    subshell_test_group_spans: Vec<Span>,
    indented_shebang_span: Option<Span>,
    indented_shebang_indent_span: Option<Span>,
    space_after_hash_bang_span: Option<Span>,
    space_after_hash_bang_whitespace_span: Option<Span>,
    shebang_not_on_first_line_span: Option<Span>,
    shebang_not_on_first_line_fix_span: Option<Span>,
    shebang_not_on_first_line_preferred_newline: Option<&'static str>,
    missing_shebang_line_span: Option<Span>,
    duplicate_shebang_flag_span: Option<Span>,
    non_absolute_shebang_span: Option<Span>,
    errexit_enabled_anywhere: bool,
    commented_continuation_comment_spans: Vec<Span>,
    comment_double_quote_nesting_spans: Vec<Span>,
    trailing_directive_comment_spans: Vec<Span>,
    condition_status_capture_spans: Vec<Span>,
    command_substitution_command_spans: Vec<Span>,
    backtick_substitution_spans: Vec<Span>,
    backtick_escaped_parameters: Vec<BacktickEscapedParameter>,
    backtick_escaped_parameter_reference_spans: Vec<Span>,
    backtick_double_escaped_parameter_spans: Vec<Span>,
    backtick_command_name_spans: Vec<Span>,
    dollar_question_after_command_spans: Vec<Span>,
    assignment_like_command_name_spans: Vec<Span>,
    bare_command_name_assignment_spans: Vec<Span>,
    subshell_assignment_sites: Vec<NamedSpan>,
    subshell_later_use_sites: Vec<NamedSpan>,
    unused_heredoc_spans: Vec<Span>,
    heredoc_missing_end_spans: Vec<Span>,
    heredoc_closer_not_alone_spans: Vec<Span>,
    misquoted_heredoc_close_spans: Vec<Span>,
    heredoc_end_space_spans: Vec<Span>,
    echo_here_doc_spans: Vec<Span>,
    spaced_tabstrip_close_spans: Vec<Span>,
    plus_equals_assignment_spans: Vec<Span>,
    array_index_arithmetic_spans: Vec<Span>,
    arithmetic_score_line_spans: Vec<Span>,
    dollar_in_arithmetic_spans: Vec<Span>,
    arithmetic_command_substitution_spans: Vec<Span>,
    function_positional_parameter_facts: FxHashMap<ScopeId, FunctionPositionalParameterFacts>,
    function_cli_dispatch_facts: FxHashMap<ScopeId, FunctionCliDispatchFacts>,
    single_quoted_fragments: Vec<SingleQuotedFragmentFact>,
    dollar_double_quoted_fragments: Vec<DollarDoubleQuotedFragmentFact>,
    open_double_quote_fragments: Vec<OpenDoubleQuoteFragmentFact>,
    suspect_closing_quote_fragments: Vec<SuspectClosingQuoteFragmentFact>,
    literal_brace_spans: Vec<Span>,
    backtick_fragments: Vec<BacktickFragmentFact>,
    legacy_arithmetic_fragments: Vec<LegacyArithmeticFragmentFact>,
    positional_parameter_fragments: Vec<PositionalParameterFragmentFact>,
    positional_parameter_operator_spans: Vec<Span>,
    double_paren_grouping_spans: Vec<Span>,
    arithmetic_update_operator_spans: Vec<Span>,
    base_prefix_arithmetic_spans: Vec<Span>,
    escape_scan_matches: Vec<EscapeScanMatch>,
    echo_backslash_escape_word_spans: Vec<Span>,
    echo_to_sed_substitution_spans: Vec<Span>,
    unicode_smart_quote_spans: Vec<Span>,
    pattern_exactly_one_extglob_spans: Vec<Span>,
    pattern_literal_spans: Vec<Span>,
    pattern_charclass_spans: Vec<Span>,
    nested_pattern_charclass_spans: FxHashSet<FactSpan>,
    nested_parameter_expansion_fragments: Vec<NestedParameterExpansionFragmentFact>,
    indirect_expansion_fragments: Vec<IndirectExpansionFragmentFact>,
    indexed_array_reference_fragments: Vec<IndexedArrayReferenceFragmentFact>,
    plain_unindexed_reference_spans: Vec<Span>,
    parameter_pattern_special_target_fragments: Vec<ParameterPatternSpecialTargetFragmentFact>,
    zsh_parameter_index_flag_fragments: Vec<ZshParameterIndexFlagFragmentFact>,
    substring_expansion_fragments: Vec<SubstringExpansionFragmentFact>,
    case_modification_fragments: Vec<CaseModificationFragmentFact>,
    replacement_expansion_fragments: Vec<ReplacementExpansionFragmentFact>,
    positional_parameter_trim_fragments: Vec<PositionalParameterTrimFragmentFact>,
    conditional_portability: ConditionalPortabilityFacts,
}

impl<'a> LinterFacts<'a> {
    pub fn build(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
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
        semantic: &'a SemanticModel,
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
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        shell: ShellDialect,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        LinterFactsBuilder::new(file, source, semantic, indexer, shell, ambient_shell_options)
        .build()
    }

    pub fn commands(&self) -> CommandFacts<'_, 'a> {
        CommandFacts::new(&self.commands, &self.fact_store)
    }

    pub fn malformed_bracket_test_spans(&self, source: &str) -> Vec<Span> {
        self.commands
            .iter()
            .filter(|fact| fact.static_utility_name_is("["))
            .filter(|fact| {
                fact.body_args()
                    .last()
                    .and_then(|word| static_word_text(word, source))
                    .as_deref()
                    != Some("]")
            })
            .map(|fact| fact.body_name_word().map_or(fact.span(), |word| word.span))
            .collect()
    }

    pub fn abort_like_bracket_test_spans(&self, source: &str) -> Vec<Span> {
        self.commands
            .iter()
            .filter_map(|fact| {
                let simple_test = fact.simple_test()?;
                simple_test
                    .is_abort_like_bracket_test(source)
                    .then_some(simple_test)
            })
            .map(|simple_test| {
                simple_test
                    .effective_operator_word()
                    .map_or_else(|| simple_test.operands()[0].span, |word| word.span)
            })
            .collect()
    }

    pub fn function_positional_parameter_facts(
        &self,
        scope: ScopeId,
    ) -> FunctionPositionalParameterFacts {
        self.function_positional_parameter_facts
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn function_cli_dispatch_facts(
        &self,
        scope: ScopeId,
    ) -> FunctionCliDispatchFacts {
        self.function_cli_dispatch_facts
            .get(&scope)
            .copied()
            .unwrap_or_default()
    }

    pub fn structural_commands(&self) -> impl Iterator<Item = CommandFactRef<'_, 'a>> + '_ {
        self.structural_command_ids
            .iter()
            .copied()
            .map(|id| self.command(id))
    }

    pub(crate) fn unset_commands_for_name(
        &self,
        name: &Name,
    ) -> impl Iterator<Item = CommandFactRef<'_, 'a>> + '_ {
        self.unset_command_ids_by_target_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .map(|id| self.command(id))
    }

    pub(crate) fn function_unset_commands_for_name(
        &self,
        name: &Name,
    ) -> impl Iterator<Item = CommandFactRef<'_, 'a>> + '_ {
        self.function_unset_command_ids_by_target_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .map(|id| self.command(id))
    }

    pub fn command(&self, id: CommandId) -> CommandFactRef<'_, 'a> {
        CommandFactRef::new(&self.commands[id.index()], &self.fact_store)
    }

    pub fn innermost_command_at(&self, offset: usize) -> Option<CommandFactRef<'_, 'a>> {
        self.innermost_command_id_at(offset)
            .map(|id| self.command(id))
    }

    pub fn innermost_command_id_at(&self, offset: usize) -> Option<CommandId> {
        precomputed_command_id_for_offset(&self.innermost_command_ids_by_offset, offset)
    }

    pub(crate) fn innermost_command_at_binding_offset(
        &self,
        offset: usize,
    ) -> Option<CommandFactRef<'_, 'a>> {
        precomputed_command_id_for_offset(&self.innermost_command_ids_by_binding_offset, offset)
            .map(|id| self.command(id))
    }

    pub fn command_parent_id(&self, id: CommandId) -> Option<CommandId> {
        self.command_parent_ids.get(id.index()).copied().flatten()
    }

    pub fn command_parent(&self, id: CommandId) -> Option<CommandFactRef<'_, 'a>> {
        self.command_parent_id(id)
            .map(|parent_id| self.command(parent_id))
    }

    pub fn command_is_dominance_barrier(&self, id: CommandId) -> bool {
        self.command_dominance_barrier_flags
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn command_id_for_stmt(&self, stmt: &Stmt) -> Option<CommandId> {
        self.command_id_for_command(&stmt.command)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn command_id_for_command(&self, command: &Command) -> Option<CommandId> {
        command_id_for_command(command, &self.command_ids_by_span)
    }

    pub fn binding_value(&self, binding_id: BindingId) -> Option<&BindingValueFact<'a>> {
        self.binding_values.get(&binding_id)
    }

    pub fn broken_assoc_key_spans(&self) -> &[Span] {
        &self.broken_assoc_key_spans
    }

    pub fn comma_array_assignment_spans(&self) -> &[Span] {
        &self.comma_array_assignment_spans
    }

    pub fn ifs_literal_backslash_assignment_value_spans(&self) -> &[Span] {
        &self.ifs_literal_backslash_assignment_value_spans
    }

    pub fn env_prefix_assignment_scope_spans(&self) -> &[Span] {
        &self.env_prefix_assignment_scope_spans
    }

    pub fn env_prefix_expansion_scope_spans(&self) -> &[Span] {
        &self.env_prefix_expansion_scope_spans
    }

    pub fn is_if_condition_command(&self, id: CommandId) -> bool {
        self.if_condition_command_ids.contains(&id)
    }

    pub fn is_elif_condition_command(&self, id: CommandId) -> bool {
        self.elif_condition_command_ids.contains(&id)
    }

    pub fn presence_tested_names(&self) -> &FxHashSet<Name> {
        &self.presence_tested_names
    }

    pub(crate) fn possible_variable_misspelling_candidate(
        &self,
        semantic: &SemanticModel,
        target_name: &str,
    ) -> Option<String> {
        if *self.possible_variable_misspelling_use_scan.get_or_init(|| {
            should_scan_possible_variable_misspelling_candidates(
                semantic,
                &self.presence_test_references_by_name,
                &self.presence_test_names_by_name,
            )
        }) {
            return scan_possible_variable_misspelling_candidate(
                semantic,
                &self.presence_test_references_by_name,
                &self.presence_test_names_by_name,
                target_name,
            );
        }

        self.possible_variable_misspelling_index
            .get_or_init(|| {
                build_possible_variable_misspelling_index(
                    semantic,
                    &self.presence_test_references_by_name,
                    &self.presence_test_names_by_name,
                )
            })
            .candidate_name(target_name)
            .map(ToOwned::to_owned)
    }

    pub fn is_presence_tested_name(&self, name: &Name, span: Span) -> bool {
        self.presence_tested_names.contains(name)
            || self
                .nested_presence_test_spans
                .get(name)
                .is_some_and(|spans| {
                    spans
                        .iter()
                        .copied()
                        .any(|outer| contains_span(outer, span))
                })
    }

    pub fn is_c006_presence_tested_name(&self, name: &Name, _span: Span) -> bool {
        self.c006_presence_tested_names.contains(name)
            || self.c006_nested_presence_test_spans.contains_key(name)
    }

    pub fn has_prior_c006_suppressing_reference(&self, name: &Name, span: Span) -> bool {
        self.c006_suppressing_reference_offsets_by_name
            .get(name)
            .is_some_and(|offsets| {
                offsets.partition_point(|offset| *offset < span.start.offset) > 0
            })
    }

    pub fn assignment_value_target_name_for_span(&self, span: Span) -> Option<&Name> {
        self.commands
            .iter()
            .filter(|command| contains_span(command.span(), span))
            .filter_map(|command| assignment_value_target_for_span(command, span))
            .min_by_key(|(_, value_span)| value_span.end.offset - value_span.start.offset)
            .map(|(name, _)| name)
    }

    pub(crate) fn presence_test_references(
        &self,
        name: &Name,
    ) -> &[PresenceTestReferenceFact] {
        self.presence_test_references_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn presence_test_names(&self, name: &Name) -> &[PresenceTestNameFact] {
        self.presence_test_names_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn presence_test_candidate_spans(
        &self,
        semantic: &SemanticModel,
    ) -> Vec<(Name, Span)> {
        let mut names = FxHashSet::<Name>::default();
        names.extend(self.presence_test_references_by_name.keys().cloned());
        names.extend(self.presence_test_names_by_name.keys().cloned());

        names
            .into_iter()
            .filter_map(|name| {
                let span = self.first_presence_test_candidate_span(semantic, &name)?;
                Some((name, span))
            })
            .collect()
    }

    fn first_presence_test_candidate_span(
        &self,
        semantic: &SemanticModel,
        candidate_name: &Name,
    ) -> Option<Span> {
        self.presence_test_references_by_name
            .get(candidate_name)
            .into_iter()
            .flatten()
            .map(|presence| semantic.reference(presence.reference_id()).span)
            .chain(
                self.presence_test_names_by_name
                    .get(candidate_name)
                    .into_iter()
                    .flatten()
                    .map(|presence| presence.tested_span()),
            )
            .min_by_key(|span| (span.start.offset, span.end.offset))
    }

    pub fn is_suppressed_subscript_reference(&self, span: Span) -> bool {
        self.suppressed_subscript_reference_spans
            .contains(&FactSpan::new(span))
    }

    pub fn is_subscript_later_suppression_reference(&self, span: Span) -> bool {
        self.subscript_later_suppression_reference_spans
            .contains(&FactSpan::new(span))
    }

    pub fn word_facts(&self) -> WordOccurrenceIter<'_, 'a> {
        WordOccurrenceIter::all(self, WordOccurrenceFilter::NonArithmetic)
    }

    pub fn arithmetic_command_word_facts(&self) -> WordOccurrenceIter<'_, 'a> {
        WordOccurrenceIter::all(self, WordOccurrenceFilter::ArithmeticCommand)
    }

    pub fn is_compound_assignment_value_word(&self, fact: WordOccurrenceRef<'_, '_>) -> bool {
        self.compound_assignment_value_word_spans
            .contains(&fact.key())
    }

    pub fn expansion_word_facts(&self, context: ExpansionContext) -> WordOccurrenceIter<'_, 'a> {
        WordOccurrenceIter::all(self, WordOccurrenceFilter::Expansion(context))
    }

    pub fn case_subject_facts(&self) -> WordOccurrenceIter<'_, 'a> {
        WordOccurrenceIter::all(self, WordOccurrenceFilter::CaseSubject)
    }

    pub fn word_fact(
        &self,
        span: Span,
        context: WordFactContext,
    ) -> Option<WordOccurrenceRef<'_, 'a>> {
        self.word_index
            .get(&FactSpan::new(span))
            .into_iter()
            .flat_map(|indices| indices.iter())
            .copied()
            .map(|id| self.word_occurrence_ref(id))
            .find(|fact| fact.context() == context)
    }

    pub fn any_word_fact(&self, span: Span) -> Option<WordOccurrenceRef<'_, 'a>> {
        self.word_index
            .get(&FactSpan::new(span))
            .and_then(|indices| indices.first().copied())
            .map(|id| self.word_occurrence_ref(id))
    }

    pub fn has_later_unquoted_command_argument_use(
        &self,
        name: &Name,
        after_offset: usize,
    ) -> bool {
        self.unquoted_command_argument_use_offsets
            .get(name)
            .is_some_and(|offsets| {
                offsets.partition_point(|offset| *offset <= after_offset) < offsets.len()
            })
    }

    pub fn array_assignment_split_word_facts(&self) -> WordOccurrenceIter<'_, 'a> {
        WordOccurrenceIter::ids(
            self,
            &self.array_assignment_split_word_ids,
            WordOccurrenceFilter::Any,
        )
    }

    fn word_occurrence_ref(&self, id: WordOccurrenceId) -> WordOccurrenceRef<'_, 'a> {
        WordOccurrenceRef { facts: self, id }
    }

    fn word_occurrence(&self, id: WordOccurrenceId) -> &WordOccurrence {
        &self.word_occurrences[id.index()]
    }

    fn word_node(&self, id: WordNodeId) -> &WordNode<'a> {
        &self.word_nodes[id.index()]
    }

    fn word_node_derived(&self, id: WordNodeId) -> &WordNodeDerived<'a> {
        word_node_derived(self.word_node(id))
    }

    pub fn brace_variable_before_bracket_spans(&self) -> &[Span] {
        &self.brace_variable_before_bracket_spans
    }

    pub fn command_is_in_completion_registered_function(&self, id: CommandId) -> bool {
        self.completion_registered_function_command_flags
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    pub fn function_headers(&self) -> &[FunctionHeaderFact<'a>] {
        &self.function_headers
    }

    pub fn function_in_alias_spans(&self) -> &[Span] {
        &self.function_in_alias_spans
    }

    pub fn alias_definition_expansion_spans(&self) -> &[Span] {
        &self.alias_definition_expansion_spans
    }

    pub fn function_body_without_braces_spans(&self) -> &[Span] {
        &self.function_body_without_braces_spans
    }

    pub fn function_parameter_fallback_spans(&self) -> &[Span] {
        &self.function_parameter_fallback_spans
    }

    pub fn redundant_return_status_spans(&self) -> &[Span] {
        &self.redundant_return_status_spans
    }

    pub fn for_headers(&self) -> &[ForHeaderFact<'a>] {
        &self.for_headers
    }

    pub fn select_headers(&self) -> &[SelectHeaderFact<'a>] {
        &self.select_headers
    }

    pub fn case_items(&self) -> &[CaseItemFact<'a>] {
        &self.case_items
    }

    pub fn case_pattern_shadows(&self) -> &[CasePatternShadowFact] {
        &self.case_pattern_shadows
    }

    pub fn case_pattern_impossible_spans(&self) -> &[Span] {
        &self.case_pattern_impossible_spans
    }

    pub fn case_pattern_expansions(&self) -> &[CasePatternExpansionFact] {
        &self.case_pattern_expansions
    }

    pub fn getopts_cases(&self) -> &[GetoptsCaseFact] {
        &self.getopts_cases
    }

    pub fn pipelines(&self) -> &[PipelineFact<'a>] {
        &self.pipelines
    }

    pub fn lists(&self) -> &[ListFact<'a>] {
        &self.lists
    }

    pub fn statement_facts(&self) -> &[StatementFact] {
        &self.statement_facts
    }

    pub fn background_semicolon_spans(&self) -> &[Span] {
        &self.background_semicolon_spans
    }

    pub fn single_test_subshell_spans(&self) -> &[Span] {
        &self.single_test_subshell_spans
    }

    pub fn subshell_test_group_spans(&self) -> &[Span] {
        &self.subshell_test_group_spans
    }

    pub fn indented_shebang_span(&self) -> Option<Span> {
        self.indented_shebang_span
    }

    pub fn indented_shebang_indent_span(&self) -> Option<Span> {
        self.indented_shebang_indent_span
    }

    pub fn space_after_hash_bang_span(&self) -> Option<Span> {
        self.space_after_hash_bang_span
    }

    pub fn space_after_hash_bang_whitespace_span(&self) -> Option<Span> {
        self.space_after_hash_bang_whitespace_span
    }

    pub fn shebang_not_on_first_line_span(&self) -> Option<Span> {
        self.shebang_not_on_first_line_span
    }

    pub fn shebang_not_on_first_line_fix_span(&self) -> Option<Span> {
        self.shebang_not_on_first_line_fix_span
    }

    pub fn shebang_not_on_first_line_preferred_newline(&self) -> Option<&'static str> {
        self.shebang_not_on_first_line_preferred_newline
    }

    pub fn missing_shebang_line_span(&self) -> Option<Span> {
        self.missing_shebang_line_span
    }

    pub fn duplicate_shebang_flag_span(&self) -> Option<Span> {
        self.duplicate_shebang_flag_span
    }

    pub fn non_absolute_shebang_span(&self) -> Option<Span> {
        self.non_absolute_shebang_span
    }

    pub fn errexit_enabled_anywhere(&self) -> bool {
        self.errexit_enabled_anywhere
    }

    pub fn commented_continuation_comment_spans(&self) -> &[Span] {
        &self.commented_continuation_comment_spans
    }

    pub fn comment_double_quote_nesting_spans(&self) -> &[Span] {
        &self.comment_double_quote_nesting_spans
    }

    pub fn trailing_directive_comment_spans(&self) -> &[Span] {
        &self.trailing_directive_comment_spans
    }

    pub fn condition_status_capture_spans(&self) -> &[Span] {
        &self.condition_status_capture_spans
    }

    pub fn command_substitution_command_spans(&self) -> &[Span] {
        &self.command_substitution_command_spans
    }

    pub fn backtick_substitution_spans(&self) -> &[Span] {
        &self.backtick_substitution_spans
    }

    pub fn backtick_escaped_parameters(&self) -> &[BacktickEscapedParameter] {
        &self.backtick_escaped_parameters
    }

    pub fn backtick_escaped_parameter_reference_spans(&self) -> &[Span] {
        &self.backtick_escaped_parameter_reference_spans
    }

    pub fn is_backtick_double_escaped_parameter_reference(&self, span: Span) -> bool {
        self.backtick_double_escaped_parameter_spans
            .contains(&span)
    }

    pub fn backtick_command_name_spans(&self) -> &[Span] {
        &self.backtick_command_name_spans
    }

    pub fn dollar_question_after_command_spans(&self) -> &[Span] {
        &self.dollar_question_after_command_spans
    }

    pub fn assignment_like_command_name_spans(&self) -> &[Span] {
        &self.assignment_like_command_name_spans
    }

    pub fn bare_command_name_assignment_spans(&self) -> &[Span] {
        &self.bare_command_name_assignment_spans
    }

    pub fn subshell_assignment_sites(&self) -> &[NamedSpan] {
        &self.subshell_assignment_sites
    }

    pub fn subshell_later_use_sites(&self) -> &[NamedSpan] {
        &self.subshell_later_use_sites
    }

    pub fn unused_heredoc_spans(&self) -> &[Span] {
        &self.unused_heredoc_spans
    }

    pub fn heredoc_missing_end_spans(&self) -> &[Span] {
        &self.heredoc_missing_end_spans
    }

    pub fn heredoc_closer_not_alone_spans(&self) -> &[Span] {
        &self.heredoc_closer_not_alone_spans
    }

    pub fn misquoted_heredoc_close_spans(&self) -> &[Span] {
        &self.misquoted_heredoc_close_spans
    }

    pub fn heredoc_end_space_spans(&self) -> &[Span] {
        &self.heredoc_end_space_spans
    }

    pub fn echo_here_doc_spans(&self) -> &[Span] {
        &self.echo_here_doc_spans
    }

    pub fn spaced_tabstrip_close_spans(&self) -> &[Span] {
        &self.spaced_tabstrip_close_spans
    }

    pub fn plus_equals_assignment_spans(&self) -> &[Span] {
        &self.plus_equals_assignment_spans
    }

    pub fn array_index_arithmetic_spans(&self) -> &[Span] {
        &self.array_index_arithmetic_spans
    }

    pub fn arithmetic_score_line_spans(&self) -> &[Span] {
        &self.arithmetic_score_line_spans
    }

    pub fn dollar_in_arithmetic_spans(&self) -> &[Span] {
        &self.dollar_in_arithmetic_spans
    }

    pub fn single_quoted_fragments(&self) -> &[SingleQuotedFragmentFact] {
        &self.single_quoted_fragments
    }

    pub fn dollar_double_quoted_fragments(&self) -> &[DollarDoubleQuotedFragmentFact] {
        &self.dollar_double_quoted_fragments
    }

    pub fn open_double_quote_fragments(&self) -> &[OpenDoubleQuoteFragmentFact] {
        &self.open_double_quote_fragments
    }

    pub fn suspect_closing_quote_fragments(&self) -> &[SuspectClosingQuoteFragmentFact] {
        &self.suspect_closing_quote_fragments
    }

    pub fn literal_brace_spans(&self) -> &[Span] {
        &self.literal_brace_spans
    }

    pub fn backtick_fragments(&self) -> &[BacktickFragmentFact] {
        &self.backtick_fragments
    }

    pub fn legacy_arithmetic_fragments(&self) -> &[LegacyArithmeticFragmentFact] {
        &self.legacy_arithmetic_fragments
    }

    pub fn positional_parameter_fragments(&self) -> &[PositionalParameterFragmentFact] {
        &self.positional_parameter_fragments
    }

    pub fn positional_parameter_operator_spans(&self) -> &[Span] {
        &self.positional_parameter_operator_spans
    }

    pub fn double_paren_grouping_spans(&self) -> &[Span] {
        &self.double_paren_grouping_spans
    }

    pub fn arithmetic_update_operator_spans(&self) -> &[Span] {
        &self.arithmetic_update_operator_spans
    }

    pub fn base_prefix_arithmetic_spans(&self) -> &[Span] {
        &self.base_prefix_arithmetic_spans
    }

    pub(crate) fn escape_scan_matches(&self) -> &[EscapeScanMatch] {
        &self.escape_scan_matches
    }

    pub fn echo_backslash_escape_word_spans(&self) -> &[Span] {
        &self.echo_backslash_escape_word_spans
    }

    pub fn echo_to_sed_substitution_spans(&self) -> &[Span] {
        &self.echo_to_sed_substitution_spans
    }

    pub fn arithmetic_command_substitution_spans(&self) -> &[Span] {
        &self.arithmetic_command_substitution_spans
    }
    pub fn unicode_smart_quote_spans(&self) -> &[Span] {
        &self.unicode_smart_quote_spans
    }

    pub fn pattern_exactly_one_extglob_spans(&self) -> &[Span] {
        &self.pattern_exactly_one_extglob_spans
    }

    pub fn pattern_literal_spans(&self) -> &[Span] {
        &self.pattern_literal_spans
    }

    pub fn pattern_charclass_spans(&self) -> &[Span] {
        &self.pattern_charclass_spans
    }

    pub fn is_nested_pattern_charclass_span(&self, span: Span) -> bool {
        self.nested_pattern_charclass_spans
            .contains(&FactSpan::new(span))
    }

    pub fn nested_parameter_expansion_fragments(&self) -> &[NestedParameterExpansionFragmentFact] {
        &self.nested_parameter_expansion_fragments
    }

    pub fn indirect_expansion_fragments(&self) -> &[IndirectExpansionFragmentFact] {
        &self.indirect_expansion_fragments
    }

    pub fn indexed_array_reference_fragments(&self) -> &[IndexedArrayReferenceFragmentFact] {
        &self.indexed_array_reference_fragments
    }

    pub fn plain_unindexed_reference_spans(&self) -> &[Span] {
        &self.plain_unindexed_reference_spans
    }

    pub fn parameter_pattern_special_target_fragments(
        &self,
    ) -> &[ParameterPatternSpecialTargetFragmentFact] {
        &self.parameter_pattern_special_target_fragments
    }

    pub fn zsh_parameter_index_flag_fragments(&self) -> &[ZshParameterIndexFlagFragmentFact] {
        &self.zsh_parameter_index_flag_fragments
    }

    pub fn substring_expansion_fragments(&self) -> &[SubstringExpansionFragmentFact] {
        &self.substring_expansion_fragments
    }

    pub fn case_modification_fragments(&self) -> &[CaseModificationFragmentFact] {
        &self.case_modification_fragments
    }

    pub fn replacement_expansion_fragments(&self) -> &[ReplacementExpansionFragmentFact] {
        &self.replacement_expansion_fragments
    }

    pub fn positional_parameter_trim_fragments(&self) -> &[PositionalParameterTrimFragmentFact] {
        &self.positional_parameter_trim_fragments
    }

    pub fn conditional_portability(&self) -> &ConditionalPortabilityFacts {
        &self.conditional_portability
    }

    pub(crate) fn possible_variable_misspelling_scope_compat_name_uses(
        &self,
    ) -> &[ComparableNameUse] {
        self.possible_variable_misspelling_scope_compat_name_uses
            .get_or_init(|| build_possible_variable_misspelling_scope_compat_name_uses(self))
    }
}

fn build_possible_variable_misspelling_scope_compat_name_uses(
    facts: &LinterFacts<'_>,
) -> Vec<ComparableNameUse> {
    if !source_may_have_scope_compat_misspelling(facts.source) {
        return Vec::new();
    }

    let mut uses = Vec::new();
    for word_fact in facts
        .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
        .chain(facts.expansion_word_facts(ExpansionContext::AssignmentValue))
        .chain(facts.case_subject_facts())
    {
        if let Some(name_use) = scope_compat_standalone_parameter_name_use(word_fact.word()) {
            uses.push(name_use);
        }
    }
    for command in facts.commands() {
        visit_command_words_for_substitutions(
            command.command(),
            command.redirects(),
            facts.source,
            &mut |word| {
                collect_scope_compat_derived_name_uses(word, facts.source, &mut uses);
            },
        );
    }
    for word in facts
        .for_headers()
        .iter()
        .flat_map(|header| header.words())
        .chain(facts.select_headers().iter().flat_map(|header| header.words()))
    {
        if let Some(mut name_use) = scope_compat_standalone_parameter_name_use(word.word()) {
            name_use.mark_derived();
            if is_interesting_scope_compat_name_use(
                facts.source,
                name_use.key().as_str(),
                name_use.kind(),
                name_use.span(),
            ) {
                uses.push(name_use);
            }
        }
    }
    uses.extend(build_flag_for_loop_source_name_uses(facts.source).into_iter().filter(|name_use| {
        is_interesting_scope_compat_name_use(
            facts.source,
            name_use.key().as_str(),
            name_use.kind(),
            name_use.span(),
        )
    }));
    dedup_comparable_name_uses(&mut uses);
    uses
}

fn source_may_have_scope_compat_misspelling(source: &str) -> bool {
    source.contains("SHELLSPEC_EXECDIR")
        || source.contains("CFLAGS")
        || source.contains("CPPFLAGS")
        || source.contains("CXXFLAGS")
}

fn scope_compat_standalone_parameter_name_use(word: &Word) -> Option<ComparableNameUse> {
    let name = standalone_comparable_parameter_name(&word.parts)?;
    Some(ComparableNameUse {
        span: word.span,
        key: ComparableNameKey(name.into()),
        kind: ComparableNameUseKind::Parameter,
    })
}

fn collect_scope_compat_derived_name_uses(
    word: &Word,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    let allow_quoted_derived_words =
        analyze_word(word, source, None).quote == WordQuote::FullyQuoted;
    collect_scope_compat_command_substitution_name_uses_in_parts(
        &word.parts,
        source,
        allow_quoted_derived_words,
        uses,
    );
}

fn collect_scope_compat_command_substitution_name_uses_in_parts(
    parts: &[WordPartNode],
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_scope_compat_command_substitution_name_uses_in_parts(
                    parts,
                    source,
                    allow_quoted_derived_words,
                    uses,
                );
            }
            WordPart::CommandSubstitution { body, .. } => {
                collect_scope_compat_command_substitution_name_uses(
                    body,
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

fn collect_scope_compat_command_substitution_name_uses(
    body: &StmtSeq,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    for visit in iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        visit_command_substitution_loop_header_words(visit.command, &mut |word| {
            push_scope_compat_command_substitution_word_use(
                word,
                source,
                allow_quoted_derived_words,
                uses,
            );
        });
        visit_command_argument_words_for_substitutions(visit.command, source, &mut |word| {
            push_scope_compat_command_substitution_word_use(
                word,
                source,
                allow_quoted_derived_words,
                uses,
            );
        });
    }
}

fn push_scope_compat_command_substitution_word_use(
    word: &Word,
    source: &str,
    allow_quoted_derived_words: bool,
    uses: &mut Vec<ComparableNameUse>,
) {
    if !allow_quoted_derived_words && analyze_word(word, source, None).quote == WordQuote::FullyQuoted
    {
        return;
    }
    if let Some(name_use) = scope_compat_standalone_derived_name_use(word, source) {
        uses.push(name_use);
    }
}

fn collect_scope_compat_arithmetic_name_use(
    word: &Word,
    source: &str,
    uses: &mut Vec<ComparableNameUse>,
) {
    if let Some(name_use) = scope_compat_standalone_derived_name_use(word, source) {
        uses.push(name_use);
    }
}

fn scope_compat_standalone_derived_name_use(
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

fn is_interesting_scope_compat_name_use(
    _source: &str,
    name: &str,
    kind: ComparableNameUseKind,
    _span: Span,
) -> bool {
    name == "SHELLSPEC_EXECDIR"
        || name == "SHELLSPEC_SPECDIR"
        || kind == ComparableNameUseKind::Derived && is_reportable_build_flag_family_name(name)
}

fn is_reportable_build_flag_family_name(name: &str) -> bool {
    let Some((_, suffix)) = split_scope_compat_build_flag_family_name(name) else {
        return false;
    };
    matches!(suffix, "CFLAGS" | "CPPFLAGS" | "CXXFLAGS")
}

fn split_scope_compat_build_flag_family_name(name: &str) -> Option<(&str, &'static str)> {
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
fn populate_array_assignment_split_scalar_expansion_spans(
    shell: ShellDialect,
    commands: &[CommandFact<'_>],
    word_nodes: &[WordNode<'_>],
    word_occurrences: &mut [WordOccurrence],
    fact_store: &mut FactStore<'_>,
    word_ids: &[WordOccurrenceId],
) {
    let mut scratch = Vec::new();
    for id in word_ids.iter().copied() {
        collect_array_assignment_split_scalar_expansion_spans(
            shell,
            id,
            commands,
            word_nodes,
            word_occurrences,
            fact_store,
            &mut scratch,
        );
        word_occurrences[id.index()].array_assignment_split_scalar_expansion_spans =
            fact_store.word_spans.push_many(scratch.drain(..));
    }
}

fn collect_array_assignment_split_scalar_expansion_spans(
    shell: ShellDialect,
    id: WordOccurrenceId,
    commands: &[CommandFact<'_>],
    word_nodes: &[WordNode<'_>],
    word_occurrences: &[WordOccurrence],
    fact_store: &FactStore<'_>,
    split_sensitive_spans: &mut Vec<Span>,
) {
    split_sensitive_spans.clear();
    let fact = &word_occurrences[id.index()];
    let word = occurrence_word(word_nodes, fact);
    let derived = word_node_derived(&word_nodes[fact.node_id.index()]);
    split_sensitive_spans.extend(
        fact_store
            .word_spans(derived.unquoted_scalar_expansion_spans)
            .iter()
            .copied(),
    );
    let use_replacement_spans = collect_array_assignment_use_replacement_expansion_spans(word);
    let brace_expansion_spans = word
        .brace_syntax()
        .iter()
        .copied()
        .filter(|_| shell_has_brace_expansion(shell))
        .filter(|brace| brace.expands())
        .map(|brace| brace.span)
        .collect::<Vec<_>>();
    let fact_span = occurrence_span(word_nodes, fact);
    let unquoted_command_substitution_spans =
        fact_store.word_spans(derived.unquoted_command_substitution_spans);

    if !unquoted_command_substitution_spans.is_empty() {
        for command in commands {
            if contains_span_strictly(fact_span, command.span())
                && unquoted_command_substitution_spans
                    .iter()
                    .any(|span| contains_span_strictly(*span, command.span()))
            {
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
    }

    split_sensitive_spans.retain(|span| {
        !use_replacement_spans.contains(span)
            && !brace_expansion_spans
                .iter()
                .any(|brace_span| contains_span(*brace_span, *span))
    });
    sort_and_dedup_spans(split_sensitive_spans);
}

fn shell_has_brace_expansion(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Bash | ShellDialect::Ksh | ShellDialect::Mksh | ShellDialect::Zsh
    )
}

fn build_flag_for_loop_source_name_uses(source: &str) -> Vec<ComparableNameUse> {
    let mut uses = Vec::new();
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let line_without_newline = line.trim_end_matches('\n');
        let trimmed = line_without_newline.trim_start();
        let leading_whitespace = line_without_newline.len() - trimmed.len();
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
                        source_position_at_offset(source, name_start - 2),
                        source_position_at_offset(source, name_start + name_len + 1),
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
        line_start += line.len();
    }
    uses
}

fn is_build_flag_source_name(name: &str) -> bool {
    matches!(
        name,
        "CFLAGS" | "CXXFLAGS" | "CPPFLAGS" | "LDFLAGS" | "GOFLAGS"
    ) || name.ends_with("_CFLAGS")
        || name.ends_with("_CXXFLAGS")
        || name.ends_with("_CPPFLAGS")
        || name.ends_with("_LDFLAGS")
}

fn source_position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }
    let mut position = Position::new();
    for char in source[..target_offset].chars() {
        position.advance(char);
    }
    Some(position)
}

fn assignment_value_span(value: &AssignmentValue) -> Option<Span> {
    match value {
        AssignmentValue::Scalar(word) => Some(word.span),
        AssignmentValue::Compound(_) => None,
    }
}

fn assignment_value_target_for_span<'a>(
    command: &'a CommandFact<'a>,
    span: Span,
) -> Option<(&'a Name, Span)> {
    command_assignments(command.command())
        .iter()
        .chain(
            declaration_operands(command.command())
                .iter()
                .filter_map(|operand| match operand {
                    DeclOperand::Assignment(assignment) => Some(assignment),
                    DeclOperand::Name(_) | DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => None,
                }),
        )
        .filter_map(|assignment| {
            assignment_value_span(&assignment.value)
                .filter(|value_span| contains_span(*value_span, span))
                .map(|value_span| (&assignment.target.name, value_span))
        })
        .min_by_key(|(_, value_span)| value_span.end.offset - value_span.start.offset)
}
