use std::sync::{Arc, OnceLock};

use memchr::memchr_iter;
use shuck_ast::{Comment, Position, Span, TextSize};
use shuck_indexer::{IndexedComment, Indexer, LineIndex};

#[derive(Debug, Clone)]
pub struct SourceMap<'a> {
    source: &'a str,
    data: Arc<SourceMapData>,
}

#[derive(Debug)]
struct SourceMapData {
    line_index: LineIndex,
    first_non_whitespace: Vec<OnceLock<Option<usize>>>,
    hash_offsets: Vec<usize>,
    track_alignment: bool,
}

impl<'a> SourceMap<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self::new_with_alignment(source, true)
    }

    #[must_use]
    #[cfg(test)]
    pub fn without_alignment_indexes(source: &'a str) -> Self {
        Self::new_with_alignment(source, false)
    }

    #[must_use]
    pub(crate) fn from_indexer(source: &'a str, indexer: &Indexer, track_alignment: bool) -> Self {
        Self::with_line_index_and_comment_offsets(
            source,
            indexer.line_index().clone(),
            indexer
                .comment_index()
                .comments()
                .iter()
                .filter(|comment| !indexer.region_index().is_heredoc(comment.range.start()))
                .map(|comment| usize::from(comment.range.start())),
            track_alignment,
        )
    }

    fn with_line_index_and_comment_offsets(
        source: &'a str,
        line_index: LineIndex,
        comment_offsets: impl IntoIterator<Item = usize>,
        track_alignment: bool,
    ) -> Self {
        let mut hash_offsets = comment_offsets.into_iter().collect::<Vec<_>>();
        hash_offsets.sort_unstable();
        hash_offsets.dedup();
        Self::from_parts(source, line_index, hash_offsets, track_alignment)
    }

    fn new_with_alignment(source: &'a str, track_alignment: bool) -> Self {
        let bytes = source.as_bytes();

        let mut hash_offsets = Vec::new();

        for offset in memchr_iter(b'#', bytes) {
            hash_offsets.push(offset);
        }

        Self::from_parts(
            source,
            LineIndex::new(source),
            hash_offsets,
            track_alignment,
        )
    }

    fn from_parts(
        source: &'a str,
        line_index: LineIndex,
        hash_offsets: Vec<usize>,
        track_alignment: bool,
    ) -> Self {
        let first_non_whitespace = (0..line_index.line_count())
            .map(|_| OnceLock::new())
            .collect();

        Self {
            source,
            data: Arc::new(SourceMapData {
                line_index,
                first_non_whitespace,
                hash_offsets,
                track_alignment,
            }),
        }
    }

    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    #[must_use]
    pub fn line_number_for_offset(&self, offset: usize) -> usize {
        self.data
            .line_index
            .line_number(TextSize::new(self.clamped_offset(offset) as u32))
    }

    #[must_use]
    pub fn span_for_offsets(&self, start: usize, end: usize) -> Span {
        let line = self.line_number_for_offset(start);
        let line_start = self
            .data
            .line_index
            .line_start(line)
            .map(usize::from)
            .unwrap_or(0);
        let text = self.source.get(start..end).unwrap_or("");
        let start_position = Position {
            line,
            column: self.source[line_start..start].chars().count() + 1,
            offset: start,
        };
        let end_position = start_position.advanced_by(text);
        Span::from_positions(start_position, end_position)
    }

    #[must_use]
    pub fn is_inline_comment(&self, offset: usize) -> bool {
        self.first_non_whitespace_for_line(self.line_index_for_offset(offset))
            .is_some_and(|first| first < offset)
    }

    #[must_use]
    pub(crate) fn source_comment(&self, comment: Comment) -> Option<SourceComment<'a>> {
        let start = usize::from(comment.range.start());
        let end = usize::from(comment.range.end());
        self.source_comment_for_offsets(start, end)
    }

    #[must_use]
    pub(crate) fn source_comment_for_offsets(
        &self,
        start: usize,
        end: usize,
    ) -> Option<SourceComment<'a>> {
        (start < end && end <= self.source.len()).then(|| SourceComment {
            text: &self.source[start..end],
            span: self.span_for_offsets(start, end),
            line: self.line_number_for_offset(start),
            inline: self.is_inline_comment(start),
        })
    }

    #[must_use]
    fn source_comment_for_indexed(&self, comment: &IndexedComment) -> Option<SourceComment<'a>> {
        let start = usize::from(comment.range.start());
        let end = usize::from(comment.range.end());
        (start < end && end <= self.source.len()).then(|| SourceComment {
            text: &self.source[start..end],
            span: self.span_for_offsets(start, end),
            line: comment.line,
            inline: !comment.is_own_line,
        })
    }

    #[must_use]
    pub(crate) fn suffix_comment_after_span(&self, span: Span) -> Option<SourceComment<'a>> {
        if span.start.line != span.end.line {
            return None;
        }
        let start = span.end.offset.min(self.source.len());
        let (_, line_end) = self.line_bounds_for_offset(start)?;
        let comment_start = self.first_comment_between(start, line_end)?;
        let prefix = self.source.get(start..comment_start)?;
        if !prefix.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            return None;
        }

        let comment_end = self
            .source
            .get(comment_start..line_end)?
            .trim_end_matches([' ', '\t', '\r'])
            .len()
            + comment_start;
        self.source_comment_for_offsets(comment_start, comment_end)
    }

    #[must_use]
    pub fn contains_comment_between(&self, start: usize, end: usize) -> bool {
        contains_offset_in_range(&self.data.hash_offsets, start, end)
    }

    #[must_use]
    pub fn first_comment_between(&self, start: usize, end: usize) -> Option<usize> {
        if start >= end {
            return None;
        }
        let index = self
            .data
            .hash_offsets
            .partition_point(|offset| *offset < start);
        self.data
            .hash_offsets
            .get(index)
            .copied()
            .filter(|offset| *offset < end)
    }

    #[must_use]
    pub fn contains_newline_between(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }

        self.data
            .line_index
            .line_start(self.line_number_for_offset(start) + 1)
            .is_some_and(|offset| usize::from(offset) <= end)
    }

    #[must_use]
    pub(crate) fn operator_starts_or_ends_line(&self, operator_span: Span) -> bool {
        let start = operator_span.start.offset;
        let end = operator_span.end.offset;
        if start >= end || end > self.source.len() {
            return false;
        }

        let Some((line_start, _)) = self.line_bounds_for_offset(start) else {
            return false;
        };
        let Some((_, line_end)) = self.line_bounds_for_offset(end.saturating_sub(1)) else {
            return false;
        };
        let Some(before) = self.source.get(line_start..start) else {
            return false;
        };
        let Some(after) = self.source.get(end..line_end) else {
            return false;
        };

        (line_start > 0 && line_edge_is_blank_or_continuation(before))
            || (line_end < self.source.len() && line_edge_is_blank_or_continuation(after))
    }

    #[must_use]
    pub(crate) fn line_indent_before_offset(&self, offset: usize) -> Option<&'a str> {
        let offset = offset.min(self.source.len());
        let (line_start, _) = self.line_bounds_for_offset(offset)?;
        let prefix = self.source.get(line_start..offset)?;
        let indent_end = prefix
            .char_indices()
            .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
            .map_or(prefix.len(), |(index, _)| index);
        prefix.get(..indent_end)
    }

    #[must_use]
    pub(crate) fn has_blank_line_immediately_before_offset(&self, offset: usize) -> bool {
        let offset = offset.min(self.source.len());
        let Some((line_start, _)) = self.line_bounds_for_offset(offset) else {
            return false;
        };
        let Some(current_prefix) = self.source.get(line_start..offset) else {
            return false;
        };
        if !line_is_blank(current_prefix) {
            return false;
        }

        let Some((previous_start, previous_end)) = self.previous_line_bounds(line_start) else {
            return false;
        };
        self.source
            .get(previous_start..previous_end)
            .is_some_and(line_is_blank)
    }

    #[must_use]
    pub(crate) fn line_bounds_for_offset(&self, offset: usize) -> Option<(usize, usize)> {
        if self.source.is_empty() {
            return None;
        }
        let line = self.line_number_for_offset(offset);
        self.line_bounds(line)
    }

    #[must_use]
    pub(crate) fn previous_line_bounds(&self, line_start: usize) -> Option<(usize, usize)> {
        if line_start == 0 || self.source.is_empty() {
            return None;
        }
        let line = self.line_number_for_offset(line_start.saturating_sub(1));
        self.line_bounds(line)
    }

    #[must_use]
    pub(crate) fn next_line_bounds(&self, line_end: usize) -> Option<(usize, usize)> {
        let next_start = line_end
            .checked_add(1)
            .filter(|offset| *offset < self.source.len())?;
        let line = self.line_number_for_offset(next_start);
        self.line_bounds(line)
    }

    #[must_use]
    pub fn has_alignment_padding_between(&self, start: usize, end: usize) -> bool {
        if start >= end || !self.data.track_alignment || self.contains_newline_between(start, end) {
            return false;
        }

        self.source
            .get(start..end)
            .is_some_and(slice_has_alignment_padding)
    }

    fn line_index_for_offset(&self, offset: usize) -> usize {
        self.line_number_for_offset(offset).saturating_sub(1)
    }

    fn first_non_whitespace_for_line(&self, line_index: usize) -> Option<usize> {
        *self.data.first_non_whitespace[line_index].get_or_init(|| {
            let range = self
                .data
                .line_index
                .line_range(line_index + 1, self.source)?;
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            first_non_whitespace_in_line(self.source, start, end)
        })
    }

    fn line_bounds(&self, line: usize) -> Option<(usize, usize)> {
        let range = self.data.line_index.line_range(line, self.source)?;
        Some((usize::from(range.start()), usize::from(range.end())))
    }

    fn clamped_offset(&self, offset: usize) -> usize {
        if self.source.is_empty() {
            0
        } else {
            offset.min(self.source.len().saturating_sub(1))
        }
    }
}

