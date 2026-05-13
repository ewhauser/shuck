use shuck_semantic::{SourceRef, SourceRefKind};

pub(crate) fn source_ref_has_unanchored_path(
    source_ref: &SourceRef,
    source: &str,
    allowed_anchors: &[String],
) -> bool {
    let raw_path = source_ref.path_span.slice(source).trim();
    let path = strip_outer_quotes(raw_path).trim();
    let outer_quote = outer_quote(raw_path);
    if path.is_empty()
        || (outer_quote != Some('\'') && starts_with_allowed_anchor(path, allowed_anchors))
        || starts_with_absolute_anchor(path)
        || starts_with_unquoted_home_anchor(raw_path, path)
        || (outer_quote != Some('\'') && is_entire_command_substitution(path))
    {
        return false;
    }

    match &source_ref.kind {
        SourceRefKind::Literal(path) => !path.is_empty(),
        SourceRefKind::Directive(_) => {
            if outer_quote == Some('\'') {
                !path.is_empty()
            } else {
                directive_runtime_path_can_be_unanchored(path)
            }
        }
        SourceRefKind::Dynamic | SourceRefKind::SingleVariableStaticTail { .. } => {
            path_has_static_path_component(path)
        }
        SourceRefKind::DirectiveDevNull => {
            if outer_quote == Some('\'') {
                !path.is_empty()
            } else {
                directive_runtime_path_can_be_unanchored(path)
            }
        }
    }
}

fn starts_with_allowed_anchor(path: &str, allowed_anchors: &[String]) -> bool {
    allowed_anchors
        .iter()
        .filter(|anchor| !anchor.is_empty())
        .any(|anchor| {
            starts_with_anchor_boundary(path, anchor)
                || starts_with_split_quoted_anchor(path, anchor)
        })
}

fn starts_with_anchor_boundary(path: &str, anchor: &str) -> bool {
    path.strip_prefix(anchor)
        .is_some_and(anchor_suffix_is_boundary)
}

fn starts_with_split_quoted_anchor(path: &str, anchor: &str) -> bool {
    let Some(rest) = path.strip_prefix('"') else {
        return false;
    };
    let Some(after_anchor) = rest.strip_prefix(anchor) else {
        return false;
    };

    let Some(after_quote) = after_anchor.strip_prefix('"') else {
        return false;
    };

    anchor_suffix_is_boundary(after_quote)
}

fn anchor_suffix_is_boundary(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix.starts_with('/')
        || suffix
            .strip_prefix(['"', '\''])
            .is_some_and(|rest| rest.starts_with('/'))
}

fn starts_with_absolute_anchor(path: &str) -> bool {
    path.starts_with('/')
}

fn starts_with_unquoted_home_anchor(raw_path: &str, path: &str) -> bool {
    raw_path == path && unquoted_home_anchor_len(path).is_some()
}

fn unquoted_home_anchor_len(path: &str) -> Option<usize> {
    let suffix = path.strip_prefix('~')?;
    if suffix.is_empty() || suffix.starts_with('/') {
        return Some(1);
    }

    let user_end = suffix.find('/').unwrap_or(suffix.len());
    let user = &suffix[..user_end];
    if user.is_empty()
        || user.starts_with(['+', '-'])
        || user.as_bytes().first().is_some_and(u8::is_ascii_digit)
        || !user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }

    Some(1 + user_end)
}

fn path_has_static_path_component(path: &str) -> bool {
    path.starts_with("./") || path.starts_with("../") || path.contains('/')
}

fn directive_runtime_path_can_be_unanchored(path: &str) -> bool {
    !path_has_dynamic_expansion(path) || path_has_static_path_component(path)
}

fn path_has_dynamic_expansion(path: &str) -> bool {
    path.contains('$') || path.contains('`')
}

fn strip_outer_quotes(text: &str) -> &str {
    if outer_quote(text).is_some() {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn outer_quote(text: &str) -> Option<char> {
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        Some(bytes[0] as char)
    } else {
        None
    }
}

fn is_entire_command_substitution(path: &str) -> bool {
    is_entire_backtick_substitution(path) || is_entire_dollar_paren_substitution(path)
}

fn is_entire_backtick_substitution(path: &str) -> bool {
    path.len() >= 2
        && path.starts_with('`')
        && path.ends_with('`')
        && !path[1..path.len() - 1].contains('`')
}

fn is_entire_dollar_paren_substitution(path: &str) -> bool {
    if !path.starts_with("$(") {
        return false;
    }

    let mut depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;
    let mut chars = path.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            continue;
        }

        if in_double_quote {
            match ch {
                '\\' => escaped = true,
                '"' => in_double_quote = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '\'' if depth > 0 => in_single_quote = true,
            '"' if depth > 0 => in_double_quote = true,
            '\\' if depth > 0 => escaped = true,
            '$' if chars.peek().is_some_and(|(_, next)| *next == '(') => {
                let _ = chars.next();
                depth += 1;
            }
            '(' if depth > 0 => depth += 1,
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return index + ch.len_utf8() == path.len();
                }
            }
            _ => {}
        }
    }

    false
}
