struct LinterFactsBuilder<'a> {
    file: &'a File,
    source: &'a str,
    semantic: &'a SemanticModel,
    _indexer: &'a Indexer,
    _file_context: &'a FileContext,
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

impl<'a> LinterFactsBuilder<'a> {
    fn new(
        file: &'a File,
        source: &'a str,
        semantic: &'a SemanticModel,
        indexer: &'a Indexer,
        file_context: &'a FileContext,
    ) -> Self {
        Self {
            file,
            source,
            semantic,
            _indexer: indexer,
            _file_context: file_context,
        }
    }

    fn build(self) -> LinterFacts<'a> {
        let source = self.source;
        let mut commands = Vec::new();
        let mut structural_command_ids = Vec::new();
        let mut command_ids_by_span = CommandLookupIndex::default();
        let mut if_condition_command_ids = FxHashSet::default();
        let mut elif_condition_command_ids = FxHashSet::default();
        let mut binding_values = FxHashMap::default();
        let mut broken_assoc_key_spans = Vec::new();
        let mut comma_array_assignment_spans = Vec::new();
        let mut ifs_literal_backslash_assignment_value_spans = Vec::new();
        let mut word_nodes = Vec::new();
        let mut word_node_ids_by_span = FxHashMap::default();
        let mut word_occurrences = Vec::new();
        let mut compound_assignment_value_word_spans = FxHashSet::default();
        let mut array_assignment_split_word_ids = Vec::new();
        let mut pattern_exactly_one_extglob_spans = Vec::new();
        let mut case_pattern_expansion_spans = Vec::new();
        let mut pattern_literal_spans = Vec::new();
        let mut pattern_charclass_spans = Vec::new();
        let mut arithmetic_summary = ArithmeticFactSummary::default();
        let mut surface_fragments = SurfaceFragmentSink::new(self.source);
        let mut functions = Vec::new();
        let mut function_body_without_braces_spans = Vec::new();
        let mut redundant_return_status_spans = Vec::new();
        let mut getopts_cases = Vec::new();
        let mut condition_status_capture_spans = Vec::new();
        let mut condition_command_substitution_spans = Vec::new();

        for traversed in query::iter_commands_with_context(
            &self.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        ) {
            let visit = traversed.visit;
            let context = traversed.context;
            let key = FactSpan::new(command_span(visit.command));
            let id = CommandId::new(commands.len());
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
            collect_broken_assoc_key_spans(visit.command, self.source, &mut broken_assoc_key_spans);
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
            let command_zsh_options = effective_command_zsh_options(
                self.semantic,
                command_span(visit.command).start.offset,
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
                },
                &normalized,
                command_zsh_options.clone(),
                WordFactOutputs {
                    word_nodes: &mut word_nodes,
                    word_node_ids_by_span: &mut word_node_ids_by_span,
                    word_occurrences: &mut word_occurrences,
                    compound_assignment_value_word_spans: &mut compound_assignment_value_word_spans,
                    array_assignment_split_word_ids: &mut array_assignment_split_word_ids,
                    case_pattern_expansion_spans: &mut case_pattern_expansion_spans,
                    pattern_literal_spans: &mut pattern_literal_spans,
                    arithmetic: &mut arithmetic_summary,
                    surface: &mut surface_fragments,
                },
            );
            let redirect_facts =
                build_redirect_facts(visit.redirects, self.source, command_zsh_options.as_ref());
            let options = CommandOptionFacts::build(visit.command, &normalized, self.source);
            let declaration_assignment_probes = build_declaration_assignment_probes(
                visit.command,
                &normalized,
                self.source,
                command_zsh_options.as_ref(),
            );
            let glued_closing_bracket_operand_span =
                build_glued_closing_bracket_operand_span(visit.command, self.source);
            let simple_test =
                build_simple_test_fact(visit.command, self.source, self._file_context);
            let conditional = build_conditional_fact(visit.command, self.source);
            commands.push(CommandFact {
                id,
                key,
                visit,
                nested_word_command,
                normalized,
                zsh_options: command_zsh_options,
                redirect_facts,
                substitution_facts: Vec::new().into_boxed_slice(),
                options,
                scope_read_source_words: Vec::new().into_boxed_slice(),
                declaration_assignment_probes,
                glued_closing_bracket_operand_span,
                simple_test,
                conditional,
            });

