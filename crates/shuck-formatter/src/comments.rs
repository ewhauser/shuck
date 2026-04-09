use std::sync::Arc;

use shuck_ast::{Comment, Position, Span};

#[derive(Debug, Clone)]
pub struct SourceMap<'a> {
    source: &'a str,
    data: Arc<SourceMapData>,
}

#[derive(Debug)]
struct SourceMapData {
    line_starts: Vec<usize>,
    first_non_whitespace: Vec<Option<usize>>,
    hash_offsets: Vec<usize>,
    tab_offsets: Vec<usize>,
    double_space_offsets: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        let line_starts = line_starts(source);
        let first_non_whitespace = line_starts
            .iter()
            .enumerate()
            .map(|(index, start)| {
                let end = line_starts.get(index + 1).copied().unwrap_or(source.len());
                source[*start..end]
                    .char_indices()
                    .find(|(_, ch)| *ch != '\n' && !ch.is_whitespace())
                    .map(|(offset, _)| start + offset)
            })
            .collect();

        let mut hash_offsets = Vec::new();
        let mut tab_offsets = Vec::new();
        let mut double_space_offsets = Vec::new();
        let bytes = source.as_bytes();
        for offset in 0..bytes.len() {
            match bytes[offset] {
                b'#' => hash_offsets.push(offset),
                b'\t' => tab_offsets.push(offset),
                b' ' if offset + 1 < bytes.len() && bytes[offset + 1] == b' ' => {
                    double_space_offsets.push(offset);
                }
                _ => {}
            }
        }

        Self {
            source,
            data: Arc::new(SourceMapData {
                line_starts,
                first_non_whitespace,
                hash_offsets,
                tab_offsets,
                double_space_offsets,
            }),
        }
    }

    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    #[must_use]
    pub fn line_number_for_offset(&self, offset: usize) -> usize {
        self.line_index_for_offset(offset) + 1
    }

    #[must_use]
    pub fn span_for_offsets(&self, start: usize, end: usize) -> Span {
        let line_index = self.line_index_for_offset(start);
        let line_start = self.data.line_starts[line_index];
        let text = self.source.get(start..end).unwrap_or("");
        let start_position = Position {
            line: line_index + 1,
            column: self.source[line_start..start].chars().count() + 1,
            offset: start,
        };
        let end_position = start_position.advanced_by(text);
        Span::from_positions(start_position, end_position)
    }

    #[must_use]
    pub fn is_inline_comment(&self, offset: usize) -> bool {
        self.data.first_non_whitespace[self.line_index_for_offset(offset)]
            .is_some_and(|first| first < offset)
    }

    #[must_use]
    pub(crate) fn source_comment(&self, comment: Comment) -> Option<SourceComment<'a>> {
        let start = usize::from(comment.range.start());
        let end = usize::from(comment.range.end());
        (start < end && end <= self.source.len()).then(|| SourceComment {
            text: &self.source[start..end],
            span: self.span_for_offsets(start, end),
            line: self.line_number_for_offset(start),
            inline: self.is_inline_comment(start),
        })
    }

    #[must_use]
    pub fn contains_comment_between(&self, start: usize, end: usize) -> bool {
        contains_offset_in_range(&self.data.hash_offsets, start, end)
    }

    #[must_use]
    pub fn contains_newline_between(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }

        let index = self
            .data
            .line_starts
            .partition_point(|offset| *offset <= start);
        self.data
            .line_starts
            .get(index)
            .is_some_and(|offset| *offset < end)
    }

    #[must_use]
    pub fn has_alignment_padding_between(&self, start: usize, end: usize) -> bool {
        if start >= end || self.contains_newline_between(start, end) {
            return false;
        }

        contains_offset_in_range(&self.data.tab_offsets, start, end)
            || end.saturating_sub(start) >= 2
                && contains_offset_in_range(
                    &self.data.double_space_offsets,
                    start,
                    end.saturating_sub(1),
                )
    }

    fn line_index_for_offset(&self, offset: usize) -> usize {
        let offset = offset.min(self.source.len().saturating_sub(1));
        match self.data.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    }
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
        self.text
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceCommentAttachment<'a> {
    leading: Vec<Vec<SourceComment<'a>>>,
    trailing: Vec<Vec<SourceComment<'a>>>,
    dangling: Vec<SourceComment<'a>>,
    ambiguous: bool,
}

