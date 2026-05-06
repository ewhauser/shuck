//! Lightweight source text scans used before or alongside semantic facts.
//!
//! The providers use these helpers for cheap ecosystem signals such as
//! assignment-shaped text, direct parameter mentions, and simple function header
//! shapes. They are deliberately conservative heuristics; full shell structure
//! should come from parser, indexer, semantic, or linter facts instead.

pub(super) fn code_before_shell_comment(line: &str) -> &str {
    let mut previous = None;
    for (index, ch) in line.char_indices() {
        if ch == '#' && shell_comment_can_start_after(previous) {
            return &line[..index];
        }
        previous = Some(ch);
    }
    line
}

fn shell_comment_can_start_after(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>'))
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
