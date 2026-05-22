use shuck_ast::{
    ArithmeticForCommand, CompoundCommand, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax,
    IfCommand, IfSyntax, RepeatCommand, RepeatSyntax, SelectCommand, Span, Stmt, StmtSeq,
    UntilCommand, WhileCommand,
};
use shuck_indexer::CloseDelimiterKind;

use crate::command::{
    branch_open_keyword_start, command_group_commands, done_close_span,
    normalized_close_keyword_span, stmt_span,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompoundBodyOpen {
    Keyword(&'static str),
    Group(char),
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompoundBodyBounds {
    facts_end: usize,
    render_end: usize,
}

impl CompoundBodyBounds {
    fn shared(end: usize) -> Self {
        Self {
            facts_end: end,
            render_end: end,
        }
    }

    fn split(facts_end: usize, render_end: usize) -> Self {
        Self {
            facts_end,
            render_end,
        }
    }

    pub(crate) fn facts_limit(self) -> Option<usize> {
        Some(self.facts_end)
    }

    pub(crate) fn render_limit(self) -> Option<usize> {
        Some(self.render_end)
    }

    pub(crate) fn render_end(self) -> usize {
        self.render_end
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CompoundBodySite<'a> {
    body: &'a StmtSeq,
    enclosing_span: Span,
    bounds: CompoundBodyBounds,
    open: CompoundBodyOpen,
    close_span: Option<Span>,
}

impl<'a> CompoundBodySite<'a> {
    pub(crate) fn for_command(
        command: &'a ForCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        match command.syntax {
            ForSyntax::InDoDone { done_span, .. } | ForSyntax::ParenDoDone { done_span, .. } => {
                Self::do_done(
                    &command.body,
                    command.span,
                    Some(done_span),
                    source_map,
                    cached_close_span,
                )
            }
            ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => {
                Self::direct(&command.body, command.span)
            }
            ForSyntax::InBrace {
                right_brace_span, ..
            }
            | ForSyntax::ParenBrace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span, source_map),
        }
    }

    pub(crate) fn foreach_command(
        command: &'a ForeachCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        match command.syntax {
            ForeachSyntax::InDoDone { done_span, .. } => Self::do_done(
                &command.body,
                command.span,
                Some(done_span),
                source_map,
                cached_close_span,
            ),
            ForeachSyntax::ParenBrace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span, source_map),
        }
    }

    pub(crate) fn repeat_command(
        command: &'a RepeatCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        match command.syntax {
            RepeatSyntax::DoDone { done_span, .. } => Self::do_done(
                &command.body,
                command.span,
                Some(done_span),
                source_map,
                cached_close_span,
            ),
            RepeatSyntax::Direct => Self::direct(&command.body, command.span),
            RepeatSyntax::Brace {
                right_brace_span, ..
            } => Self::brace(&command.body, command.span, right_brace_span, source_map),
        }
    }

    pub(crate) fn while_command(
        command: &'a WhileCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        Self::do_done(
            &command.body,
            command.span,
            command.done_span,
            source_map,
            cached_close_span,
        )
    }

    pub(crate) fn until_command(
        command: &'a UntilCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        Self::do_done(
            &command.body,
            command.span,
            command.done_span,
            source_map,
            cached_close_span,
        )
    }

    pub(crate) fn select_command(
        command: &'a SelectCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        Self::do_done(
            &command.body,
            command.span,
            Some(command.done_span),
            source_map,
            cached_close_span,
        )
    }

    pub(crate) fn arithmetic_for_command(
        command: &'a ArithmeticForCommand,
        source_map: &crate::comments::SourceMap<'_>,
        cached_close_span: Option<Span>,
    ) -> Self {
        Self::do_done(
            &command.body,
            command.span,
            command.done_span,
            source_map,
            cached_close_span,
        )
    }

    pub(crate) fn single_body_command(
        command: &'a CompoundCommand,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Option<Self> {
        match command {
            CompoundCommand::For(command) => Some(Self::for_command(command, source_map, None)),
            CompoundCommand::Repeat(command) => {
                Some(Self::repeat_command(command, source_map, None))
            }
            CompoundCommand::Foreach(command) => {
                Some(Self::foreach_command(command, source_map, None))
            }
            CompoundCommand::ArithmeticFor(command) => {
                Some(Self::arithmetic_for_command(command, source_map, None))
            }
            CompoundCommand::While(command) => Some(Self::while_command(command, source_map, None)),
            CompoundCommand::Until(command) => Some(Self::until_command(command, source_map, None)),
            CompoundCommand::Select(command) => {
                Some(Self::select_command(command, source_map, None))
            }
            _ => None,
        }
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
            bounds: CompoundBodyBounds::shared(function_end_offset),
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

    pub(crate) fn bounds(self) -> CompoundBodyBounds {
        self.bounds
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
        branch_open_keyword_start(self.body, source, keyword)
    }

    pub(crate) fn open_suffix_span(
        self,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Option<Span> {
        let keyword = self.open_keyword()?;
        let source = source_map.source();
        let keyword_offset = branch_open_keyword_start(self.body, source, keyword)?;
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
        cached_close_span: Option<Span>,
    ) -> Self {
        let close_span = cached_close_span.or_else(|| {
            done_close_span(source_map.source(), source_map, enclosing_span, done_span)
        });
        let upper_bound = close_span.map(|span| span.start.offset).unwrap_or_else(|| {
            done_span.map_or(enclosing_span.end.offset, |span| span.start.offset)
        });
        Self {
            body,
            enclosing_span,
            bounds: CompoundBodyBounds::shared(upper_bound),
            open: CompoundBodyOpen::Keyword("do"),
            close_span,
        }
    }

    fn direct(body: &'a StmtSeq, enclosing_span: Span) -> Self {
        Self {
            body,
            enclosing_span,
            bounds: CompoundBodyBounds::shared(enclosing_span.end.offset),
            open: CompoundBodyOpen::Direct,
            close_span: None,
        }
    }

    fn brace(
        body: &'a StmtSeq,
        enclosing_span: Span,
        right_brace_span: Span,
        source_map: &crate::comments::SourceMap<'_>,
    ) -> Self {
        let close_span = source_map
            .close_delimiter_span(enclosing_span, CloseDelimiterKind::RightBrace)
            .unwrap_or_else(|| {
                normalized_close_keyword_span(
                    source_map.source(),
                    source_map,
                    right_brace_span,
                    "}",
                )
            });
        Self {
            body,
            enclosing_span,
            bounds: CompoundBodyBounds::split(
                right_brace_span.start.offset,
                enclosing_span.end.offset,
            ),
            open: CompoundBodyOpen::Group('{'),
            close_span: Some(close_span),
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
            bounds: CompoundBodyBounds::shared(upper_bound),
            open,
            close_span: None,
        }
    }
}
