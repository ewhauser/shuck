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

pub(crate) fn operator_starts_or_ends_line(source: &str, operator_span: Span) -> bool {
    let start = operator_span.start.offset;
    let end = operator_span.end.offset;
    if start >= end || end > source.len() {
        return false;
    }

    let line_start = source[..start]
        .rfind('\n')
        .map_or(0, |offset| offset.saturating_add(1));
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |offset| end.saturating_add(offset));
    let has_previous_line = line_start > 0;
    let has_next_line = line_end < source.len();
    let before = &source[line_start..start];
    let after = &source[end..line_end];

    (has_previous_line && line_edge_is_blank_or_continuation(before))
        || (has_next_line && line_edge_is_blank_or_continuation(after))
}

fn line_edge_is_blank_or_continuation(text: &str) -> bool {
    let trimmed = text.trim_matches(|ch| matches!(ch, ' ' | '\t' | '\r'));
    trimmed.is_empty() || trimmed == "\\"
}

pub(crate) fn line_has_shell_comment_before(source: &str, offset: usize) -> bool {
    let upper = offset.min(source.len());
    let line_start = source[..upper]
        .rfind('\n')
        .map_or(0, |newline| newline.saturating_add(1));
    let mut cursor = line_start;
    while cursor < upper {
        let Some(ch) = source[cursor..].chars().next() else {
            break;
        };
        match ch {
            '\'' => {
                cursor = skip_single_quoted(source, cursor + ch.len_utf8(), upper);
            }
            '"' => {
                cursor = skip_double_quoted(source, cursor + ch.len_utf8(), upper);
            }
            '#' if shell_comment_can_start(source, cursor) => return true,
            _ => cursor += ch.len_utf8(),
        }
    }
    false
}

pub(crate) fn branch_keyword_offset(
    source: &str,
    start: usize,
    end: usize,
    keyword: &str,
) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let mut line_start = start;
    while line_start < end {
        let line_end = source[line_start..end]
            .find('\n')
            .map_or(end, |offset| line_start + offset);
        let line = source.get(line_start..line_end)?;
        let mut search_start = 0;
        while let Some(relative) = line[search_start..].find(keyword) {
            let keyword_start = search_start + relative;
            let keyword_end = keyword_start + keyword.len();
            if branch_keyword_candidate_matches(line, keyword_start, keyword_end) {
                return Some(line_start + keyword_start);
            }
            search_start = keyword_end;
        }
        line_start = line_end.saturating_add(1);
    }
    None
}

fn branch_keyword_candidate_matches(line: &str, start: usize, end: usize) -> bool {
    if !shell_keyword_boundaries_match(line, start, end) {
        return false;
    }

    let prefix = &line[..start];
    let trimmed = prefix.trim_start_matches([' ', '\t']);
    if trimmed.starts_with('#') {
        return false;
    }

    let before = prefix.trim_end_matches([' ', '\t']);
    before.is_empty() || before.ends_with(';') || before.ends_with('&')
}

#[derive(Debug, Clone)]
pub(crate) struct BranchPrefixComment {
    pub(crate) offset: usize,
    pub(crate) text: String,
    pub(crate) source_indent: usize,
}

pub(crate) fn branch_prefix_first_comment_offset(
    source: &str,
    start: usize,
    end: usize,
) -> Option<usize> {
    branch_prefix_comments(source, start, end)
        .first()
        .map(|comment| comment.offset)
}

pub(crate) fn branch_prefix_comments(
    source: &str,
    start: usize,
    end: usize,
) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let Some(slice) = source.get(start..end) else {
        return Vec::new();
    };
    let keyword_indent = line_indent_before_offset(source, end).unwrap_or("");

    let mut comments = Vec::new();
    let mut in_branch_prefix_run = false;
    let mut offset = start;
    for line in slice.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        if trimmed.starts_with('#')
            && (in_branch_prefix_run || text.get(..indent) == Some(keyword_indent))
        {
            comments.push(BranchPrefixComment {
                offset: offset + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
                source_indent: indent,
            });
            in_branch_prefix_run = true;
        } else if !trimmed.is_empty() {
            in_branch_prefix_run = false;
        }
        offset += line.len();
    }
    comments
}

pub(crate) fn own_line_comments_in_region(
    source: &str,
    start: usize,
    end: usize,
) -> Vec<BranchPrefixComment> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    let Some(next_line_start) = source
        .get(start..end)
        .and_then(|slice| slice.find('\n').map(|offset| start + offset + 1))
    else {
        return Vec::new();
    };
    let Some(slice) = source.get(next_line_start..end) else {
        return Vec::new();
    };

    let mut comments = Vec::new();
    let mut offset = next_line_start;
    for line in slice.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start_matches([' ', '\t']);
        let indent = text.len().saturating_sub(trimmed.len());
        if trimmed.starts_with('#') {
            comments.push(BranchPrefixComment {
                offset: offset + indent,
                text: trimmed.trim_end_matches([' ', '\t', '\r']).to_string(),
                source_indent: indent,
            });
        }
        offset += line.len();
    }
    comments
}

pub(crate) fn last_uncommented_shell_keyword_before(
    source: &str,
    search_end: usize,
    keyword: &str,
) -> Option<usize> {
    let mut search_end = search_end.min(source.len());
    loop {
        let offset = source[..search_end].rfind(keyword)?;
        let end = offset + keyword.len();
        if shell_keyword_boundaries_match(source, offset, end)
            && !line_has_shell_comment_before(source, offset)
        {
            return Some(offset);
        }
        search_end = offset;
    }
}