fn line_edge_is_blank_or_continuation(text: &str) -> bool {
    let trimmed = text.trim_matches(|ch| matches!(ch, ' ' | '\t' | '\r'));
    trimmed.is_empty() || trimmed == "\\"
}

fn line_is_blank(text: &str) -> bool {
    text.trim_matches([' ', '\t', '\r']).is_empty()
}

fn slice_has_alignment_padding(slice: &str) -> bool {
    let mut previous_was_space = false;
    for byte in slice.bytes() {
        match byte {
            b'\t' => return true,
            b' ' if previous_was_space => return true,
            b' ' => previous_was_space = true,
            _ => previous_was_space = false,
        }
    }
    false
}

fn first_non_whitespace_in_line(source: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut offset = start;
    while offset < end {
        let byte = bytes[offset];
        if byte.is_ascii() {
            if !byte.is_ascii_whitespace() {
                return Some(offset);
            }
            offset += 1;
            continue;
        }

        let ch = source[offset..end].chars().next()?;
        if !ch.is_whitespace() {
            return Some(offset);
        }
        offset += ch.len_utf8();
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceComment<'a> {
    text: &'a str,
    span: Span,
    line: usize,
    inline: bool,
}

impl<'a> SourceComment<'a> {
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text.trim_end_matches([' ', '\t', '\r'])
    }

    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    #[must_use]
    pub fn inline(&self) -> bool {
        self.inline
    }
}