            if let Command::Function(function) = visit.command {
                functions.push(function);
                if let Some(span) = function_body_without_braces_span(function) {
                    function_body_without_braces_spans.push(span);
                }
                collect_terminal_redundant_return_status_spans(
                    function,
                    &mut redundant_return_status_spans,
                );
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
                        for (condition, branch) in &command.elif_branches {
                            collect_condition_status_capture_from_body(
                                previous_condition,
                                condition,
                                self.source,
                                &mut condition_status_capture_spans,
                            );
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
                        if let Some(case) = build_getopts_case_fact_for_while(command, self.source)
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

            match visit.command {
                Command::Compound(CompoundCommand::If(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );

                    for (condition, _) in &command.elif_branches {
                        collect_condition_command_substitution_from_body(
                            condition,
                            self.source,
                            &mut condition_command_substitution_spans,
                        );
                    }
                }
                Command::Compound(CompoundCommand::While(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );
                }
                Command::Compound(CompoundCommand::Until(command)) => {
                    collect_condition_command_substitution_from_body(
                        &command.condition,
                        self.source,
                        &mut condition_command_substitution_spans,
                    );
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

        let substitution_facts =
            build_substitution_facts(&commands, &command_ids_by_span, self.source);
        for (fact, substitutions) in commands.iter_mut().zip(substitution_facts) {
            fact.substitution_facts = substitutions;
        }

        let presence_tested_names = build_presence_tested_names(&commands, self.source);
        let function_headers =
            build_function_header_facts(self.semantic, &functions, &commands, self.source);
        sort_and_dedup_spans(&mut condition_status_capture_spans);
        sort_and_dedup_spans(&mut condition_command_substitution_spans);
        sort_and_dedup_spans(&mut case_pattern_expansion_spans);
        let function_in_alias_spans = build_function_in_alias_spans(&commands, self.source);
        let function_parameter_fallback_spans = build_function_parameter_fallback_spans(
            &commands,
            &structural_command_ids,
            self.source,
        );
        let for_headers = build_for_header_facts(&commands, &command_ids_by_span, self.source);
        let select_headers =
            build_select_header_facts(&commands, &command_ids_by_span, self.source);
        let case_items = build_case_item_facts(&commands);
        let case_pattern_shadows = build_case_pattern_shadow_facts(&commands, self.source);
        let case_pattern_impossible_spans =
            build_case_pattern_impossible_spans(&commands, self.source);
        let pipelines = build_pipeline_facts(&commands, &command_ids_by_span);
        let scope_read_source_words =
            build_scope_read_source_words(&commands, &pipelines, &if_condition_command_ids);
        for (fact, words) in commands.iter_mut().zip(scope_read_source_words) {
            fact.scope_read_source_words = words;
        }
        let lists = build_list_facts(&commands, &command_ids_by_span, self.source);
        annotate_conditional_assignment_shortcuts(self.semantic, &lists, &mut binding_values);
        let statement_facts =
            build_statement_facts(&commands, &command_ids_by_span, &self.file.body);
        let background_semicolon_spans =
            build_background_semicolon_spans(&commands, &case_items, self.source);
        let single_test_subshell_spans =
            build_single_test_subshell_spans(&commands, &command_ids_by_span, self.source);
        let subshell_test_group_spans =
            build_subshell_test_group_spans(&commands, &command_ids_by_span, self.source);
        let shebang_header_facts = build_shebang_header_facts(self.source);
        let errexit_enabled_anywhere = shebang_header_facts.enables_errexit
            || commands
                .iter()
                .filter_map(|fact| fact.options().set())
                .any(|set| set.errexit_change == Some(true));
        let commented_continuation_comment_spans =
            build_commented_continuation_comment_spans(self.source, self._indexer);
        let trailing_directive_comment_spans = build_trailing_directive_comment_spans(
            self.file,
            &case_items,
            self.source,
            self._indexer,
        );
        let backtick_command_name_spans = build_backtick_command_name_spans(&commands);
        let dollar_question_after_command_spans =
            build_dollar_question_after_command_spans(&self.file.body, self.source);
        let nonpersistent_assignment_spans =
            build_nonpersistent_assignment_spans(self.semantic, &commands);
        let heredoc_summary =
            build_heredoc_fact_summary(&commands, self.source, self.file.span.end.offset);
        let plus_equals_assignment_spans = build_plus_equals_assignment_spans(&commands);
        let literal_brace_spans = build_literal_brace_spans(
            &word_nodes,
            &word_occurrences,
            &commands,
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
            parameter_pattern_special_targets,
            zsh_parameter_index_flags,
            substring_expansions,
            case_modifications,
            replacement_expansions,
            star_glob_removals,
            subscript_spans,
        } = surface_fragments.finish();
        let function_positional_parameter_facts = build_function_positional_parameter_facts(
            self.semantic,
            &commands,
            &positional_parameters,
        );
        let double_paren_grouping_spans = build_double_paren_grouping_spans(&commands, self.source);
        let arithmetic_for_update_operator_spans =
            build_arithmetic_for_update_operator_spans(&commands, self.source);
        let base_prefix_arithmetic_spans =
            build_base_prefix_arithmetic_spans(&self.file.body, self.source);
        let subscript_index_reference_spans =
            build_subscript_index_reference_spans(self.semantic, &subscript_spans);
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
                file_context: self._file_context,
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
                nested_pattern_charclass_spans: &nested_pattern_charclass_spans,
            },
            source,
        );
        let EnvPrefixScopeSpans {
            assignment_scope_spans: env_prefix_assignment_scope_spans,
            expansion_scope_spans: env_prefix_expansion_scope_spans,
        } = build_env_prefix_scope_spans(self.source, &commands);
        let mut word_index = FxHashMap::<FactSpan, SmallVec<[WordOccurrenceId; 2]>>::default();
        let mut word_occurrence_ids_by_command =
            vec![SmallVec::<[WordOccurrenceId; 4]>::new(); commands.len()];
        for (index, fact) in word_occurrences.iter().enumerate() {
            let id = WordOccurrenceId::new(index);
            word_index
                .entry(occurrence_key(&word_nodes, fact))
                .or_default()
                .push(id);
            word_occurrence_ids_by_command[fact.command_id.index()].push(id);
        }
        let echo_to_sed_substitution_spans = build_echo_to_sed_substitution_spans(
            &commands,
            &pipelines,
            &backticks,
            &word_nodes,
            &word_occurrences,
            &word_index,
            source,
        );
        let assignment_like_command_name_spans =
            build_assignment_like_command_name_spans(&commands, self.source);
        let bare_command_name_assignment_spans =
            build_bare_command_name_assignment_spans(
                &commands,
                &word_nodes,
                &word_occurrences,
                &word_index,
                source,
            );
        let unquoted_command_argument_use_offsets =
            build_unquoted_command_argument_use_offsets(
                self.semantic,
                &word_nodes,
                &word_occurrences,
            );
        let brace_variable_before_bracket_spans =
            build_brace_variable_before_bracket_spans(&word_nodes, &word_occurrences, source);
        let alias_definition_expansion_spans =
            build_alias_definition_expansion_spans(
                &commands,
                &word_nodes,
                &word_occurrences,
                &word_index,
                source,
            );

        LinterFacts {
            source,
            commands,
            structural_command_ids,
            command_ids_by_span,
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
            subscript_index_reference_spans,
            compound_assignment_value_word_spans,
            word_nodes,
            word_occurrences,
            word_index,
            word_occurrence_ids_by_command,
            unquoted_command_argument_use_offsets,
            array_assignment_split_word_ids,
            brace_variable_before_bracket_spans,
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
            case_pattern_expansion_spans,
            getopts_cases,
            pipelines,
            lists,
            statement_facts,
            background_semicolon_spans,
            single_test_subshell_spans,
            subshell_test_group_spans,
            indented_shebang_span: shebang_header_facts.indented_shebang_span,
            space_after_hash_bang_span: shebang_header_facts.space_after_hash_bang_span,
            shebang_not_on_first_line_span: shebang_header_facts.shebang_not_on_first_line_span,
            missing_shebang_line_span: shebang_header_facts.missing_shebang_line_span,
            duplicate_shebang_flag_span: shebang_header_facts.duplicate_shebang_flag_span,
            non_absolute_shebang_span: shebang_header_facts.non_absolute_shebang_span,
            errexit_enabled_anywhere,
            commented_continuation_comment_spans,
            trailing_directive_comment_spans,
            condition_status_capture_spans,
            condition_command_substitution_spans,
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
            arithmetic_for_update_operator_spans,
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
            parameter_pattern_special_target_fragments: parameter_pattern_special_targets,
            zsh_parameter_index_flag_fragments: zsh_parameter_index_flags,
            substring_expansion_fragments: substring_expansions,
            case_modification_fragments: case_modifications,
            replacement_expansion_fragments: replacement_expansions,
            star_glob_removal_fragments: star_glob_removals,
            conditional_portability,
        }
    }
}
