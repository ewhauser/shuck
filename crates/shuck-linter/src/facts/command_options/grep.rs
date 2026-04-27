use super::*;

pub(super) fn parse_grep_command<'a>(
    args: &[&'a Word],
    source: &str,
) -> Option<GrepCommandFacts<'a>> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut saw_separate_option_argument = false;
    let mut uses_only_matching = false;
    let mut uses_fixed_strings = false;
    let mut explicit_pattern_source = false;
    let mut patterns = Vec::new();

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                saw_separate_option_argument = true;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                saw_separate_option_argument = true;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching" {
            uses_only_matching = true;
            index += 1;
            continue;
        }

        if text == "--fixed-strings" {
            uses_fixed_strings = true;
            index += 1;
            continue;
        }

        if matches!(
            text.as_ref(),
            "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
        ) {
            uses_fixed_strings = false;
            index += 1;
            continue;
        }

        if text == "--regexp" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::LongOptionSeparate,
                    patterns.is_empty(),
                    saw_separate_option_argument,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text.starts_with("--regexp=") {
            explicit_pattern_source = true;
            patterns.push(grep_prefixed_pattern_fact(
                word,
                source,
                "--regexp=".len(),
                GrepPatternSourceKind::LongOptionAttached,
                patterns.is_empty(),
                saw_separate_option_argument,
            ));
            index += 1;
            continue;
        }

        if text == "--file" {
            explicit_pattern_source = true;
            saw_separate_option_argument |= args.get(index + 1).is_some();
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            let consumes_next =
                grep_long_option_takes_argument(text.as_ref()) && args.get(index + 1).is_some();
            saw_separate_option_argument |= consumes_next;
            index += if consumes_next { 2 } else { 1 };
            continue;
        }

        if text == "-e" {
            explicit_pattern_source = true;
            if let Some(pattern_word) = args.get(index + 1) {
                patterns.push(grep_pattern_fact(
                    pattern_word,
                    source,
                    GrepPatternSourceKind::ShortOptionSeparate,
                    patterns.is_empty(),
                    saw_separate_option_argument,
                ));
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if text == "-f" {
            explicit_pattern_source = true;
            saw_separate_option_argument |= args.get(index + 1).is_some();
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'o' {
                uses_only_matching = true;
            }

            if flag == 'F' {
                uses_fixed_strings = true;
            }

            if matches!(flag, 'E' | 'G' | 'P') {
                uses_fixed_strings = false;
            }

            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_some() {
                    patterns.push(grep_prefixed_pattern_fact(
                        word,
                        source,
                        2,
                        GrepPatternSourceKind::ShortOptionAttached,
                        patterns.is_empty(),
                        saw_separate_option_argument,
                    ));
                } else if let Some(pattern_word) = args.get(index + 1) {
                    patterns.push(grep_pattern_fact(
                        pattern_word,
                        source,
                        GrepPatternSourceKind::ShortOptionSeparate,
                        patterns.is_empty(),
                        saw_separate_option_argument,
                    ));
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if flag == 'f' {
                    explicit_pattern_source = true;
                }
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            saw_separate_option_argument = true;
            index += 1;
        }
    }

    if !explicit_pattern_source && let Some(pattern_word) = args.get(index) {
        patterns.push(grep_pattern_fact(
            pattern_word,
            source,
            GrepPatternSourceKind::ImplicitOperand,
            patterns.is_empty(),
            saw_separate_option_argument,
        ));
    }

    Some(GrepCommandFacts {
        uses_only_matching,
        uses_fixed_strings,
        patterns: patterns.into_boxed_slice(),
    })
}

pub(super) fn grep_file_operand_words<'a>(args: &[&'a Word], source: &str) -> Vec<&'a Word> {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;
    let mut explicit_pattern_source = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;

        if text == "--only-matching"
            || text == "--fixed-strings"
            || matches!(
                text.as_ref(),
                "--basic-regexp" | "--extended-regexp" | "--perl-regexp"
            )
        {
            index += 1;
            continue;
        }

        if text == "--regexp" || text == "--file" {
            explicit_pattern_source = true;
            index += if args.get(index + 1).is_some() { 2 } else { 1 };
            continue;
        }

        if text.starts_with("--regexp=") || text.starts_with("--file=") {
            explicit_pattern_source = true;
            index += 1;
            continue;
        }

        if text.starts_with("--") {
            index += if grep_long_option_takes_argument(text.as_ref())
                && args.get(index + 1).is_some()
            {
                2
            } else {
                1
            };
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == 'e' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if flag == 'f' {
                explicit_pattern_source = true;
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }

            if grep_option_takes_argument(flag) {
                if chars.peek().is_none() {
                    consume_next_argument = true;
                }
                break;
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    if !explicit_pattern_source && args.get(index).is_some() {
        index += 1;
    }

    args.get(index..).unwrap_or(&[]).to_vec()
}

fn grep_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    source_kind: GrepPatternSourceKind,
    is_first_pattern: bool,
    follows_separate_option_argument: bool,
) -> GrepPatternFact<'a> {
    grep_prefixed_pattern_fact(
        word,
        source,
        0,
        source_kind,
        is_first_pattern,
        follows_separate_option_argument,
    )
}

