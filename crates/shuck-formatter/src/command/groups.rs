use super::*;

pub(super) fn command_group_attachment_span(
    command: &Command,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    let (commands, open) = command_group_commands(command)?;
    group_attachment_span(
        commands.as_slice(),
        source_map,
        open,
        matching_group_close(open),
    )
}

pub(crate) fn stmt_group_attachment_or_verbatim_span(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> Option<Span> {
    stmt_group_attachment_or_verbatim_span_with_heredoc(
        stmt,
        source_map,
        classify_stmt_contains_heredoc,
    )
}

pub(crate) fn stmt_group_attachment_or_verbatim_span_with_heredoc<F>(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
    stmt_contains_heredoc: F,
) -> Option<Span>
where
    F: Fn(&Stmt) -> bool + Copy,
{
    let (commands, open) = command_group_commands(&stmt.command)?;
    Some(
        group_attachment_span_with_heredoc(
            commands.as_slice(),
            source_map,
            open,
            matching_group_close(open),
            stmt_contains_heredoc,
        )
        .unwrap_or_else(|| stmt_verbatim_span_with_source_map(stmt, source_map)),
    )
}

pub(super) fn stmt_group_base_span_with_heredoc<F>(
    stmt: &Stmt,
    commands: &StmtSeq,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    stmt_contains_heredoc: F,
) -> Span
where
    F: Fn(&Stmt) -> bool + Copy,
{
    group_attachment_span_with_heredoc(
        commands.as_slice(),
        source_map,
        open,
        matching_group_close(open),
        stmt_contains_heredoc,
    )
    .unwrap_or_else(|| stmt_span(stmt))
}

pub(super) fn group_verbatim_span_impl(
    commands: &[Stmt],
    source_map: &SourceMap<'_>,
    open: char,
    close: char,
) -> Span {
    let source = source_map.source();
    let inner = commands
        .iter()
        .map(|command| stmt_verbatim_span_impl(command, source_map))
        .reduce(Span::merge)
        .unwrap_or_default();
    if inner == Span::new() {
        return inner;
    }

    let Some(open_offset) = source[..inner.start.offset].rfind(open) else {
        return inner;
    };
    let wrapper_prefix_start = open_offset + open.len_utf8();
    if !source[wrapper_prefix_start..inner.start.offset].contains('#') {
        return inner;
    }
    let Some(close_offset) = find_group_close_offset(source, inner.end.offset, close) else {
        return inner;
    };

    span_for_offsets(source_map, open_offset, close_offset + close.len_utf8())
}

pub(crate) fn group_open_suffix<'a>(
    commands: &[Stmt],
    source_map: &'a crate::comments::SourceMap<'a>,
    open: char,
) -> Option<(Span, &'a str)> {
    let source = source_map.source();
    let first = commands.first()?;
    let first_start = stmt_group_attachment_start_offset(first, source_map);
    let open_offset = find_group_open_offset_before_stmt(source_map, first_start, open)?;
    let (_, line_end) = source_map.line_bounds_for_offset(open_offset)?;
    let suffix_start = open_offset + open.len_utf8();
    let suffix = source.get(suffix_start..line_end)?;
    suffix
        .trim_start_matches(char::is_whitespace)
        .starts_with('#')
        .then(|| (source_map.span_for_offsets(suffix_start, line_end), suffix))
}

