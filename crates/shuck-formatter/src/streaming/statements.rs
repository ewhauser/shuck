use super::*;

#[derive(Clone, Copy)]
pub(super) enum SimpleCommandPart<'a> {
    Assignment(&'a Assignment),
    Name,
    Argument(&'a Word),
    Redirect(&'a Redirect),
}

impl SimpleCommandPart<'_> {
    fn start_offset(&self, command: &SimpleCommand) -> usize {
        match self {
            Self::Assignment(assignment) => assignment.span.start.offset,
            Self::Name => command.name.span.start.offset,
            Self::Argument(word) => word.span.start.offset,
            Self::Redirect(redirect) => redirect.span.start.offset,
        }
    }

    fn end_offset(&self, command: &SimpleCommand) -> usize {
        match self {
            Self::Assignment(assignment) => assignment.span.end.offset,
            Self::Name => command.name.span.end.offset,
            Self::Argument(word) => word.span.end.offset,
            Self::Redirect(redirect) => redirect.span.end.offset,
        }
    }

    fn bare_command_gap_end(&self, command: &SimpleCommand, source: &str) -> usize {
        match self {
            Self::Argument(word) => word_gap_end_before_trailing_continuation(word, source),
            _ => self.end_offset(command),
        }
    }
}

fn move_interspersed_redirects_after_arguments<'a>(parts: &mut Vec<SimpleCommandPart<'a>>) {
    let mut saw_argument = false;
    let mut needs_reorder = false;

    for part in parts.iter() {
        match part {
            SimpleCommandPart::Argument(_) => saw_argument = true,
            SimpleCommandPart::Redirect(_) if saw_argument => {
                needs_reorder = true;
                break;
            }
            _ => {}
        }
    }

    if !needs_reorder {
        return;
    }

    saw_argument = false;
    let mut deferred_redirects = Vec::new();
    let mut reordered = Vec::with_capacity(parts.len());

    for part in parts.drain(..) {
        match part {
            SimpleCommandPart::Argument(_) => {
                saw_argument = true;
                reordered.push(part);
            }
            SimpleCommandPart::Redirect(_) if saw_argument => deferred_redirects.push(part),
            _ => reordered.push(part),
        }
    }

    reordered.extend(deferred_redirects);
    *parts = reordered;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BinaryListItem<'a> {
    operator: BinaryOp,
    operator_span: Span,
    stmt: &'a Stmt,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MultilineCompoundAssignmentPlacement {
    Inline,
    Standalone,
}

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    pub(super) fn format_stmt_sequence(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> Result<()> {
        self.format_stmt_sequence_with_leading_filter(statements, upper_bound, None)
    }

    pub(super) fn format_stmt_sequence_with_leading_filter(
        &mut self,
        statements: &StmtSeq,
        upper_bound: Option<usize>,
        first_leading_min_offset: Option<usize>,
    ) -> Result<()> {
        let source = self.source();
        let compact_layout = self.options().compact_layout();
        let minify = self.options().minify();
        let attachments = (!minify).then(|| self.facts().sequence(statements, upper_bound).clone());
        let compact = compact_layout
            && attachments
                .as_ref()
                .is_none_or(|sequence| !sequence.has_comments());

        if statements.is_empty() {
            if let Some(attachment) = attachments.as_ref() {
                let comments = attachment.dangling();
                if let Some((first, rest)) = comments.split_first() {
                    self.write_comment(first);
                    let mut previous = first;
                    for comment in rest {
                        self.write_line_breaks(line_gap_break_count(
                            previous.line(),
                            comment.line(),
                        ));
                        self.write_comment(comment);
                        previous = comment;
                    }
                }
            }
            return Ok(());
        }

        if first_leading_min_offset.is_none()
            && attachments
                .as_ref()
                .is_some_and(crate::facts::SequenceFacts::is_ambiguous)
            && let Some(span) = sequence_verbatim_span(statements, self.source_map())
        {
            if let Some(attachment) = attachments.as_ref()
                && let Some(first) = statements.first()
            {
                let leading = attachment
                    .leading_for(0)
                    .iter()
                    .copied()
                    .filter(|comment| comment.span().end.offset <= span.start.offset)
                    .collect::<Vec<_>>();
                self.emit_leading_comments(
                    &leading,
                    self.facts().stmt(first).render_span().start.line,
                );
            }
            self.write_verbatim(span.slice(source));
            if let Some(attachment) = attachments.as_ref() {
                self.emit_dangling_comments(attachment.dangling());
            }
            return Ok(());
        }

        for (index, stmt) in statements.iter().enumerate() {
            if let Some(attachment) = attachments.as_ref() {
                let next_line =
                    stmt_render_start_line(stmt, self.source(), self.source_map(), self.options());
                if index == 0
                    && let Some(min_offset) = first_leading_min_offset
                {
                    let leading = attachment
                        .leading_for(index)
                        .iter()
                        .copied()
                        .filter(|comment| comment.span().start.offset >= min_offset)
                        .collect::<Vec<_>>();
                    self.emit_leading_comments(&leading, next_line);
                } else {
                    self.emit_leading_comments(attachment.leading_for(index), next_line);
                }
            }

            self.format_stmt(stmt)?;

            if let Some(attachment) = attachments.as_ref() {
                self.emit_trailing_comments_for_stmt(attachment.trailing_for(index));
            }

            if index + 1 < statements.len() {
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
                    let current_end = self.stmt_rendered_end_line(stmt);
                    let next_start =
                        self.stmt_sequence_next_start_line(statements, index, attachments.as_ref());
                    let breaks = if stmt_is_redirect_only(&statements[index + 1], source)
                        || !self.facts().background_has_explicit_line_break(stmt)
                    {
                        1
                    } else {
                        line_gap_break_count(current_end, next_start)
                    };
                    self.write_line_breaks(breaks);
                } else if compact {
                    self.write_text("; ");
                } else {
                    let current_end = self.stmt_rendered_end_line(stmt);
                    let next_start =
                        self.stmt_sequence_next_start_line(statements, index, attachments.as_ref());
                    self.write_line_breaks(line_gap_break_count(current_end, next_start));
                }
            }
        }

        self.flush_pending_heredocs();

        if let Some(attachment) = attachments.as_ref() {
            let previous_line = statements
                .last()
                .map(|stmt| self.stmt_rendered_end_line(stmt));
            self.emit_dangling_comments_after(attachment.dangling(), previous_line);
        }
        Ok(())
    }

    pub(super) fn format_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        let source = self.source();
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() {
            let rendered = stmt_facts.render_span().slice(source);
            if matches!(&stmt.command, Command::Simple(command) if simple_command_uses_synthetic_words(command, source))
            {
                self.write_rendered_shell_text(rendered);
            } else {
                self.write_verbatim(rendered);
            }
            return Ok(());
        }

        if stmt.negated {
            self.write_text("! ");
        }

        let command_span = command_format_span(&stmt.command);
        let emit_redirects_first = !stmt.redirects.is_empty()
            && command_span != Span::new()
            && stmt
                .redirects
                .iter()
                .all(|redirect| redirect.span.start.offset < command_span.start.offset);

        if emit_redirects_first {
            self.format_redirect_list(&stmt.redirects);
            if command_span != Span::new() {
                self.write_space();
            }
        }

        let redirects_formatted_with_command = matches!(&stmt.command, Command::Simple(_))
            && !stmt.redirects.is_empty()
            && !emit_redirects_first;

        match &stmt.command {
            Command::Simple(command) if redirects_formatted_with_command => {
                self.format_simple_command_with_redirects(command, &stmt.redirects);
            }
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                self.format_brace_group(commands, Some(stmt_span(stmt).end.offset))?;
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                self.format_subshell(commands, Some(stmt_span(stmt).end.offset))?;
            }
            _ => self.format_command(&stmt.command)?,
        }

        if !stmt.redirects.is_empty() && !emit_redirects_first && !redirects_formatted_with_command
        {
            if redirect_list_starts_on_continuation_line(
                command_span,
                &stmt.redirects,
                self.facts(),
            ) {
                self.line_continuation();
                self.write_indent_units(1);
            } else if redirect_list_needs_leading_space(command_span, &stmt.redirects, source) {
                self.write_space();
            }
            self.format_redirect_list(&stmt.redirects);
        }

        if self.facts().stmt_contains_heredoc(stmt) {
            self.queue_heredocs(&stmt.redirects);
        }

        match stmt.terminator {
            Some(StmtTerminator::Background(operator)) => {
                self.write_space();
                self.write_text(render_background_operator(operator));
            }
            Some(StmtTerminator::Semicolon)
                if stmt_semicolon_terminator_starts_on_continuation_line(
                    stmt,
                    self.source_map(),
                ) =>
            {
                self.line_continuation();
                self.write_indent_units(1);
                self.write_text(";");
            }
            _ => {}
        }

        Ok(())
    }

    pub(super) fn format_command(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Simple(command) => self.format_simple_command(command),
            Command::Builtin(command) => self.format_builtin_command(command),
            Command::Decl(command) => self.format_decl_clause(command),
            Command::Binary(command) => self.format_binary_command(command),
            Command::Compound(compound) => self.format_compound_command(compound),
            Command::Function(function) => self.format_function(function),
            Command::AnonymousFunction(function) => self.format_anonymous_function(function),
        }
    }

    pub(super) fn format_compound_command(&mut self, command: &CompoundCommand) -> Result<()> {
        match command {
            CompoundCommand::If(command) => self.format_if(command),
            CompoundCommand::For(command) => self.format_for(command),
            CompoundCommand::Repeat(command) => self.format_repeat(command),
            CompoundCommand::Foreach(command) => self.format_foreach(command),
            CompoundCommand::ArithmeticFor(command) => self.format_arithmetic_for(command),
            CompoundCommand::While(command) => self.format_while(command),
            CompoundCommand::Until(command) => self.format_until(command),
            CompoundCommand::Case(command) => self.format_case(command),
            CompoundCommand::Select(command) => self.format_select(command),
            CompoundCommand::Subshell(commands) => self.format_subshell(commands, None),
            CompoundCommand::BraceGroup(commands) => self.format_brace_group(commands, None),
            CompoundCommand::Arithmetic(command) => self.format_arithmetic(command),
            CompoundCommand::Time(command) => self.format_time(command),
            CompoundCommand::Conditional(command) => self.format_conditional(command),
            CompoundCommand::Coproc(command) => self.format_coproc(command),
            CompoundCommand::Always(command) => self.format_always(command),
        }
    }

    pub(super) fn format_assignments(&mut self, assignments: &[Assignment]) -> Option<usize> {
        let mut previous_end = None;
        for assignment in assignments {
            self.write_command_gap(previous_end, assignment.span.start.offset);
            self.write_assignment(assignment);
            previous_end = Some(assignment.span.end.offset);
        }
        previous_end
    }

    pub(super) fn format_simple_command(&mut self, command: &SimpleCommand) -> Result<()> {
        let mut rendered_name = self.take_scratch_buffer();
        self.render_word_to_buffer(&command.name, &mut rendered_name);
        if command.args.is_empty()
            && command.assignments.len() == 1
            && rendered_name.is_empty()
            && multiline_compound_assignment_lines(&command.assignments[0], self.source()).is_some()
        {
            self.restore_scratch_buffer(rendered_name);
            return self.format_standalone_multiline_compound_assignment(&command.assignments[0]);
        }

        self.format_simple_command_parts(command, &[], rendered_name);
        Ok(())
    }

    pub(super) fn format_simple_command_with_redirects(
        &mut self,
        command: &SimpleCommand,
        redirects: &[Redirect],
    ) {
        let mut rendered_name = self.take_scratch_buffer();
        self.render_word_to_buffer(&command.name, &mut rendered_name);
        self.format_simple_command_parts(command, redirects, rendered_name);
    }

    pub(super) fn format_simple_command_parts(
        &mut self,
        command: &SimpleCommand,
        redirects: &[Redirect],
        rendered_name: String,
    ) {
        let source = self.source();
        let mut parts = Vec::with_capacity(
            command.assignments.len()
                + usize::from(!rendered_name.is_empty())
                + command.args.len()
                + redirects.len(),
        );
        parts.extend(
            command
                .assignments
                .iter()
                .map(SimpleCommandPart::Assignment),
        );
        if !rendered_name.is_empty() {
            parts.push(SimpleCommandPart::Name);
        }
        parts.extend(command.args.iter().map(SimpleCommandPart::Argument));
        parts.extend(redirects.iter().map(SimpleCommandPart::Redirect));
        parts.sort_by_key(|part| part.start_offset(command));
        move_interspersed_redirects_after_arguments(&mut parts);

        let keep_assignment_continuations_flush_left =
            command.assignments.first().is_some_and(|assignment| {
                assignment_source_has_command_substitution(assignment, source)
            });
        let mut previous_part = None;
        let mut previous_end = None;
        let mut part_index = 0;
        while part_index < parts.len() {
            let part = parts[part_index];
            if matches!(
                (previous_part, part),
                (
                    Some(SimpleCommandPart::Assignment(_)),
                    SimpleCommandPart::Assignment(_)
                ) if keep_assignment_continuations_flush_left
            ) && previous_end.is_some_and(|previous_end| {
                self.facts()
                    .contains_newline_between(previous_end, part.start_offset(command))
            }) {
                self.line_continuation();
            } else if let SimpleCommandPart::Redirect(redirect) = &part {
                self.write_redirect_gap(previous_part, previous_end, redirect, command);
            } else {
                self.write_command_gap(previous_end, part.start_offset(command));
            }
            let end_offset = if redirects.is_empty() {
                part.bare_command_gap_end(command, source)
            } else {
                part.end_offset(command)
            };
            match part {
                SimpleCommandPart::Assignment(assignment) => self.write_assignment(assignment),
                SimpleCommandPart::Name => self.write_rendered_name_text(&rendered_name),
                SimpleCommandPart::Argument(argument) => self.write_word(argument),
                SimpleCommandPart::Redirect(redirect) => {
                    if let Some(SimpleCommandPart::Redirect(next)) =
                        parts.get(part_index + 1).copied()
                        && append_both_redirect_pair_matches_source(redirect, next, source)
                    {
                        self.format_append_both_redirect(redirect);
                        part_index += 1;
                    } else {
                        self.format_redirect(redirect);
                    }
                }
            }
            previous_part = Some(part);
            previous_end = Some(end_offset);
            part_index += 1;
        }
        self.restore_scratch_buffer(rendered_name);
    }

    pub(super) fn write_redirect_gap(
        &mut self,
        previous_part: Option<SimpleCommandPart<'_>>,
        previous_end: Option<usize>,
        redirect: &Redirect,
        command: &SimpleCommand,
    ) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if previous_end == redirect.span.start.offset {
            if redirect_has_adjacent_numeric_fd_prefix(
                previous_part,
                redirect,
                command,
                self.source(),
            ) {
                return;
            }
            if !redirect_is_attached_process_substitution(Span::new(), redirect, self.source()) {
                self.write_space();
            }
            return;
        }
        self.write_command_gap(Some(previous_end), redirect.span.start.offset);
    }

    pub(super) fn format_builtin_command(&mut self, command: &BuiltinCommand) -> Result<()> {
        let (span, name, assignments, primary, extra_args) = builtin_like_parts(command);
        self.format_builtin_like(name, span.start, assignments, primary, extra_args)
    }

    pub(super) fn format_builtin_like(
        &mut self,
        name: &str,
        start: shuck_ast::Position,
        assignments: &[shuck_ast::Assignment],
        primary: Option<&Word>,
        extra_args: &[Word],
    ) -> Result<()> {
        let mut previous_end = self.format_assignments(assignments);
        let name_span = Span::from_positions(start, start.advanced_by(name));
        self.write_command_gap(previous_end, name_span.start.offset);
        self.write_text(name);
        previous_end = Some(name_span.end.offset);
        if let Some(primary) = primary {
            self.write_command_gap(previous_end, primary.span.start.offset);
            self.write_word(primary);
            previous_end = Some(primary.span.end.offset);
        }
        for argument in extra_args {
            self.write_command_gap(previous_end, argument.span.start.offset);
            self.write_word(argument);
            previous_end = Some(argument.span.end.offset);
        }
        Ok(())
    }

    pub(super) fn format_decl_clause(&mut self, command: &DeclClause) -> Result<()> {
        let mut previous_end = self.format_assignments(&command.assignments);
        self.write_command_gap(previous_end, command.variant_span.start.offset);
        self.write_text(command.variant.as_ref());
        previous_end = Some(command.variant_span.end.offset);
        for operand in &command.operands {
            let span = match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
                DeclOperand::Name(name) => name.span,
                DeclOperand::Assignment(assignment) => assignment.span,
            };
            self.write_command_gap(previous_end, span.start.offset);
            self.write_decl_operand(operand);
            previous_end = Some(span.end.offset);
        }
        Ok(())
    }

    pub(super) fn write_command_gap(&mut self, previous_end: Option<usize>, next_start: usize) {
        let Some(previous_end) = previous_end else {
            return;
        };
        if previous_end == next_start {
            return;
        }
        if self
            .source()
            .get(previous_end..next_start)
            .is_some_and(|between| between.contains('\n'))
        {
            self.line_continuation();
            self.write_indent_units(1);
        } else {
            self.write_space();
        }
    }

    pub(super) fn write_word_list_preserving_breaks_after(
        &mut self,
        words: &[Word],
        first_previous_end: Option<usize>,
    ) {
        let mut previous_end = first_previous_end;
        for word in words {
            if let Some(previous_end) = previous_end {
                self.write_command_gap(Some(previous_end), word.span.start.offset);
            } else {
                self.write_space();
            }
            self.write_word(word);
            previous_end = Some(word_gap_end_before_trailing_continuation(
                word,
                self.source(),
            ));
        }
    }

    pub(super) fn format_do_done_body(
        &mut self,
        site: CompoundBodySite<'_>,
        close: &'static str,
    ) -> Result<()> {
        let body = site.body();
        let body_upper_bound = site.renderer_upper_bound();
        let close_span = site.close_span();
        let enclosing_span = site.enclosing_span();
        let has_open_suffix = self
            .facts()
            .sequence(body, Some(body_upper_bound))
            .group_open_suffix_span()
            .is_some();
        if !has_open_suffix {
            if self.can_inline_body_with_upper_bound(body, enclosing_span, Some(body_upper_bound)) {
                self.write_text("; do ");
                self.format_inline_stmts(body)?;
                self.write_text("; ");
                self.write_text(close);
                self.write_close_suffix_after_span(close_span);
                return Ok(());
            }

            let single_stmt_separator = if self.body_starts_with_inline_do_brace_group(body) {
                Some(self.inline_do_brace_group_done_separator(body, enclosing_span))
            } else if self.body_starts_with_inline_do_if(body) {
                Some("; ")
            } else {
                None
            };
            if let Some(separator) = single_stmt_separator {
                self.write_text("; do ");
                self.format_stmt(&body[0])?;
                self.write_text(separator);
                self.write_text(close);
                self.write_close_suffix_after_span(close_span);
                return Ok(());
            }
        }

        self.format_multiline_do_done_body(site, close, "; do")
    }

    pub(super) fn format_split_do_done_body(
        &mut self,
        site: CompoundBodySite<'_>,
        close: &'static str,
    ) -> Result<()> {
        self.format_multiline_do_done_body(site, close, "do")
    }

    pub(super) fn format_multiline_do_done_body(
        &mut self,
        site: CompoundBodySite<'_>,
        close: &'static str,
        open: &'static str,
    ) -> Result<()> {
        let body = site.body();
        let body_upper_bound = site.renderer_upper_bound();
        let close_span = site.close_span();

        self.write_text(open);
        self.write_compound_body_open_suffix(site);
        let preserve_open_blank = self
            .facts()
            .sequence(body, Some(body_upper_bound))
            .has_blank_line_after_open();
        self.format_body_with_upper_bound_and_open_blank(
            body,
            Some(body_upper_bound),
            preserve_open_blank,
        )?;
        self.write_unmodeled_branch_background_terminator(body, body_upper_bound);
        if self
            .facts()
            .sequence(body, Some(body_upper_bound))
            .has_blank_line_before_close()
        {
            self.newline();
        }
        self.finish_block_with_close_suffix(close, close_span);
        Ok(())
    }

    pub(super) fn single_unadorned_compound_stmt<'a>(
        &self,
        body: &'a StmtSeq,
    ) -> Option<&'a CompoundCommand> {
        let [stmt] = body.as_slice() else {
            return None;
        };
        if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
            return None;
        }
        match &stmt.command {
            Command::Compound(command) => Some(command),
            _ => None,
        }
    }

    pub(super) fn source_line_before_offset_ends_with_do(&self, offset: usize) -> bool {
        let source = self.source();
        let line_start = source[..offset]
            .rfind('\n')
            .map_or(0, |offset| offset.saturating_add(1));
        source[line_start..offset]
            .trim_end_matches([' ', '\t', '\r'])
            .ends_with("do")
    }

    pub(super) fn body_starts_with_inline_do_brace_group(&self, body: &StmtSeq) -> bool {
        let Some(CompoundCommand::BraceGroup(commands)) = self.single_unadorned_compound_stmt(body)
        else {
            return false;
        };
        let Some(group_span) = self
            .facts()
            .sequence(commands, None)
            .group_attachment_span()
        else {
            return false;
        };
        self.source_line_before_offset_ends_with_do(group_span.start.offset)
    }

    pub(super) fn body_starts_with_inline_do_if(&self, body: &StmtSeq) -> bool {
        let Some(CompoundCommand::If(command)) = self.single_unadorned_compound_stmt(body) else {
            return false;
        };
        if !matches!(command.syntax, IfSyntax::ThenFi { .. }) {
            return false;
        }
        self.source_line_before_offset_ends_with_do(command.span.start.offset)
    }

    pub(super) fn inline_do_brace_group_done_separator(
        &self,
        body: &StmtSeq,
        enclosing_span: Span,
    ) -> &'static str {
        let [stmt] = body.as_slice() else {
            return "; ";
        };
        let Command::Compound(CompoundCommand::BraceGroup(commands)) = &stmt.command else {
            return "; ";
        };
        let Some(group_span) = self
            .facts()
            .sequence(commands, None)
            .group_attachment_span()
        else {
            return "; ";
        };
        let source = self.source();
        let between = source
            .get(group_span.end.offset..enclosing_span.end.offset)
            .unwrap_or_default()
            .trim_start_matches([' ', '\t', '\r']);
        if between.starts_with(';') {
            return "; ";
        }
        if brace_group_last_stmt_allows_done_without_semicolon(commands) {
            " "
        } else {
            "; "
        }
    }

    pub(super) fn write_decl_operand(&mut self, operand: &DeclOperand) {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => self.write_word(word),
            DeclOperand::Name(name) => self.write_var_ref(name),
            DeclOperand::Assignment(assignment)
                if multiline_compound_assignment_lines(assignment, self.source()).is_some() =>
            {
                self.format_inline_multiline_compound_assignment(assignment);
            }
            DeclOperand::Assignment(assignment) => self.write_assignment(assignment),
        }
    }

    pub(super) fn format_inline_multiline_compound_assignment(&mut self, assignment: &Assignment) {
        if self.multiline_compound_assignment_needs_structural_elements(assignment) {
            self.format_structural_multiline_compound_assignment(assignment);
            return;
        }
        if self.format_escaped_multiline_double_quoted_compound_assignment(assignment) {
            return;
        }
        if self.compound_assignment_should_preserve_multiline_literal_layout(assignment) {
            self.write_multiline_compound_literal_assignment(assignment);
            return;
        }

        let Some(layout) = multiline_compound_assignment_layout(assignment, self.source()) else {
            self.write_assignment(assignment);
            return;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        self.write_multiline_compound_assignment_layout_body(
            &layout,
            MultilineCompoundAssignmentPlacement::Inline,
        );
        self.write_multiline_compound_assignment_layout_close(&layout);
    }

    pub(super) fn multiline_compound_assignment_needs_structural_elements(
        &self,
        assignment: &Assignment,
    ) -> bool {
        let AssignmentValue::Compound(array) = &assignment.value else {
            return false;
        };
        let raw = assignment.span.slice(self.source());
        if !raw.contains("$(")
            || !raw.contains(';')
            || raw.contains('#')
            || raw.contains("\n\n")
            || self
                .facts()
                .assignment_has_multiline_literal_source(assignment, self.source())
        {
            return false;
        }

        array
            .elements
            .iter()
            .any(|element| word_contains_command_substitution(array_elem_parts(element).1))
    }

    pub(super) fn format_structural_multiline_compound_assignment(
        &mut self,
        assignment: &Assignment,
    ) {
        let AssignmentValue::Compound(array) = &assignment.value else {
            self.write_assignment(assignment);
            return;
        };

        self.write_assignment_head(assignment);
        self.write_text("(");
        for element in &array.elements {
            self.newline();
            self.write_indent_units(1);
            self.write_array_element(element, false);
        }
        self.newline();
        self.write_text(")");
    }

    pub(super) fn write_array_element(
        &mut self,
        element: &ArrayElem,
        escaped_multiline_indent: bool,
    ) {
        let (key, value, op) = array_elem_parts(element);
        if let Some(key) = key {
            self.write_keyed_array_element(key, value, op, escaped_multiline_indent);
        } else {
            self.write_array_element_value(value, escaped_multiline_indent);
        }
    }

    pub(super) fn write_keyed_array_element(
        &mut self,
        key: &shuck_ast::Subscript,
        value: &Word,
        op: &str,
        escaped_multiline_indent: bool,
    ) {
        self.write_text("[");
        self.write_rendered(|scratch, source, _| {
            render_subscript_to_buf(key, source, scratch);
        });
        self.write_text("]");
        self.write_text(op);
        self.write_array_element_value(value, escaped_multiline_indent);
    }

    pub(super) fn write_array_element_value(
        &mut self,
        value: &Word,
        escaped_multiline_indent: bool,
    ) {
        if escaped_multiline_indent {
            self.write_word_with_escaped_multiline_substitution_indent(value);
        } else {
            self.write_word(value);
        }
    }

    pub(super) fn write_standalone_multiline_compound_assignment_layout(
        &mut self,
        layout: &crate::command::MultilineCompoundAssignmentLayout,
    ) {
        self.write_multiline_compound_assignment_layout_body(
            layout,
            MultilineCompoundAssignmentPlacement::Standalone,
        );
        self.write_multiline_compound_assignment_layout_close(layout);
    }

    pub(super) fn write_multiline_compound_assignment_layout_body(
        &mut self,
        layout: &crate::command::MultilineCompoundAssignmentLayout,
        placement: MultilineCompoundAssignmentPlacement,
    ) {
        let body_start = if layout.open_inline {
            if let Some(first) = layout.lines.first() {
                self.write_text(first);
            }
            1
        } else {
            0
        };

        if body_start >= layout.lines.len() {
            return;
        }

        let mut inline_command_substitution_open = layout.open_inline
            && layout.lines.first().is_some_and(|line| {
                RawShellText::new(line).has_unclosed_command_substitution_open()
                    && !multiline_compound_assignment_command_substitution_body_prefix(line)
                        .is_empty()
            });
        for (index, line) in layout.lines[body_start..].iter().enumerate() {
            self.newline();
            let closes_inline_assignment =
                layout.close_inline && body_start + index + 1 == layout.lines.len();
            let extra_indent = if inline_command_substitution_open {
                0
            } else {
                multiline_compound_assignment_line_extra_indent(line, closes_inline_assignment)
            };
            match placement {
                MultilineCompoundAssignmentPlacement::Inline => {
                    self.write_indent_units(extra_indent);
                    self.write_text(line);
                }
                MultilineCompoundAssignmentPlacement::Standalone => {
                    self.with_extra_prefix_indent(extra_indent, |formatter| {
                        formatter.write_text(line);
                    });
                }
            }
            if inline_command_substitution_open
                && line.trim_start_matches([' ', '\t']).starts_with(')')
            {
                inline_command_substitution_open = false;
            }
        }
    }

    pub(super) fn write_multiline_compound_assignment_layout_close(
        &mut self,
        layout: &crate::command::MultilineCompoundAssignmentLayout,
    ) {
        if layout.close_inline {
            self.write_text(")");
        } else {
            self.newline();
            self.write_text(")");
        }
    }

    pub(super) fn format_binary_command(&mut self, command: &BinaryCommand) -> Result<()> {
        match command.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => self.format_pipeline(command),
            BinaryOp::And | BinaryOp::Or => self.format_command_list(command),
        }
    }

    pub(super) fn format_pipeline(&mut self, pipeline: &BinaryCommand) -> Result<()> {
        let mut statements = Vec::new();
        let mut operators = Vec::new();
        collect_pipeline_parts(pipeline, &mut statements, &mut operators, &|command| {
            (command.op, command.op_span)
        });

        let mut operator_breaks =
            pipeline_operator_breaks(&statements, &operators, self.source(), self.source_map());
        if self.facts().pipeline_has_explicit_line_break(pipeline)
            && !operator_breaks.iter().any(|broken| *broken)
        {
            operator_breaks.fill(true);
        }
        let operator_next_line = self.options().binary_next_line();

        for (index, stmt) in statements.iter().enumerate() {
            if index == 0 {
                self.format_pipeline_stmt(stmt)?;
                continue;
            }
            if index > 0 {
                let (operator, operator_span) = operators
                    .get(index - 1)
                    .map(|(operator, span)| (binary_operator(operator), *span))
                    .unwrap_or(("|", stmt.span));
                let break_here = operator_breaks.get(index - 1).copied().unwrap_or(false);
                if break_here && operator_next_line {
                    self.line_continuation();
                    self.with_extra_prefix_indent(
                        self.pipeline_continuation_indent,
                        |formatter| {
                            formatter.write_text(operator);
                            formatter.write_space();
                            formatter.format_pipeline_stmt_after_operator(stmt, operator_span)
                        },
                    )?;
                    continue;
                }
                if break_here {
                    self.write_space();
                    self.write_text(operator);
                    self.newline();
                    self.emit_pipeline_interstitial_comments(stmt, operator_span);
                    self.with_extra_prefix_indent(
                        self.pipeline_continuation_indent,
                        |formatter| {
                            formatter.format_pipeline_stmt_after_operator(stmt, operator_span)
                        },
                    )?;
                    continue;
                }
                self.write_space();
                self.write_text(operator);
                self.write_space();
            }
            self.format_stmt(stmt)?;
        }

        Ok(())
    }

    pub(super) fn emit_pipeline_interstitial_comments(&mut self, stmt: &Stmt, operator_span: Span) {
        if stmt.leading_comments.iter().any(|comment| {
            self.source_map()
                .source_comment(*comment)
                .is_some_and(|comment| {
                    !comment.inline() && comment.span().start.offset >= operator_span.end.offset
                })
        }) {
            return;
        }
        let command_start = interstitial_comment_end(
            stmt,
            operator_span.end.offset,
            self.source(),
            self.source_map(),
        );
        if command_start <= operator_span.end.offset {
            return;
        }
        let comments = self.own_line_comments_in_region(operator_span.end.offset, command_start);
        for comment in comments {
            self.with_extra_prefix_indent(self.pipeline_continuation_indent, |formatter| {
                formatter.write_text(&comment.text);
            });
            self.newline();
        }
    }

    pub(super) fn own_line_comments_in_region(
        &self,
        start: usize,
        end: usize,
    ) -> Vec<BranchPrefixComment> {
        self.facts().own_line_comments_in_region(start, end)
    }

    pub(super) fn format_pipeline_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        self.format_pipeline_stmt_with_leading_comment_start(stmt, None)
    }

    pub(super) fn format_pipeline_stmt_after_operator(
        &mut self,
        stmt: &Stmt,
        operator_span: Span,
    ) -> Result<()> {
        self.format_pipeline_stmt_with_leading_comment_start(stmt, Some(operator_span.end.offset))
    }

    pub(super) fn format_pipeline_stmt_with_leading_comment_start(
        &mut self,
        stmt: &Stmt,
        min_comment_start: Option<usize>,
    ) -> Result<()> {
        let statement_start =
            stmt_attachment_span(stmt, self.source(), self.source_map(), self.options())
                .start
                .offset;
        let next_line =
            stmt_render_start_line(stmt, self.source(), self.source_map(), self.options());
        let leading = stmt
            .leading_comments
            .iter()
            .filter_map(|comment| self.source_map().source_comment(*comment))
            .filter(|comment| {
                !comment.inline()
                    && comment.span().end.offset <= statement_start
                    && min_comment_start
                        .is_none_or(|min_start| comment.span().start.offset >= min_start)
            })
            .collect::<Vec<_>>();
        if let Some(operator_end) = min_comment_start {
            self.emit_pipeline_leading_comments_after_operator(&leading, next_line, operator_end);
        } else {
            self.emit_leading_comments(&leading, next_line);
        }
        self.format_stmt(stmt)
    }

    pub(super) fn format_command_list(&mut self, list: &BinaryCommand) -> Result<()> {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.format_stmt(first)?;
        for item in &rest {
            self.format_command_list_item(item, Self::format_stmt, true)?;
        }
        Ok(())
    }

    pub(super) fn format_command_list_item(
        &mut self,
        item: &BinaryListItem<'_>,
        format_stmt: fn(&mut Self, &Stmt) -> Result<()>,
        pipeline_indent: bool,
    ) -> Result<()> {
        if self
            .facts()
            .list_item_has_explicit_line_break(item.operator_span)
        {
            self.write_text(list_item_separator(item.operator, false));
            self.newline();
            self.with_indent(|formatter| {
                let emitted_interstitial_comments = formatter
                    .emit_command_list_interstitial_comments(item.stmt, item.operator_span);
                if pipeline_indent && stmt_is_pipeline(item.stmt) {
                    formatter.with_pipeline_continuation_indent(0, |formatter| {
                        formatter.with_group_body_leading_filter(
                            emitted_interstitial_comments,
                            |formatter| format_stmt(formatter, item.stmt),
                        )
                    })
                } else {
                    formatter.with_group_body_leading_filter(
                        emitted_interstitial_comments,
                        |formatter| format_stmt(formatter, item.stmt),
                    )
                }
            })?;
            return Ok(());
        }

        self.write_text(list_item_separator(item.operator, true));
        format_stmt(self, item.stmt)
    }

    pub(super) fn emit_command_list_interstitial_comments(
        &mut self,
        stmt: &Stmt,
        operator_span: Span,
    ) -> bool {
        let command_start = interstitial_comment_end(
            stmt,
            operator_span.end.offset,
            self.source(),
            self.source_map(),
        );
        if command_start <= operator_span.end.offset {
            return false;
        }
        let comments = self.own_line_comments_in_region(operator_span.end.offset, command_start);
        let emitted = !comments.is_empty();
        for comment in comments {
            self.write_text(&comment.text);
            self.newline();
        }
        emitted
    }

    pub(super) fn format_inline_stmts(&mut self, commands: &StmtSeq) -> Result<()> {
        for (index, stmt) in commands.iter().enumerate() {
            if index > 0 {
                if matches!(
                    commands[index - 1].terminator,
                    Some(StmtTerminator::Background(_))
                ) {
                    self.write_space();
                } else {
                    self.write_text("; ");
                }
            }
            self.format_inline_stmt(stmt)?;
        }
        Ok(())
    }

    pub(super) fn format_inline_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        if let Stmt {
            command: Command::Compound(CompoundCommand::Case(command)),
            negated: false,
            redirects,
            terminator: None,
            ..
        } = stmt
            && redirects.is_empty()
            && self.can_format_case_inline(command)
        {
            return self.format_inline_case(command);
        }

        if let Stmt {
            command: Command::Binary(command),
            negated: false,
            redirects,
            terminator: None,
            ..
        } = stmt
            && redirects.is_empty()
            && matches!(command.op, BinaryOp::And | BinaryOp::Or)
            && self.command_list_needs_inline_case(command)
        {
            return self.format_inline_command_list(command);
        }

        self.format_stmt(stmt)
    }

    pub(super) fn command_list_needs_inline_case(&self, list: &BinaryCommand) -> bool {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.stmt_is_inline_case(first)
            || rest.iter().any(|item| self.stmt_is_inline_case(item.stmt))
    }

    pub(super) fn stmt_is_inline_case(&self, stmt: &Stmt) -> bool {
        matches!(
            stmt,
            Stmt {
                command: Command::Compound(CompoundCommand::Case(command)),
                negated: false,
                redirects,
                terminator: None,
                ..
            } if redirects.is_empty() && self.can_format_case_inline(command)
        )
    }

    pub(super) fn format_inline_command_list(&mut self, list: &BinaryCommand) -> Result<()> {
        let mut rest = Vec::new();
        let first = collect_command_list_first(list, &mut rest);
        self.format_inline_stmt(first)?;
        for item in &rest {
            self.format_command_list_item(item, Self::format_inline_stmt, false)?;
        }
        Ok(())
    }

    pub(super) fn can_format_case_inline(&self, command: &CaseCommand) -> bool {
        command.cases.iter().all(|item| {
            item.body.is_empty()
                || item.body.len() == 1
                    && (self.facts().case_item_was_inline_in_source(item)
                        || case_item_pattern_body_terminator_was_inline_in_source(
                            item,
                            self.source(),
                        )
                        || case_item_body_was_inline_without_terminator(item))
                    && !self
                        .facts()
                        .sequence(&item.body, Some(command.span.end.offset))
                        .has_comments()
        })
    }

    pub(super) fn format_inline_case(&mut self, command: &CaseCommand) -> Result<()> {
        let esac_span =
            last_shell_keyword_span(self.source(), self.source_map(), command.span, "esac");
        self.write_text("case ");
        self.write_word(&command.word);
        self.write_text(" in");
        for item in &command.cases {
            self.write_space();
            for (index, pattern) in item.patterns.iter().enumerate() {
                if index > 0 {
                    self.write_text(" | ");
                }
                self.write_pattern(pattern);
            }
            self.write_text(")");
            if item.body.is_empty() {
                self.write_space();
                self.write_text(case_terminator(item.terminator));
            } else {
                self.write_space();
                self.format_inline_stmts(&item.body)?;
                self.write_space();
                self.write_text(case_terminator(item.terminator));
            }
        }
        self.write_text(" esac");
        self.write_close_suffix_after_span(esac_span);
        Ok(())
    }
}

