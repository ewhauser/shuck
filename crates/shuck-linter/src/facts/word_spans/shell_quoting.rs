use super::*;

pub fn double_quoted_scalar_affix_span(word: &Word) -> Option<Span> {
    if !word.is_fully_double_quoted() {
        return None;
    }

    let mut saw_literal = false;
    let mut saw_scalar_expansion = false;
    let mut literal_span = None;
    if !collect_double_quoted_scalar_affix_state(
        &word.parts,
        &mut saw_literal,
        &mut saw_scalar_expansion,
        &mut literal_span,
    ) {
        return None;
    }

    (saw_literal && saw_scalar_expansion)
        .then_some(literal_span)
        .flatten()
}

pub fn word_shell_quoting_literal_span(word: &Word, source: &str) -> Option<Span> {
    let mut excluded = Vec::new();
    collect_literal_scan_exclusions(&word.parts, &mut excluded);

    merge_adjacent_spans(
        word_literal_scan_segments_excluding_expansions(word, source),
        source,
    )
    .into_iter()
    .find_map(|span| {
        let normalized = normalize_shell_quoting_segment_span(word, span, source);
        text_contains_shell_quoting_literals(normalized.slice(source))
            .then(|| shell_quoting_literal_run_span(word, normalized, &excluded, source))
    })
}

pub fn word_shell_quoting_literal_run_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let text = word.span.slice(source);
    let mut cursor = if word.is_fully_double_quoted() && text.starts_with('"') {
        1
    } else {
        0
    };
    let limit = if word.is_fully_double_quoted() && text.ends_with('"') {
        text.len().saturating_sub(1)
    } else {
        text.len()
    };
    let mut saw_expansion = false;
    let mut in_single = false;
    let mut in_double = word.is_fully_double_quoted() && text.starts_with('"');
    let mut index = cursor;

    while index < limit {
        let tail = &text[index..limit];
        let Some(ch) = tail.chars().next() else {
            break;
        };
        if ch == '\'' && !in_double && !text_position_is_escaped(text, index) {
            in_single = !in_single;
            index += ch.len_utf8();
            continue;
        }
        if ch == '"' && !in_single && !text_position_is_escaped(text, index) {
            in_double = !in_double;
            index += ch.len_utf8();
            continue;
        }
        if !in_single && matches!(ch, '$' | '`') && !text_position_is_escaped(text, index) {
            saw_expansion = true;
            if let Some(span) = word_shell_quoting_segment_span_in_source(word, text, cursor, index)
            {
                return Some(span);
            }
            index += shell_quoting_expansion_len(tail);
            cursor = index;
            continue;
        }
        index += ch.len_utf8();
    }

    if let Some(span) = word_shell_quoting_segment_span_in_source(word, text, cursor, limit) {
        return Some(span);
    }
    if !saw_expansion && text_contains_shell_quoting_literals(&text[..limit]) {
        return Some(word.span);
    }

    None
}

pub fn word_double_quoted_scalar_only_expansion_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_scalar_only_expansion_spans(&word.parts, false, &mut spans)
        .then_some(spans)
        .filter(|spans| !spans.is_empty())
        .unwrap_or_default()
}

pub fn word_unquoted_word_after_single_quoted_segment_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();

    for (index, part) in word.parts.iter().enumerate() {
        if !is_non_dollar_single_quoted(part) {
            continue;
        }
        if single_quoted_fragment_inner_text(part, source).is_some_and(|text| text.ends_with('\\'))
        {
            continue;
        }

        for next_part in word.parts.iter().skip(index + 1) {
            if next_part.kind.is_quoted() {
                break;
            }

            let WordPart::Literal(text) = &next_part.kind else {
                continue;
            };
            if literal_contains_unquoted_word_chars(text.as_str(source, next_part.span)) {
                spans.push(next_part.span);
            }
        }
    }

    spans
}

pub fn word_unquoted_scalar_between_double_quoted_segments_spans(
    word: &Word,
    candidate_spans: &[Span],
) -> Vec<Span> {
    if word.parts.len() < 3 {
        return Vec::new();
    }

    word.parts
        .windows(3)
        .filter_map(|window| {
            let [left, middle, right] = window else {
                return None;
            };

            (matches!(left.kind, WordPart::DoubleQuoted { .. })
                && candidate_spans.contains(&middle.span)
                && matches!(right.kind, WordPart::DoubleQuoted { .. }))
            .then_some(middle.span)
        })
        .collect()
}

pub fn word_nested_dynamic_double_quote_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_nested_dynamic_double_quote_spans(&word.parts, false, &mut spans);
    spans
}

