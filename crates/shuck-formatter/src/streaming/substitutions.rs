use super::*;

impl<'source, 'facts> ShellRenderer<'source, 'facts> {
    pub(super) fn write_rendered_shell_text(&mut self, text: &str) {
        if text.contains('\n') {
            if self.line_start() {
                self.write_indent();
            }
            self.write_verbatim(text);
        } else {
            self.write_text(text);
        }
    }

    pub(super) fn write_text_preserving_current_line_indent(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start() {
            self.indent_column_for_level(self.indent_level())
        } else {
            self.line_indent_column()
        };
        let mut active_heredoc: Option<RenderedHeredocTail> = None;
        let mut remaining = text;
        while !remaining.is_empty() {
            let (line, next, had_newline) = split_first_line(remaining);

            if let Some(heredoc) = active_heredoc.as_ref() {
                if heredoc.strip_tabs {
                    if self.line_start() && !line.is_empty() {
                        self.write_indent_to_column(base_indent_column);
                    }
                    self.push_output_str(line);
                    self.writer.set_line_start(false);
                } else {
                    self.write_verbatim(heredoc.body_line(line));
                }
                if heredoc.closes(line) {
                    active_heredoc = None;
                }
            } else {
                if self.line_start() && !line.is_empty() {
                    self.write_indent_to_column(base_indent_column);
                }
                self.push_output_str(line);
                self.writer.set_line_start(false);
                active_heredoc = rendered_heredoc_tail_start(line);
            }

            if had_newline {
                self.push_output_str(self.line_ending());
                self.writer.set_line_start(true);
            }
            remaining = next;
        }
    }