fn redirect_has_adjacent_numeric_fd_prefix(
    previous_part: Option<SimpleCommandPart<'_>>,
    redirect: &Redirect,
    command: &SimpleCommand,
    source: &str,
) -> bool {
    if matches!(redirect.kind, RedirectKind::OutputBoth) {
        return false;
    }
    let Some(SimpleCommandPart::Argument(word)) = previous_part else {
        return false;
    };
    if word.span.end.offset != redirect.span.start.offset {
        return false;
    }
    let Some(raw) = source.get(word.span.start.offset..word.span.end.offset) else {
        return false;
    };
    raw.chars().all(|ch| ch.is_ascii_digit())
        && word.span.start.offset > command.name.span.end.offset
}

fn brace_group_last_stmt_allows_done_without_semicolon(commands: &StmtSeq) -> bool {
    let Some(last) = commands.last() else {
        return false;
    };
    command_allows_done_without_semicolon(&last.command)
}

fn command_allows_done_without_semicolon(command: &Command) -> bool {
    match command {
        Command::Compound(command) => compound_allows_done_without_semicolon(command),
        Command::Binary(binary) => command_allows_done_without_semicolon(&binary.right.command),
        _ => false,
    }
}

fn compound_allows_done_without_semicolon(command: &CompoundCommand) -> bool {
    match command {
        CompoundCommand::Case(_) => true,
        CompoundCommand::BraceGroup(commands)
        | CompoundCommand::For(ForCommand { body: commands, .. })
        | CompoundCommand::Repeat(RepeatCommand { body: commands, .. })
        | CompoundCommand::Foreach(ForeachCommand { body: commands, .. })
        | CompoundCommand::While(WhileCommand { body: commands, .. })
        | CompoundCommand::Until(UntilCommand { body: commands, .. })
        | CompoundCommand::Select(SelectCommand { body: commands, .. }) => {
            brace_group_last_stmt_allows_done_without_semicolon(commands)
        }
        CompoundCommand::ArithmeticFor(command) => {
            brace_group_last_stmt_allows_done_without_semicolon(&command.body)
        }
        _ => false,
    }
}

