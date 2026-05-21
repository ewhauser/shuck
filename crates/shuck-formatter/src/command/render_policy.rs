use super::*;

pub(crate) fn line_gap_break_count(current_line: usize, next_line: usize) -> usize {
    next_line.saturating_sub(current_line).clamp(1, 2)
}

pub(crate) fn rendered_stmt_end_line_with_heredoc<F>(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    stmt_contains_heredoc: F,
) -> usize
where
    F: Fn(&Stmt) -> bool + Copy,
{
    match &stmt.command {
        Command::Function(_) | Command::AnonymousFunction(_) => {
            span_render_end_line(stmt_span(stmt), source, source_map)
        }
        _ if stmt_contains_heredoc(stmt) => span_render_end_line(
            stmt_verbatim_span_with_source_map(stmt, source_map),
            source,
            source_map,
        ),
        Command::Binary(command) => rendered_stmt_end_line_with_heredoc(
            &command.right,
            source,
            source_map,
            stmt_contains_heredoc,
        ),
        _ => {
            if let Some((commands, open)) = command_group_commands(&stmt.command) {
                let mut span = stmt_group_base_span_with_heredoc(
                    stmt,
                    commands,
                    source_map,
                    open,
                    stmt_contains_heredoc,
                );
                for redirect in &stmt.redirects {
                    span = merge_non_empty_span(span, redirect.span);
                }
                if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
                    && let Some(terminator_span) = stmt.terminator_span
                {
                    span = merge_non_empty_span(span, terminator_span);
                }
                span_render_end_line(span, source, source_map)
            } else {
                span_render_end_line(stmt_format_span(stmt), source, source_map)
            }
        }
    }
}

pub(crate) fn span_render_end_line(
    span: Span,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    let mut end = span.end.offset.min(source.len());
    while end > span.start.offset
        && source
            .as_bytes()
            .get(end - 1)
            .is_some_and(u8::is_ascii_whitespace)
    {
        end -= 1;
    }

    if end == span.start.offset {
        span.start.line
    } else {
        source_map.line_number_for_offset(end - 1)
    }
}

pub(crate) fn stmt_has_trailing_comment(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let raw = stmt_span(stmt);
    let formatted = stmt_format_span(stmt);
    raw.end.offset > formatted.end.offset
        && source_map.contains_comment_between(formatted.end.offset, raw.end.offset)
}

pub(crate) fn should_render_verbatim_with_heredoc(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
    contains_heredoc: bool,
) -> bool {
    (!options.simplify()
        && matches!(&stmt.command, Command::Simple(command) if simple_command_uses_synthetic_words(command, source_map.source())))
        || (options.keep_padding() && stmt_has_alignment_sensitive_padding(stmt, source_map))
        || (contains_heredoc
            && !matches!(stmt.command, Command::Binary(_))
            && stmt_has_trailing_comment(stmt, source_map))
}

pub(crate) fn stmt_attachment_span(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    stmt_attachment_span_with_heredoc(
        stmt,
        source,
        source_map,
        options,
        classify_stmt_contains_heredoc,
    )
}

pub(crate) fn stmt_attachment_span_with_heredoc<F>(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
    stmt_contains_heredoc: F,
) -> Span
where
    F: Fn(&Stmt) -> bool + Copy,
{
    let span = if should_render_verbatim_with_heredoc(
        stmt,
        source_map,
        options,
        stmt_contains_heredoc(stmt),
    ) {
        stmt_verbatim_span_with_source_map(stmt, source_map)
    } else if let Command::Function(command) = &stmt.command {
        function_attachment_span(command)
    } else if let Command::AnonymousFunction(command) = &stmt.command {
        anonymous_function_attachment_span(command)
    } else if let Some((commands, open)) = command_group_commands(&stmt.command) {
        stmt.redirects.iter().fold(
            stmt_group_base_span_with_heredoc(
                stmt,
                commands,
                source_map,
                open,
                stmt_contains_heredoc,
            ),
            |span, redirect| span.merge(redirect.span),
        )
    } else {
        complete_stmt_span(
            stmt,
            command_attachment_span(&stmt.command, source, source_map, options),
        )
    };
    extend_compound_close_suffix_attachment_span(span, stmt, source, source_map)
}