    pub(super) fn write_command_substitution_assignment_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let base_indent_column = if self.line_start() {
            self.indent_column_for_level(self.indent_level())
        } else {
            self.line_indent_column()
        };
        let starts_with_block_command_substitution = text
            .lines()
            .next()
            .is_some_and(|line| line.trim_end_matches([' ', '\t', '\r']).ends_with("$("));
        let strip_context_indent = !starts_with_block_command_substitution;
        let indent_unit = self.options.indent_unit_columns();
        let inline_pipeline_indent_column = base_indent_column + indent_unit;
        let mut next_pipeline_indent_column = None;
        let mut active_shell_pipeline_indent_column: Option<usize> = None;
        let mut active_shell_line_was_pipeline_stage = false;
        let mut next_block_line_is_pipeline_stage = false;
        let mut next_block_line_aligns_with_command_continuation = false;
        let mut command_continuation_active = false;
        let mut pipeline_quote_state = RenderedLineQuoteState::default();
        let mut remaining = text;
        while !remaining.is_empty() {
            let line_started_as_command_continuation = command_continuation_active;
            let pipeline_indent_column = next_pipeline_indent_column;
            let closes_block_command_substitution = starts_with_block_command_substitution
                && command_substitution_assignment_line_closes_block(remaining);
            let close_line_has_context_indent = closes_block_command_substitution
                && remaining.lines().next().is_some_and(|line| {
                    rendered_line_indent_column(line, self.options()) >= base_indent_column
                });
            let pipeline_stage_indent = self.line_start()
                && !remaining.starts_with('\n')
                && pipeline_indent_column.is_some()
                && !closes_block_command_substitution
                && !remaining
                    .trim_start_matches([' ', '\t', '\r'])
                    .starts_with('\n');
            let add_context_indent = self.line_start()
                && !remaining.starts_with('\n')
                && !pipeline_stage_indent
                && !close_line_has_context_indent
                && command_substitution_assignment_line_needs_context_indent(
                    remaining,
                    self.options(),
                );
            if pipeline_stage_indent {
                self.write_indent_to_column(pipeline_indent_column.unwrap_or_default());
            }
            if add_context_indent {
                self.write_indent_to_column(base_indent_column);
            }

            let (line, next, had_newline) = split_first_line_including_newline(remaining);
            let line = if pipeline_stage_indent {
                line.trim_start_matches([' ', '\t'])
            } else if add_context_indent && strip_context_indent {
                strip_assignment_context_indent(line, base_indent_column, self.options())
            } else {
                line
            };
            if had_newline {
                let adjusted_block_pipeline_stage;
                let line = if starts_with_block_command_substitution
                    && next_block_line_is_pipeline_stage
                    && next_block_line_aligns_with_command_continuation
                    && !pipeline_stage_indent
                    && let Some(shell_indent_column) = active_shell_pipeline_indent_column
                {
                    let target_column = shell_indent_column.saturating_sub(base_indent_column);
                    let line_indent_column = rendered_line_indent_column(line, self.options());
                    if line_indent_column > target_column {
                        adjusted_block_pipeline_stage = Some(rendered_line_with_indent_column(
                            line,
                            target_column,
                            self.options(),
                        ));
                        adjusted_block_pipeline_stage.as_deref().unwrap_or(line)
                    } else {
                        line
                    }
                } else {
                    line
                };
                let emitted_indent_column = emitted_line_indent_column(
                    line,
                    pipeline_indent_column,
                    add_context_indent,
                    base_indent_column,
                    self.options(),
                );
                if let Some(shell_indent_column) = command_substitution_shell_text_indent_column(
                    line,
                    pipeline_quote_state.in_quote(),
                    emitted_indent_column,
                    base_indent_column,
                    indent_unit,
                ) {
                    active_shell_pipeline_indent_column = Some(shell_indent_column);
                    active_shell_line_was_pipeline_stage =
                        pipeline_stage_indent || next_block_line_is_pipeline_stage;
                    next_block_line_is_pipeline_stage = false;
                    next_block_line_aligns_with_command_continuation = false;
                }
                self.push_output_str(line);
                let line_continues_command = !pipeline_quote_state.in_quote()
                    && line_without_continuation_backslash(line.trim_end_matches('\n')).is_some();
                let continuation = command_substitution_pipeline_stage_continuation(
                    line,
                    pipeline_stage_indent,
                    &mut pipeline_quote_state,
                );
                next_pipeline_indent_column = next_command_substitution_pipeline_indent_column(
                    continuation,
                    starts_with_block_command_substitution,
                    inline_pipeline_indent_column,
                    active_shell_pipeline_indent_column,
                    active_shell_line_was_pipeline_stage,
                    indent_unit,
                    pipeline_indent_column,
                );
                if matches!(
                    continuation,
                    CommandSubstitutionPipelineContinuation::StructuralPipe {
                        line_started_in_quote: false
                    }
                ) && starts_with_block_command_substitution
                {
                    next_block_line_is_pipeline_stage = true;
                    next_block_line_aligns_with_command_continuation =
                        line_started_as_command_continuation;
                }
                command_continuation_active = line_continues_command;
                self.writer.set_line_start(true);
                remaining = next;
            } else {
                self.push_output_str(line);
                self.writer.set_line_start(false);
                break;
            }
        }
    }

    pub(super) fn write_shell_text_with_heredoc_tails(
        &mut self,
        text: &str,
        assignment_context: bool,
    ) {
        if assignment_context && !rendered_text_starts_with_block_command_substitution(text) {
            let base_indent_column = if self.line_start() {
                self.indent_column_for_level(self.indent_level())
            } else if self.line_indent_column() > 0 {
                self.line_indent_column()
            } else {
                self.column()
            };
            if self.line_start() {
                self.write_indent_to_column(base_indent_column);
            }
            self.write_shell_text_preserving_heredoc_tails(text, HeredocTailTextMode::Assignment);
            return;
        }
        self.write_shell_text_preserving_heredoc_tails(text, HeredocTailTextMode::Rendered);
    }

    pub(super) fn write_shell_text_preserving_heredoc_tails(
        &mut self,
        text: &str,
        mode: HeredocTailTextMode,
    ) {
        let mut active_heredoc: Option<RenderedHeredocTail> = None;
        let mut rest = text;

        while !rest.is_empty() {
            let (line, next, had_newline) = split_first_line(rest);

            if let Some(heredoc) = active_heredoc.as_ref() {
                self.write_verbatim(heredoc.body_line(line));
                if heredoc.closes(line) {
                    active_heredoc = None;
                }
            } else {
                let heredoc = rendered_heredoc_tail_start(line);
                let normalized = heredoc
                    .is_some()
                    .then(|| normalize_rendered_heredoc_start_spacing(line))
                    .flatten();
                let line = if self.options().space_redirects() {
                    line
                } else {
                    normalized.as_deref().unwrap_or(line)
                };
                match mode {
                    HeredocTailTextMode::Rendered => self.write_text(line),
                    HeredocTailTextMode::Assignment => self.write_verbatim(line),
                }
                active_heredoc = heredoc;
            }

            if had_newline {
                self.push_output_str(self.line_ending());
                self.writer.set_line_start(true);
            }
            rest = next;
        }
    }

    pub(super) fn write_word(&mut self, word: &Word) {
        let mut scratch = self.take_scratch_buffer();
        self.render_word_with_facts_to_buffer(word, &mut scratch);
        if rendered_shell_text_has_heredoc_tail(&scratch)
            && (word_contains_command_heredoc(word)
                || word_source_has_shell_substitution(word, self.source())
                || rendered_text_has_shell_substitution(&scratch))
        {
            self.write_shell_text_with_heredoc_tails(&scratch, true);
        } else if scratch.contains('\n')
            && (word_is_quoted_formattable_command_substitution_only(word, self.source())
                || word_contains_process_substitution(word))
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if word_has_multiline_literal_source(word, self.source()) {
            if scratch.contains('\n')
                && (word_contains_command_substitution(word)
                    || rendered_text_has_shell_substitution(&scratch))
                && let Some(normalized) =
                    normalize_rendered_leading_list_operator_continuations(&scratch)
            {
                self.write_command_substitution_assignment_text(&normalized);
            } else {
                self.write_rendered_shell_text(&scratch);
            }
        } else if scratch.contains('\n')
            && (word_contains_command_substitution(word)
                || rendered_text_has_shell_substitution(&scratch))
        {
            if rendered_text_has_leading_list_operator_line(&scratch) {
                self.write_command_substitution_assignment_text(&scratch);
            } else {
                self.write_text_preserving_current_line_indent(&scratch);
            }
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_pattern(&mut self, pattern: &Pattern) {
        self.write_rendered(|scratch, source, options| {
            render_pattern_syntax_to_buf(pattern, source, options, scratch);
        });
    }

    pub(super) fn write_case_pattern(&mut self, item: &CaseItem, pattern: &Pattern) {
        let mut scratch = self.take_scratch_buffer();
        render_pattern_syntax_to_buf(pattern, self.source(), self.options(), &mut scratch);
        if case_item_pattern_close_paren_on_own_line(item, self.source(), self.source_map()) {
            trim_trailing_pattern_line_continuation(&mut scratch);
        }
        self.write_text(&scratch);
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_var_ref(&mut self, reference: &VarRef) {
        self.write_rendered(|scratch, source, _| {
            render_var_ref_to_buf(reference, source, scratch);
        });
    }

    pub(super) fn write_assignment(&mut self, assignment: &Assignment) {
        if assignment_has_quoted_backslash_continuation_literal(assignment, self.source()) {
            self.write_rendered_shell_text(assignment.span.slice(self.source()));
            return;
        }
        if let Some(normalized) = normalize_scalar_assignment_unquoted_continuations(
            assignment,
            self.source(),
            self.facts(),
        ) {
            self.write_text(&normalized);
            return;
        }

        let source_map = self.source_map().clone();
        let mut scratch = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_assignment_with_facts_to_buf(
                assignment,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut scratch,
            );
        }
        if rendered_shell_text_has_heredoc_tail(&scratch)
            && (assignment_contains_command_heredoc(assignment)
                || assignment_source_has_command_substitution(assignment, self.source())
                || rendered_text_has_shell_substitution(&scratch))
        {
            self.write_shell_text_with_heredoc_tails(&scratch, true);
        } else if scratch.contains('\n')
            && assignment_value_is_quoted_formattable_command_substitution_only(
                assignment,
                self.source(),
            )
        {
            self.write_text_preserving_current_line_indent(&scratch);
        } else if scratch.contains('\n')
            && assignment_source_has_command_substitution(assignment, self.source())
        {
            if compound_assignment_is_single_case_command_substitution(assignment) {
                self.write_text_preserving_current_line_indent(&scratch);
            } else if assignment_has_multiline_literal_source(assignment, self.source()) {
                if assignment_value_is_quoted_command_substitution_only(assignment) {
                    self.write_command_substitution_assignment_text(&scratch);
                } else if assignment_source_has_leading_pipe_continuation(assignment, self.source())
                {
                    self.write_text_preserving_current_line_indent(&scratch);
                } else {
                    let continuation_indent = self
                        .options
                        .indent_prefix(self.indent_level().saturating_add(1));
                    let normalized = normalize_literal_assignment_command_substitution_pipelines(
                        &scratch,
                        &continuation_indent,
                    );
                    self.write_rendered_shell_text(&normalized);
                }
            } else {
                self.write_text_preserving_current_line_indent(&scratch);
            }
        } else if assignment_has_multiline_literal_source(assignment, self.source()) {
            self.write_rendered_shell_text(&scratch);
        } else {
            self.write_text(&scratch);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_assignment_head(&mut self, assignment: &Assignment) {
        self.write_rendered(|scratch, source, _| {
            render_assignment_head_to_buf(assignment, source, scratch);
        });
    }

    pub(super) fn write_rendered_name_text(&mut self, rendered_name: &str) {
        if rendered_shell_text_has_heredoc_tail(rendered_name)
            && rendered_text_has_shell_substitution(rendered_name)
        {
            self.write_shell_text_with_heredoc_tails(
                rendered_name,
                rendered_text_starts_like_assignment_with_substitution(rendered_name),
            );
        } else {
            self.write_text(rendered_name);
        }
    }

    pub(super) fn format_standalone_multiline_compound_assignment(
        &mut self,
        assignment: &shuck_ast::Assignment,
    ) -> Result<()> {
        let source = self.source();
        if compound_assignment_is_single_case_command_substitution(assignment) {
            self.write_assignment(assignment);
            return Ok(());
        }

        if self.format_escaped_multiline_double_quoted_compound_assignment(assignment) {
            return Ok(());
        }

        if self.compound_assignment_should_preserve_multiline_literal_layout(assignment) {
            self.write_multiline_compound_literal_assignment(assignment);
            return Ok(());
        }

        let Some(layout) = multiline_compound_assignment_layout(assignment, source) else {
            self.write_assignment(assignment);
            return Ok(());
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        self.write_standalone_multiline_compound_assignment_layout(&layout);
        Ok(())
    }

    pub(super) fn compound_assignment_source_has_line_continuations(raw: &str) -> bool {
        raw.contains("\\\n") || raw.contains("\\\r\n")
    }

    pub(super) fn compound_assignment_source_has_escaped_multiline_double_quoted_item(
        raw: &str,
    ) -> bool {
        if !Self::compound_assignment_source_has_line_continuations(raw) {
            return false;
        }

        let Some(open) = raw.find('(') else {
            return false;
        };
        let Some(close) = raw.rfind(')') else {
            return false;
        };
        if close <= open {
            return false;
        }

        raw.get(open + 1..close)
            .is_some_and(|body| body.contains("\"\\\n") || body.contains("\"\\\r\n"))
    }

    pub(super) fn compound_assignment_should_preserve_multiline_literal_layout(
        &self,
        assignment: &Assignment,
    ) -> bool {
        let source = self.source();
        if !assignment_has_multiline_literal_source(assignment, source) {
            return false;
        }

        let raw = assignment.span.slice(source);
        !Self::compound_assignment_source_has_line_continuations(raw)
    }

    pub(super) fn format_escaped_multiline_double_quoted_compound_assignment(
        &mut self,
        assignment: &Assignment,
    ) -> bool {
        if !Self::compound_assignment_source_has_escaped_multiline_double_quoted_item(
            assignment.span.slice(self.source()),
        ) {
            return false;
        }

        let AssignmentValue::Compound(array) = &assignment.value else {
            return false;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        for (index, element) in array.elements.iter().enumerate() {
            if index > 0 {
                self.newline();
                self.write_indent_units(1);
            }
            self.write_array_element(element, true);
        }
        self.write_text(")");
        true
    }

    pub(super) fn write_word_with_escaped_multiline_substitution_indent(&mut self, word: &Word) {
        let source_map = self.source_map().clone();
        let mut scratch = self.take_scratch_buffer();
        {
            let facts = self.facts();
            render_escaped_multiline_word_syntax_with_facts_to_buf(
                word,
                self.source(),
                self.options(),
                &source_map,
                facts,
                &mut scratch,
            );
        }

        let normalized =
            normalize_escaped_multiline_word_command_substitution_indent(&scratch, self.options());
        let rendered = normalized.as_deref().unwrap_or(&scratch);
        if rendered.contains('\n')
            && rendered_text_has_shell_substitution(rendered)
            && let Some(normalized) =
                normalize_rendered_leading_list_operator_continuations(rendered)
        {
            self.write_command_substitution_assignment_text(&normalized);
        } else if rendered.contains('\n') {
            self.write_rendered_shell_text(rendered);
        } else {
            self.write_text(rendered);
        }
        self.restore_scratch_buffer(scratch);
    }

    pub(super) fn write_multiline_compound_literal_assignment(&mut self, assignment: &Assignment) {
        let raw = assignment.span.slice(self.source());
        let Some((head, tail)) = raw.split_once('\n') else {
            self.write_text(raw);
            return;
        };

        self.write_text(head);
        let mut quote = multiline_literal_quote_state_after_line(head, None);
        for line in tail.lines() {
            self.newline();
            if quote.is_some() {
                self.write_verbatim(line.trim_end_matches('\r'));
                quote = multiline_literal_quote_state_after_line(line, quote);
                continue;
            }

            let trimmed = line.trim_start_matches([' ', '\t']).trim_end_matches('\r');
            if trimmed.starts_with(')') {
                self.write_text(trimmed);
            } else {
                self.write_indent_units(1);
                self.write_text(trimmed);
            }
            quote = multiline_literal_quote_state_after_line(trimmed, quote);
        }
    }
}
