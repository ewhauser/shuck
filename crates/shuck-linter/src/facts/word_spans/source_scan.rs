use super::*;

pub(crate) fn command_prefix_is_empty_or_assignments(prefix: &str) -> bool {
    let mut index = 0;
    while index < prefix.len() {
        skip_shell_whitespace(prefix.as_bytes(), &mut index);
        if index >= prefix.len() {
            return true;
        }

        let Some(end) = shell_word_end(prefix, index) else {
            return false;
        };
        if simple_assignment_word(&prefix[index..end]) {
            index = end;
            continue;
        }
        if let Some(redirection_end) = redirection_prefix_end(prefix, index) {
            index = redirection_end;
            continue;
        }
        return false;
    }
    true
}

pub(crate) fn skip_shell_whitespace(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

pub(crate) fn shell_word_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        if !in_double_quote && byte.is_ascii_whitespace() {
            break;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                index = skip_balanced_shell_construct(text, index + 2, b'{', b'}')?;
            }
            b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'`' => {
                index = skip_legacy_backtick_construct(text, index + 1)?;
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    (!in_single_quote && !in_double_quote).then_some(index)
}

pub(crate) fn skip_balanced_shell_construct(
    text: &str,
    mut index: usize,
    open: u8,
    close: u8,
) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                index = skip_balanced_shell_construct(text, index + 2, b'{', b'}')?;
            }
            b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                index = skip_balanced_shell_construct(text, index + 2, b'(', b')')?;
            }
            b'`' => {
                index = skip_legacy_backtick_construct(text, index + 1)?;
            }
            _ if byte == open && !in_double_quote => {
                depth += 1;
                index += 1;
            }
            _ if byte == close && !in_double_quote => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