pub(crate) fn group_attachment_span(
    commands: &[Stmt],
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> Option<Span> {
    group_attachment_span_with_heredoc(
        commands,
        source_map,
        open,
        close,
        classify_stmt_contains_heredoc,
    )
}

pub(crate) fn group_attachment_span_with_heredoc<F>(
    commands: &[Stmt],
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
    stmt_contains_heredoc: F,
) -> Option<Span>
where
    F: Fn(&Stmt) -> bool + Copy,
{
    let source = source_map.source();
    let first = commands.first()?;
    let open_offset = find_group_open_offset_before_stmt(
        source_map,
        stmt_group_attachment_start_offset(first, source_map),
        open,
    )?;
    let sequence_end = commands
        .iter()
        .map(|command| {
            stmt_group_attachment_end_offset_with_heredoc(
                command,
                source_map,
                stmt_contains_heredoc,
            )
        })
        .max()
        .unwrap_or(0);
    let end = find_group_close_offset(source, sequence_end, close)
        .map(|offset| offset + close.len_utf8())
        .unwrap_or(sequence_end);
    Some(source_map.span_for_offsets(open_offset, end))
}

pub(crate) fn stmt_start_after_operator(
    stmt: &Stmt,
    operator_end: usize,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    match &stmt.command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => {
            group_open_offset_after_operator(
                stmt,
                commands.as_slice(),
                operator_end,
                source,
                source_map,
                '{',
                '}',
            )
        }
        Command::Compound(CompoundCommand::Subshell(commands)) => group_open_offset_after_operator(
            stmt,
            commands.as_slice(),
            operator_end,
            source,
            source_map,
            '(',
            ')',
        ),
        _ => command_format_span(&stmt.command).start.offset,
    }
}

