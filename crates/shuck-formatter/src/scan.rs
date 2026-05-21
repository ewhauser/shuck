use shuck_ast::Span;

use crate::comments::SourceMap;
use crate::source::SourceView;

pub(crate) fn branch_keyword_offset(
    source: &str,
    start: usize,
    end: usize,
    keyword: &str,
) -> Option<usize> {
    SourceView::new(source).branch_keyword_offset(start, end, keyword)
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    SourceView::new(source).last_uncommented_shell_keyword_before(search_end, keyword)
}

pub(crate) fn last_shell_keyword_start(source: &str, span: Span, keyword: &str) -> Option<usize> {
    SourceView::new(source).last_shell_keyword_start(span, keyword)
}

pub(crate) fn last_shell_keyword_start_between(
    source: &str,
    lower: usize,
    upper: usize,
    keyword: &str,
) -> Option<usize> {
    SourceView::new(source).last_shell_keyword_start_between(lower, upper, keyword)
}

pub(crate) fn last_shell_keyword_end(text: &str, keyword: &str) -> Option<usize> {
    last_shell_keyword_start_between(text, 0, text.len(), keyword)
        .map(|start| start + keyword.len())
}

pub(crate) fn line_indent_before_offset(source: &str, offset: usize) -> Option<&str> {
    SourceView::new(source).line_indent_before_offset(offset)
}

pub(crate) fn source_between_offsets(source: &str, start: usize, end: usize) -> Option<&str> {
    SourceView::new(source).slice_between(start, end)
}

pub(crate) fn shell_keyword_at(source: &str, offset: usize, upper: usize, keyword: &str) -> bool {
    SourceView::new(source).shell_keyword_at(offset, upper, keyword)
}

pub(crate) fn normalized_close_keyword_span(
    source: &str,
    source_map: &SourceMap<'_>,
    span: Span,
    keyword: &str,
) -> Span {
    let start = span.start.offset.min(source.len());
    let end = start.saturating_add(keyword.len()).min(source.len());
    if SourceView::new(source).slice_between(start, end) == Some(keyword) {
        source_map.span_for_offsets(start, end)
    } else {
        span
    }
}