/// Formatter-owned attachment view over parser-indexed comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommentAttachmentModel<'a> {
    comments: Box<[SourceComment<'a>]>,
}

impl<'a> CommentAttachmentModel<'a> {
    pub(crate) fn from_indexer(source_map: &SourceMap<'a>, indexer: &Indexer) -> Self {
        let mut comments = indexer
            .comment_index()
            .comments()
            .iter()
            .filter(|comment| !indexer.region_index().is_heredoc(comment.range.start()))
            .filter_map(|comment| source_map.source_comment_for_indexed(comment))
            .collect::<Vec<_>>();
        comments.sort_by_key(|comment| comment.span().start.offset);

        Self {
            comments: comments.into_boxed_slice(),
        }
    }

    pub(crate) fn attach_sequence(
        &self,
        lower_bound: usize,
        upper_bound: Option<usize>,
        skip_span: Option<Span>,
        child_spans: &[Span],
    ) -> SequenceCommentAttachment<'a> {
        let comments = self.comment_window(lower_bound, upper_bound);
        if child_spans.is_empty() {
            let dangling = comments
                .iter()
                .copied()
                .filter(|comment| {
                    upper_bound.is_none_or(|limit| comment.span().end.offset <= limit)
                })
                .filter(|comment| {
                    skip_span.is_none_or(|span| !span_contains_comment(span, *comment))
                })
                .collect::<Vec<_>>();
            let ambiguous = dangling.iter().any(SourceComment::inline);
            return SequenceCommentAttachment::with_dangling(dangling, ambiguous);
        }

        compute_sequence_attachment(comments, child_spans, upper_bound, skip_span)
    }

    fn comment_window(
        &self,
        lower_bound: usize,
        upper_bound: Option<usize>,
    ) -> &[SourceComment<'a>] {
        let start_index = self
            .comments
            .partition_point(|comment| comment.span().start.offset < lower_bound);
        let end_index = upper_bound.map_or(self.comments.len(), |limit| {
            self.comments
                .partition_point(|comment| comment.span().start.offset < limit)
        });

        &self.comments[start_index..end_index.max(start_index)]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceCommentAttachment<'a> {
    leading: Vec<Vec<SourceComment<'a>>>,
    trailing: Vec<Vec<SourceComment<'a>>>,
    dangling: Vec<SourceComment<'a>>,
    ambiguous: bool,
}

