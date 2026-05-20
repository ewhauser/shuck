use super::*;

impl<'source, 'facts> ShellRenderer<'source, 'facts> {
    pub(super) fn can_inline_body(&self, commands: &StmtSeq, enclosing_span: Span) -> bool {
        self.can_inline_body_with_upper_bound(
            commands,
            enclosing_span,
            Some(enclosing_span.end.offset),
        )
    }

    pub(super) fn can_inline_body_with_upper_bound(
        &self,
        commands: &StmtSeq,
        enclosing_span: Span,
        upper_bound: Option<usize>,
    ) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };
        if matches!(command.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(command)
        {
            return false;
        }

        if self.facts().sequence(commands, upper_bound).has_comments() {
            return false;
        }

        self.options().compact_layout()
            || stmt_span(command).start.line == enclosing_span.start.line
    }

    pub(super) fn can_inline_group(&self, commands: &StmtSeq, open_char: char) -> bool {
        let [command] = commands.as_slice() else {
            return false;
        };

        self.can_inline_stmt(command)
            && self.can_inline_body(commands, stmt_span(command))
            && (stmt_span(command).start.line == stmt_span(command).end.line
                || self.group_delimiters_attach_to_wrapped_body(commands, open_char))
    }

    pub(super) fn group_has_inline_source_shape(
        &self,
        commands: &StmtSeq,
        open_char: char,
    ) -> bool {
        self.facts().group_was_inline_in_source(commands)
            || self.group_delimiters_attach_to_wrapped_body(commands, open_char)
    }

    pub(super) fn group_delimiters_attach_to_wrapped_body(
        &self,
        commands: &StmtSeq,
        open_char: char,
    ) -> bool {
        let (Some(first), Some(last)) = (commands.first(), commands.last()) else {
            return false;
        };
        let Some(group_span) = group_attachment_span(
            commands.as_slice(),
            self.source_map(),
            open_char,
            matching_group_close(open_char),
        ) else {
            return false;
        };

        group_span.start.line == stmt_format_span(first).start.line
            && group_span.end.line == stmt_format_span(last).end.line
    }

    pub(super) fn can_inline_source_line_subshell(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        if self.facts().sequence(commands, upper_bound).has_comments()
            || self.facts().stmt(stmt).preserve_verbatim()
            || self.facts().stmt(stmt).has_trailing_comment()
        {
            return false;
        }
        if commands.span.start.line != commands.span.end.line {
            return false;
        }

        true
    }

    pub(super) fn can_format_multiline_subshell_inline(
        &self,
        commands: &StmtSeq,
        upper_bound: Option<usize>,
    ) -> bool {
        let [stmt] = commands.as_slice() else {
            return false;
        };
        if self
            .facts()
            .sequence(commands, upper_bound)
            .group_open_suffix_span()
            .is_some()
            || self.facts().sequence(commands, upper_bound).has_comments()
        {
            return false;
        }
        let Some(group_span) =
            group_attachment_span(commands.as_slice(), self.source_map(), '(', ')')
        else {
            return false;
        };
        let group_source = group_span.slice(self.source());
        if !group_source.contains('\n')
            || group_source.contains("\\\n")
            || group_source.contains("\\\r\n")
        {
            return false;
        }

        let source = self.source();
        let first_start = stmt_span(stmt).start.offset.min(source.len());
        let open_end = group_span.start.offset.saturating_add('('.len_utf8());
        if source
            .get(open_end..first_start)
            .is_none_or(|between| between.contains('\n'))
        {
            return false;
        }

        let close_offset = group_close_offset(source, group_span, upper_bound, ')', ')'.len_utf8());
        let stmt_end = stmt_span(stmt)
            .end
            .offset
            .min(close_offset)
            .min(source.len());
        source
            .get(stmt_end..close_offset)
            .is_some_and(|between| !between.contains('\n'))
    }

    pub(super) fn can_inline_stmt(&self, stmt: &Stmt) -> bool {
        let stmt_facts = self.facts().stmt(stmt);
        if stmt_facts.preserve_verbatim() || stmt_facts.has_trailing_comment() {
            return false;
        }

        matches!(
            &stmt.command,
            Command::Simple(_)
                | Command::Builtin(_)
                | Command::Decl(_)
                | Command::Function(_)
                | Command::Binary(_)
                | Command::Compound(
                    CompoundCommand::Conditional(_)
                        | CompoundCommand::Arithmetic(_)
                        | CompoundCommand::Time(_)
                )
        )
    }
}