pub(crate) fn last_shell_keyword_start(source: &str, span: Span, keyword: &str) -> Option<usize> {
    let upper = span.end.offset.min(source.len());
    let lower = span.start.offset.min(upper);
    last_shell_keyword_start_between(source, lower, upper, keyword)
}

pub(crate) fn last_shell_keyword_start_between(
    source: &str,
    lower: usize,
    upper: usize,
    keyword: &str,
) -> Option<usize> {
    let upper = upper.min(source.len());
    let lower = lower.min(upper);
    let slice = source.get(lower..upper)?;
    slice
        .match_indices(keyword)
        .filter_map(|(start, _)| {
            let end = start + keyword.len();
            shell_keyword_boundaries_match(slice, start, end).then_some(lower + start)
        })
        .last()
}

pub(crate) fn last_shell_keyword_end(text: &str, keyword: &str) -> Option<usize> {
    last_shell_keyword_start_between(text, 0, text.len(), keyword)
        .map(|start| start + keyword.len())
}

pub(crate) fn line_indent_before_offset(source: &str, offset: usize) -> Option<&str> {
    let offset = offset.min(source.len());
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes.get(line_start - 1) != Some(&b'\n') {
        line_start -= 1;
    }
    let line = source.get(line_start..offset)?;
    let indent_end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
        .map_or(line.len(), |(index, _)| index);
    line.get(..indent_end)
}

pub(crate) fn source_between_offsets(source: &str, start: usize, end: usize) -> Option<&str> {
    let lower = start.min(end).min(source.len());
    let upper = start.max(end).min(source.len());
    source.get(lower..upper)
}

pub(crate) fn has_newline_between_offsets(source: &str, start: usize, end: usize) -> bool {
    source_between_offsets(source, start, end).is_some_and(|between| between.contains('\n'))
}

pub(crate) fn close_suffix_comment_offsets(
    source: &str,
    close_span: Span,
) -> Option<(usize, usize)> {
    if close_span.start.line != close_span.end.line {
        return None;
    }
    let start = close_span.end.offset.min(source.len());
    let suffix_source = source.get(start..)?;
    let line_end = suffix_source
        .find('\n')
        .map_or(source.len(), |offset| start + offset);
    let suffix = source.get(start..line_end)?;
    let mut comment_start = None;
    for (offset, ch) in suffix.char_indices() {
        match ch {
            ' ' | '\t' => {}
            '#' => {
                comment_start = Some(start + offset);
                break;
            }
            _ => return None,
        }
    }
    let comment_start = comment_start?;
    let comment_end = source
        .get(comment_start..line_end)?
        .trim_end_matches([' ', '\t', '\r'])
        .len()
        + comment_start;
    Some((comment_start, comment_end))
}

pub(crate) fn redirect_operator_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start).copied()? {
        b'>' => Some(match bytes.get(start + 1).copied() {
            Some(b'>' | b'|' | b'&') => start + 2,
            _ => start + 1,
        }),
        b'<' => Some(match bytes.get(start + 1).copied() {
            Some(b'<' | b'>' | b'&') => {
                if bytes.get(start + 2) == Some(&b'<') {
                    start + 3
                } else {
                    start + 2
                }
            }
            _ => start + 1,
        }),
        _ => None,
    }
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
    matching_close_keyword_start(source, span, "fi", |source, offset, upper| {
        shell_keyword_at(source, offset, upper, "if").then_some("if".len())
    })
}

pub(crate) fn matching_done_close_start(source: &str, span: Span) -> Option<usize> {
    matching_close_keyword_start(source, span, "done", |source, offset, upper| {
        loop_open_keyword_at(source, offset, upper).then(|| {
            source[offset..]
                .chars()
                .take_while(char::is_ascii_alphabetic)
                .map(char::len_utf8)
                .sum()
        })
    })
}

fn matching_close_keyword_start(
    source: &str,
    span: Span,
    close_keyword: &str,
    mut open_len_at: impl FnMut(&str, usize, usize) -> Option<usize>,
) -> Option<usize> {
    let upper = span.end.offset.min(source.len());
    let mut offset = span.start.offset.min(upper);
    let mut depth = 0usize;
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        if let Some(next) = shell_quoted_or_comment_end(source, offset, upper, ch) {
            offset = next;
            continue;
        }

        if let Some(open_len) = open_len_at(source, offset, upper) {
            depth = depth.saturating_add(1);
            offset += open_len;
            continue;
        }
        if shell_keyword_at(source, offset, upper, close_keyword) {
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            offset += close_keyword.len();
            continue;
        }
        offset += ch.len_utf8();
    }
    None
}

fn shell_quoted_or_comment_end(
    source: &str,
    offset: usize,
    upper: usize,
    ch: char,
) -> Option<usize> {
    match ch {
        '\'' => Some(skip_single_quoted(source, offset + ch.len_utf8(), upper)),
        '"' => Some(skip_double_quoted(source, offset + ch.len_utf8(), upper)),
        '#' if shell_comment_can_start(source, offset) => Some(
            source[offset..]
                .find('\n')
                .map_or(upper, |newline| offset + newline + 1),
        ),
        _ => None,
    }
}
