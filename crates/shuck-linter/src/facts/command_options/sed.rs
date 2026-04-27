use super::*;

pub(super) fn parse_sed_command(args: &[&Word], source: &str) -> SedCommandFacts {
    SedCommandFacts {
        has_single_substitution_script: sed_has_single_substitution_script(
            args,
            source,
            SedScriptQuoteMode::ShellOnly,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SedScriptQuoteMode {
    ShellOnly,
    AllowBacktickEscapedDoubleQuotes,
}

pub(crate) fn sed_script_text<'a>(
    args: &[&Word],
    source: &'a str,
    quote_mode: SedScriptQuoteMode,
) -> Option<Cow<'a, str>> {
    match args {
        [script] => Some(Cow::Borrowed(strip_matching_sed_script_quotes_in_source(
            script.span.slice(source),
            quote_mode,
        ))),
        [first, .., last]
            if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
                && first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"") =>
        {
            let mut text = String::new();
            for (index, word) in args.iter().enumerate() {
                if index != 0 {
                    text.push(' ');
                }
                text.push_str(word.span.slice(source));
            }
            Some(Cow::Owned(
                strip_backtick_escaped_double_quotes_in_source(&text).to_owned(),
            ))
        }
        _ => None,
    }
}

pub(crate) fn sed_has_single_substitution_script(
    args: &[&Word],
    source: &str,
    quote_mode: SedScriptQuoteMode,
) -> bool {
    sed_script_text(args, source, quote_mode)
        .or_else(|| match args {
            [flag, words @ ..] if static_word_text(flag, source).as_deref() == Some("-e") => {
                sed_script_text(words, source, quote_mode)
            }
            _ => None,
        })
        .as_deref()
        .is_some_and(is_simple_sed_substitution_script)
}

pub(super) fn is_echo_portability_flag(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };

    !flags.is_empty()
        && flags
            .bytes()
            .all(|byte| matches!(byte, b'n' | b'e' | b'E' | b's'))
}

fn is_simple_sed_substitution_script(text: &str) -> bool {
    let Some(remainder) = text.strip_prefix('s') else {
        return false;
    };

    let Some(delimiter) = remainder.chars().next() else {
        return false;
    };
    if delimiter.is_whitespace() || delimiter == '\\' {
        return false;
    }

    let pattern_start = 1 + delimiter.len_utf8();
    let Some((pattern_end, pattern_has_escaped_delimiter)) =
        find_sed_substitution_section(text, pattern_start, delimiter)
    else {
        return false;
    };
    let replacement_start = pattern_end + delimiter.len_utf8();
    let Some((replacement_end, replacement_has_escaped_delimiter)) =
        find_sed_substitution_section(text, replacement_start, delimiter)
    else {
        return false;
    };

    let flags = &text[replacement_end + delimiter.len_utf8()..];
    if flags.chars().any(|ch| ch.is_whitespace() || ch == ';') {
        return false;
    }

    let pattern = &text[pattern_start..pattern_end];
    let replacement = &text[replacement_start..replacement_end];
    !pattern_has_escaped_delimiter
        && !replacement_has_escaped_delimiter
        && !uses_delimiter_sensitive_match_escape(pattern, replacement, delimiter)
}

pub(crate) fn find_sed_substitution_section(
    text: &str,
    start: usize,
    delimiter: char,
) -> Option<(usize, bool)> {
    let _ = text.get(start..)?;
    let mut index = start;
    let mut saw_escaped_delimiter = false;
    let mut escaped = false;
    let mut character_class_contents = None;

    while index < text.len() {
        let mut chars = text[index..].chars();
        let Some(ch) = chars.next() else {
            break;
        };
        let next = index + ch.len_utf8();

        if let Some(contents) = character_class_contents.as_mut() {
            if escaped {
                if ch == delimiter {
                    saw_escaped_delimiter = true;
                }
                *contents += 1;
                escaped = false;
            } else {
                match ch {
                    '\\' => {
                        escaped = true;
                    }
                    '^' if *contents == 0 => {}
                    ']' if *contents > 0 => {
                        character_class_contents = None;
                    }
                    _ => {
                        *contents += 1;
                    }
                }
            }
            index = next;
            continue;
        }

        if escaped {
            if ch == delimiter {
                saw_escaped_delimiter = true;
            }
            escaped = false;
            index = next;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
            }
            '[' => {
                character_class_contents = Some(0);
            }
            ch if ch == delimiter => return Some((index, saw_escaped_delimiter)),
            _ => {}
        }
        index = next;
    }

    None
}

fn uses_delimiter_sensitive_match_escape(
    pattern: &str,
    replacement: &str,
    delimiter: char,
) -> bool {
    delimiter == '/'
        && pattern.contains(delimiter)
        && is_backslash_prefixed_match_escape(replacement)
}

fn is_backslash_prefixed_match_escape(replacement: &str) -> bool {
    replacement == r"\\&"
        || replacement.strip_prefix(r"\\").is_some_and(|rest| {
            matches!(rest.as_bytes(), [b'\\', b'1'..=b'9', ..])
                && rest[1..].bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn strip_matching_sed_script_quotes_in_source(text: &str, quote_mode: SedScriptQuoteMode) -> &str {
    if quote_mode == SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes
        && text.len() >= 4
        && text.starts_with("\\\"")
        && text.ends_with("\\\"")
    {
        strip_backtick_escaped_double_quotes_in_source(text)
    } else {
        strip_shell_matching_quotes_in_source(text)
    }
}

pub(super) fn strip_shell_matching_quotes_in_source(text: &str) -> &str {
    if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn strip_backtick_escaped_double_quotes_in_source(text: &str) -> &str {
    debug_assert!(text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\""));
    &text[2..text.len() - 2]
}