pub(crate) fn skip_legacy_backtick_construct(text: &str, mut index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_single_quote {
            if byte == b'\'' {
                in_single_quote = false;
            }
            index += 1;
            continue;
        }

        if byte == b'\\' {
            index = advance_escaped_shell_char(text, index);
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => {
                in_single_quote = true;
                index += 1;
            }
            b'"' => {
                in_double_quote = !in_double_quote;
                index += 1;
            }
            b'`' if !in_double_quote => return Some(index + 1),
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

pub(crate) fn advance_escaped_shell_char(text: &str, index: usize) -> usize {
    let next = advance_shell_char(text, index);
    if next < text.len() {
        advance_shell_char(text, next)
    } else {
        next
    }
}

pub(crate) fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
}

pub(crate) fn simple_assignment_word(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = word[..eq].strip_suffix('+').unwrap_or(&word[..eq]);
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(crate) fn redirection_prefix_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut operator_start = start;
    while operator_start < bytes.len() && bytes[operator_start].is_ascii_digit() {
        operator_start += 1;
    }

    let operator_len = redirection_operator_len(text.get(operator_start..)?)?;
    let mut target_start = operator_start + operator_len;
    skip_shell_whitespace(bytes, &mut target_start);
    if target_start >= text.len() {
        return None;
    }

    shell_word_end(text, target_start)
}

pub(crate) fn redirection_operator_len(text: &str) -> Option<usize> {
    [
        "&>>", "<<<", "<>", ">>", "<<", "<&", ">&", ">|", "&>", "<", ">",
    ]
    .into_iter()
    .find(|operator| text.starts_with(operator))
    .map(str::len)
}

pub(crate) fn escaped_reference_ends_standalone_word(
    source: &str,
    start: usize,
    limit: usize,
) -> bool {
    let Some(rest) = source.get(start..limit) else {
        return false;
    };
    rest.chars().next().is_none_or(|ch| {
        ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>' | '(' | ')')
    })
}

pub(crate) fn continued_line_chain_start(
    target: Position,
    containing_span: Span,
    source: &str,
) -> Option<Position> {
    let original_start = source[..target.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let containing_line_start = source[..containing_span.start.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let mut chain_start = containing_line_start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_comment = false;
    let mut previous_char = None;
    let mut trailing_backslashes = 0usize;
    let mut index = containing_span.start.offset.saturating_add(1);

    while index < target.offset {
        if source[index..].starts_with("\r\n") {
            index += "\r\n".len();
            if !in_comment && !in_single_quote && trailing_backslashes % 2 == 1 {
                trailing_backslashes = 0;
                continue;
            }
            chain_start = index;
            in_comment = false;
            trailing_backslashes = 0;
            previous_char = Some('\n');
            continue;
        }

        let ch = source[index..]
            .chars()
            .next()
            .expect("index should remain on UTF-8 boundaries");
        let ch_len = ch.len_utf8();
        index += ch_len;

        if ch == '\n' {
            if !in_comment && !in_single_quote && trailing_backslashes % 2 == 1 {
                trailing_backslashes = 0;
                continue;
            }
            chain_start = index;
            in_comment = false;
            trailing_backslashes = 0;
            previous_char = Some('\n');
            continue;
        }

        if in_comment {
            trailing_backslashes = 0;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            previous_char = Some(ch);
            trailing_backslashes = 0;
            continue;
        }

        let backslash_escaped = trailing_backslashes % 2 == 1;
        match ch {
            '\'' if !in_double_quote && !backslash_escaped => {
                in_single_quote = true;
                trailing_backslashes = 0;
            }
            '"' if !backslash_escaped => {
                in_double_quote = !in_double_quote;
                trailing_backslashes = 0;
            }
            '#' if !in_double_quote && backtick_shell_comment_can_start(previous_char) => {
                in_comment = true;
                trailing_backslashes = 0;
            }
            '\\' => {
                trailing_backslashes += 1;
            }
            _ => {
                trailing_backslashes = 0;
            }
        }

        previous_char = Some(ch);
    }

    (chain_start != original_start)
        .then(|| position_at_offset(source, chain_start))
        .flatten()
}

pub(crate) fn line_has_escaped_newline_continuation(line: &str) -> bool {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_comment = false;
    let mut previous_char = None;
    let mut trailing_backslashes = 0usize;

    for ch in line.chars() {
        if in_comment {
            trailing_backslashes = 0;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            previous_char = Some(ch);
            trailing_backslashes = 0;
            continue;
        }

        let backslash_escaped = trailing_backslashes % 2 == 1;
        match ch {
            '\'' if !in_double_quote && !backslash_escaped => {
                in_single_quote = true;
                trailing_backslashes = 0;
            }
            '"' if !backslash_escaped => {
                in_double_quote = !in_double_quote;
                trailing_backslashes = 0;
            }
            '#' if !in_double_quote && backtick_shell_comment_can_start(previous_char) => {
                in_comment = true;
                trailing_backslashes = 0;
            }
            '\\' => {
                trailing_backslashes += 1;
            }
            _ => {
                trailing_backslashes = 0;
            }
        }

        previous_char = Some(ch);
    }

    !in_comment && !in_single_quote && trailing_backslashes % 2 == 1
}

pub(crate) fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

pub(crate) fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

pub(crate) fn scan_span_excluding(span: Span, excluded: &[Span], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scan_span_excluding(span, excluded, source, &mut spans);
    spans
}

pub(crate) fn collect_scan_span_excluding(
    span: Span,
    excluded: &[Span],
    source: &str,
    spans: &mut Vec<Span>,
) {
    if excluded.is_empty() {
        spans.push(span);
        return;
    }

    let mut cursor = span.start.offset;
    for excluded_span in excluded.iter().copied().filter(|excluded_span| {
        excluded_span.end.offset > span.start.offset && excluded_span.start.offset < span.end.offset
    }) {
        let segment_end = excluded_span.start.offset.min(span.end.offset);
        if cursor < segment_end {
            spans.push(scan_span_segment(span, cursor, segment_end, source));
        }
        cursor = cursor.max(excluded_span.end.offset).min(span.end.offset);
    }

    if cursor < span.end.offset {
        spans.push(scan_span_segment(span, cursor, span.end.offset, source));
    }
}

pub(crate) fn merge_adjacent_spans(spans: Vec<Span>, source: &str) -> Vec<Span> {
    let mut merged: Vec<Span> = Vec::new();

    for span in spans {
        if let Some(previous) = merged.last_mut()
            && spans_share_literal_run(*previous, span, source)
        {
            *previous = Span::from_positions(previous.start, span.end);
            continue;
        }

        merged.push(span);
    }

    merged
}

pub(crate) fn spans_share_literal_run(previous: Span, next: Span, source: &str) -> bool {
    if previous.end.offset >= next.start.offset {
        return true;
    }

    let gap = &source[previous.end.offset..next.start.offset];
    !gap.contains('$') && !gap.contains('`')
}

pub(crate) fn scan_span_segment(span: Span, start: usize, end: usize, source: &str) -> Span {
    let segment_start = span.start.advanced_by(&source[span.start.offset..start]);
    let segment_end = span.start.advanced_by(&source[span.start.offset..end]);
    Span::from_positions(segment_start, segment_end)
}

pub(crate) fn span_is_backslash_escaped(span: Span, source: &str) -> bool {
    offset_is_backslash_escaped(span.start.offset, source)
}

pub(crate) fn offset_is_backslash_escaped(offset: usize, source: &str) -> bool {
    if offset == 0 {
        return false;
    }

    let bytes = source.as_bytes();
    let mut index = offset;
    let mut backslash_count = 0usize;
    while index > 0 && bytes[index - 1] == b'\\' {
        backslash_count += 1;
        index -= 1;
    }

    backslash_count % 2 == 1
}

pub(crate) fn span_is_escaped(span: Span, source: &str) -> bool {
    span_is_backslash_escaped(span, source)
}

#[cfg(test)]
mod tests {
    use super::line_has_escaped_newline_continuation;

    #[test]
    fn escaped_newline_continuations_require_an_odd_backslash_count() {
        assert!(line_has_escaped_newline_continuation("echo foo \\"));
        assert!(line_has_escaped_newline_continuation("echo foo \\\\\\"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\"));
        assert!(!line_has_escaped_newline_continuation("echo foo"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\   "));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\   "));
        assert!(line_has_escaped_newline_continuation("echo foo \\\r"));
        assert!(!line_has_escaped_newline_continuation("echo foo \\\\\r"));
        assert!(!line_has_escaped_newline_continuation(r"printf 'foo\"));
        assert!(!line_has_escaped_newline_continuation(r"printf # foo\"));
        assert!(line_has_escaped_newline_continuation(r#"printf "foo\"#));
    }
}