fn sequence_verbatim_span(statements: &StmtSeq, source_map: &SourceMap<'_>) -> Option<Span> {
    statements
        .iter()
        .map(|stmt| stmt_verbatim_span_with_source_map(stmt, source_map))
        .reduce(Span::merge)
}

fn multiline_compound_assignment_line_extra_indent(
    line: &str,
    closes_inline_assignment: bool,
) -> usize {
    if line.is_empty() {
        return 0;
    }
    if closes_inline_assignment && line == ")" {
        return 0;
    }
    if closes_inline_assignment
        && let Some(rest) = line.strip_prefix(')')
        && !rest.is_empty()
        && !rest.starts_with([' ', '\t'])
    {
        return 0;
    }
    1
}

fn stmt_is_pipeline(stmt: &Stmt) -> bool {
    matches!(
        &stmt.command,
        Command::Binary(command) if matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    )
}

fn stmt_is_redirect_only(stmt: &Stmt, source: &str) -> bool {
    matches!(
        &stmt.command,
        Command::Simple(command)
            if command.assignments.is_empty()
                && command.args.is_empty()
                && stmt_source_starts_with_redirect(stmt, source)
    )
}

fn stmt_source_starts_with_redirect(stmt: &Stmt, source: &str) -> bool {
    let text = stmt_span(stmt)
        .slice(source)
        .trim_start_matches([' ', '\t']);
    let bytes = text.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    matches!(bytes.get(index), Some(b'<' | b'>'))
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItem<'a>>,
) -> &'a Stmt {
    collect_binary_list_first_with(command, rest, &|command| BinaryListItem {
        operator: command.op,
        operator_span: command.op_span,
        stmt: command.right.as_ref(),
    })
}

fn list_item_separator(operator: BinaryOp, inline: bool) -> &'static str {
    match (operator, inline) {
        (BinaryOp::And, true) => " && ",
        (BinaryOp::And, false) => " &&",
        (BinaryOp::Or, true) => " || ",
        (BinaryOp::Or, false) => " ||",
        (BinaryOp::Pipe | BinaryOp::PipeAll, true) => "; ",
        (BinaryOp::Pipe | BinaryOp::PipeAll, false) => ";",
    }
}