impl<'a> SequenceCommentAttachment<'a> {
    pub(crate) fn new(child_count: usize) -> Self {
        Self {
            leading: vec![Vec::new(); child_count],
            trailing: vec![Vec::new(); child_count],
            dangling: Vec::new(),
            ambiguous: false,
        }
    }

    pub(crate) fn with_dangling(dangling: Vec<SourceComment<'a>>, ambiguous: bool) -> Self {
        Self {
            leading: Vec::new(),
            trailing: Vec::new(),
            dangling,
            ambiguous,
        }
    }

    #[must_use]
    pub fn leading_for(&self, index: usize) -> &[SourceComment<'a>] {
        self.leading.get(index).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn trailing_for(&self, index: usize) -> &[SourceComment<'a>] {
        self.trailing.get(index).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn dangling(&self) -> &[SourceComment<'a>] {
        &self.dangling
    }

    #[must_use]
    pub fn is_ambiguous(&self) -> bool {
        self.ambiguous
    }

    #[must_use]
    pub fn has_comments(&self) -> bool {
        self.ambiguous
            || !self.dangling.is_empty()
            || self.leading.iter().any(|comments| !comments.is_empty())
            || self.trailing.iter().any(|comments| !comments.is_empty())
    }
}

fn compute_sequence_attachment<'a>(
    items: &[SourceComment<'a>],
    child_spans: &[Span],
    upper_bound: Option<usize>,
    skip_span: Option<Span>,
) -> SequenceCommentAttachment<'a> {
    let mut attachment = SequenceCommentAttachment::new(child_spans.len());
    if child_spans.is_empty() || items.is_empty() {
        return attachment;
    }

    let first_child_start = child_spans[0].start.offset;
    let last_child_end = child_spans
        .last()
        .map(|span| span.end.offset)
        .unwrap_or(first_child_start);
    let limit_end = upper_bound.unwrap_or(usize::MAX);
    let mut child_cursor = 0;

    let mut index = 0;
    while index < items.len() {
        let comment = items[index];
        let start = comment.span.start.offset;
        let end = comment.span.end.offset;
        if start >= limit_end {
            break;
        }
        if end > limit_end {
            index += 1;
            continue;
        }
        if skip_span.is_some_and(|span| span_contains_comment(span, comment)) {
            index += 1;
            continue;
        }

        while child_cursor < child_spans.len() && child_spans[child_cursor].end.offset <= start {
            child_cursor += 1;
        }

        let prev = child_cursor.checked_sub(1);
        let next = child_spans
            .get(child_cursor)
            .and_then(|span| (span.start.offset >= end).then_some(child_cursor));
        let current = child_spans.get(child_cursor);
        let inside_current =
            current.is_some_and(|span| span.start.offset <= start && end <= span.end.offset);

        if inside_current {
            index += 1;
            continue;
        }

        if comment.inline {
            if let Some(prev_idx) = prev
                && child_spans[prev_idx].end.line == comment.line
                && child_spans[prev_idx].start.offset <= start
            {
                attachment.trailing[prev_idx].push(comment);
                index += 1;
                continue;
            }

            match (prev, next) {
                (Some(prev_idx), next_idx)
                    if next_idx.is_none_or(|next_idx| prev_idx + 1 == next_idx)
                        && child_spans[prev_idx].end.line == comment.line =>
                {
                    attachment.trailing[prev_idx].push(comment);
                }
                _ => attachment.ambiguous = true,
            }
            index += 1;
            continue;
        }

        if end <= first_child_start {
            let run_end = advance_comment_run(items, index, limit_end, skip_span, |candidate| {
                candidate.span.end.offset <= first_child_start
            });
            for item in &items[index..run_end] {
                attachment.leading[0].push(*item);
            }
            index = run_end;
        } else if let Some(next_idx) = next {
            let gap_start = prev
                .map(|prev_idx| child_spans[prev_idx].end.offset)
                .unwrap_or(0);
            let gap_end = child_spans[next_idx].start.offset;
            let run_end = advance_comment_run(items, index, limit_end, skip_span, |candidate| {
                candidate.span.start.offset >= gap_start && candidate.span.end.offset <= gap_end
            });
            for item in &items[index..run_end] {
                attachment.leading[next_idx].push(*item);
            }
            index = run_end;
        } else if start >= last_child_end {
            let run_end = advance_comment_run(items, index, limit_end, skip_span, |candidate| {
                candidate.span.start.offset >= last_child_end
            });
            for item in &items[index..run_end] {
                attachment.dangling.push(*item);
            }
            index = run_end;
        } else {
            index += 1;
        }
    }

    attachment
}

fn advance_comment_run<'a>(
    items: &[SourceComment<'a>],
    start_index: usize,
    limit_end: usize,
    skip_span: Option<Span>,
    belongs: impl Fn(SourceComment<'a>) -> bool,
) -> usize {
    let mut index = start_index;
    while index < items.len() {
        let comment = items[index];
        if comment.span.start.offset >= limit_end
            || comment.inline
            || comment.span.end.offset > limit_end
            || skip_span.is_some_and(|span| span_contains_comment(span, comment))
            || !belongs(comment)
        {
            break;
        }
        index += 1;
    }
    index
}

fn span_contains_comment(span: Span, comment: SourceComment<'_>) -> bool {
    span.start.offset <= comment.span.start.offset && comment.span.end.offset <= span.end.offset
}

fn contains_offset_in_range(offsets: &[usize], start: usize, end: usize) -> bool {
    if start >= end {
        return false;
    }

    let index = offsets.partition_point(|offset| *offset < start);
    offsets.get(index).is_some_and(|offset| *offset < end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_indexer::{Indexer, IndexerOptions};
    use shuck_parser::parser::Parser;

    #[test]
    fn source_map_uses_indexed_lines_and_comment_contexts() {
        let source = "  # leading\ncmd  # inline\n\n\u{2003}# unicode-space\nécho\t  ok\n";
        let source_map = SourceMap::new(source);

        assert_eq!(
            line_starts(&source_map),
            vec![0, 12, 26, 27, 46, source.len()]
        );
        let first_non_whitespace = (0..source_map.data.line_index.line_count())
            .map(|line_index| source_map.first_non_whitespace_for_line(line_index))
            .collect::<Vec<_>>();
        assert_eq!(
            first_non_whitespace,
            vec![Some(2), Some(12), None, Some(30), Some(46), None]
        );

        let leading_hash = source.find('#').unwrap();
        let inline_hash = source.find("# inline").unwrap();
        let unicode_hash = source.find("# unicode-space").unwrap();
        assert!(!source_map.is_inline_comment(leading_hash));
        assert!(source_map.is_inline_comment(inline_hash));
        assert!(!source_map.is_inline_comment(unicode_hash));

        let tab = source.find('\t').unwrap();
        let double_space = source.find("  ok").unwrap();
        assert!(source_map.has_alignment_padding_between(tab, tab + 1));
        assert!(source_map.has_alignment_padding_between(double_space, double_space + 2));
        assert!(source_map.contains_newline_between(leading_hash, inline_hash));
    }

    #[test]
    fn source_map_can_skip_alignment_only_indexes() {
        let source = "cmd\t  arg\n";
        let source_map = SourceMap::without_alignment_indexes(source);

        assert_eq!(line_starts(&source_map), vec![0, source.len()]);
        assert_eq!(source_map.first_non_whitespace_for_line(0), Some(0));
        assert!(!source_map.has_alignment_padding_between(3, 6));
    }

    #[test]
    fn comment_attachment_model_uses_indexed_comments() {
        let source =
            "echo '# not a comment'\n# leading\necho hi # inline\ncat <<EOF\n# heredoc\nEOF\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::for_file_with_options(
            source,
            &output.file,
            IndexerOptions::new().with_source_layout_indexes(true),
        );
        let source_map = SourceMap::from_indexer(source, &indexer, true);

        let attachments = CommentAttachmentModel::from_indexer(&source_map, &indexer);
        let starts = attachments
            .comments
            .iter()
            .map(|comment| comment.span().start.offset)
            .collect::<Vec<_>>();

        assert!(!starts.contains(&source.find("# not a comment").unwrap()));
        assert!(starts.contains(&source.find("# leading").unwrap()));
        assert!(starts.contains(&source.find("# inline").unwrap()));
        assert!(!starts.contains(&source.find("# heredoc").unwrap()));
    }

    fn line_starts(source_map: &SourceMap<'_>) -> Vec<usize> {
        (1..=source_map.data.line_index.line_count())
            .filter_map(|line| source_map.data.line_index.line_start(line))
            .map(usize::from)
            .collect()
    }
}
