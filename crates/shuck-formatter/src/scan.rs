use shuck_ast::Span;

use crate::comments::SourceMap;
use crate::raw_syntax::RawShellScanner;

pub(crate) fn line_has_shell_comment_before(source: &str, offset: usize) -> bool {
    let upper = offset.min(source.len());
    let line_start = source[..upper]
        .rfind('\n')
        .map_or(0, |newline| newline.saturating_add(1));
    RawShellScanner::bounded(source, upper)
        .find_comment(line_start, upper)
        .is_some()
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

pub(crate) fn shell_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    let end = offset.saturating_add(keyword.len());
    end <= upper
        && source.get(offset..end) == Some(keyword)
        && shell_keyword_boundaries_match(source, offset, end)
}

fn shell_control_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    shell_keyword_at(source, offset, upper, keyword)
        && shell_keyword_has_command_prefix(source, offset)
}

fn shell_keyword_has_command_prefix(source: &str, offset: usize) -> bool {
    let prefix = &source[..offset];
    let Some((previous_offset, previous)) = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | '\r'))
    else {
        return true;
    };

    if previous == '\n' || matches!(previous, ';' | '&' | '|' | '(' | ')' | '{' | '!') {
        return true;
    }

    let word_end = previous_offset + previous.len_utf8();
    let word_start = prefix[..word_end]
        .char_indices()
        .rev()
        .find(|(_, ch)| !is_shell_keyword_char(*ch))
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    matches!(
        &prefix[word_start..word_end],
        "do" | "then" | "else" | "elif" | "time" | "coproc"
    )
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
        shell_control_keyword_at(source, offset, upper, "if").then_some("if".len())
    })
}

pub(crate) fn matching_done_close_start(source: &str, span: Span) -> Option<usize> {
    matching_close_keyword_start(source, span, "done", |source, offset, upper| {
        ["for", "select", "while", "until", "foreach", "repeat"]
            .iter()
            .find(|keyword| shell_control_keyword_at(source, offset, upper, keyword))
            .map(|_| {
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
    let scanner = RawShellScanner::bounded(source, upper);
    while offset < upper {
        let ch = source[offset..].chars().next()?;
        if let Some(next) = scanner.skip_quoted_or_comment_at(offset) {
            offset = next;
            continue;
        }

        if let Some(open_len) = open_len_at(source, offset, upper) {
            depth = depth.saturating_add(1);
            offset += open_len;
            continue;
        }
        if shell_control_keyword_at(source, offset, upper, close_keyword) {
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