fn grep_prefixed_pattern_fact<'a>(
    word: &'a Word,
    source: &str,
    prefix_len: usize,
    source_kind: GrepPatternSourceKind,
    is_first_pattern: bool,
    follows_separate_option_argument: bool,
) -> GrepPatternFact<'a> {
    let (static_text, glob_style_star_replacement_spans) =
        cooked_static_word_text_with_source_spans(word, source)
            .and_then(|(text, source_spans)| {
                let text = text.get(prefix_len..)?.to_owned();
                let source_spans = source_spans.get(prefix_len..)?.to_vec();
                Some((text, source_spans))
            })
            .map(|(text, source_spans)| {
                let spans = grep_pattern_glob_style_star_replacement_spans(&text, &source_spans);
                (Some(text.into_boxed_str()), spans.into_boxed_slice())
            })
            .unwrap_or_else(|| (None, Box::new([])));
    let starts_with_glob_style_star = static_text
        .as_deref()
        .is_some_and(|text| text.starts_with('*') || text == "^*");
    let has_glob_style_star_confusion = !glob_style_star_replacement_spans.is_empty();

    GrepPatternFact {
        word,
        static_text,
        source_kind,
        is_first_pattern,
        follows_separate_option_argument,
        starts_with_glob_style_star,
        has_glob_style_star_confusion,
        glob_style_star_replacement_spans,
    }
}

fn cooked_static_word_text_with_source_spans(
    word: &Word,
    source: &str,
) -> Option<(String, Vec<Span>)> {
    let mut cooked = Vec::new();
    let mut source_spans = Vec::new();
    collect_cooked_static_word_text_parts_with_source_spans(
        &word.parts,
        source,
        false,
        &mut cooked,
        &mut source_spans,
    )
    .then_some(())?;

    Some((String::from_utf8(cooked).ok()?, source_spans))
}

fn collect_cooked_static_word_text_parts_with_source_spans(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let slice = text.as_str(source, part.span);
                if in_double_quotes {
                    push_cooked_double_quoted_literal_text_with_source_spans(
                        slice,
                        part.span.start,
                        out,
                        source_spans,
                    );
                } else {
                    push_cooked_unquoted_literal_text_with_source_spans(
                        slice,
                        part.span.start,
                        out,
                        source_spans,
                    );
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                push_cooked_literal_text_with_source_spans(
                    text,
                    value.span().start,
                    out,
                    source_spans,
                );
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_cooked_static_word_text_parts_with_source_spans(
                    parts,
                    source,
                    true,
                    out,
                    source_spans,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ZshQualifiedGlob(_) => return false,
        }
    }

    true
}

fn push_cooked_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    for (index, ch) in text.char_indices() {
        let span = Span::from_positions(
            start_position.advanced_by(&text[..index]),
            start_position.advanced_by(&text[..index + ch.len_utf8()]),
        );
        push_cooked_char_with_source_span(ch, span, out, source_spans);
    }
}

fn push_cooked_unquoted_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\\' {
            if let Some((next_index, escaped)) = chars.next()
                && escaped != '\n'
            {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..next_index + escaped.len_utf8()]),
                );
                push_cooked_char_with_source_span(escaped, span, out, source_spans);
            }
            continue;
        }

        let span = Span::from_positions(
            start_position.advanced_by(&text[..index]),
            start_position.advanced_by(&text[..index + ch.len_utf8()]),
        );
        push_cooked_char_with_source_span(ch, span, out, source_spans);
    }
}

