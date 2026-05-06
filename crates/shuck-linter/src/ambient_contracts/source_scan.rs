//! Lightweight source text scans used before or alongside semantic facts.
//!
//! The providers use these helpers for cheap ecosystem signals such as
//! assignment-shaped text, direct parameter mentions, and simple function header
//! shapes. They are deliberately conservative heuristics; full shell structure
//! should come from parser, indexer, semantic, or linter facts instead.

pub(super) fn code_before_shell_comment(line: &str) -> &str {
    let mut previous_was_whitespace = true;
    for (index, ch) in line.char_indices() {
        if ch == '#' && previous_was_whitespace {
            return &line[..index];
        }
        previous_was_whitespace = ch.is_whitespace();
    }
    line
}

pub(super) fn parse_shell_name_at(source: &str, start: usize) -> Option<(&str, usize)> {
    let mut chars = source[start..].char_indices();
    let (_, first) = chars.next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }

    let mut end = start + first.len_utf8();
    for (offset, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end = start + offset + ch.len_utf8();
        } else {
            break;
        }
    }

    Some((&source[start..end], end))
}

pub(super) fn has_probable_function_definition(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(probable_function_definition)
}

pub(super) fn has_source_command(source: &str) -> bool {
    source.lines().map(str::trim).any(|trimmed| {
        trimmed.starts_with("source ")
            || trimmed.starts_with(". ")
            || trimmed.starts_with("\\source ")
            || trimmed.starts_with("\\. ")
    })
}

fn probable_function_definition(trimmed: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        return rest.contains('{');
    }

    trimmed.contains("() {") || trimmed.contains("(){")
}

pub(super) fn source_mentions_any(source: &str, names: &[&str]) -> bool {
    names.iter().any(|name| source_mentions_name(source, name))
}

pub(super) fn source_mentions_name(source: &str, name: &str) -> bool {
    source.contains(&format!("${name}"))
        || source.contains(&format!("${{{name}}}"))
        || source.contains(&format!("${{{name}["))
        || source.contains(&format!("${{{name}:"))
}

pub(super) fn source_assigns_name(source: &str, name: &str) -> bool {
    source.match_indices(name).any(|(offset, _)| {
        let before = source[..offset].chars().next_back();
        let after = source[offset + name.len()..].chars().next();
        let starts_token = before.is_none_or(|ch| !is_shell_name_char(ch));
        let assignment_like = matches!(after, Some('=') | Some('['))
            || (after == Some('+') && source[offset + name.len()..].chars().nth(1) == Some('='));
        starts_token && assignment_like
    })
}

fn is_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub(super) fn shell_assignment_token(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(super) fn is_shell_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