fn group_open_offset_after_operator(
    stmt: &Stmt,
    commands: &[Stmt],
    operator_end: usize,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> usize {
    let search_end = commands
        .first()
        .map(|first| stmt_group_attachment_start_offset(first, source_map))
        .unwrap_or_else(|| stmt_span(stmt).end.offset);

    find_group_open_offset_between(source, operator_end, search_end, open)
        .or_else(|| {
            group_attachment_span(commands, source_map, open, close).map(|span| span.start.offset)
        })
        .unwrap_or_else(|| command_format_span(&stmt.command).start.offset)
}

fn find_group_open_offset_between(
    source: &str,
    search_start: usize,
    search_end: usize,
    open: char,
) -> Option<usize> {
    let mut offset = search_start.min(source.len());
    let upper = search_end.min(source.len());

    while offset < upper {
        let tail = &source[offset..upper];
        let ch = tail.chars().next()?;
        if let Some(next) = skip_escaped_or_quoted(source, offset, upper, ch) {
            offset = next;
            continue;
        }
        if ch == '#' && shell_comment_can_start(source, offset) {
            offset = tail
                .find('\n')
                .map_or(upper, |newline| offset + newline + 1);
            continue;
        }

        if ch == open {
            return Some(offset);
        }

        offset += ch.len_utf8();
    }

    None
}

fn find_group_open_offset_before_stmt(
    source_map: &SourceMap<'_>,
    search_end: usize,
    open: char,
) -> Option<usize> {
    let source = source_map.source();
    let mut line_end = search_end.min(source.len());

    loop {
        let lookup_offset = if line_end == 0 {
            0
        } else {
            line_end.saturating_sub(usize::from(line_end == source.len()))
        };
        let (line_start, indexed_line_end) = source_map.line_bounds_for_offset(lookup_offset)?;
        let search_line_end = line_end.min(indexed_line_end);
        if let Some(open_offset) =
            find_group_open_offset_on_line(source, line_start, search_line_end, open)
        {
            return Some(open_offset);
        }

        if line_start == 0 {
            break;
        }
        line_end = line_start.saturating_sub(1);
    }

    None
}

fn find_group_open_offset_on_line(
    source: &str,
    line_start: usize,
    line_end: usize,
    open: char,
) -> Option<usize> {
    let mut last_open = None;
    let mut offset = line_start;

    while offset < line_end {
        let ch = source[offset..].chars().next()?;

        if let Some(next) = skip_escaped_or_quoted(source, offset, line_end, ch) {
            offset = next;
            continue;
        }
        if ch == '#' && shell_comment_can_start(source, offset) {
            break;
        }

        if ch == open {
            last_open = Some(offset);
        }
        offset += ch.len_utf8();
    }

    last_open
}

fn stmt_group_attachment_start_offset(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    stmt_group_attachment_or_verbatim_span(stmt, source_map)
        .unwrap_or_else(|| stmt_verbatim_span_with_source_map(stmt, source_map))
        .start
        .offset
}

fn stmt_group_attachment_end_offset_with_heredoc<F>(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
    stmt_contains_heredoc: F,
) -> usize
where
    F: Fn(&Stmt) -> bool + Copy,
{
    if let Some(span) =
        stmt_group_attachment_or_verbatim_span_with_heredoc(stmt, source_map, stmt_contains_heredoc)
    {
        return span.end.offset;
    }

    match &stmt.command {
        Command::Function(_) | Command::AnonymousFunction(_) => stmt_span(stmt).end.offset,
        _ if stmt_contains_heredoc(stmt) => {
            stmt_verbatim_span_with_source_map(stmt, source_map)
                .end
                .offset
        }
        _ => stmt_span(stmt).end.offset,
    }
}

fn find_group_close_offset(source: &str, sequence_end: usize, close: char) -> Option<usize> {
    let close_len = close.len_utf8();
    let capped_end = sequence_end.min(source.len());
    if let Some(offset) = find_group_close_offset_after_sequence(source, capped_end, close) {
        return Some(offset);
    }

    let trimmed_end = source[..capped_end]
        .trim_end_matches(char::is_whitespace)
        .len();
    if trimmed_end >= close_len
        && source
            .get(trimmed_end - close_len..trimmed_end)
            .is_some_and(|slice| slice.starts_with(close))
    {
        return Some(trimmed_end - close_len);
    }

    None
}

fn find_group_close_offset_after_sequence(
    source: &str,
    sequence_end: usize,
    close: char,
) -> Option<usize> {
    let mut offset = sequence_end.min(source.len());
    while offset < source.len() {
        let tail = &source[offset..];
        if tail.starts_with("\\\n") {
            offset += "\\\n".len();
            continue;
        }
        let ch = tail.chars().next()?;
        if ch.is_whitespace() {
            offset += ch.len_utf8();
            continue;
        }
        if ch == ';' {
            offset += ch.len_utf8();
            continue;
        }
        if ch == '#' {
            offset = tail
                .find('\n')
                .map(|newline| offset + newline + 1)
                .unwrap_or(source.len());
            continue;
        }
        return (ch == close).then_some(offset);
    }

    None
}

pub(crate) fn group_was_inline_in_source(
    commands: &[Stmt],
    source_map: &crate::comments::SourceMap<'_>,
    open: char,
    close: char,
) -> bool {
    group_attachment_span(commands, source_map, open, close)
        .map(|span| !span.slice(source_map.source()).contains('\n'))
        .unwrap_or(false)
}

pub(crate) fn matching_group_close(open: char) -> char {
    match open {
        '{' => '}',
        '(' => ')',
        other => other,
    }
}

pub(super) fn find_empty_group_open_offset(
    source: &str,
    mut close_offset: usize,
    open: char,
) -> Option<usize> {
    close_offset = close_offset.min(source.len());
    while close_offset > 0 {
        let ch = source[..close_offset].chars().next_back()?;
        close_offset -= ch.len_utf8();
        if ch.is_whitespace() {
            continue;
        }
        return (ch == open).then_some(close_offset);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use shuck_parser::parser::Parser;

    fn parse(source: &str) -> shuck_ast::File {
        Parser::new(source).parse().unwrap().file
    }

    fn first_brace_group_verbatim_span(source: &str) -> Span {
        let file = parse(source);
        let brace_group = match &file.body[0].command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => commands,
            _ => panic!("expected brace group"),
        };

        let source_map = SourceMap::new(source);
        group_verbatim_span_impl(brace_group.as_slice(), &source_map, '{', '}')
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_with_semicolon_terminated_body() {
        let source = "{ # note\n  echo ok; # inside\n}\n";
        let span = first_brace_group_verbatim_span(source);

        assert_eq!(span.slice(source), "{ # note\n  echo ok; # inside\n}");
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_around_heredoc_bodies() {
        let source = "{ # note\n  cat <<EOF\npayload\nEOF\n}\n";
        let span = first_brace_group_verbatim_span(source);

        assert_eq!(span.slice(source), "{ # note\n  cat <<EOF\npayload\nEOF\n}");
    }

    #[test]
    fn group_verbatim_span_keeps_wrapper_comments_across_line_continuations() {
        let source = "{ # note\n  echo ok; \\\n}\n";
        let span = first_brace_group_verbatim_span(source);

        assert_eq!(span.slice(source), "{ # note\n  echo ok; \\\n}");
    }
}