fn push_cooked_double_quoted_literal_text_with_source_spans(
    text: &str,
    start_position: Position,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch != '\\' {
            let span = Span::from_positions(
                start_position.advanced_by(&text[..index]),
                start_position.advanced_by(&text[..index + ch.len_utf8()]),
            );
            push_cooked_char_with_source_span(ch, span, out, source_spans);
            continue;
        }

        match chars.next() {
            Some((next_index, escaped @ ('$' | '"' | '\\' | '`'))) => {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..next_index + escaped.len_utf8()]),
                );
                push_cooked_char_with_source_span(escaped, span, out, source_spans);
            }
            Some((_next_index, '\n')) => {}
            Some((next_index, other)) => {
                let backslash_span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..index + ch.len_utf8()]),
                );
                push_cooked_char_with_source_span('\\', backslash_span, out, source_spans);

                let span = Span::from_positions(
                    start_position.advanced_by(&text[..next_index]),
                    start_position.advanced_by(&text[..next_index + other.len_utf8()]),
                );
                push_cooked_char_with_source_span(other, span, out, source_spans);
            }
            None => {
                let span = Span::from_positions(
                    start_position.advanced_by(&text[..index]),
                    start_position.advanced_by(&text[..index + ch.len_utf8()]),
                );
                push_cooked_char_with_source_span('\\', span, out, source_spans);
            }
        }
    }
}

fn push_cooked_char_with_source_span(
    ch: char,
    source_span: Span,
    out: &mut Vec<u8>,
    source_spans: &mut Vec<Span>,
) {
    let mut buf = [0u8; 4];
    let encoded = ch.encode_utf8(&mut buf).as_bytes();
    out.extend_from_slice(encoded);
    source_spans.extend(std::iter::repeat_n(source_span, encoded.len()));
}

fn grep_pattern_glob_style_star_replacement_spans(text: &str, source_spans: &[Span]) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();

    if bytes.is_empty() {
        return spans;
    }

    if text.starts_with('^')
        || ends_with_unescaped_dollar(bytes)
        || bytes.contains(&b'[')
        || bytes.contains(&b'+')
    {
        return spans;
    }
    if first_unescaped_star_index(bytes).is_some_and(|index| index == 0) {
        return spans;
    }

    let mut index = 0usize;
    while let Some(star_index) = next_unescaped_star_index(bytes, index) {
        if bytes.get(star_index + 1) == Some(&b'\\') {
            index = star_index + 1;
            continue;
        }

        let Some(previous) = previous_unescaped_byte(bytes, star_index) else {
            index = star_index + 1;
            continue;
        };

        if matches!(
            previous,
            b'.' | b']' | b')' | b'*' | b'?' | b'|' | b'$' | b'{' | b'(' | b'\\'
        ) || previous.is_ascii_whitespace()
        {
            index = star_index + 1;
            continue;
        }

        if let Some(span) = source_spans.get(star_index).copied() {
            spans.push(span);
        }
        index = star_index + 1;
    }

    spans
}

fn first_unescaped_star_index(bytes: &[u8]) -> Option<usize> {
    next_unescaped_star_index(bytes, 0)
}

fn next_unescaped_star_index(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'*' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn previous_unescaped_byte(bytes: &[u8], index: usize) -> Option<u8> {
    let mut candidate = index;
    while candidate > 0 {
        candidate -= 1;
        if !is_escaped(bytes, candidate) {
            return Some(bytes[candidate]);
        }
    }
    None
}

fn ends_with_unescaped_dollar(bytes: &[u8]) -> bool {
    bytes
        .last()
        .is_some_and(|byte| *byte == b'$' && !is_escaped(bytes, bytes.len() - 1))
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}
fn grep_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'A' | 'B' | 'C' | 'D' | 'd' | 'e' | 'f' | 'm')
}

fn grep_long_option_takes_argument(option: &str) -> bool {
    let Some(name) = option.strip_prefix("--") else {
        return false;
    };
    if name.contains('=') {
        return false;
    }

    matches!(
        name,
        "after-context"
            | "before-context"
            | "binary-files"
            | "context"
            | "devices"
            | "directories"
            | "exclude"
            | "exclude-dir"
            | "exclude-from"
            | "file"
            | "group-separator"
            | "include"
            | "label"
            | "max-count"
            | "regexp"
    )
}
