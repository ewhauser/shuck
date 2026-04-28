use super::*;

pub(crate) fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

pub(crate) fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
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
