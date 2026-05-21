use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompoundBodyOpen {
    Keyword(&'static str),
    Group(char),
    Direct,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompoundBodySite<'a> {
    body: &'a StmtSeq,
    enclosing_span: Span,
    facts_upper_bound: usize,
    renderer_upper_bound: usize,
    open: CompoundBodyOpen,
    close_span: Option<Span>,
}

impl<'a> CompoundBodySite<'a> {
    pub(crate) fn for_command(
        command: &'a ForCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        match command.syntax {
            ForSyntax::InDoDone { done_span, .. } | ForSyntax::ParenDoDone { done_span, .. } => {
                Self::do_done(&command.body, command.span, Some(done_span), source_map)
            }
            ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => {
                Self::direct(&command.body, command.span)
            }
            ForSyntax::InBrace {
                right_brace_span, ..
            }
            | ForSyntax::ParenBrace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span),
        }
    }

    pub(crate) fn foreach_command(
        command: &'a ForeachCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        match command.syntax {
            ForeachSyntax::InDoDone { done_span, .. } => {
                Self::do_done(&command.body, command.span, Some(done_span), source_map)
            }
            ForeachSyntax::ParenBrace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span),
        }
    }

    pub(crate) fn repeat_command(
        command: &'a RepeatCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        match command.syntax {
            RepeatSyntax::DoDone { done_span, .. } => {
                Self::do_done(&command.body, command.span, Some(done_span), source_map)
            }
            RepeatSyntax::Direct => Self::direct(&command.body, command.span),
            RepeatSyntax::Brace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span),
        }
    }

    pub(crate) fn while_command(
        command: &'a WhileCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        Self::do_done(&command.body, command.span, command.done_span, source_map)
    }

    pub(crate) fn until_command(
        command: &'a UntilCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        Self::do_done(&command.body, command.span, command.done_span, source_map)
    }

    pub(crate) fn select_command(
        command: &'a SelectCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        Self::do_done(
            &command.body,
            command.span,
            Some(command.done_span),
            source_map,
        )
    }

    pub(crate) fn arithmetic_for_command(
        command: &'a ArithmeticForCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        Self::do_done(&command.body, command.span, command.done_span, source_map)
    }

    pub(crate) fn if_then_branch(
        command: &IfCommand,
        body: &'a StmtSeq,
        upper_bound: usize,
    ) -> Self {
        let open = match command.syntax {
            IfSyntax::ThenFi { .. } => CompoundBodyOpen::Keyword("then"),
            IfSyntax::Brace { .. } => CompoundBodyOpen::Group('{'),
        };
        Self::branch(body, command.span, upper_bound, open)
    }

    pub(crate) fn if_else_branch(
        command: &IfCommand,
        body: &'a StmtSeq,
        upper_bound: usize,
    ) -> Self {
        let open = match command.syntax {
            IfSyntax::ThenFi { .. } => CompoundBodyOpen::Keyword("else"),
            IfSyntax::Brace { .. } => CompoundBodyOpen::Group('{'),
        };
        Self::branch(body, command.span, upper_bound, open)
    }

    pub(crate) fn function_group_body(body: &'a Stmt, function_end_offset: usize) -> Option<Self> {
        if body.negated || !body.redirects.is_empty() || body.terminator.is_some() {
            return None;
        }
        let (commands, open) = command_group_commands(&body.command)?;
        Some(Self {
            body: commands,
            enclosing_span: stmt_span(body),
            facts_upper_bound: function_end_offset,
            renderer_upper_bound: function_end_offset,
            open: CompoundBodyOpen::Group(open),
            close_span: None,
        })
    }

    pub(crate) fn body(self) -> &'a StmtSeq {
        self.body
    }

    pub(crate) fn enclosing_span(self) -> Span {
        self.enclosing_span
    }

    pub(crate) fn facts_upper_bound(self) -> usize {
        self.facts_upper_bound
    }

    pub(crate) fn renderer_upper_bound(self) -> usize {
        self.renderer_upper_bound
    }

    pub(crate) fn open(self) -> CompoundBodyOpen {
        self.open
    }

    pub(crate) fn group_open_char(self) -> Option<char> {
        match self.open {
            CompoundBodyOpen::Group(open) => Some(open),
            CompoundBodyOpen::Keyword(_) | CompoundBodyOpen::Direct => None,
        }
    }

    pub(crate) fn open_keyword(self) -> Option<&'static str> {
        match self.open {
            CompoundBodyOpen::Keyword(keyword) => Some(keyword),
            CompoundBodyOpen::Group(_) | CompoundBodyOpen::Direct => None,
        }
    }

    pub(crate) fn open_keyword_start(self, source: &str) -> Option<usize> {
        let keyword = self.open_keyword()?;
        super::structure::branch_open_keyword_start(self.body, source, keyword)
    }

    pub(crate) fn open_suffix_span(
        self,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Option<Span> {
        let keyword = self.open_keyword()?;
        let source = source_map.source();
        let keyword_offset =
            super::structure::branch_open_keyword_start(self.body, source, keyword)?;
        let (_, line_end) = source_map.line_bounds_for_offset(keyword_offset)?;
        let suffix_start = keyword_offset + keyword.len();
        let suffix = source.get(suffix_start..line_end)?;
        suffix
            .trim_start_matches(char::is_whitespace)
            .starts_with('#')
            .then(|| source_map.span_for_offsets(suffix_start, line_end))
    }

    pub(crate) fn open_end_offset(self, source: &str) -> Option<usize> {
        let keyword = self.open_keyword()?;
        self.open_keyword_start(source)
            .map(|start| start + keyword.len())
    }

    pub(crate) fn close_span(self) -> Option<Span> {
        self.close_span
    }

    fn do_done(
        body: &'a StmtSeq,
        enclosing_span: Span,
        done_span: Option<Span>,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        let close_span = super::render_policy::done_close_span(
            source_map.source(),
            source_map,
            enclosing_span,
            done_span,
        );
        let upper_bound = close_span.map(|span| span.start.offset).unwrap_or_else(|| {
            done_span.map_or(enclosing_span.end.offset, |span| span.start.offset)
        });
        Self {
            body,
            enclosing_span,
            facts_upper_bound: upper_bound,
            renderer_upper_bound: upper_bound,
            open: CompoundBodyOpen::Keyword("do"),
            close_span,
        }
    }

    fn direct(body: &'a StmtSeq, enclosing_span: Span) -> Self {
        Self {
            body,
            enclosing_span,
            facts_upper_bound: enclosing_span.end.offset,
            renderer_upper_bound: enclosing_span.end.offset,
            open: CompoundBodyOpen::Direct,
            close_span: None,
        }
    }

    fn brace(body: &'a StmtSeq, enclosing_span: Span, right_brace_span: Span) -> Self {
        // Preserve the legacy split: facts are keyed at the closing brace, while
        // renderer call sites ask for the enclosing command's full span.
        Self {
            body,
            enclosing_span,
            facts_upper_bound: right_brace_span.start.offset,
            renderer_upper_bound: enclosing_span.end.offset,
            open: CompoundBodyOpen::Group('{'),
            close_span: None,
        }
    }

    fn branch(
        body: &'a StmtSeq,
        enclosing_span: Span,
        upper_bound: usize,
        open: CompoundBodyOpen,
    ) -> Self {
        Self {
            body,
            enclosing_span,
            facts_upper_bound: upper_bound,
            renderer_upper_bound: upper_bound,
            open,
            close_span: None,
        }
    }
}
