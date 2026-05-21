use super::*;
use shuck_ast::raw_shell;

pub(crate) fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

pub(crate) fn advance_shell_char(text: &str, index: usize) -> usize {
    raw_shell::advance_shell_char(text, index)
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
    raw_shell::offset_is_backslash_escaped(source, offset)
}

pub(crate) fn span_is_escaped(span: Span, source: &str) -> bool {
    span_is_backslash_escaped(span, source)
}
