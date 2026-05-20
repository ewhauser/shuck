use shuck_ast::Span;

use crate::comments::SourceMap;

pub(crate) fn leading_shell_indent(line: &str) -> &str {
    let indent_end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
        .map_or(line.len(), |(index, _)| index);
    &line[..indent_end]
}

pub(crate) fn common_indent_prefix<'a>(left: &'a str, right: &str) -> &'a str {
    let len = left
        .as_bytes()
        .iter()
        .zip(right.as_bytes())
        .take_while(|(left, right)| left == right)
        .count();
    &left[..len]
}

pub(crate) fn refine_common_indent(common: &mut Option<String>, indent: &str) -> bool {
    *common = Some(match common.take() {
        Some(previous) => common_indent_prefix(&previous, indent).to_string(),
        None => indent.to_string(),
    });
    common.as_deref() == Some("")
}

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

pub(crate) fn normalized_close_keyword_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> Span {
    let start = span.start.offset.min(source.len());
    let end = start.saturating_add(keyword.len()).min(source.len());
    if source.get(start..end) == Some(keyword) {
        source_map.span_for_offsets(start, end)
    } else {
        span
    }
}

pub(crate) fn matching_if_close_start(source: &str, span: Span) -> Option<usize> {
    let upper = span.end.offset.min(source.len());
    let mut offset = span.start.offset.min(upper);
    let mut depth = 0usize;
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        match ch {
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => {
                offset = source[offset..]
                    .find('\n')
                    .map_or(upper, |newline| offset + newline + 1);
                continue;
            }
            _ => {}
        }

        if shell_keyword_at(source, offset, upper, "if") {
            depth = depth.saturating_add(1);
            offset += "if".len();
            continue;
        }
        if shell_keyword_at(source, offset, upper, "fi") {
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            offset += "fi".len();
            continue;
        }
        offset += ch.len_utf8();
    }
    None
}

pub(crate) fn matching_done_close_start(source: &str, span: Span) -> Option<usize> {
    let upper = span.end.offset.min(source.len());
    let mut offset = span.start.offset.min(upper);
    let mut depth = 0usize;
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        match ch {
            '\'' => {
                offset = skip_single_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '"' => {
                offset = skip_double_quoted(source, offset + ch.len_utf8(), upper);
                continue;
            }
            '#' if shell_comment_can_start(source, offset) => {
                offset = source[offset..]
                    .find('\n')
                    .map_or(upper, |newline| offset + newline + 1);
                continue;
            }
            _ => {}
        }

        if loop_open_keyword_at(source, offset, upper) {
            depth = depth.saturating_add(1);
            offset += source[offset..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphabetic())
                .map(char::len_utf8)
                .sum::<usize>();
            continue;
        }
        if shell_keyword_at(source, offset, upper, "done") {
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            offset += "done".len();
            continue;
        }
        offset += ch.len_utf8();
    }
    None
}