pub(super) fn collect_double_quoted_scalar_affix_state(
    parts: &[WordPartNode],
    saw_literal: &mut bool,
    saw_scalar_expansion: &mut bool,
    literal_span: &mut Option<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                *saw_literal = true;
                if literal_span.is_none() {
                    *literal_span = Some(part.span);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_affix_state(
                    parts,
                    saw_literal,
                    saw_scalar_expansion,
                    literal_span,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                *saw_scalar_expansion = true;
            }
            WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

pub(super) fn collect_double_quoted_scalar_only_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_only_expansion_spans(parts, true, spans) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                if !inside_double_quotes {
                    return false;
                }
                spans.push(part.span);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

pub(super) fn normalize_shell_quoting_segment_span(word: &Word, span: Span, source: &str) -> Span {
    let mut start = span.start;
    let mut end = span.end;
    let text = span.slice(source);
    if word.is_fully_double_quoted() {
        if span.start.offset == word.span.start.offset && text.starts_with('"') {
            start = start.advanced_by("\"");
        }
        if span.end.offset == word.span.end.offset && text.ends_with('"') {
            end = span.start.advanced_by(&text[..text.len() - 1]);
        }
    }

    let normalized = Span::from_positions(start, end);
    let normalized_text = normalized.slice(source);
    if normalized_text.ends_with('\\')
        && let Some(next) = source
            .get(normalized.end.offset..)
            .and_then(|tail| tail.chars().next())
        && matches!(next, '"' | '\'')
    {
        let quote = if next == '"' { "\"" } else { "'" };
        return Span::from_positions(normalized.start, normalized.end.advanced_by(quote));
    }

    normalized
}

pub(super) fn text_contains_shell_quoting_literals(text: &str) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '\\' {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] == '\\' {
            end += 1;
        }
        if chars.get(end).is_some_and(|next| {
            matches!(next, '"' | '\'') || (next.is_whitespace() && !matches!(next, '\n' | '\r'))
        }) {
            return true;
        }

        index = end;
    }

    false
}

pub(super) fn text_position_is_escaped(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    let mut cursor = offset;
    let mut backslashes = 0usize;
    while cursor > 0 {
        cursor -= 1;
        if bytes[cursor] != b'\\' {
            break;
        }
        backslashes += 1;
    }

    backslashes % 2 == 1
}

pub(super) fn shell_quoting_literal_run_span(
    word: &Word,
    span: Span,
    excluded: &[Span],
    source: &str,
) -> Span {
    let start = excluded
        .iter()
        .copied()
        .filter(|excluded_span| excluded_span.start.offset < span.start.offset)
        .map(|excluded_span| excluded_span.end)
        .max_by_key(|position| position.offset)
        .unwrap_or(word.span.start);
    let end = excluded
        .iter()
        .copied()
        .filter(|excluded_span| excluded_span.start.offset > start.offset)
        .map(|excluded_span| excluded_span.start)
        .min_by_key(|position| position.offset)
        .unwrap_or(word.span.end);

    normalize_shell_quoting_segment_span(word, Span::from_positions(start, end), source)
}

pub(super) fn word_shell_quoting_segment_span_in_source(
    word: &Word,
    text: &str,
    start: usize,
    end: usize,
) -> Option<Span> {
    let segment = &text[start..end];
    if !text_contains_shell_quoting_literals(segment) {
        return None;
    }

    let trimmed_start = if let Some(anchor) = first_shell_quoting_escape_anchor(segment) {
        segment[..anchor]
            .rfind('\'')
            .map_or(start, |quote| start + quote + 1)
    } else {
        start
    };

    Some(Span::from_positions(
        word.span.start.advanced_by(&text[..trimmed_start]),
        word.span.start.advanced_by(&text[..end]),
    ))
}

pub(super) fn first_shell_quoting_escape_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    first_shell_quoting_anchor(text)
}

pub(super) fn first_shell_quoting_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if matches!(ch, '"' | '\'') {
            return Some(offset);
        }
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    None
}

pub(super) fn shell_quoting_expansion_len(text: &str) -> usize {
    if text.starts_with('`') {
        return closing_backtick_offset(text).unwrap_or(1);
    }
    if !text.starts_with('$') {
        return 1;
    }

    if text.starts_with("${") {
        return braced_expansion_len(text).unwrap_or(2);
    }
    if text.starts_with("$(") {
        return paren_expansion_len(text).unwrap_or(2);
    }

    let bytes = text.as_bytes();
    let Some(&next) = bytes.get(1) else {
        return 1;
    };
    if (next as char).is_ascii_alphabetic() || next == b'_' {
        let mut end = 2usize;
        while let Some(byte) = bytes.get(end) {
            let ch = *byte as char;
            if ch.is_ascii_alphanumeric() || ch == '_' {
                end += 1;
                continue;
            }
            break;
        }
        return end;
    }
    if (next as char).is_ascii_digit() || b"@*#?$!-".contains(&next) {
        return 2;
    }

    1
}

