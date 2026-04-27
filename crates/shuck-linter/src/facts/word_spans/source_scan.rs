use super::*;

pub fn word_standalone_literal_backslash_span(word: &Word, source: &str) -> Option<Span> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    if !matches!(part.kind, WordPart::Literal(_)) {
        return None;
    }

    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    if bytes.len() != 2 || bytes[0] != b'\\' {
        return None;
    }

    let target = bytes[1];
    if !target.is_ascii_lowercase() || matches!(target, b'n' | b'r' | b't') {
        return None;
    }

    Some(Span::from_positions(word.span.start, word.span.start))
}

pub fn word_unquoted_star_parameter_spans(word: &Word, unquoted_array_spans: &[Span]) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| {
            (unquoted_array_spans.contains(&span) && part_uses_star_splat(part)).then_some(span)
        })
        .collect()
}

impl EscapedBacktickParameterSyntax {
    pub(super) fn name(&self) -> Option<&shuck_ast::Name> {
        match self {
            Self::Simple { name, .. } => Some(name),
            Self::ComplexUnsafe { .. } => None,
        }
    }

    pub(super) fn expansion_len(&self) -> usize {
        match self {
            Self::Simple { expansion_len, .. } | Self::ComplexUnsafe { expansion_len } => {
                *expansion_len
            }
        }
    }
}

pub(super) fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

pub(super) fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }

    let mut position = Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

pub(super) fn span_within_literal(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
}

pub(super) fn scan_span_excluding(span: Span, excluded: &[Span], source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_scan_span_excluding(span, excluded, source, &mut spans);
    spans
}

pub(super) fn collect_scan_span_excluding(
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

pub(super) fn merge_adjacent_spans(spans: Vec<Span>, source: &str) -> Vec<Span> {
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

pub(super) fn spans_share_literal_run(previous: Span, next: Span, source: &str) -> bool {
    if previous.end.offset >= next.start.offset {
        return true;
    }

    let gap = &source[previous.end.offset..next.start.offset];
    !gap.contains('$') && !gap.contains('`')
}

pub(super) fn scan_span_segment(span: Span, start: usize, end: usize, source: &str) -> Span {
    let segment_start = span.start.advanced_by(&source[span.start.offset..start]);
    let segment_end = span.start.advanced_by(&source[span.start.offset..end]);
    Span::from_positions(segment_start, segment_end)
}

pub(super) fn word_surface_bytes(
    word: &Word,
    source: &str,
) -> Option<(Vec<u8>, Vec<Option<usize>>)> {
    if word.has_quoted_parts() {
        return None;
    }

    let word_start = word.span.start.offset;
    let mut surface = Vec::new();
    let mut source_offsets = Vec::new();

    for part in &word.parts {
        match &part.kind {
            WordPart::Literal(_) => {
                let part_text = part.span.slice(source);
                let relative_start = part.span.start.offset.checked_sub(word_start)?;
                for (index, byte) in part_text.as_bytes().iter().copied().enumerate() {
                    surface.push(byte);
                    source_offsets.push(Some(relative_start + index));
                }
            }
            WordPart::DoubleQuoted { .. } | WordPart::SingleQuoted { .. } => return None,
            WordPart::ZshQualifiedGlob(_)
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {
                surface.push(b'_');
                source_offsets.push(None);
            }
        }
    }

    Some((surface, source_offsets))
}

pub(super) fn word_surface_span_from_bounds(
    word: &Word,
    source: &str,
    source_offsets: &[Option<usize>],
    start: usize,
    end: usize,
) -> Option<Span> {
    let start_offset = source_offsets.get(start).copied().flatten()?;
    let end_offset = source_offsets.get(end).copied().flatten()?;
    let word_text = word.span.slice(source);

    Some(Span::from_positions(
        word.span.start.advanced_by(&word_text[..start_offset]),
        word.span.start.advanced_by(&word_text[..end_offset + 1]),
    ))
}

pub(super) fn find_parenthesized_alternation_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'(' || byte_is_backslash_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let Some(close) = matching_group_end(bytes, index) else {
            index += 1;
            continue;
        };

        if bytes[index + 1..close]
            .iter()
            .enumerate()
            .any(|(offset, byte)| {
                *byte == b'|' && !byte_is_backslash_escaped(bytes, index + 1 + offset)
            })
        {
            return Some((index, close));
        }

        index = close + 1;
    }

    None
}

pub(super) fn matching_group_end(bytes: &[u8], open_index: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut cursor = open_index + 1;

    while cursor < bytes.len() {
        if byte_is_backslash_escaped(bytes, cursor) {
            cursor += 1;
            continue;
        }

        match bytes[cursor] {
            b'(' => {
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => {}
        }

        cursor += 1;
    }

    None
}

pub(super) fn span_is_backslash_escaped(span: Span, source: &str) -> bool {
    offset_is_backslash_escaped(span.start.offset, source)
}

pub(super) fn offset_is_backslash_escaped(offset: usize, source: &str) -> bool {
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

pub(super) fn span_is_escaped(span: Span, source: &str) -> bool {
    span_is_backslash_escaped(span, source)
}
