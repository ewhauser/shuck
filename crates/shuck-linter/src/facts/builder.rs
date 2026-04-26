struct LinterFactsBuilder<'a> {
    file: &'a File,
    source: &'a str,
    semantic: &'a SemanticModel,
    _indexer: &'a Indexer,
    _file_context: &'a FileContext,
    shell: ShellDialect,
    ambient_shell_options: AmbientShellOptions,
}

#[derive(Debug, Default)]
struct FactBuildCapacity {
    commands: usize,
    structural_commands: usize,
    functions: usize,
}

#[derive(Debug, Default)]
struct ArithmeticFactSummary {
    array_index_arithmetic_spans: Vec<Span>,
    arithmetic_score_line_spans: Vec<Span>,
    dollar_in_arithmetic_spans: Vec<Span>,
    arithmetic_command_substitution_spans: Vec<Span>,
}

#[derive(Debug, Default)]
struct HeredocFactSummary {
    unused_heredoc_spans: Vec<Span>,
    heredoc_missing_end_spans: Vec<Span>,
    heredoc_closer_not_alone_spans: Vec<Span>,
    misquoted_heredoc_close_spans: Vec<Span>,
    heredoc_end_space_spans: Vec<Span>,
    echo_here_doc_spans: Vec<Span>,
    spaced_tabstrip_close_spans: Vec<Span>,
}

fn estimate_fact_build_capacity(file: &File) -> FactBuildCapacity {
    let mut capacity = FactBuildCapacity::default();
    walk_commands(
        &file.body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit, context| {
            capacity.commands += 1;
            if !context.nested_word_command {
                capacity.structural_commands += 1;
            }
            if matches!(visit.command, Command::Function(_)) {
                capacity.functions += 1;
            }
        },
    );
    capacity
}

impl<'a> LinterFactsBuilder<'a> {
    fn new(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        file_context: &'a FileContext,
        shell: ShellDialect,
        ambient_shell_options: AmbientShellOptions,
    ) -> Self {
        Self {
            file,
            source,
            semantic,
            _indexer: indexer,
            _file_context: file_context,
            shell,
            ambient_shell_options,
        }
    }

    fn build(self) -> LinterFacts<'a> {
        let source = self.source;
        let capacity = estimate_fact_build_capacity(self.file);
        let estimated_word_nodes = capacity.commands.saturating_mul(2);
        let estimated_word_occurrences = capacity.commands.saturating_mul(3);

        let mut commands = Vec::with_capacity(capacity.commands);
        let mut redirect_fact_store = ListArena::new();
        let mut declaration_assignment_probe_store = ListArena::new();
        let mut structural_command_ids = Vec::with_capacity(capacity.structural_commands);
        let mut command_ids_by_span =
            CommandLookupIndex::with_capacity_and_hasher(capacity.commands, Default::default());
        let mut command_parent_ids = Vec::with_capacity(capacity.commands);
        let mut command_child_ids_by_parent = Vec::<Vec<CommandId>>::with_capacity(capacity.commands);
        let mut active_parent_commands = Vec::<OpenParentCommand>::new();
        let mut if_condition_command_ids =
            FxHashSet::with_capacity_and_hasher(capacity.commands / 4, Default::default());
        let mut elif_condition_command_ids =
            FxHashSet::with_capacity_and_hasher(capacity.commands / 8, Default::default());
        let mut binding_values = FxHashMap::default();
        let mut broken_assoc_key_spans = Vec::new();
        let mut comma_array_assignment_spans = Vec::new();
        let mut ifs_literal_backslash_assignment_value_spans = Vec::new();
        let mut word_nodes = Vec::with_capacity(estimated_word_nodes);
        let mut word_spans = ListArena::with_capacity(estimated_word_nodes.saturating_mul(2));
        let mut word_span_scratch = Vec::new();
        let mut word_node_ids_by_span =
            FxHashMap::with_capacity_and_hasher(estimated_word_nodes, Default::default());
        let mut word_occurrences = Vec::with_capacity(estimated_word_occurrences);
        let mut pending_arithmetic_word_occurrences =
            Vec::with_capacity(capacity.commands.saturating_div(4));
        let mut compound_assignment_value_word_spans = FxHashSet::default();
        let mut array_assignment_split_word_ids =
            Vec::with_capacity(capacity.commands.saturating_div(8));
        let mut seen_word_occurrences = FxHashSet::default();
        let mut seen_pending_arithmetic_word_occurrences = FxHashSet::default();
        let mut assoc_binding_visibility_memo = FxHashMap::default();
        let mut pattern_exactly_one_extglob_spans = Vec::new();
        let mut case_pattern_expansions = Vec::new();
        let mut pattern_literal_spans = Vec::new();
        let mut pattern_charclass_spans = Vec::new();
        let mut arithmetic_summary = ArithmeticFactSummary::default();
        let mut surface_fragments = SurfaceFragmentSink::new(self.source);
        let mut functions = Vec::with_capacity(capacity.functions);
        let mut function_body_without_braces_spans = Vec::with_capacity(capacity.functions);
        let redundant_return_status_spans = Vec::new();
        let mut getopts_cases = Vec::new();
        let mut condition_status_capture_spans = Vec::new();
        let mut command_substitution_command_spans = Vec::new();
        let mut arithmetic_update_operator_spans = Vec::new();
        let mut base_prefix_arithmetic_spans = Vec::new();

