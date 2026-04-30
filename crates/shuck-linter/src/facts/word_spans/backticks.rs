use super::*;

pub(crate) fn shellcheck_collapsed_backtick_part_span(
    span: Span,
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Span {
    let deescaped =
        shellcheck_deescaped_backtick_part_span(span, locator, backtick_spans).unwrap_or(span);
    collapse_backtick_continuation_span(deescaped, locator, backtick_spans).unwrap_or(deescaped)
}

pub(crate) fn collapse_backtick_continuation_span(
    span: Span,
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Option<Span> {
    let source = locator.source();
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let chain_start = continued_line_chain_start(span.start, containing_span, locator)?;
    Some(Span::from_positions(
        shellcheck_collapsed_position(chain_start, span.start, source),
        shellcheck_collapsed_position(chain_start, span.end, source),
    ))
}

pub(crate) fn shellcheck_deescaped_backtick_part_span(
    span: Span,
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Option<Span> {
    let source = locator.source();
    let containing_span = containing_backtick_substitution_span(span, backtick_spans)?;
    let content_start = containing_span.start.offset.saturating_add('`'.len_utf8());
    let start_removed = backtick_removed_escape_count(source, content_start, span.start.offset)?;
    let end_removed = backtick_removed_escape_count(source, content_start, span.end.offset)?;
    if start_removed == 0 && end_removed == 0 {
        return None;
    }

    Some(Span::from_positions(
        locator.position_at_offset(span.start.offset.checked_sub(start_removed)?)?,
        locator.position_at_offset(span.end.offset.checked_sub(end_removed)?)?,
    ))
}

pub(crate) fn backtick_removed_escape_count(
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

pub(crate) fn containing_backtick_substitution_span(
    target: Span,
    backtick_spans: &[Span],
) -> Option<Span> {
    backtick_spans
        .iter()
        .copied()
        .find(|span| span_contains(*span, target))
}

#[derive(Clone, Copy, Default)]
pub(crate) struct BacktickQuoteContext {
    in_single_quote: bool,
    in_double_quote: bool,
    in_comment: bool,
    previous_char: Option<char>,
}

pub(crate) fn backtick_shell_comment_can_start(previous_char: Option<char>) -> bool {
    previous_char.is_none_or(|ch| {
        ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
    })
}

pub(crate) fn backtick_substitution_spans(locator: Locator<'_>) -> Vec<Span> {
    let source = locator.source();
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
                    let Some(start) = locator.position_at_offset(start_offset) else {
                        index += ch_len;
                        continue;
                    };
                    let Some(end) = locator.position_at_offset(index + ch_len) else {
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
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Vec<BacktickEscapedParameter> {
    let source = locator.source();
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
                            locator.position_at_offset(diagnostic_start_offset)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(diagnostic_end) =
                            locator.position_at_offset(diagnostic_start_offset + expansion_len)
                        else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_start) = locator.position_at_offset(index) else {
                            index += escaped.len_utf8();
                            continue;
                        };
                        let Some(reference_end) = locator.position_at_offset(index + expansion_len)
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
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Vec<Span> {
    let source = locator.source();
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
                    && let Some(start) = locator.position_at_offset(base_offset + dollar_offset)
                    && let Some(end_position) = locator.position_at_offset(base_offset + end_offset)
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
    locator: Locator<'_>,
    backtick_spans: &[Span],
) -> Vec<Span> {
    let source = locator.source();
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
                        if let Some(start) = locator.position_at_offset(index)
                            && let Some(end_position) =
                                locator.position_at_offset(index + expansion_len)
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

pub(crate) fn escaped_backtick_parameter_is_standalone_command_name(
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

pub(crate) fn backtick_command_segment_start(source: &str, start: usize, end: usize) -> usize {
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

pub(crate) enum EscapedBacktickParameterSyntax {
    Simple {
        name: shuck_ast::Name,
        expansion_len: usize,
    },
    ComplexUnsafe {
        expansion_len: usize,
    },
}

impl EscapedBacktickParameterSyntax {
    fn name(&self) -> Option<&shuck_ast::Name> {
        match self {
            Self::Simple { name, .. } => Some(name),
            Self::ComplexUnsafe { .. } => None,
        }
    }

    fn expansion_len(&self) -> usize {
        match self {
            Self::Simple { expansion_len, .. } | Self::ComplexUnsafe { expansion_len } => {
                *expansion_len
            }
        }
    }
}

pub(crate) fn escaped_backtick_parameter_syntax(
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

pub(crate) fn continued_line_chain_start(
    target: Position,
    containing_span: Span,
    locator: Locator<'_>,
) -> Option<Position> {
    let source = locator.source();
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
        .then(|| locator.position_at_offset(chain_start))
        .flatten()
}

pub(crate) fn shellcheck_collapsed_position(
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

#[cfg(test)]
mod tests {
    use shuck_ast::Span;
    use shuck_indexer::LineIndex;

    use super::{
        backtick_double_escaped_parameter_spans, backtick_escaped_parameters,
        backtick_substitution_spans, shellcheck_collapsed_backtick_part_span,
    };
    use crate::Locator;

    fn shellcheck_collapsed_backtick_part_span_in_source(span: Span, source: &str) -> Span {
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let backtick_spans = backtick_substitution_spans(locator);
        shellcheck_collapsed_backtick_part_span(span, locator, &backtick_spans)
    }

    fn span_at(source: &str, start_offset: usize, end_offset: usize) -> Span {
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        Span::from_positions(
            locator
                .position_at_offset(start_offset)
                .expect("expected start position"),
            locator
                .position_at_offset(end_offset)
                .expect("expected end position"),
        )
    }

    #[test]
    fn shellcheck_collapsed_backtick_part_span_in_source_ignores_single_quoted_backticks() {
        let source = "printf '%s\\n' '`'\\\n  \"$foo\"\\\n  '`'\n";
        let start_offset = source.find("$foo").expect("expected expansion");
        let end_offset = start_offset + "$foo".len();
        let span = span_at(source, start_offset, end_offset);

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
        let span = span_at(source, start_offset, end_offset);

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
        let span = span_at(source, start_offset, end_offset);

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
        let span = span_at(source, start_offset, end_offset);

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
        let span = span_at(source, start_offset, end_offset);

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
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let backtick_spans = backtick_substitution_spans(locator);
        let escaped = backtick_escaped_parameters(locator, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_append_assignment_prefixes() {
        let source = "`VAR+=x \\$cmd arg`";
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let backtick_spans = backtick_substitution_spans(locator);
        let escaped = backtick_escaped_parameters(locator, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    #[test]
    fn backtick_escaped_parameters_accept_redirection_prefixes() {
        let source = "`>/tmp/out 2>\"/tmp err\" FOO=bar \\$cmd arg`";
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let backtick_spans = backtick_substitution_spans(locator);
        let escaped = backtick_escaped_parameters(locator, &backtick_spans);

        assert_eq!(escaped.len(), 1);
        assert!(escaped[0].standalone_command_name);
    }

    fn span_for_text(source: &str, text: &str) -> Span {
        let start_offset = source.find(text).expect("expected text");
        let end_offset = start_offset + text.len();
        span_at(source, start_offset, end_offset)
    }

    #[test]
    fn backtick_double_escaped_parameter_spans_track_quoted_templates() {
        let source =
            r#"`echo "foreach dir {puts \\$dir} literal \\\\$literal"` `echo "plain $missing"`"#;
        let line_index = LineIndex::new(source);
        let locator = Locator::new(source, &line_index);
        let backtick_spans = backtick_substitution_spans(locator);
        let escaped = backtick_double_escaped_parameter_spans(locator, &backtick_spans);

        assert_eq!(
            escaped
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }
}