pub(super) fn is_non_dollar_single_quoted(part: &WordPartNode) -> bool {
    matches!(part.kind, WordPart::SingleQuoted { dollar: false, .. })
}

pub(super) fn single_quoted_fragment_inner_text<'a>(
    part: &WordPartNode,
    source: &'a str,
) -> Option<&'a str> {
    let WordPart::SingleQuoted { dollar: false, .. } = part.kind else {
        return None;
    };

    part.span
        .slice(source)
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''))
}

pub(super) fn literal_contains_unquoted_word_chars(text: &str) -> bool {
    !text.is_empty()
        && text.as_bytes().iter().all(u8::is_ascii_alphanumeric)
        && text.as_bytes().iter().any(u8::is_ascii_alphanumeric)
}

pub(super) fn collect_nested_dynamic_double_quote_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for (index, part) in parts.iter().enumerate() {
        let WordPart::DoubleQuoted { parts: inner, .. } = &part.kind else {
            continue;
        };

        if inside_double_quotes
            && double_quoted_parts_contain_dynamic_content(inner)
            && (neighbor_is_literal(parts.get(index.wrapping_sub(1)))
                || neighbor_is_literal(parts.get(index + 1)))
        {
            spans.push(part.span);
        }

        collect_nested_dynamic_double_quote_spans(inner, true, spans);
    }
}

pub(super) fn double_quoted_parts_contain_dynamic_content(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => double_quoted_parts_contain_dynamic_content(parts),
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
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
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => true,
    })
}

pub(super) fn neighbor_is_literal(part: Option<&WordPartNode>) -> bool {
    matches!(part.map(|part| &part.kind), Some(WordPart::Literal(_)))
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::facts::word_spans::*;
    #[allow(unused_imports)]
    use shuck_ast::Span;
    #[allow(unused_imports)]
    use shuck_parser::parser::Parser;

    #[test]
    fn word_unquoted_word_after_single_quoted_segment_spans_tracks_literal_suffix_words() {
        let source = "\
printf '%s\\n' 'foo'Default'baz' 'foo'123'baz' 'foo'-'baz' 'foo''baz' 'foo'$bar'baz' $'foo'Default'baz' '/x/'d ^default'\\s'via 'left'lib$SUFFIX'right' 'left'fuzz_ng_$SUFFIX'right'
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_unquoted_word_after_single_quoted_segment_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["Default", "123", "d", "via", "lib"]);
    }

    #[test]
    fn word_unquoted_word_after_single_quoted_segment_ignores_escaped_quote_bridges() {
        let source = "\
printf '%s\\n' 's/foo/'\\''bar'\\''/g' 'foo'Default'baz'
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| word_unquoted_word_after_single_quoted_segment_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["Default"]);
    }

    #[test]
    fn word_unquoted_scalar_between_double_quoted_segments_tracks_dynamic_middle_parts() {
        let source = "\
printf '%s\\n' \"$a\"$b\"$c\" \"left \"$d\"\" \"\"$e\" right\" \"left \"$(printf '%s' ok)\" right\" \"a\"b\"c\" prefix\"$f\"suffix \"$g\"$@\"$h\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| {
                let unquoted_scalar_spans = unquoted_scalar_expansion_part_spans(word, source)
                    .into_iter()
                    .chain(unquoted_command_substitution_part_spans_in_source(
                        word, source,
                    ))
                    .collect::<Vec<_>>();
                word_unquoted_scalar_between_double_quoted_segments_spans(
                    word,
                    &unquoted_scalar_spans,
                )
            })
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$b", "$d", "$e", "$(printf '%s' ok)"]);
    }

    #[test]
    fn word_double_quoted_scalar_only_expansion_spans_ignore_literal_affixes() {
        let source = "\
printf '%s\\n' \"$a\" \"$a\"\"$b\" \"prefix$a\" \"$a$(printf '%s' x)\" $a \"$a\"/\"$b\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_double_quoted_scalar_only_expansion_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$a", "$a", "$b"]);
    }

    #[test]
    fn word_nested_dynamic_double_quote_spans_track_reopened_quotes_inside_outer_quotes() {
        let source = "\
printf '%s\\n' \"\n-DLZ4_HOME=\"${TERMUX_PREFIX}\"\n-DPROTOBUF_HOME=\"$(printf '%s' proto)\"\n\"\n
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = word_nested_dynamic_double_quote_spans(&command.args[1])
            .into_iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, Vec::<&str>::new());
    }
}