        walk_commands(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let key = FactSpan::new(command_span(visit.command));
                let id = CommandId::new(commands.len());
                let span = command_span(visit.command);
                while active_parent_commands
                    .last()
                    .is_some_and(|candidate| candidate.end_offset < span.end.offset)
                {
                    active_parent_commands.pop();
                }
                let parent_id = active_parent_commands.last().map(|command| command.id);
                command_parent_ids.push(parent_id);
                command_child_ids_by_parent.push(Vec::new());
                if let Some(parent_id) = parent_id {
                    command_child_ids_by_parent[parent_id.index()].push(id);
                }
                active_parent_commands.push(OpenParentCommand {
                    id,
                    end_offset: span.end.offset,
                });
                let lookup_kind = command_lookup_kind(visit.command);
                let entries = command_ids_by_span.entry(key).or_default();
                let previous = entries.iter().find(|entry| entry.kind == lookup_kind);
                debug_assert!(previous.is_none(), "duplicate command lookup key");
                entries.push(CommandLookupEntry {
                    kind: lookup_kind,
                    id,
                });

                if context.in_if_condition {
                    if_condition_command_ids.insert(id);
                }
                if context.in_elif_condition {
                    elif_condition_command_ids.insert(id);
                }
                collect_binding_values(
                    visit.command,
                    self.semantic,
                    self.source,
                    &mut binding_values,
                );
                collect_broken_assoc_key_spans(
                    visit.command,
                    self.source,
                    &mut broken_assoc_key_spans,
                );
                collect_command_substitution_command_span(
                    visit.command,
                    self.source,
                    &mut command_substitution_command_spans,
                );
                collect_comma_array_assignment_spans(
                    visit.command,
                    self.source,
                    &mut comma_array_assignment_spans,
                );
                collect_ifs_literal_backslash_assignment_value_spans(
                    visit.command,
                    self.source,
                    &mut ifs_literal_backslash_assignment_value_spans,
                );
                let normalized = command::normalize_command(visit.command, self.source);
                let command_start_offset = command_span(visit.command).start.offset;
                let scope = self.semantic.scope_at(command_start_offset);
                let command_zsh_options = effective_command_zsh_options(
                    self.semantic,
                    command_start_offset,
                    &normalized,
                );
                let nested_word_command = context.nested_word_command;
                if !nested_word_command {
                    structural_command_ids.push(id);
                }
                build_word_facts_for_command(
                    visit,
                    self.source,
                    self.semantic,
                    WordFactCommandContext {
                        command_id: id,
                        nested_word_command,
                        scope,
                    },
                    &normalized,
                    command_zsh_options.clone(),
                    WordFactOutputs {
                        word_nodes: &mut word_nodes,
                        word_spans: &mut word_spans,
                        word_span_scratch: &mut word_span_scratch,
                        word_node_ids_by_span: &mut word_node_ids_by_span,
                        word_occurrences: &mut word_occurrences,
                        pending_arithmetic_word_occurrences:
                            &mut pending_arithmetic_word_occurrences,
                        compound_assignment_value_word_spans:
                            &mut compound_assignment_value_word_spans,
                        array_assignment_split_word_ids: &mut array_assignment_split_word_ids,
                        seen_word_occurrences: &mut seen_word_occurrences,
                        seen_pending_arithmetic_word_occurrences:
                            &mut seen_pending_arithmetic_word_occurrences,
                        assoc_binding_visibility_memo: &mut assoc_binding_visibility_memo,
                        case_pattern_expansions: &mut case_pattern_expansions,
                        pattern_literal_spans: &mut pattern_literal_spans,
                        arithmetic: &mut arithmetic_summary,
                        surface: &mut surface_fragments,
                    },
                );
                collect_base_prefix_spans_in_command(
                    visit.command,
                    self.source,
                    &mut base_prefix_arithmetic_spans,
                );
                collect_arithmetic_update_operator_spans_in_command(
                    visit.command,
                    self.semantic,
                    scope,
                    self.source,
                    &mut arithmetic_update_operator_spans,
                );
                for redirect in visit.redirects {
                    if let Some(word) = redirect.word_target() {
                        collect_base_prefix_spans_in_word(
                            word,
                            self.source,
                            &mut base_prefix_arithmetic_spans,
                        );
                        collect_arithmetic_update_operator_spans_in_word(
                            word,
                            self.semantic,
                            self.source,
                            &mut arithmetic_update_operator_spans,
                        );
                    } else if let Some(heredoc) = redirect.heredoc()
                        && heredoc.delimiter.expands_body
                    {
                        collect_arithmetic_update_operator_spans_in_heredoc_body(
                            &heredoc.body.parts,
                            self.semantic,
                            self.source,
                            &mut arithmetic_update_operator_spans,
                        );
                    }
                }
                let redirect_facts = build_redirect_facts(
                    visit.redirects,
                    Some(self.semantic),
                    self.source,
                    command_zsh_options.as_ref(),
                );
                let redirect_fact_range = redirect_fact_store.push_many(redirect_facts);
                let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
                let declaration_assignment_probes = build_declaration_assignment_probes(
                    visit.command,
                    &normalized,
                    self.source,
                    command_zsh_options.as_ref(),
                );
                let declaration_assignment_probe_range =
                    declaration_assignment_probe_store.push_many(declaration_assignment_probes);
                let glued_closing_bracket_operand_span =
                    build_glued_closing_bracket_operand_span(visit.command, self.source);
                let glued_closing_bracket_insert_offset =
                    build_glued_closing_bracket_insert_offset(visit.command, self.source);
                let simple_test =
                    build_simple_test_fact(visit.command, self.source, self._file_context);
                let conditional = build_conditional_fact(visit.command, self.source);
                commands.push(CommandFact {
                    id,
                    key,
                    visit,
                    nested_word_command,
                    scope,
                    normalized,
                    zsh_options: command_zsh_options,
                    redirect_facts: redirect_fact_range,
                    substitution_facts: IdRange::empty(),
                    options,
                    scope_read_source_words: IdRange::empty(),
                    scope_name_read_uses: IdRange::empty(),
                    scope_heredoc_name_read_uses: IdRange::empty(),
                    scope_name_write_uses: IdRange::empty(),
                    declaration_assignment_probes: declaration_assignment_probe_range,
                    glued_closing_bracket_operand_span,
                    glued_closing_bracket_insert_offset,
                    linebreak_in_test_anchor_span: None,
                    linebreak_in_test_insert_offset: None,
                    simple_test,
                    conditional,
                });

                if let Command::Function(function) = visit.command {
                    functions.push(function);
                    if let Some(span) = function_body_without_braces_span(function) {
                        function_body_without_braces_spans.push(span);
                    }
                }

                if !nested_word_command {
                    match visit.command {
                        Command::Compound(CompoundCommand::If(command)) => {
                            collect_condition_status_capture_from_body(
                                &command.condition,
                                &command.then_branch,
                                self.source,
                                &mut condition_status_capture_spans,
                            );

                            let mut previous_condition = &command.condition;
                            for (index, (condition, branch)) in
                                command.elif_branches.iter().enumerate()
                            {
                                if index > 0
                                    || !stmt_seq_contains_nested_control_flow(&command.then_branch)
                                {
                                    collect_condition_status_capture_from_body(
                                        previous_condition,
                                        condition,
                                        self.source,
                                        &mut condition_status_capture_spans,
                                    );
                                }
                                collect_condition_status_capture_from_body(
                                    condition,
                                    branch,
                                    self.source,
                                    &mut condition_status_capture_spans,
                                );
                                previous_condition = condition;
                            }

                            if let Some(else_branch) = &command.else_branch {
                                collect_condition_status_capture_from_body(
                                    previous_condition,
                                    else_branch,
                                    self.source,
                                    &mut condition_status_capture_spans,
                                );
                            }
                        }
                        Command::Compound(CompoundCommand::While(command)) => {
                            collect_condition_status_capture_from_body(
                                &command.condition,
                                &command.body,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                            if let Some(case) =
                                build_getopts_case_fact_for_while(command, self.source)
                            {
                                getopts_cases.push(case);
                            }
                        }
                        Command::Compound(CompoundCommand::Until(command)) => {
                            collect_condition_status_capture_from_body(
                                &command.condition,
                                &command.body,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
                        }
                        Command::Binary(command)
                            if matches!(command.op, BinaryOp::And | BinaryOp::Or) =>
                        {
                            if stmt_terminals_are_test_commands(&command.left, self.source) {
                                collect_status_parameter_spans_in_stmt(
                                    &command.right,
                                    self.source,
                                    &mut condition_status_capture_spans,
                                );
                            }
                        }
                        Command::Simple(_)
                        | Command::Builtin(_)
                        | Command::Decl(_)
                        | Command::Binary(_)
                        | Command::Compound(_)
                        | Command::Function(_)
                        | Command::AnonymousFunction(_) => {}
                    }
                }
            },
        );

        arithmetic_update_operator_spans
            .sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
        arithmetic_update_operator_spans.dedup();

        let mut fact_store = FactStore::empty();
        fact_store.redirect_facts = redirect_fact_store;
        fact_store.declaration_assignment_probes = declaration_assignment_probe_store;
        fact_store.word_spans = word_spans;

        let command_facts_require_source_order = !command_facts_are_source_ordered(&commands);
        let (command_parent_ids, command_child_index) = if !command_facts_require_source_order {
            (
                command_parent_ids,
                CommandChildIndex::from_parent_lists(command_child_ids_by_parent),
            )
        } else {
            let command_parent_ids =
                build_command_parent_ids(&commands, command_facts_require_source_order);
            let mut command_child_ids_by_parent = vec![Vec::new(); commands.len()];
            for (index, parent_id) in command_parent_ids.iter().copied().enumerate() {
                if let Some(parent_id) = parent_id {
                    command_child_ids_by_parent[parent_id.index()].push(CommandId::new(index));
                }
            }
            (
                command_parent_ids,
                CommandChildIndex::from_parent_lists(command_child_ids_by_parent),
            )
        };

        populate_linebreak_in_test_facts(&mut commands, self.source);
        populate_substitution_fact_ranges(
            &mut commands,
            &mut fact_store,
            &command_ids_by_span,
            &command_child_index,
            self.source,
        );

        let presence_tested_names =
            build_presence_tested_names(&commands, self.source, self.semantic);
        let function_headers =
            build_function_header_facts(self.semantic, &functions, &commands, self.source);
        let function_cli_dispatch_facts = build_function_cli_dispatch_facts(
            self.semantic,
            &function_headers,
            self.file,
            self.source,
        );
        collect_condition_status_capture_from_sequences(
            &self.file.body,
            self.source,
            &mut condition_status_capture_spans,
        );
        let mut precise_function_guard_suppressions = Vec::new();
        collect_precise_function_return_guard_suppressions(
            &self.file.body,
            self.source,
            &mut precise_function_guard_suppressions,
        );
        if !precise_function_guard_suppressions.is_empty() {
            condition_status_capture_spans
                .retain(|span| !precise_function_guard_suppressions.contains(span));
        }
        condition_status_capture_spans
            .retain(|span| matches!(span.slice(self.source), "$?" | "${?}"));
        sort_and_dedup_spans(&mut condition_status_capture_spans);
        sort_and_dedup_spans(&mut command_substitution_command_spans);
        sort_and_dedup_case_pattern_expansions(&mut case_pattern_expansions);
        let function_in_alias_spans = build_function_in_alias_spans(&commands, self.source);
        let function_parameter_fallback_spans = build_function_parameter_fallback_spans(
            &commands,
            &structural_command_ids,
            self.source,
        );
        let for_headers = build_for_header_facts(&commands, &command_ids_by_span, self.source);
        let select_headers =
            build_select_header_facts(&commands, &command_ids_by_span, self.source);
        let case_items = build_case_item_facts(&commands, self.source);
        let case_pattern_shadows = build_case_pattern_shadow_facts(&commands, self.source);
        let case_pattern_impossible_spans =
            build_case_pattern_impossible_spans(&commands, self.source);
        let pipelines = build_pipeline_facts(&commands, &command_ids_by_span, &command_child_index);
        populate_scope_fact_ranges(
            &mut commands,
            &mut fact_store,
            &pipelines,
            &if_condition_command_ids,
            source,
        );
        let lists = build_list_facts(
            &commands,
            &command_ids_by_span,
            &command_child_index,
            self.source,
        );
        let completion_registered_function_command_flags =
            build_completion_registered_function_command_flags(
                self.semantic,
                &commands,
                &lists,
                self.source,
            );
        annotate_conditional_assignment_value_paths(self.semantic, &lists, &mut binding_values);
        let statement_facts =
            build_statement_facts(&commands, &command_ids_by_span, &self.file.body);
        let background_semicolon_spans =
            build_background_semicolon_spans(&commands, &case_items, self.source);
        let single_test_subshell_spans =
            build_single_test_subshell_spans(
                &commands,
                &command_ids_by_span,
                &command_child_index,
                self.source,
            );
        let subshell_test_group_spans =
            build_subshell_test_group_spans(
                &commands,
                &command_ids_by_span,
                &command_child_index,
                self.source,
            );
        let shebang_header_facts = build_shebang_header_facts(self.source);
        let errexit_enabled_anywhere = self.ambient_shell_options.errexit
            || shebang_header_facts.enables_errexit
            || commands
                .iter()
                .filter_map(|fact| fact.options().set())
                .any(|set| set.errexit_change == Some(true));
        let pipefail_enabled_anywhere = self.ambient_shell_options.pipefail
            || commands
                .iter()
                .filter_map(|fact| fact.options().set())
                .any(|set| set.pipefail_change == Some(true));
        let commented_continuation_comment_spans =
            build_commented_continuation_comment_spans(self.source, self._indexer);
        let comment_double_quote_nesting_spans =
            build_comment_double_quote_nesting_spans(self.source, self._indexer);
        let trailing_directive_comment_spans = build_trailing_directive_comment_spans(
            self.file,
            &case_items,
            self.source,
            self._indexer,
        );
        let backtick_command_name_spans = build_backtick_command_name_spans(&commands);
        let dollar_question_after_command_spans =
            build_dollar_question_after_command_spans(&self.file.body, self.source);
        let nonpersistent_assignment_spans = build_nonpersistent_assignment_spans(
            self.semantic,
            &commands,
            self.source,
            matches!(self.shell, ShellDialect::Bash) && pipefail_enabled_anywhere,
            command_facts_require_source_order,
        );
        let heredoc_summary =
            build_heredoc_fact_summary(&commands, self.source, self.file.span.end.offset);
        let plus_equals_assignment_spans = build_plus_equals_assignment_spans(&commands);
        let literal_brace_spans = build_literal_brace_spans(
            &word_nodes,
            &word_occurrences,
            CommandFacts::new(&commands, &fact_store),
            &fact_store,
            source,
            self._indexer.region_index().heredoc_ranges(),
        );
        let SurfaceFragmentFacts {
            single_quoted,
            dollar_double_quoted,
            open_double_quotes,
            suspect_closing_quotes,
            backticks,
            legacy_arithmetic,
            positional_parameters,
            positional_parameter_operator_spans,
            unicode_smart_quote_spans,
            pattern_exactly_one_extglob_spans: surface_pattern_exactly_one_extglob_spans,
            pattern_charclass_spans: surface_pattern_charclass_spans,
            parameter_pattern_spans,
            nested_pattern_charclass_spans,
            nested_parameter_expansions,
            indirect_expansions,
            indexed_array_references,
            plain_unindexed_references,
            parameter_pattern_special_targets,
            zsh_parameter_index_flags,
            substring_expansions,
            case_modifications,
            replacement_expansions,
            positional_parameter_trims,
            suppressed_subscript_spans,
            subscript_later_suppression_spans,
            arithmetic_only_suppressed_subscript_spans,
        } = surface_fragments.finish();
        let function_positional_parameter_facts = build_function_positional_parameter_facts(
            self.semantic,
            &commands,
            &positional_parameters,
        );
        let double_paren_grouping_spans = build_double_paren_grouping_spans(&commands, self.source);
        let suppressed_subscript_reference_spans = build_suppressed_subscript_reference_spans(
            self.semantic,
            &suppressed_subscript_spans,
            &arithmetic_only_suppressed_subscript_spans,
        );
        let subscript_later_suppression_reference_spans =
            build_subscript_later_suppression_reference_spans(
                self.semantic,
                &subscript_later_suppression_spans,
            );
        pattern_exactly_one_extglob_spans.extend(surface_pattern_exactly_one_extglob_spans);
        pattern_charclass_spans.extend(surface_pattern_charclass_spans);
        let escape_scan_matches = build_escape_scan_matches(
            &commands,
            &word_nodes,
            &word_occurrences,
            EscapeScanInputs {
                pattern_literal_spans: &pattern_literal_spans,
                pattern_charclass_spans: &pattern_charclass_spans,
                parameter_pattern_spans: &parameter_pattern_spans,
                single_quoted_fragments: &single_quoted,
                backtick_fragments: &backticks,
            },
            EscapeScanContext {
                source: self.source,
            },
        );
        let echo_backslash_escape_word_spans =
            build_echo_backslash_escape_word_spans(&commands, self.source);
        let nested_pattern_charclass_spans = nested_pattern_charclass_spans
            .into_iter()
            .map(FactSpan::new)
            .collect();
        let conditional_portability = build_conditional_portability_facts(
            &commands,
            &elif_condition_command_ids,
            ConditionalPortabilityInputs {
                word_nodes: &word_nodes,
                word_occurrences: &word_occurrences,
                pattern_exactly_one_extglob_spans: &pattern_exactly_one_extglob_spans,
                pattern_charclass_spans: &pattern_charclass_spans,
                parameter_pattern_spans: &parameter_pattern_spans,
                nested_pattern_charclass_spans: &nested_pattern_charclass_spans,
            },
            source,
        );
        let EnvPrefixScopeSpans {
            assignment_scope_spans: env_prefix_assignment_scope_spans,
            expansion_scope_spans: env_prefix_expansion_scope_spans,
        } = build_env_prefix_scope_spans(self.source, &commands);
        word_occurrences.extend(
            pending_arithmetic_word_occurrences
                .into_iter()
                .map(|pending| WordOccurrence {
                    node_id: pending.node_id,
                    command_id: pending.command_id,
                    nested_word_command: pending.nested_word_command,
                    context: WordFactContext::ArithmeticCommand,
                    host_kind: pending.host_kind,
                    runtime_literal: RuntimeLiteralAnalysis::default(),
                    operand_class: None,
                    enclosing_expansion_context: Some(pending.enclosing_expansion_context),
                    array_assignment_split_scalar_expansion_spans: IdRange::empty(),
                }),
        );
        let mut word_index = FxHashMap::<FactSpan, SmallVec<[WordOccurrenceId; 2]>>::default();
        word_index.reserve(word_occurrences.len());
        let mut word_occurrence_offsets_by_command = vec![0usize; commands.len()];
        for fact in &word_occurrences {
            word_occurrence_offsets_by_command[fact.command_id.index()] += 1;
        }
        let mut next_word_occurrence_offset = 0usize;
        let word_occurrence_ids_by_command = word_occurrence_offsets_by_command
            .iter_mut()
            .map(|count| {
                let range = IdRange::from_start_len(next_word_occurrence_offset, *count);
                *count = next_word_occurrence_offset;
                next_word_occurrence_offset = range.end_index();
                range
            })
            .collect::<Vec<_>>();
        let mut word_occurrence_ids =
            vec![WordOccurrenceId::new(0); next_word_occurrence_offset];
        for (index, fact) in word_occurrences.iter().enumerate() {
            let id = WordOccurrenceId::new(index);
            word_index
                .entry(occurrence_key(&word_nodes, fact))
                .or_default()
                .push(id);
            let command_index = fact.command_id.index();
            let offset = word_occurrence_offsets_by_command[command_index];
            word_occurrence_ids[offset] = id;
            word_occurrence_offsets_by_command[command_index] += 1;
        }
        let mut word_occurrence_id_store = ListArena::with_capacity(word_occurrence_ids.len());
        let all_word_occurrence_ids = word_occurrence_id_store.push_many(word_occurrence_ids);
        debug_assert_eq!(all_word_occurrence_ids.start_index(), 0);
        debug_assert_eq!(
            all_word_occurrence_ids.end_index(),
            next_word_occurrence_offset
        );
        fact_store.word_occurrence_ids = word_occurrence_id_store;
        fact_store.word_occurrence_ids_by_command = word_occurrence_ids_by_command;
        populate_array_assignment_split_scalar_expansion_spans(
            self.shell,
            &commands,
            &word_nodes,
            &mut word_occurrences,
            &mut fact_store,
            &array_assignment_split_word_ids,
        );
        let echo_to_sed_substitution_spans = build_echo_to_sed_substitution_spans(
            CommandFacts::new(&commands, &fact_store),
            &pipelines,
            &backticks,
            WordFactLookup {
                nodes: &word_nodes,
                occurrences: &word_occurrences,
                word_index: &word_index,
                fact_store: &fact_store,
                source,
            },
        );
        let assignment_like_command_name_spans =
            build_assignment_like_command_name_spans(&commands, self.source);
        let bare_command_name_assignment_spans = build_bare_command_name_assignment_spans(
            &commands,
            &word_nodes,
            &word_occurrences,
            &word_index,
            source,
        );
        let unquoted_command_argument_use_offsets = build_unquoted_command_argument_use_offsets(
            self.semantic,
            &word_nodes,
            &word_occurrences,
        );
        let brace_variable_before_bracket_spans =
            build_brace_variable_before_bracket_spans(&word_nodes, &word_occurrences, source);
        let alias_definition_expansion_spans = build_alias_definition_expansion_spans(
            &commands,
            &fact_store,
            &word_nodes,
            &word_occurrences,
            &word_index,
            source,
        );
        let innermost_command_ids_by_offset = build_innermost_command_ids_by_offset(
            &commands,
            commands
                .iter()
                .map(|command| command.span().start.offset)
                .collect(),
            command_facts_require_source_order,
        );
        let innermost_command_ids_by_binding_offset = build_innermost_command_ids_by_offset(
            &commands,
            self.semantic
                .bindings()
                .iter()
                .map(|binding| binding.span.start.offset)
                .collect(),
            command_facts_require_source_order,
        );
        let command_dominance_barrier_flags = build_command_dominance_barrier_flags(&commands);
        let c006_suppressing_reference_offsets_by_name =
            build_c006_suppressing_reference_offsets_by_name(
                self.semantic,
                &commands,
                &innermost_command_ids_by_offset,
                &subscript_later_suppression_reference_spans,
            );

        let mut backtick_substitution_spans = word_spans::backtick_substitution_spans(source);
        backtick_substitution_spans.retain(|span| {
            !self
                ._indexer
                .region_index()
                .is_quoted_heredoc(TextSize::new(span.start.offset as u32))
        });
        let backtick_escaped_parameters =
            word_spans::backtick_escaped_parameters(source, &backtick_substitution_spans);
        let backtick_double_escaped_parameter_spans =
            word_spans::backtick_double_escaped_parameter_spans(
                source,
                &backtick_substitution_spans,
            );
        LinterFacts {
            source,
            commands,
            structural_command_ids,
            command_ids_by_span,
            innermost_command_ids_by_offset,
            innermost_command_ids_by_binding_offset,
            command_parent_ids,
            command_dominance_barrier_flags,
            if_condition_command_ids,
            elif_condition_command_ids,
            binding_values,
            broken_assoc_key_spans,
            comma_array_assignment_spans,
            ifs_literal_backslash_assignment_value_spans,
            env_prefix_assignment_scope_spans,
            env_prefix_expansion_scope_spans,
            presence_tested_names: presence_tested_names.global_names,
            nested_presence_test_spans: presence_tested_names.nested_command_spans_by_name,
            c006_presence_tested_names: presence_tested_names.c006_global_names,
            c006_nested_presence_test_spans: presence_tested_names
                .c006_nested_command_spans_by_name,
            c006_suppressing_reference_offsets_by_name,
            presence_test_references_by_name: presence_tested_names.references_by_name,
            presence_test_names_by_name: presence_tested_names.names_by_name,
            possible_variable_misspelling_use_scan: OnceLock::new(),
            possible_variable_misspelling_index: OnceLock::new(),
            possible_variable_misspelling_scope_compat_name_uses: OnceLock::new(),
            suppressed_subscript_reference_spans,
            subscript_later_suppression_reference_spans,
            compound_assignment_value_word_spans,
            word_nodes,
            word_occurrences,
            word_index,
            fact_store,
            unquoted_command_argument_use_offsets,
            array_assignment_split_word_ids,
            brace_variable_before_bracket_spans,
            completion_registered_function_command_flags,
            function_headers,
            function_in_alias_spans,
            alias_definition_expansion_spans,
            function_body_without_braces_spans,
            function_parameter_fallback_spans,
            redundant_return_status_spans,
            for_headers,
            select_headers,
            case_items,
            case_pattern_shadows,
            case_pattern_impossible_spans,
            case_pattern_expansions,
            getopts_cases,
            pipelines,
            lists,
            statement_facts,
            background_semicolon_spans,
            single_test_subshell_spans,
            subshell_test_group_spans,
            indented_shebang_span: shebang_header_facts.indented_shebang_span,
            indented_shebang_indent_span: shebang_header_facts.indented_shebang_indent_span,
            space_after_hash_bang_span: shebang_header_facts.space_after_hash_bang_span,
            space_after_hash_bang_whitespace_span: shebang_header_facts
                .space_after_hash_bang_whitespace_span,
            shebang_not_on_first_line_span: shebang_header_facts.shebang_not_on_first_line_span,
            shebang_not_on_first_line_fix_span: shebang_header_facts
                .shebang_not_on_first_line_fix_span,
            shebang_not_on_first_line_preferred_newline: shebang_header_facts
                .shebang_not_on_first_line_preferred_newline,
            missing_shebang_line_span: shebang_header_facts.missing_shebang_line_span,
            duplicate_shebang_flag_span: shebang_header_facts.duplicate_shebang_flag_span,
            non_absolute_shebang_span: shebang_header_facts.non_absolute_shebang_span,
            errexit_enabled_anywhere,
            commented_continuation_comment_spans,
            comment_double_quote_nesting_spans,
            trailing_directive_comment_spans,
            condition_status_capture_spans,
            command_substitution_command_spans,
            backtick_substitution_spans,
            backtick_escaped_parameters,
            backtick_double_escaped_parameter_spans,
            backtick_command_name_spans,
            dollar_question_after_command_spans,
            assignment_like_command_name_spans,
            bare_command_name_assignment_spans,
            subshell_assignment_sites: nonpersistent_assignment_spans.subshell_assignment_sites,
            subshell_later_use_sites: nonpersistent_assignment_spans.subshell_later_use_sites,
            unused_heredoc_spans: heredoc_summary.unused_heredoc_spans,
            heredoc_missing_end_spans: heredoc_summary.heredoc_missing_end_spans,
            heredoc_closer_not_alone_spans: heredoc_summary.heredoc_closer_not_alone_spans,
            misquoted_heredoc_close_spans: heredoc_summary.misquoted_heredoc_close_spans,
            heredoc_end_space_spans: heredoc_summary.heredoc_end_space_spans,
            echo_here_doc_spans: heredoc_summary.echo_here_doc_spans,
            spaced_tabstrip_close_spans: heredoc_summary.spaced_tabstrip_close_spans,
            plus_equals_assignment_spans,
            array_index_arithmetic_spans: arithmetic_summary.array_index_arithmetic_spans,
            arithmetic_score_line_spans: arithmetic_summary.arithmetic_score_line_spans,
            dollar_in_arithmetic_spans: arithmetic_summary.dollar_in_arithmetic_spans,
            arithmetic_command_substitution_spans: arithmetic_summary
                .arithmetic_command_substitution_spans,
            function_positional_parameter_facts,
            function_cli_dispatch_facts,
            single_quoted_fragments: single_quoted,
            dollar_double_quoted_fragments: dollar_double_quoted,
            open_double_quote_fragments: open_double_quotes,
            suspect_closing_quote_fragments: suspect_closing_quotes,
            literal_brace_spans,
            backtick_fragments: backticks,
            legacy_arithmetic_fragments: legacy_arithmetic,
            positional_parameter_fragments: positional_parameters,
            positional_parameter_operator_spans,
            double_paren_grouping_spans,
            arithmetic_update_operator_spans,
            base_prefix_arithmetic_spans,
            escape_scan_matches,
            echo_backslash_escape_word_spans,
            echo_to_sed_substitution_spans,
            unicode_smart_quote_spans,
            pattern_exactly_one_extglob_spans,
            pattern_literal_spans,
            pattern_charclass_spans,
            nested_pattern_charclass_spans,
            nested_parameter_expansion_fragments: nested_parameter_expansions,
            indirect_expansion_fragments: indirect_expansions,
            indexed_array_reference_fragments: indexed_array_references,
            plain_unindexed_reference_spans: plain_unindexed_references,
            parameter_pattern_special_target_fragments: parameter_pattern_special_targets,
            zsh_parameter_index_flag_fragments: zsh_parameter_index_flags,
            substring_expansion_fragments: substring_expansions,
            case_modification_fragments: case_modifications,
            replacement_expansion_fragments: replacement_expansions,
            positional_parameter_trim_fragments: positional_parameter_trims,
            conditional_portability,
        }
    }
}

