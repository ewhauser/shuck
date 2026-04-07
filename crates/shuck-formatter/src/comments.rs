use shuck_ast::{Comment, Span};

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
}

#[derive(Debug, Clone)]
pub struct CommentAttachmentIndex<'a> {
    items: Vec<SourceComment<'a>>,
    claimed: Vec<bool>,
}

pub type Comments<'a> = CommentAttachmentIndex<'a>;

impl<'a> CommentAttachmentIndex<'a> {
    #[must_use]
    pub fn from_ast(source: &'a str, comments: &[Comment]) -> Self {
        let line_starts = line_starts(source);
        let mut items = Vec::with_capacity(comments.len());

        for comment in comments {
            let start = usize::from(comment.range.start());
            let end = usize::from(comment.range.end());
            if start >= end || end > source.len() {
                continue;
            }

            let line = line_number_for_offset(&line_starts, start);
            items.push(SourceComment {
                text: &source[start..end],
                span: span_for_offsets(source, start, end),
                line,
                inline: is_inline_comment(source, start),
            });
        }

        let claimed = vec![false; items.len()];
        Self { items, claimed }
    }

    pub fn attach_sequence(
        &mut self,
        child_spans: &[Span],
        upper_bound: Option<usize>,
    ) -> SequenceCommentAttachment<'a> {
        let mut attachment = SequenceCommentAttachment::new(child_spans.len());
        if child_spans.is_empty() {
            return attachment;
        }

        let first_child_start = child_spans[0].start.offset;
        let last_child_end = child_spans
            .last()
            .map(|span| span.end.offset)
            .unwrap_or(first_child_start);
        let limit_end = upper_bound.unwrap_or(usize::MAX);

        for index in 0..self.items.len() {
            if self.claimed[index] {
                continue;
            }
            let comment = self.items[index];
            let start = comment.span.start.offset;
            let end = comment.span.end.offset;

            if end > limit_end {
                continue;
            }

            if comment.inline {
                if let Some((prev_idx, _)) =
                    child_spans.iter().enumerate().rev().find(|(_, span)| {
                        span.end.line == comment.line && span.start.offset <= start
                    })
                {
                    attachment.trailing[prev_idx].push(comment);
                    self.claimed[index] = true;
                    continue;
                }

                if child_spans
                    .iter()
                    .any(|span| span.start.offset <= start && end <= span.end.offset)
                {
                    attachment.ambiguous = true;
                    continue;
                }

                let prev = child_spans
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, span)| span.end.offset <= start)
                    .map(|(idx, _)| idx);
                let next = child_spans
                    .iter()
                    .enumerate()
                    .find(|(_, span)| span.start.offset >= end)
                    .map(|(idx, _)| idx);

                match (prev, next) {
                    (Some(prev_idx), Some(next_idx))
                        if prev_idx + 1 == next_idx
                            && child_spans[prev_idx].end.line == comment.line =>
                    {
                        attachment.trailing[prev_idx].push(comment);
                        self.claimed[index] = true;
                    }
                    (Some(prev_idx), None) if child_spans[prev_idx].end.line == comment.line => {
                        attachment.trailing[prev_idx].push(comment);
                        self.claimed[index] = true;
                    }
                    _ => attachment.ambiguous = true,
                }
                continue;
            }

            if child_spans
                .iter()
                .any(|span| span.start.offset <= start && end <= span.end.offset)
            {
                continue;
            }

            if end <= first_child_start {
                attachment.leading[0].push(comment);
                self.claimed[index] = true;
                continue;
            }

            let next = child_spans
                .iter()
                .enumerate()
                .find(|(_, span)| span.start.offset >= end)
                .map(|(idx, _)| idx);

            if let Some(next_idx) = next {
                attachment.leading[next_idx].push(comment);
                self.claimed[index] = true;
            } else if start >= last_child_end {
                attachment.dangling.push(comment);
                self.claimed[index] = true;
            }
        }

        attachment
    }

    pub fn take_leading_before(&mut self, line: usize) -> Vec<SourceComment<'a>> {
        let mut taken = Vec::new();
        for (index, comment) in self.items.iter().copied().enumerate() {
            if self.claimed[index] {
                continue;
            }
            if comment.line < line || (comment.line == line && comment.inline) {
                self.claimed[index] = true;
                taken.push(comment);
            }
        }
        taken
    }

    pub fn take_inline_for_line(&mut self, line: usize) -> Vec<SourceComment<'a>> {
        let mut taken = Vec::new();
        for (index, comment) in self.items.iter().copied().enumerate() {
            if self.claimed[index] {
                continue;
            }
            if comment.line == line && comment.inline {
                self.claimed[index] = true;
                taken.push(comment);
            }
        }
        taken
    }

    pub fn take_remaining(&mut self) -> Vec<SourceComment<'a>> {
        let mut remaining = Vec::new();
        for (index, comment) in self.items.iter().copied().enumerate() {
            if self.claimed[index] {
                continue;
            }
            self.claimed[index] = true;
            remaining.push(comment);
        }
        remaining
    }
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

fn line_number_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(index) => index + 1,
        Err(index) => index,
    }
}

fn span_for_offsets(source: &str, start: usize, end: usize) -> Span {
    let start_text = &source[..start];
    let text = &source[start..end];
    let line = start_text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = start_text
        .rsplit_once('\n')
        .map_or(start_text.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    let start_position = shuck_ast::Position {
        line,
        column,
        offset: start,
    };
    let end_position = start_position.advanced_by(text);
    Span::from_positions(start_position, end_position)
}

fn is_inline_comment(source: &str, start: usize) -> bool {
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    source[line_start..start]
        .chars()
        .any(|character| !character.is_whitespace())
}
