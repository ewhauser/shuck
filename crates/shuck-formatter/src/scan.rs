pub(crate) fn loop_open_keyword_at(source: &str, offset: usize, upper: usize) -> bool {
    ["for", "select", "while", "until", "foreach", "repeat"]
        .iter()
        .any(|keyword| shell_keyword_at(source, offset, upper, keyword))
}

pub(crate) fn shell_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    let end = offset.saturating_add(keyword.len());
    end <= upper
        && source.get(offset..end) == Some(keyword)
        && shell_keyword_boundaries_match(source, offset, end)
}

pub(crate) fn shell_keyword_boundaries_match(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(|ch| !is_shell_keyword_char(ch))
        && after.is_none_or(|ch| !is_shell_keyword_char(ch))
}

fn is_shell_keyword_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

pub(crate) fn shell_comment_can_start(source: &str, offset: usize) -> bool {
    source[..offset]
        .chars()
        .next_back()
        .is_none_or(|ch| ch == '\n' || ch.is_whitespace() || matches!(ch, ';' | '&' | '|'))
}

pub(crate) fn skip_single_quoted(source: &str, mut offset: usize, upper: usize) -> usize {
    while offset < upper {
        let Some(ch) = source[offset..].chars().next() else {
            break;
        };
        offset += ch.len_utf8();
        if ch == '\'' {
            break;
        }
    }
    offset
}

pub(crate) fn skip_double_quoted(source: &str, mut offset: usize, upper: usize) -> usize {
    while offset < upper {
        let Some(ch) = source[offset..].chars().next() else {
            break;
        };
        offset += ch.len_utf8();
        if ch == '\\' {
            if let Some(escaped) = source[offset..].chars().next() {
                offset += escaped.len_utf8();
            }
        } else if ch == '"' {
            break;
        }
    }
    offset
}