fn extend_compound_close_suffix_attachment_span(
    span: Span,
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Span {
    let Some(close_span) = stmt_compound_close_span(stmt, source, source_map) else {
        return span;
    };
    if let Some(comment) = source_map.suffix_comment_after_span(close_span) {
        merge_non_empty_span(span, comment.span())
    } else {
        span
    }
}

fn stmt_compound_close_span(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    let Command::Compound(command) = &stmt.command else {
        return None;
    };
    match command {
        CompoundCommand::If(command) => Some(if_close_span(command, source, source_map)),
        CompoundCommand::For(command) => match command.syntax {
            ForSyntax::InDoDone { done_span, .. } | ForSyntax::ParenDoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            ForSyntax::InBrace {
                right_brace_span, ..
            }
            | ForSyntax::ParenBrace {
                right_brace_span, ..
            } => normalized_brace_close_span(source, source_map, right_brace_span),
            ForSyntax::InDirect { .. } | ForSyntax::ParenDirect { .. } => None,
        },
        CompoundCommand::Repeat(command) => match command.syntax {
            RepeatSyntax::DoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            RepeatSyntax::Brace {
                right_brace_span, ..
            } => normalized_brace_close_span(source, source_map, right_brace_span),
            RepeatSyntax::Direct => None,
        },
        CompoundCommand::Foreach(command) => match command.syntax {
            ForeachSyntax::InDoDone { done_span, .. } => {
                done_close_span(source, source_map, command.span, Some(done_span))
            }
            ForeachSyntax::ParenBrace {
                right_brace_span, ..
            } => normalized_brace_close_span(source, source_map, right_brace_span),
        },
        CompoundCommand::ArithmeticFor(command) => {
            done_close_span(source, source_map, command.span, None)
        }
        CompoundCommand::While(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Until(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Select(command) => done_close_span(source, source_map, command.span, None),
        CompoundCommand::Case(command) => last_shell_keyword_start(source, command.span, "esac")
            .map(|start| source_map.span_for_offsets(start, start + "esac".len())),
        _ => None,
    }
}

fn normalized_brace_close_span(
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    span: Span,
) -> Option<Span> {
    Some(normalized_close_keyword_span(source, source_map, span, "}"))
}

pub(crate) fn if_close_span(
    command: &IfCommand,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> Span {
    let (syntax_close, keyword) = match command.syntax {
        IfSyntax::ThenFi { fi_span, .. } => (fi_span, "fi"),
        IfSyntax::Brace {
            right_brace_span, ..
        } => (right_brace_span, "}"),
    };
    let syntax_close = normalized_close_keyword_span(source, source_map, syntax_close, keyword);
    if span_starts_with_keyword(source, syntax_close, keyword) {
        return syntax_close;
    }
    matching_if_close_start(source, command.span)
        .map(|start| source_map.span_for_offsets(start, start + keyword.len()))
        .unwrap_or(syntax_close)
}

pub(crate) fn done_close_span(
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    span: Span,
    fallback: Option<Span>,
) -> Option<Span> {
    if let Some(fallback) = fallback {
        let normalized = normalized_close_keyword_span(source, source_map, fallback, "done");
        if span_starts_with_keyword(source, normalized, "done") {
            return Some(normalized);
        }
    }

    let span_end = span.end.offset.min(source.len());
    if let Some(start) = span_end.checked_sub("done".len())
        && source.get(start..span_end) == Some("done")
    {
        return Some(source_map.span_for_offsets(start, span_end));
    }

    matching_done_close_start(source, span)
        .map(|start| source_map.span_for_offsets(start, start + "done".len()))
        .or_else(|| {
            fallback.map(|span| normalized_close_keyword_span(source, source_map, span, "done"))
        })
}

fn span_starts_with_keyword(source: &str, span: Span, keyword: &str) -> bool {
    let start = span.start.offset.min(source.len());
    let end = start.saturating_add(keyword.len()).min(source.len());
    source.get(start..end) == Some(keyword)
}

fn command_attachment_span(
    command: &Command,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    match command {
        Command::Binary(command) => {
            stmt_attachment_span(&command.left, source, source_map, options).merge(
                stmt_attachment_span(&command.right, source, source_map, options),
            )
        }
        _ => command_format_span(command),
    }
}

pub(crate) fn stmt_render_start_line(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> usize {
    if let Some((commands, open)) = command_group_commands(&stmt.command) {
        group_render_start_line(stmt, commands.as_slice(), source, source_map, open, options)
    } else {
        stmt_attachment_span(stmt, source, source_map, options)
            .start
            .line
    }
}

fn group_render_start_line(
    stmt: &Stmt,
    commands: &[Stmt],
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    options: &crate::options::ResolvedShellFormatOptions,
) -> usize {
    group_attachment_span(commands, source_map, open, matching_group_close(open))
        .map(|span| span.start.line)
        .or_else(|| {
            find_empty_group_open_offset(source, stmt_span(stmt).start.offset, open)
                .map(|offset| source_map.line_number_for_offset(offset))
        })
        .unwrap_or_else(|| {
            stmt_attachment_span(stmt, source, source_map, options)
                .start
                .line
        })
}

fn stmt_has_alignment_sensitive_padding(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let mut spans = stmt_token_spans(stmt);
    spans.retain(|span| span != &Span::new() && span.start.offset < span.end.offset);
    spans.sort_by_key(|span| span.start.offset);
    spans.windows(2).any(|window| {
        let [left, right] = window else {
            return false;
        };
        if right.start.offset <= left.end.offset {
            return false;
        }
        source_map.has_alignment_padding_between(left.end.offset, right.start.offset)
    })
}

pub(crate) fn case_item_was_inline_in_source(item: &CaseItem) -> bool {
    let Some(stmt) = item.body.first() else {
        return false;
    };

    item.patterns
        .last()
        .is_some_and(|pattern| pattern.span.end.line == stmt_span(stmt).start.line)
        && item
            .terminator_span
            .is_some_and(|span| span.start.line == stmt_format_span(stmt).end.line)
}

pub(crate) fn case_item_body_upper_bound(item: &CaseItem, fallback: usize) -> Option<usize> {
    Some(
        item.terminator_span
            .map(|span| span.start.offset)
            .unwrap_or(fallback),
    )
}

fn command_token_spans(command: &Command) -> Vec<Span> {
    match command {
        Command::Simple(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            if !command.name.parts.is_empty() {
                spans.push(command.name.span);
            }
            spans.extend(command.args.iter().map(|word| word.span));
            spans
        }
        Command::Builtin(command) => {
            let (span, name, assignments, primary, extra_args) = builtin_like_parts(command);
            builtin_like_token_spans(span.start, name, assignments, primary, extra_args)
        }
        Command::Decl(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            spans.push(command.variant_span);
            spans.extend(command.operands.iter().map(|operand| match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
                DeclOperand::Name(name) => name.span,
                DeclOperand::Assignment(assignment) => assignment.span,
            }));
            spans
        }
        Command::Binary(command) => vec![command.span],
        Command::Compound(command) => vec![compound_format_span(command)],
        Command::Function(command) => vec![
            function_header_span(command),
            stmt_format_span(&command.body),
        ],
        Command::AnonymousFunction(command) => {
            let mut spans = vec![
                anonymous_function_header_span(command),
                stmt_format_span(&command.body),
            ];
            spans.extend(command.args.iter().map(|argument| argument.span));
            spans
        }
    }
}

fn builtin_like_token_spans(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
) -> Vec<Span> {
    let mut spans = assignments
        .iter()
        .map(|assignment| assignment.span)
        .collect::<Vec<_>>();
    spans.push(Span::from_positions(start, start.advanced_by(name)));
    if let Some(primary) = primary {
        spans.push(primary.span);
    }
    spans.extend(extra_args.iter().map(|argument| argument.span));
    spans
}

fn stmt_token_spans(stmt: &Stmt) -> Vec<Span> {
    let mut spans = if stmt.negated {
        vec![Span::from_positions(
            stmt.span.start,
            stmt.span.start.advanced_by("!"),
        )]
    } else {
        Vec::new()
    };
    spans.extend(command_token_spans(&stmt.command));
    spans.extend(stmt.redirects.iter().map(|redirect| redirect.span));
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        spans.push(terminator_span);
    }
    spans
}
