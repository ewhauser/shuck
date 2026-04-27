use super::*;

pub fn unescaped_backtick_command_substitution_span(span: Span, source: &str) -> Option<Span> {
    let normalized = normalize_command_substitution_span(span, source);
    let text = normalized.slice(source);
    if !text.starts_with('`') || !text.ends_with('`') || span_is_escaped(normalized, source) {
        return None;
    }

    Some(normalized)
}

pub(crate) fn shellcheck_collapsed_backtick_part_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Span {
    let deescaped =
        shellcheck_deescaped_backtick_part_span(span, source, backtick_spans).unwrap_or(span);
    collapse_backtick_continuation_span(deescaped, source, backtick_spans).unwrap_or(deescaped)
}

pub(crate) fn shellcheck_collapsed_backtick_part_span_in_source(span: Span, source: &str) -> Span {
    let backtick_spans = backtick_substitution_spans(source);
    shellcheck_collapsed_backtick_part_span(span, source, &backtick_spans)
}

pub(super) fn span_inside_escaped_parameter_template(
    word_span: Span,
    span: Span,
    source: &str,
) -> bool {
    if span.start.offset < word_span.start.offset || span.start.offset >= word_span.end.offset {
        return false;
    }

    let text = word_span.slice(source);
    let relative_offset = span.start.offset - word_span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("\\${") {
            let dollar_offset = index + '\\'.len_utf8();
            if offset_is_backslash_escaped(word_span.start.offset + dollar_offset, source)
                && let Some(end_offset) = escaped_parameter_template_end(text, dollar_offset)
            {
                let body_start = dollar_offset + "${".len();
                let body_end = end_offset.saturating_sub('}'.len_utf8());
                if relative_offset >= body_start && relative_offset < body_end {
                    return true;
                }
                index = end_offset;
                continue;
            }
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }

    false
}

