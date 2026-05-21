use super::*;

pub(super) fn stmt_semicolon_terminator_starts_on_continuation_line(
    stmt: &Stmt,
    source_map: &SourceMap<'_>,
) -> bool {
    let Some(terminator_span) = stmt.terminator_span else {
        return false;
    };
    let render_end = stmt
        .redirects
        .last()
        .map(|redirect| redirect.span.end.offset)
        .unwrap_or_else(|| command_format_span(&stmt.command).end.offset);
    source_map.contains_newline_between(render_end, terminator_span.start.offset)
}

pub(super) fn stmt_rendered_end_line_after_format(
    stmt: &Stmt,
    source: &str,
    source_map: &SourceMap<'_>,
    fallback: usize,
) -> usize {
    if matches!(stmt.terminator, Some(StmtTerminator::Semicolon))
        && stmt_semicolon_terminator_starts_on_continuation_line(stmt, source_map)
        && let Some(terminator_span) = stmt.terminator_span
    {
        return terminator_span.start.line;
    }
    match &stmt.command {
        Command::Binary(command) => {
            return stmt_rendered_end_line_after_format(
                command.right.as_ref(),
                source,
                source_map,
                fallback,
            );
        }
        _ if stmt.redirects.is_empty() && stmt.terminator.is_none() => {
            if let Some((commands, open)) = command_group_commands(&stmt.command)
                && let Some(span) = group_attachment_span(
                    commands.as_slice(),
                    source_map,
                    open,
                    matching_group_close(open),
                )
            {
                let close = matching_group_close(open);
                let close_offset = group_close_offset(
                    source,
                    span,
                    Some(stmt_span(stmt).end.offset),
                    close,
                    close.len_utf8(),
                );
                return source_map.line_number_for_offset(close_offset);
            }
        }
        _ => {}
    }
    fallback
}

pub(super) fn gap_has_blank_line(source: &str, start: usize, end: usize) -> bool {
    source_between_offsets(source, start, end)
        .is_some_and(|gap| gap.bytes().filter(|byte| *byte == b'\n').count() >= 2)
}

pub(super) fn group_close_offset(
    source: &str,
    span: Span,
    upper_bound: Option<usize>,
    close_char: char,
    close_len: usize,
) -> usize {
    let fallback = span.end.offset.saturating_sub(close_len);
    let search_end = upper_bound
        .map(|offset| offset.saturating_add(close_len))
        .unwrap_or(span.end.offset)
        .min(source.len())
        .max(span.start.offset);
    source
        .get(span.start.offset..search_end)
        .and_then(|text| text.rfind(close_char))
        .map_or(fallback, |offset| span.start.offset + offset)
}

pub(super) fn trim_trailing_gap_before_offset(source: &str, mut offset: usize) -> usize {
    let bytes = source.as_bytes();
    while offset > 0 && matches!(bytes[offset - 1], b' ' | b'\t' | b'\r' | b'\n') {
        offset -= 1;
    }
    offset
}
