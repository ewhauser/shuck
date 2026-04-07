use std::ops::Range;

use shuck_ast::{Comment, Command, CompoundCommand, File, Stmt, StmtSeq, TextRange, TextSize};

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
    /// Build from AST-owned comments and source text.
    pub fn new(source: &str, line_index: &LineIndex, file: &File) -> Self {
        let mut comments = Vec::new();
        collect_file_comments(file, &mut comments);
        let mut indexed_comments = comments
            .into_iter()
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

fn collect_file_comments(file: &File, comments: &mut Vec<Comment>) {
    collect_stmt_seq_comments(&file.body, comments);
}

fn collect_stmt_seq_comments(sequence: &StmtSeq, comments: &mut Vec<Comment>) {
    comments.extend(sequence.leading_comments.iter().copied());
    for stmt in sequence.iter() {
        collect_stmt_comments(stmt, comments);
    }
    comments.extend(sequence.trailing_comments.iter().copied());
}

fn collect_stmt_comments(stmt: &Stmt, comments: &mut Vec<Comment>) {
    comments.extend(stmt.leading_comments.iter().copied());
    if let Some(comment) = stmt.inline_comment {
        comments.push(comment);
    }
    collect_command_comments(&stmt.command, comments);
}

fn collect_command_comments(command: &Command, comments: &mut Vec<Comment>) {
    match command {
        Command::Binary(command) => {
            collect_stmt_comments(&command.left, comments);
            collect_stmt_comments(&command.right, comments);
        }
        Command::Compound(command) => collect_compound_comments(command, comments),
        Command::Function(function) => collect_stmt_comments(&function.body, comments),
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_compound_comments(command: &CompoundCommand, comments: &mut Vec<Comment>) {
    match command {
        CompoundCommand::If(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.then_branch, comments);
            for (condition, body) in &command.elif_branches {
                collect_stmt_seq_comments(condition, comments);
                collect_stmt_seq_comments(body, comments);
            }
            if let Some(body) = &command.else_branch {
                collect_stmt_seq_comments(body, comments);
            }
        }
        CompoundCommand::For(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::ArithmeticFor(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::While(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Until(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Case(command) => {
            for case in &command.cases {
                collect_stmt_seq_comments(&case.body, comments);
            }
        }
        CompoundCommand::Select(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_stmt_seq_comments(body, comments);
        }
        CompoundCommand::Time(command) => {
            if let Some(inner) = &command.command {
                collect_stmt_comments(inner, comments);
            }
        }
        CompoundCommand::Coproc(command) => collect_stmt_comments(&command.body, comments),
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn comments(source: &str) -> CommentIndex {
        let output = Parser::new(source).parse().unwrap();
        let lines = LineIndex::new(source);
        CommentIndex::new(source, &lines, &output.file)
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
