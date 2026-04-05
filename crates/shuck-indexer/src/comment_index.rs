use std::ops::Range;

use shuck_ast::{Comment, TextRange, TextSize};

use crate::LineIndex;

/// A source comment with resolved positional metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedComment {
    /// Byte range of the comment in source (including the `#`).
    pub range: TextRange,
    /// The 1-based line number this comment appears on.
    pub line: usize,
    /// Whether this comment is the only non-whitespace content on its line.
    pub is_own_line: bool,
}

/// Comment ranges and position metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentIndex {
    comments: Vec<IndexedComment>,
    line_comment_ranges: Vec<Range<usize>>,
}

impl CommentIndex {
    /// Build from parser comments and source text.
    pub fn new(source: &str, line_index: &LineIndex, comments: &[Comment]) -> Self {
        let mut indexed_comments = comments
            .iter()
            .filter(|comment| {
                let start = usize::from(comment.range.start());
                let end = usize::from(comment.range.end());
                end <= source.len()
                    && source.is_char_boundary(start)
                    && source.is_char_boundary(end)
            })
            .map(|comment| {
                let line = line_index.line_number(comment.range.start());
                let line_range = line_index
                    .line_range(line, source)
                    .unwrap_or_else(|| TextRange::new(comment.range.start(), comment.range.end()));
                let before_comment =
                    &source[usize::from(line_range.start())..usize::from(comment.range.start())];
                // A comment may span past the line end (e.g. parser bug or
                // multi-line heredoc comment). Clamp to avoid panicking.
                let after_end = usize::from(comment.range.end()).min(usize::from(line_range.end()));
                let after_comment = &source[after_end..usize::from(line_range.end())];

                IndexedComment {
                    range: comment.range,
                    line,
                    is_own_line: is_horizontal_whitespace(before_comment)
                        && is_horizontal_whitespace(after_comment),
                }
            })
            .collect::<Vec<_>>();

        indexed_comments.sort_unstable_by_key(|comment| {
            (comment.range.start().to_u32(), comment.range.end().to_u32())
        });

        let mut counts = vec![0usize; line_index.line_count()];
        for comment in &indexed_comments {
            counts[comment.line - 1] += 1;
        }

        let mut start = 0usize;
        let line_comment_ranges = counts
            .into_iter()
            .map(|count| {
                let range = start..start + count;
                start += count;
                range
            })
            .collect();

        Self {
            comments: indexed_comments,
            line_comment_ranges,
        }
    }

    /// All comments in source order.
    pub fn comments(&self) -> &[IndexedComment] {
        &self.comments
    }

    /// Comments on a specific 1-based line.
    pub fn comments_on_line(&self, line: usize) -> &[IndexedComment] {
        let Some(range) = line
            .checked_sub(1)
            .and_then(|index| self.line_comment_ranges.get(index))
        else {
            return &[];
        };

        &self.comments[range.start..range.end]
    }

    /// Whether the given byte offset falls inside a comment.
    pub fn is_comment(&self, offset: TextSize) -> bool {
        let index = self
            .comments
            .partition_point(|comment| comment.range.start() <= offset);
        index
            .checked_sub(1)
            .and_then(|candidate| self.comments.get(candidate))
            .is_some_and(|comment| contains(comment.range, offset))
    }
}

fn contains(range: TextRange, offset: TextSize) -> bool {
    range.start() <= offset && offset < range.end()
}

fn is_horizontal_whitespace(text: &str) -> bool {
    text.chars().all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn comments(source: &str) -> CommentIndex {
        let output = Parser::new(source).parse().unwrap();
        let lines = LineIndex::new(source);
        CommentIndex::new(source, &lines, &output.comments)
    }

    #[test]
    fn distinguishes_own_line_and_inline_comments() {
        let source = "# head\necho hi # tail\n";
        let index = comments(source);

        assert!(index.comments()[0].is_own_line);
        assert!(!index.comments()[1].is_own_line);
        assert_eq!(index.comments_on_line(2).len(), 1);
    }

    #[test]
    fn includes_shebang_and_supports_point_queries() {
        let source = "#!/bin/bash\necho ok\n";
        let index = comments(source);

        let shebang_offset = TextSize::new(0);
        let echo_offset = TextSize::new(source.find("echo").unwrap() as u32);

        assert_eq!(index.comments().len(), 1);
        assert!(index.is_comment(shebang_offset));
        assert!(!index.is_comment(echo_offset));
    }

    #[test]
    fn groups_comments_by_line() {
        let source = "echo hi # one\n# two\n";
        let index = comments(source);

        assert_eq!(index.comments_on_line(1).len(), 1);
        assert_eq!(index.comments_on_line(2).len(), 1);
        assert!(index.comments_on_line(3).is_empty());
    }
}
