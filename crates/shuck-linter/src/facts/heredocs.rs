#[cfg_attr(shuck_profiling, inline(never))]
fn build_heredoc_fact_summary(
    commands: &[CommandFact<'_>],
    source: &str,
    file_end: usize,
) -> HeredocFactSummary {
    let mut summary = HeredocFactSummary::default();

    for command in commands {
        let unused_heredoc_command = command.literal_name() == Some("")
            && command.body_span().start.offset == command.body_span().end.offset;
        let echo_here_doc_command = command.effective_name_is("echo")
            && command
                .redirects()
                .iter()
                .any(|redirect| is_heredoc_redirect_kind(redirect.kind));

        if echo_here_doc_command {
            summary
                .echo_here_doc_spans
                .push(command.span_in_source(source));
        }

        for redirect in command.redirects() {
            if !is_heredoc_redirect_kind(redirect.kind) {
                continue;
            }

            if unused_heredoc_command {
                summary.unused_heredoc_spans.push(redirect.span);
            }

            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let reaches_file_end = heredoc.body.span.end.offset == file_end;
            if reaches_file_end {
                summary.heredoc_missing_end_spans.push(redirect.span);
            }

            let delimiter = heredoc.delimiter.cooked.as_str();
            if delimiter.is_empty() {
                continue;
            }

            if let Some(span) = heredoc_end_space_span(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.heredoc_end_space_spans.push(span);
            }

            if redirect.kind == RedirectKind::HereDocStrip {
                summary
                    .spaced_tabstrip_close_spans
                    .extend(spaced_tabstrip_close_spans(
                        heredoc.body.span,
                        delimiter,
                        source,
                    ));
            }

            if !reaches_file_end {
                continue;
            }

            if let Some(span) = heredoc_closer_not_alone_span(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.heredoc_closer_not_alone_spans.push(span);
            }

            if has_misquoted_heredoc_close(
                heredoc.body.span,
                delimiter,
                heredoc.delimiter.strip_tabs,
                source,
            ) {
                summary.misquoted_heredoc_close_spans.push(redirect.span);
            }
        }
    }

    summary
}

fn is_heredoc_redirect_kind(kind: RedirectKind) -> bool {
    matches!(kind, RedirectKind::HereDoc | RedirectKind::HereDocStrip)
}

fn heredoc_closer_not_alone_span(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> Option<Span> {
    let mut line_start_offset = body_span.start.offset;
    for raw_line in body_span.slice(source).split_inclusive('\n') {
        let (candidate_line, tab_prefix_len) = normalized_heredoc_line(raw_line, strip_tabs);
        if !candidate_line.ends_with(delimiter)
            || is_quoted_delimiter_variant(candidate_line, delimiter)
        {
            line_start_offset += raw_line.len();
            continue;
        }

        let prefix = &candidate_line[..candidate_line.len() - delimiter.len()];
        if !prefix.chars().any(|ch| !ch.is_whitespace()) {
            line_start_offset += raw_line.len();
            continue;
        }

        let delimiter_start_offset = line_start_offset + tab_prefix_len + prefix.len();
        let delimiter_end_offset = delimiter_start_offset + delimiter.len();
        let start = position_at_offset_opt(source, delimiter_start_offset)?;
        let end = position_at_offset_opt(source, delimiter_end_offset)?;
        return Some(Span::from_positions(start, end));
    }

    None
}

fn has_misquoted_heredoc_close(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> bool {
    body_span
        .slice(source)
        .split_inclusive('\n')
        .map(|raw_line| normalized_heredoc_line(raw_line, strip_tabs).0)
        .filter(|candidate_line| *candidate_line != delimiter)
        .any(|candidate_line| is_quoted_delimiter_variant(candidate_line, delimiter))
}

fn heredoc_end_space_span(
    body_span: Span,
    delimiter: &str,
    strip_tabs: bool,
    source: &str,
) -> Option<Span> {
    let line_start_offset = body_span.end.offset;
    let remainder = source.get(line_start_offset..)?;
    let raw_line = remainder.split_inclusive('\n').next().unwrap_or(remainder);
    let (candidate_line, tab_prefix_len) = normalized_heredoc_line(raw_line, strip_tabs);
    let trailing = candidate_line.strip_prefix(delimiter)?;
    if trailing.is_empty() || !trailing.chars().all(|ch| matches!(ch, ' ' | '\t')) {
        return None;
    }

    let trailing_start_offset = line_start_offset + tab_prefix_len + delimiter.len();
    let start = position_at_offset_opt(source, trailing_start_offset)?;
    Some(Span::from_positions(start, start))
}

fn spaced_tabstrip_close_spans(body_span: Span, delimiter: &str, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut line_start_offset = body_span.start.offset;
    for raw_line in body_span.slice(source).split_inclusive('\n') {
        let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        if is_spaced_tabstrip_close_line(line_without_newline, delimiter)
            && let Some(position) = position_at_offset_opt(source, line_start_offset)
        {
            spans.push(Span::from_positions(position, position));
        }
        line_start_offset += raw_line.len();
    }

    spans
}

fn normalized_heredoc_line(raw_line: &str, strip_tabs: bool) -> (&str, usize) {
    let line_without_newline = raw_line.trim_end_matches('\n').trim_end_matches('\r');
    if strip_tabs {
        let trimmed = line_without_newline.trim_start_matches('\t');
        (trimmed, line_without_newline.len() - trimmed.len())
    } else {
        (line_without_newline, 0)
    }
}

fn is_quoted_delimiter_variant(candidate_line: &str, delimiter: &str) -> bool {
    candidate_line != delimiter && trim_quote_like_wrappers(candidate_line) == delimiter
}

fn trim_quote_like_wrappers(text: &str) -> &str {
    text.trim_matches(|ch| matches!(ch, '\'' | '"' | '\\'))
}

fn is_spaced_tabstrip_close_line(line: &str, delimiter: &str) -> bool {
    if line.trim_start_matches('\t') == delimiter {
        return false;
    }

    let line_without_trailing_ws = line.trim_end_matches([' ', '\t']);
    let leading_len = line_without_trailing_ws.len()
        - line_without_trailing_ws
            .trim_start_matches([' ', '\t'])
            .len();
    if leading_len == 0 {
        return false;
    }

    let leading = &line_without_trailing_ws[..leading_len];
    let rest = &line_without_trailing_ws[leading_len..];
    leading.contains(' ') && rest == delimiter
}

fn position_at_offset_opt(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}