impl<'a> SequenceCommentAttachment<'a> {
    fn new(child_count: usize) -> Self {
        Self {
            leading: vec![Vec::new(); child_count],
            trailing: vec![Vec::new(); child_count],
            dangling: Vec::new(),
            ambiguous: false,
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

    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<Vec<SourceComment<'a>>>,
        Vec<Vec<SourceComment<'a>>>,
        Vec<SourceComment<'a>>,
        bool,
    ) {
        (self.leading, self.trailing, self.dangling, self.ambiguous)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceCommentAnalysis<'a> {
    pub(crate) attachment: SequenceCommentAttachment<'a>,
    pub(crate) claimed_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct CommentAttachmentIndex<'a> {
    source_map: SourceMap<'a>,
    items: Arc<[SourceComment<'a>]>,
    claimed: Vec<bool>,
    next_unclaimed: usize,
}

pub type Comments<'a> = CommentAttachmentIndex<'a>;

impl<'a> CommentAttachmentIndex<'a> {
    #[must_use]
    pub fn from_ast(source: &'a str, comments: &[Comment]) -> Self {
        let source_map = SourceMap::new(source);
        let mut items = comments
            .iter()
            .filter_map(|comment| source_map.source_comment(*comment))
            .collect::<Vec<_>>();
        items.sort_by_key(|comment| comment.span.start.offset);

        let claimed = vec![false; items.len()];
        Self {
            source_map,
            items: Arc::from(items.into_boxed_slice()),
            claimed,
            next_unclaimed: 0,
        }
    }

    #[must_use]
    pub fn source_map(&self) -> &SourceMap<'a> {
        &self.source_map
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub(crate) fn inspect_sequence(
        &self,
        child_spans: &[Span],
        upper_bound: Option<usize>,
    ) -> SequenceCommentAnalysis<'a> {
        compute_sequence_attachment(
            &self.items,
            &self.claimed,
            self.next_unclaimed,
            child_spans,
            upper_bound,
        )
    }

    pub(crate) fn claim_sequence(&mut self, analysis: &SequenceCommentAnalysis<'a>) {
        for index in &analysis.claimed_indices {
            self.claimed[*index] = true;
        }
        self.advance_next_unclaimed();
    }

    pub fn attach_sequence(
        &mut self,
        child_spans: &[Span],
        upper_bound: Option<usize>,
    ) -> SequenceCommentAttachment<'a> {
        let analysis = self.inspect_sequence(child_spans, upper_bound);
        self.claim_sequence(&analysis);
        analysis.attachment
    }

    pub fn take_remaining(&mut self) -> Vec<SourceComment<'a>> {
        let mut remaining = Vec::new();
        for index in self.next_unclaimed..self.items.len() {
            if self.claimed[index] {
                continue;
            }
            self.claimed[index] = true;
            remaining.push(self.items[index]);
        }
        self.advance_next_unclaimed();
        remaining
    }

    pub fn claim_in_span(&mut self, span: Span) {
        for index in self.next_unclaimed..self.items.len() {
            let comment = self.items[index];
            if comment.span.start.offset > span.end.offset {
                break;
            }
            if self.claimed[index] {
                continue;
            }
            if span.start.offset <= comment.span.start.offset
                && comment.span.end.offset <= span.end.offset
            {
                self.claimed[index] = true;
            }
        }
        self.advance_next_unclaimed();
    }

    pub fn claim_lines(&mut self, start_line: usize, end_line: usize) {
        for index in self.next_unclaimed..self.items.len() {
            let comment = self.items[index];
            if comment.line > end_line {
                break;
            }
            if self.claimed[index] {
                continue;
            }
            if (start_line..=end_line).contains(&comment.line) {
                self.claimed[index] = true;
            }
        }
        self.advance_next_unclaimed();
    }

    fn advance_next_unclaimed(&mut self) {
        while self.next_unclaimed < self.claimed.len() && self.claimed[self.next_unclaimed] {
            self.next_unclaimed += 1;
        }
    }
}