fn build_c006_suppressing_reference_offsets_by_name(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    innermost_command_ids_by_offset: &CommandOffsetLookup,
    subscript_later_suppression_reference_spans: &FxHashSet<FactSpan>,
) -> FxHashMap<Name, Vec<usize>> {
    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();

    for reference in semantic.references() {
        if c006_reference_suppresses_later_references(
            semantic,
            commands,
            innermost_command_ids_by_offset,
            subscript_later_suppression_reference_spans,
            reference,
        ) {
            offsets_by_name
                .entry(reference.name.clone())
                .or_default()
                .push(reference.span.start.offset);
        }
    }

    for offsets in offsets_by_name.values_mut() {
        offsets.sort_unstable();
        offsets.dedup();
    }

    offsets_by_name
}

fn c006_reference_suppresses_later_references(
    semantic: &SemanticModel,
    commands: &[CommandFact<'_>],
    innermost_command_ids_by_offset: &CommandOffsetLookup,
    subscript_later_suppression_reference_spans: &FxHashSet<FactSpan>,
    reference: &Reference,
) -> bool {
    semantic.is_guarded_parameter_reference(reference.id)
        || semantic.is_defaulting_parameter_operand_reference(reference.id)
        || c006_subscript_reference_suppresses_later_references(
            commands,
            innermost_command_ids_by_offset,
            subscript_later_suppression_reference_spans,
            reference,
        )
}

fn c006_subscript_reference_suppresses_later_references(
    commands: &[CommandFact<'_>],
    innermost_command_ids_by_offset: &CommandOffsetLookup,
    subscript_later_suppression_reference_spans: &FxHashSet<FactSpan>,
    reference: &Reference,
) -> bool {
    if !subscript_later_suppression_reference_spans.contains(&FactSpan::new(reference.span)) {
        return false;
    }

    precomputed_command_id_for_offset(
        innermost_command_ids_by_offset,
        reference.span.start.offset,
    )
    .and_then(|id| commands.get(id.index()))
    .and_then(CommandFact::static_utility_name)
    .is_none_or(|name| !matches!(name, "unset" | "[" | "[[" | "test"))
}

fn stmt_seq_contains_nested_control_flow(body: &StmtSeq) -> bool {
    body.iter().any(stmt_contains_nested_control_flow)
}

fn stmt_contains_nested_control_flow(stmt: &Stmt) -> bool {
    match &stmt.command {
        Command::Binary(command) => {
            stmt_contains_nested_control_flow(&command.left)
                || stmt_contains_nested_control_flow(&command.right)
        }
        Command::Compound(
            CompoundCommand::If(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::For(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Case(_)
            | CompoundCommand::Always(_),
        ) => true,
        Command::Compound(CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body)) => {
            body.iter().any(stmt_contains_nested_control_flow)
        }
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_ref()
            .is_some_and(|stmt| stmt_contains_nested_control_flow(stmt)),
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => false,
    }
}

fn populate_linebreak_in_test_facts(commands: &mut [CommandFact<'_>], source: &str) {
    for index in 0..commands.len().saturating_sub(1) {
        let (current_slice, next_slice) = commands.split_at_mut(index + 1);
        let current = &mut current_slice[index];
        let next = &next_slice[0];
        let Some((anchor_span, insert_offset)) =
            build_linebreak_in_test_site(current, next, source)
        else {
            continue;
        };

        current.linebreak_in_test_anchor_span = Some(anchor_span);
        current.linebreak_in_test_insert_offset = Some(insert_offset);
    }
}

fn build_linebreak_in_test_site(
    current: &CommandFact<'_>,
    next: &CommandFact<'_>,
    source: &str,
) -> Option<(Span, usize)> {
    if !current.static_utility_name_is("[")
        || !next.static_utility_name_is("]")
        || !next.body_args().is_empty()
    {
        return None;
    }

    let last_arg_is_closing_bracket = current
        .body_args()
        .last()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("]");
    let current_span = current.span();
    if last_arg_is_closing_bracket {
        return None;
    }
    let insert_offset = linebreak_in_test_insert_offset(current_span, source)?;

    let between = source.get(current_span.end.offset..next.span().start.offset)?;
    if !between.chars().all(|char| matches!(char, ' ' | '\t')) {
        return None;
    }

    let anchor_span = current
        .body_args()
        .last()
        .map(|word| word.span)
        .or_else(|| current.body_name_word().map(|word| word.span))
        .map(|span| Span::from_positions(span.end, span.end))
        .unwrap_or_else(|| Span::from_positions(current_span.end, current_span.end));
    Some((anchor_span, insert_offset))
}

fn linebreak_in_test_insert_offset(span: Span, source: &str) -> Option<usize> {
    let text = span.slice(source);
    if text.ends_with("\r\n") {
        Some(span.end.offset - 2)
    } else if text.ends_with('\n') {
        Some(span.end.offset - 1)
    } else {
        None
    }
}

fn sort_and_dedup_case_pattern_expansions(expansions: &mut Vec<CasePatternExpansionFact>) {
    let mut seen = FxHashSet::default();
    expansions.retain(|fact| seen.insert(FactSpan::new(fact.span())));
    expansions.sort_by_key(|fact| (fact.span().start.offset, fact.span().end.offset));
}

#[cfg(test)]
mod builder_tests {
    use shuck_ast::{Position, Span};

    use super::linebreak_in_test_insert_offset;

    #[test]
    fn linebreak_in_test_insert_offset_targets_lf_newlines() {
        let source = "if [ \"$x\" = y\n";
        let span = Span::from_positions(Position::new(), Position::new().advanced_by(source));
        let insert_offset =
            linebreak_in_test_insert_offset(span, source).expect("expected LF insert offset");

        assert_eq!(&source[insert_offset..], "\n");
    }

    #[test]
    fn linebreak_in_test_insert_offset_targets_crlf_newlines() {
        let source = "if [ \"$x\" = y\r\n";
        let span = Span::from_positions(Position::new(), Position::new().advanced_by(source));
        let insert_offset =
            linebreak_in_test_insert_offset(span, source).expect("expected CRLF insert offset");

        assert_eq!(&source[insert_offset..], "\r\n");
    }
}
