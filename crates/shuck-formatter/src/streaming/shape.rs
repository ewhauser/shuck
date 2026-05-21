use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ThenFiIfLayout {
    RawGroupedCondition { raw_condition: String },
    SplitCondition,
    InlineThen,
    InlineThenElse,
    InlineThenMultilineElse,
    InlineThenNestedIf,
    InlineChain,
    Expanded(ExpandedThenFiIfLayout),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpandedThenFiIfLayout {
    Compact,
    Multiline { inline_else_close: bool },
}

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    pub(super) fn then_fi_if_layout(
        &self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
    ) -> ThenFiIfLayout {
        let source = self.source();
        let fi_upper_bound = fi_span.start.offset;
        let no_elifs = command.elif_branches.is_empty();

        if no_elifs
            && let Some(raw_condition) = raw_grouped_if_condition(
                command,
                then_span,
                source,
                self.source_map(),
                self.options(),
                self.facts(),
            )
        {
            return ThenFiIfLayout::RawGroupedCondition { raw_condition };
        }

        if if_condition_starts_after_keyword(
            command,
            then_span,
            source,
            self.source_map(),
            self.options(),
            self.facts(),
        ) || if_condition_has_explicit_statement_break(
            command,
            then_span,
            source,
            self.source_map(),
            self.facts(),
        ) {
            return ThenFiIfLayout::SplitCondition;
        }

        let can_inline_then = no_elifs
            && self.can_inline_body_with_upper_bound(
                &command.then_branch,
                command.span,
                Some(fi_upper_bound),
            );

        if no_elifs && command.else_branch.is_none() && can_inline_then {
            return ThenFiIfLayout::InlineThen;
        }

        if can_inline_then && let Some(else_branch) = &command.else_branch {
            let can_inline_else = self.can_inline_body_with_upper_bound(
                else_branch,
                command.span,
                Some(fi_upper_bound),
            );
            if can_inline_else {
                return ThenFiIfLayout::InlineThenElse;
            }
            if !self.options().compact_layout() {
                return ThenFiIfLayout::InlineThenMultilineElse;
            }
        }

        if no_elifs
            && command.else_branch.is_none()
            && self.then_branch_starts_with_inline_if(command, then_span, fi_span)
        {
            return ThenFiIfLayout::InlineThenNestedIf;
        }

        if self.can_inline_if_chain(command, fi_span) {
            return ThenFiIfLayout::InlineChain;
        }

        if self.options().compact_layout() {
            ThenFiIfLayout::Expanded(ExpandedThenFiIfLayout::Compact)
        } else {
            ThenFiIfLayout::Expanded(ExpandedThenFiIfLayout::Multiline {
                inline_else_close: command
                    .else_branch
                    .as_ref()
                    .is_some_and(|body| self.can_inline_else_branch_close(command, body, fi_span)),
            })
        }
    }

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
        _open_char: char,
    ) -> bool {
        let (Some(first), Some(last)) = (commands.first(), commands.last()) else {
            return false;
        };
        let Some(group_span) = self
            .facts()
            .sequence(commands, None)
            .group_attachment_span()
        else {
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
        let Some(group_span) = self
            .facts()
            .sequence(commands, upper_bound)
            .group_attachment_span()
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

    pub(super) fn can_inline_else_branch_close(
        &self,
        command: &IfCommand,
        body: &StmtSeq,
        fi_span: Span,
    ) -> bool {
        let [stmt] = body.as_slice() else {
            return false;
        };
        if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
            || !self.can_inline_stmt(stmt)
            || self
                .facts()
                .sequence(body, Some(fi_span.start.offset))
                .has_comments()
        {
            return false;
        };
        let Some((_, else_offset)) = self
            .facts()
            .if_next_branch_region(command, command.elif_branches.len())
        else {
            return false;
        };
        let else_line = self.source_map().line_number_for_offset(else_offset);
        let body_line = stmt_span(stmt).start.line;
        else_line == body_line && body_line == fi_span.start.line
    }

    pub(super) fn can_inline_if_chain(&self, command: &IfCommand, fi_span: Span) -> bool {
        if command.elif_branches.is_empty() || command.span.start.line != fi_span.end.line {
            return false;
        }

        let source = self.source();
        if !self.can_inline_body_with_upper_bound(
            &command.then_branch,
            command.span,
            Some(if_branch_upper_bound(
                command,
                0,
                source,
                self.source_map(),
                self.facts(),
            )),
        ) {
            return false;
        }

        for (index, (_, body)) in command.elif_branches.iter().enumerate() {
            if !self.can_inline_body_with_upper_bound(
                body,
                command.span,
                Some(if_branch_upper_bound(
                    command,
                    index + 1,
                    source,
                    self.source_map(),
                    self.facts(),
                )),
            ) {
                return false;
            }
        }

        command.else_branch.as_ref().is_none_or(|body| {
            self.can_inline_body_with_upper_bound(body, command.span, Some(fi_span.start.offset))
        })
    }

    pub(super) fn then_branch_starts_with_inline_if(
        &self,
        command: &IfCommand,
        then_span: Span,
        fi_span: Span,
    ) -> bool {
        if command.span.start.line != fi_span.end.line {
            return false;
        }
        let [stmt] = command.then_branch.as_slice() else {
            return false;
        };
        if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
            return false;
        }
        let Command::Compound(CompoundCommand::If(inner)) = &stmt.command else {
            return false;
        };
        matches!(inner.syntax, IfSyntax::ThenFi { .. })
            && then_span.end.line == inner.span.start.line
            && !self
                .facts()
                .sequence(
                    &command.then_branch,
                    Some(if_branch_upper_bound(
                        command,
                        0,
                        self.source(),
                        self.source_map(),
                        self.facts(),
                    )),
                )
                .has_comments()
    }
}