fn compute_sequence_attachment<'a>(
    items: &[SourceComment<'a>],
    claimed: &[bool],
    start_index: usize,
    child_spans: &[Span],
    upper_bound: Option<usize>,
) -> SequenceCommentAnalysis<'a> {
    let mut attachment = SequenceCommentAttachment::new(child_spans.len());
    if child_spans.is_empty() {
        return SequenceCommentAnalysis {
            attachment,
            claimed_indices: Vec::new(),
        };
    }

    let mut claimed_indices = Vec::new();
    let first_child_start = child_spans[0].start.offset;
    let last_child_end = child_spans
        .last()
        .map(|span| span.end.offset)
        .unwrap_or(first_child_start);
    let limit_end = upper_bound.unwrap_or(usize::MAX);
    let mut child_cursor = 0;

    let mut index = start_index;
    while index < items.len() {
        if claimed[index] {
            index += 1;
            continue;
        }

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
                claimed_indices.push(index);
                index += 1;
                continue;
            }

            match (prev, next) {
                (Some(prev_idx), Some(next_idx))
                    if prev_idx + 1 == next_idx
                        && child_spans[prev_idx].end.line == comment.line =>
                {
                    attachment.trailing[prev_idx].push(comment);
                    claimed_indices.push(index);
                }
                (Some(prev_idx), None) if child_spans[prev_idx].end.line == comment.line => {
                    attachment.trailing[prev_idx].push(comment);
                    claimed_indices.push(index);
                }
                _ => attachment.ambiguous = true,
            }
            index += 1;
            continue;
        }

        if end <= first_child_start {
            let run_end = collect_comment_run(items, claimed, index, limit_end, |comment| {
                comment.span.end.offset <= first_child_start
            });
            for (run_index, item) in items.iter().enumerate().take(run_end).skip(index) {
                attachment.leading[0].push(*item);
                claimed_indices.push(run_index);
            }
            index = run_end;
        } else if let Some(next_idx) = next {
            let target_prev = prev;
            let target_next = Some(next_idx);
            let run_end = collect_comment_run(items, claimed, index, limit_end, |candidate| {
                let (candidate_prev, candidate_next) = surrounding_children(
                    child_spans,
                    candidate.span.start.offset,
                    candidate.span.end.offset,
                );
                candidate_prev == target_prev && candidate_next == target_next
            });
            for (run_index, item) in items.iter().enumerate().take(run_end).skip(index) {
                attachment.leading[next_idx].push(*item);
                claimed_indices.push(run_index);
            }
            index = run_end;
        } else if start >= last_child_end {
            let run_end = collect_comment_run(items, claimed, index, limit_end, |comment| {
                comment.span.start.offset >= last_child_end
            });
            for (run_index, item) in items.iter().enumerate().take(run_end).skip(index) {
                attachment.dangling.push(*item);
                claimed_indices.push(run_index);
            }
            index = run_end;
        } else {
            index += 1;
        }
    }

    SequenceCommentAnalysis {
        attachment,
        claimed_indices,
    }
}

pub(crate) fn inspect_sequence_comments<'a>(
    items: &[SourceComment<'a>],
    child_spans: &[Span],
    upper_bound: Option<usize>,
) -> SequenceCommentAttachment<'a> {
    compute_sequence_attachment(
        items,
        &vec![false; items.len()],
        0,
        child_spans,
        upper_bound,
    )
    .attachment
}

fn collect_comment_run<'a>(
    items: &[SourceComment<'a>],
    claimed: &[bool],
    start_index: usize,
    limit_end: usize,
    belongs: impl Fn(SourceComment<'a>) -> bool,
) -> usize {
    let mut index = start_index;
    while index < items.len() {
        if claimed[index] {
            break;
        }
        let comment = items[index];
        if comment.inline || comment.span.end.offset > limit_end || !belongs(comment) {
            break;
        }
        index += 1;
    }
    index
}

fn surrounding_children(
    child_spans: &[Span],
    start: usize,
    end: usize,
) -> (Option<usize>, Option<usize>) {
    let cursor = child_spans.partition_point(|span| span.end.offset <= start);
    let prev = cursor.checked_sub(1);
    let next = child_spans
        .get(cursor)
        .and_then(|span| (span.start.offset >= end).then_some(cursor));
    (prev, next)
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' && offset + 1 < source.len() {
            starts.push(offset + 1);
        }
    }
    starts
}

fn contains_offset_in_range(offsets: &[usize], start: usize, end: usize) -> bool {
    if start >= end {
        return false;
    }

    let index = offsets.partition_point(|offset| *offset < start);
    offsets.get(index).is_some_and(|offset| *offset < end)
}