pub(super) fn escaped_parameter_template_end(text: &str, dollar_offset: usize) -> Option<usize> {
    if dollar_offset >= text.len() || !text[dollar_offset..].starts_with("${") {
        return None;
    }

    let bytes = text.as_bytes();
    let mut index = dollar_offset + "${".len();
    let mut depth = 1usize;
    let mut quote_state = EscapedTemplateQuote::None;

    while index < bytes.len() {
        let byte = bytes[index];
        match quote_state {
            EscapedTemplateQuote::Single => {
                if byte == b'\'' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::Double => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::None => {}
        }

        match byte {
            b'\\' => {
                index += usize::from(index + 1 < bytes.len()) + 1;
            }
            b'\'' => {
                quote_state = EscapedTemplateQuote::Single;
                index += 1;
            }
            b'"' => {
                quote_state = EscapedTemplateQuote::Double;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                depth += 1;
                index += "${".len();
            }
            b'}' => {
                depth -= 1;
                index += '}'.len_utf8();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EscapedTemplateQuote {
    None,
    Single,
    Double,
}

pub(super) fn collapse_backtick_continuation_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Option<Span> {
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let chain_start = continued_line_chain_start(span.start, containing_span, source)?;
    Some(Span::from_positions(
        shellcheck_collapsed_position(chain_start, span.start, source),
        shellcheck_collapsed_position(chain_start, span.end, source),
    ))
}

pub(super) fn shellcheck_deescaped_backtick_part_span(
    span: Span,
    source: &str,
    backtick_spans: &[Span],
) -> Option<Span> {
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let content_start = containing_span.start.offset.saturating_add('`'.len_utf8());
    let start_removed = backtick_removed_escape_count(source, content_start, span.start.offset)?;
    let end_removed = backtick_removed_escape_count(source, content_start, span.end.offset)?;
    if start_removed == 0 && end_removed == 0 {
        return None;
    }

    Some(Span::from_positions(
        position_at_offset(source, span.start.offset.checked_sub(start_removed)?)?,
        position_at_offset(source, span.end.offset.checked_sub(end_removed)?)?,
    ))
}

pub(super) fn backtick_removed_escape_count(
    source: &str,
    start: usize,
    end: usize,
) -> Option<usize> {
    let mut removed = 0usize;
    let mut index = start;
    while index < end {
        let ch = source[index..].chars().next()?;
        let ch_len = ch.len_utf8();
        if ch != '\\' {
            index += ch_len;
            continue;
        }

        let next_offset = index + ch_len;
        if next_offset >= end {
            break;
        }
        let escaped = source[next_offset..].chars().next()?;
        if matches!(escaped, '$' | '`' | '\\') {
            removed += 1;
            index = next_offset + escaped.len_utf8();
        } else {
            index += ch_len;
        }
    }

    Some(removed)
}

pub(super) fn containing_backtick_substitution_span(
    target: Span,
    backtick_spans: &[Span],
) -> Option<Span> {
    backtick_spans
        .iter()
        .copied()
        .find(|span| span_contains(*span, target))
}

#[derive(Clone, Copy, Default)]
pub(super) struct BacktickQuoteContext {
    in_single_quote: bool,
    in_double_quote: bool,
    in_comment: bool,
    previous_char: Option<char>,
}

pub(super) fn backtick_shell_comment_can_start(previous_char: Option<char>) -> bool {
    previous_char.is_none_or(|ch| {
        ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
    })
}

pub(crate) fn backtick_substitution_spans(source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut contexts = vec![BacktickQuoteContext::default()];
    let mut backtick_start_offsets = Vec::<usize>::new();
    let mut index = 0usize;

    while index < source.len() {
        let ch = source[index..]
            .chars()
            .next()
            .expect("index should remain on UTF-8 boundaries");
        let ch_len = ch.len_utf8();

        if contexts
            .last()
            .expect("scanner should always retain a root context")
            .in_comment
        {
            if ch == '\n' {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_comment = false;
                context.previous_char = Some(ch);
            }
            index += ch_len;
            continue;
        }

        if contexts
            .last()
            .expect("scanner should always retain a root context")
            .in_single_quote
        {
            if ch == '\'' {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .in_single_quote = false;
            }
            contexts
                .last_mut()
                .expect("scanner should always retain a root context")
                .previous_char = Some(ch);
            index += ch_len;
            continue;
        }

        match ch {
            '\\' => {
                index += ch_len;
                if index < source.len() {
                    let escaped = source[index..]
                        .chars()
                        .next()
                        .expect("index should remain on UTF-8 boundaries");
                    index += escaped.len_utf8();
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(escaped);
                } else {
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some('\\');
                }
            }
            '\'' if !contexts
                .last()
                .expect("scanner should always retain a root context")
                .in_double_quote =>
            {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_single_quote = true;
                context.previous_char = Some(ch);
                index += ch_len;
            }
            '"' => {
                let context = contexts
                    .last_mut()
                    .expect("scanner should always retain a root context");
                context.in_double_quote = !context.in_double_quote;
                context.previous_char = Some(ch);
                index += ch_len;
            }
            '#' if !contexts
                .last()
                .expect("scanner should always retain a root context")
                .in_double_quote
                && backtick_shell_comment_can_start(
                    contexts
                        .last()
                        .expect("scanner should always retain a root context")
                        .previous_char,
                ) =>
            {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .in_comment = true;
                index += ch_len;
            }
            '`' => {
                if let Some(start_offset) = backtick_start_offsets.pop() {
                    let _ = contexts.pop();
                    let Some(start) = position_at_offset(source, start_offset) else {
                        index += ch_len;
                        continue;
                    };
                    let Some(end) = position_at_offset(source, index + ch_len) else {
                        index += ch_len;
                        continue;
                    };
                    spans.push(Span::from_positions(start, end));
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(ch);
                } else {
                    backtick_start_offsets.push(index);
                    contexts
                        .last_mut()
                        .expect("scanner should always retain a root context")
                        .previous_char = Some(ch);
                    contexts.push(BacktickQuoteContext::default());
                }
                index += ch_len;
            }
            _ => {
                contexts
                    .last_mut()
                    .expect("scanner should always retain a root context")
                    .previous_char = Some(ch);
                index += ch_len;
            }
        }
    }

    spans
}

pub(crate) fn backtick_escaped_parameters(
    source: &str,
    backtick_spans: &[Span],
) -> Vec<BacktickEscapedParameter> {
    let mut spans = Vec::new();

    for backtick_span in backtick_spans {
        let mut index = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let end = backtick_span.end.offset.saturating_sub('`'.len_utf8());
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut removed_escapes = 0usize;

        while index < end {
            let ch = source[index..]
                .chars()
                .next()
                .expect("index should remain on UTF-8 boundaries");
            let ch_len = ch.len_utf8();

            match ch {
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                    index += ch_len;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                    index += ch_len;
                }
                '\\' if !in_single_quote => {
                    let slash_offset = index;
                    index += ch_len;
                    if index >= end {
                        continue;
                    }

                    let escaped = source[index..]
                        .chars()
                        .next()
                        .expect("index should remain on UTF-8 boundaries");
                    if escaped == '$'
                        && !in_double_quote
                        && let Some(parameter) =
                            escaped_backtick_parameter_syntax(source, index, end)
                    {
                        let expansion_len = parameter.expansion_len();
                        let diagnostic_start_offset = slash_offset.saturating_sub(removed_escapes);
                        let Some(diagnostic_start) =
                            position_at_offset(source, diagnostic_start_offset)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(diagnostic_end) =
                            position_at_offset(source, diagnostic_start_offset + expansion_len)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_start) = position_at_offset(source, index) else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_end) = position_at_offset(source, index + expansion_len)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        spans.push(BacktickEscapedParameter {
                            name: parameter.name().cloned(),
                            diagnostic_span: Span::from_positions(diagnostic_start, diagnostic_end),
                            reference_span: Span::from_positions(reference_start, reference_end),
                            standalone_command_name:
                                escaped_backtick_parameter_is_standalone_command_name(
                                    source,
                                    *backtick_span,
                                    slash_offset,
                                    index + expansion_len,
                                ),
                        });
                        removed_escapes += 1;
                        index += expansion_len;
                    } else {
                        index += escaped.len_utf8();
                    }
                }
                _ => {
                    index += ch_len;
                }
            }
        }
    }

    spans.sort_by_key(|parameter| {
        (
            parameter.diagnostic_span.start.offset,
            parameter.diagnostic_span.end.offset,
            parameter.reference_span.start.offset,
            parameter.reference_span.end.offset,
        )
    });
    spans.dedup();
    spans
}

pub(crate) fn backtick_escaped_parameter_reference_spans(
    source: &str,
    backtick_spans: &[Span],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for backtick_span in backtick_spans {
        let base_offset = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let end = backtick_span.end.offset.saturating_sub('`'.len_utf8());
        let Some(text) = source.get(base_offset..end) else {
            continue;
        };
        let mut index = 0usize;

        while index < text.len() {
            if text[index..].starts_with("\\${") {
                let dollar_offset = index + '\\'.len_utf8();
                if offset_is_backslash_escaped(base_offset + dollar_offset, source)
                    && let Some(end_offset) = escaped_parameter_template_end(text, dollar_offset)
                    && let Some(start) = position_at_offset(source, base_offset + dollar_offset)
                    && let Some(end_position) = position_at_offset(source, base_offset + end_offset)
                {
                    spans.push(Span::from_positions(start, end_position));
                    index = end_offset;
                    continue;
                }
            }

            let Some(ch) = text[index..].chars().next() else {
                break;
            };
            index += ch.len_utf8();
        }
    }

    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

pub(crate) fn backtick_double_escaped_parameter_spans(
    source: &str,
    backtick_spans: &[Span],
) -> Vec<Span> {
    let mut spans = Vec::new();

    for backtick_span in backtick_spans {
        let mut index = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let end = backtick_span.end.offset.saturating_sub('`'.len_utf8());
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while index < end {
            let ch = source[index..]
                .chars()
                .next()
                .expect("index should remain on UTF-8 boundaries");
            let ch_len = ch.len_utf8();

            match ch {
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                    index += ch_len;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                    index += ch_len;
                }
                '\\' if !in_single_quote => {
                    let slash_start = index;
                    while index < end && source.as_bytes().get(index) == Some(&b'\\') {
                        index += '\\'.len_utf8();
                    }
                    let slash_count = index.saturating_sub(slash_start);
                    if in_double_quote
                        && slash_count == 2
                        && source.as_bytes().get(index) == Some(&b'$')
                        && let Some(parameter) =
                            escaped_backtick_parameter_syntax(source, index, end)
                    {
                        let expansion_len = parameter.expansion_len();
                        if let Some(start) = position_at_offset(source, index)
                            && let Some(end_position) =
                                position_at_offset(source, index + expansion_len)
                        {
                            spans.push(Span::from_positions(start, end_position));
                        }
                        index += expansion_len;
                    } else if slash_count % 2 == 1 && index < end {
                        let escaped = source[index..]
                            .chars()
                            .next()
                            .expect("index should remain on UTF-8 boundaries");
                        index += escaped.len_utf8();
                    }
                }
                _ => {
                    index += ch_len;
                }
            }
        }
    }

    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

pub(super) fn escaped_backtick_parameter_is_standalone_command_name(
    source: &str,
    backtick_span: Span,
    slash_offset: usize,
    expansion_end: usize,
) -> bool {
    let segment_start = backtick_command_segment_start(
        source,
        backtick_span.start.offset.saturating_add('`'.len_utf8()),
        slash_offset,
    );
    let Some(prefix) = source.get(segment_start..slash_offset) else {
        return false;
    };
    if !command_prefix_is_empty_or_assignments(prefix) {
        return false;
    }

    let suffix_limit = backtick_span.end.offset.saturating_sub('`'.len_utf8());
    escaped_reference_ends_standalone_word(source, expansion_end, suffix_limit)
}

pub(super) fn backtick_command_segment_start(source: &str, start: usize, end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut segment_start = start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while cursor < end {
        let byte = bytes[cursor];
        if escaped {
            escaped = false;
            cursor += 1;
            continue;
        }
        if byte == b'\\' && !in_single_quote {
            escaped = true;
            cursor += 1;
            continue;
        }

        match byte {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'\n' | b';' if !in_single_quote && !in_double_quote => {
                segment_start = cursor + 1;
            }
            b'&' | b'|' if !in_single_quote && !in_double_quote => {
                let separator = byte;
                cursor += 1;
                while cursor < end && bytes[cursor] == separator {
                    cursor += 1;
                }
                segment_start = cursor;
                continue;
            }
            _ => {}
        }

        cursor += 1;
    }

    segment_start
}

pub(super) fn command_prefix_is_empty_or_assignments(prefix: &str) -> bool {
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

pub(super) fn skip_shell_whitespace(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

pub(super) fn shell_word_end(text: &str, start: usize) -> Option<usize> {
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

pub(super) fn skip_balanced_shell_construct(
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

pub(super) fn skip_legacy_backtick_construct(text: &str, mut index: usize) -> Option<usize> {
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

pub(super) fn advance_escaped_shell_char(text: &str, index: usize) -> usize {
    let next = advance_shell_char(text, index);
    if next < text.len() {
        advance_shell_char(text, next)
    } else {
        next
    }
}

pub(super) fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
}

pub(super) fn simple_assignment_word(word: &str) -> bool {
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

pub(super) fn redirection_prefix_end(text: &str, start: usize) -> Option<usize> {
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

pub(super) fn redirection_operator_len(text: &str) -> Option<usize> {
    [
        "&>>", "<<<", "<>", ">>", "<<", "<&", ">&", ">|", "&>", "<", ">",
    ]
    .into_iter()
    .find(|operator| text.starts_with(operator))
    .map(str::len)
}

pub(super) fn escaped_reference_ends_standalone_word(
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

pub(super) enum EscapedBacktickParameterSyntax {
    Simple {
        name: shuck_ast::Name,
        expansion_len: usize,
    },
    ComplexUnsafe {
        expansion_len: usize,
    },
}

pub(super) fn escaped_backtick_parameter_syntax(
    source: &str,
    dollar_offset: usize,
    end: usize,
) -> Option<EscapedBacktickParameterSyntax> {
    let next_offset = dollar_offset + '$'.len_utf8();
    let next = source.get(next_offset..end)?.chars().next()?;

    if matches!(next, '?' | '#' | '@' | '*' | '!' | '$' | '-') {
        return None;
    }
    if next.is_ascii_digit() {
        return Some(EscapedBacktickParameterSyntax::Simple {
            name: shuck_ast::Name::new(next.to_string()),
            expansion_len: "$0".len(),
        });
    }
    if next == '{' {
        let close_relative = source.get(next_offset + '{'.len_utf8()..end)?.find('}')?;
        let close_offset = next_offset + '{'.len_utf8() + close_relative;
        let inner = source.get(next_offset + '{'.len_utf8()..close_offset)?;
        let first = inner.chars().next()?;
        if matches!(first, '?' | '#' | '@' | '*' | '!' | '$' | '-') {
            return None;
        }
        let name_text = inner
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect::<String>();
        if name_text.is_empty() {
            return None;
        }
        let expansion_len = close_offset + '}'.len_utf8() - dollar_offset;
        if name_text.len() == inner.len() {
            return Some(EscapedBacktickParameterSyntax::Simple {
                name: shuck_ast::Name::new(name_text),
                expansion_len,
            });
        }
        let operator = &inner[name_text.len()..];
        if operator.starts_with(":+") || operator.starts_with('+') {
            return None;
        }
        return Some(EscapedBacktickParameterSyntax::ComplexUnsafe { expansion_len });
    }
    if next.is_ascii_alphabetic() || next == '_' {
        let mut cursor = next_offset;
        while cursor < end {
            let ch = source[cursor..]
                .chars()
                .next()
                .expect("cursor should remain on UTF-8 boundaries");
            if !(ch.is_ascii_alphanumeric() || ch == '_') {
                break;
            }
            cursor += ch.len_utf8();
        }
        let name = source.get(next_offset..cursor)?;
        return Some(EscapedBacktickParameterSyntax::Simple {
            name: shuck_ast::Name::new(name),
            expansion_len: cursor - dollar_offset,
        });
    }

    None
}

pub(super) fn continued_line_chain_start(
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

pub(super) fn line_has_escaped_newline_continuation(line: &str) -> bool {
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

pub(super) fn shellcheck_collapsed_position(
    chain_start: Position,
    target: Position,
    source: &str,
) -> Position {
    let mut line = chain_start.line;
    let mut column = chain_start.column;
    let mut in_collapsed_continuation = false;
    let prefix = &source[chain_start.offset..target.offset];
    let mut index = 0usize;

    while index < prefix.len() {
        if prefix[index..].starts_with("\\\r\n") {
            index += "\\\r\n".len();
            in_collapsed_continuation = true;
            continue;
        }

        if prefix[index..].starts_with("\\\n") {
            index += "\\\n".len();
            in_collapsed_continuation = true;
            continue;
        }

        let ch = prefix[index..]
            .chars()
            .next()
            .expect("prefix iteration should stay on UTF-8 boundaries");
        if ch == '\n' {
            line += 1;
            column = 1;
            in_collapsed_continuation = false;
        } else if ch == '\t' && in_collapsed_continuation {
            column = ((column - 1) / 8 + 1) * 8 + 2;
        } else {
            column += 1;
        }
        index += ch.len_utf8();
    }

    Position {
        line,
        column,
        offset: target.offset,
    }
}

pub(super) fn widen_backtick_command_substitution_span(span: Span, source: &str) -> Option<Span> {
    let mut index = span.start.offset;
    let bytes = source.as_bytes();
    if bytes.get(index)? != &b'`' {
        return None;
    }
    index += 1;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'`' => {
                index += 1;
                let start = position_at_offset(source, span.start.offset)?;
                let end = position_at_offset(source, index)?;
                return Some(Span::from_positions(start, end));
            }
            _ => index += 1,
        }
    }

    None
}

pub(super) fn closing_backtick_offset(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    chars.next()?;
    for (offset, ch) in chars {
        if ch == '`' && !text_position_is_escaped(text, offset) {
            return Some(offset + 1);
        }
    }

    None
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

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_single_quoted_backticks() {
        let source = "printf '%s\\n' '`'\\\n  \"$foo\"\\\n  '`'\n";
        let start_offset = source.find("$foo").expect("expected expansion");
        let end_offset = start_offset + "$foo".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_backticks_in_comments() {
        let source = "# `\nprintf '%s\\n' \\\n  \"$foo\"\n# `\n";
        let start_offset = source.find("$foo").expect("expected expansion");
        let end_offset = start_offset + "$foo".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_single_quoted_backslashes() {
        let source = "echo `printf '%s\\n' 'foo\\\n$bar'`\n";
        let start_offset = source.find("$bar").expect("expected expansion");
        let end_offset = start_offset + "$bar".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_preserves_multiline_single_quote_context()
    {
        let source = "echo `printf '%s\\n' '\nfoo\\\n$bar'`\n";
        let start_offset = source.find("$bar").expect("expected expansion");
        let end_offset = start_offset + "$bar".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_clears_escape_state_after_continuations() {
        let source = "echo `printf '%s\\n' foo\\\n'$bar\\\n'\n$baz`\n";
        let start_offset = source.find("$baz").expect("expected expansion");
        let end_offset = start_offset + "$baz".len();
        let span = Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        );

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_counts_removed_backslash_pairs() {
        let source = r#"echo `sed -e "s/'/'\\\\\''/g" $2`"#;
        let span = span_for_text(source, "$2");

        let adjusted = shellcheck_collapsed_backtick_part_span_in_source(span, source);

        assert_eq!(adjusted.start.line, span.start.line);
        assert_eq!(adjusted.end.line, span.end.line);
        assert_eq!(adjusted.start.column, span.start.column - 2);
        assert_eq!(adjusted.end.column, span.end.column - 2);
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_counts_escaped_dollars() {
        let source = r#"echo `echo \$x $y`"#;
        let span = span_for_text(source, "$y");

        let adjusted = shellcheck_collapsed_backtick_part_span_in_source(span, source);

        assert_eq!(adjusted.start.column, span.start.column - 1);
        assert_eq!(adjusted.end.column, span.end.column - 1);
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_keeps_literal_backslashes() {
        let source = r#"echo `echo \a $x`"#;
        let span = span_for_text(source, "$x");

        assert_eq!(
            shellcheck_collapsed_backtick_part_span_in_source(span, source),
            span
        );
    }

    #[test]
    fn backtick_escaped_parameters_keep_quoted_assignment_prefixes_together() {
        let source = "`VAR=\"a b\" OTHER=$(printf '%s\\n' value) \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_append_assignment_prefixes() {
        let source = "`VAR+=x \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_redirection_prefixes() {
        let source = "`>/tmp/out 2>\"/tmp err\" FOO=bar \\$cmd arg`";
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_escaped_parameters(source, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    fn span_for_text(source: &str, text: &str) -> Span {
        let start_offset = source.find(text).expect("expected text");
        let end_offset = start_offset + text.len();
        Span::from_positions(
            position_at_offset(source, start_offset).expect("expected start position"),
            position_at_offset(source, end_offset).expect("expected end position"),
        )
    }

    #[test]
    fn backtick_double_escaped_parameter_spans_track_quoted_templates() {
        let source =
            r#"`echo "foreach dir {puts \\$dir} literal \\\\$literal"` `echo "plain $missing"`"#;
        let backtick_spans = backtick_substitution_spans(source);
        let escaped = backtick_double_escaped_parameter_spans(source, &backtick_spans);

        assert_eq!(
            escaped
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }
}
