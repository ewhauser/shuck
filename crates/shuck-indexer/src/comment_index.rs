use std::ops::Range;

use shuck_ast::{
    ArrayElem, Assignment, AssignmentValue, BuiltinCommand, Command, Comment, CompoundCommand,
    ConditionalExpr, DeclOperand, File, Pattern, PatternPart, Redirect, Stmt, StmtSeq, TextRange,
    TextSize, Word, WordPart, WordPartNode,
};

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
    collect_redirect_comments(&stmt.redirects, comments);
    collect_command_comments(&stmt.command, comments);
}

fn collect_command_comments(command: &Command, comments: &mut Vec<Comment>) {
    match command {
        Command::Simple(command) => {
            collect_word_comments(&command.name, comments);
            for argument in &command.args {
                collect_word_comments(argument, comments);
            }
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
        }
        Command::Builtin(command) => collect_builtin_comments(command, comments),
        Command::Decl(command) => {
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_comments(word, comments);
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_comments(assignment, comments);
                    }
                    DeclOperand::Name(reference) => {
                        if let Some(subscript) = &reference.subscript
                            && let Some(expression) = &subscript.arithmetic_ast
                        {
                            collect_arithmetic_expr_comments(expression, comments);
                        }
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_stmt_comments(&command.left, comments);
            collect_stmt_comments(&command.right, comments);
        }
        Command::Compound(command) => collect_compound_comments(command, comments),
        Command::Function(function) => collect_stmt_comments(&function.body, comments),
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
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                for word in words {
                    collect_word_comments(word, comments);
                }
            }
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Repeat(command) => {
            collect_word_comments(&command.count, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Foreach(command) => {
            for word in &command.words {
                collect_word_comments(word, comments);
            }
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::ArithmeticFor(command) => {
            collect_stmt_seq_comments(&command.body, comments)
        }
        CompoundCommand::While(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Until(command) => {
            collect_stmt_seq_comments(&command.condition, comments);
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Case(command) => {
            collect_word_comments(&command.word, comments);
            for case in &command.cases {
                for pattern in &case.patterns {
                    collect_pattern_comments(pattern, comments);
                }
                collect_stmt_seq_comments(&case.body, comments);
            }
        }
        CompoundCommand::Select(command) => {
            for word in &command.words {
                collect_word_comments(word, comments);
            }
            collect_stmt_seq_comments(&command.body, comments);
        }
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_stmt_seq_comments(body, comments);
        }
        CompoundCommand::Always(command) => {
            collect_stmt_seq_comments(&command.body, comments);
            collect_stmt_seq_comments(&command.always_body, comments);
        }
        CompoundCommand::Time(command) => {
            if let Some(inner) = &command.command {
                collect_stmt_comments(inner, comments);
            }
        }
        CompoundCommand::Coproc(command) => collect_stmt_comments(&command.body, comments),
        CompoundCommand::Conditional(command) => {
            collect_conditional_expr_comments(&command.expression, comments);
        }
        CompoundCommand::Arithmetic(_) => {}
    }
}

fn collect_builtin_comments(command: &BuiltinCommand, comments: &mut Vec<Comment>) {
    match command {
        BuiltinCommand::Break(command) => {
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
            if let Some(depth) = &command.depth {
                collect_word_comments(depth, comments);
            }
            for argument in &command.extra_args {
                collect_word_comments(argument, comments);
            }
        }
        BuiltinCommand::Continue(command) => {
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
            if let Some(depth) = &command.depth {
                collect_word_comments(depth, comments);
            }
            for argument in &command.extra_args {
                collect_word_comments(argument, comments);
            }
        }
        BuiltinCommand::Return(command) => {
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
            if let Some(code) = &command.code {
                collect_word_comments(code, comments);
            }
            for argument in &command.extra_args {
                collect_word_comments(argument, comments);
            }
        }
        BuiltinCommand::Exit(command) => {
            for assignment in &command.assignments {
                collect_assignment_comments(assignment, comments);
            }
            if let Some(code) = &command.code {
                collect_word_comments(code, comments);
            }
            for argument in &command.extra_args {
                collect_word_comments(argument, comments);
            }
        }
    }
}

fn collect_assignment_comments(assignment: &Assignment, comments: &mut Vec<Comment>) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_comments(word, comments),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => collect_word_comments(word, comments),
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_comments(value, comments)
                    }
                }
            }
        }
    }
}

fn collect_redirect_comments(redirects: &[Redirect], comments: &mut Vec<Comment>) {
    for redirect in redirects {
        if let Some(word) = redirect.word_target() {
            collect_word_comments(word, comments);
        }
        if let Some(heredoc) = redirect.heredoc() {
            collect_word_comments(&heredoc.body, comments);
        }
    }
}

fn collect_pattern_comments(pattern: &Pattern, comments: &mut Vec<Comment>) {
    for part in &pattern.parts {
        if let PatternPart::Word(word) = &part.kind {
            collect_word_comments(word, comments);
        }
    }
}

fn collect_conditional_expr_comments(expression: &ConditionalExpr, comments: &mut Vec<Comment>) {
    match expression {
        ConditionalExpr::Binary(expression) => {
            collect_conditional_expr_comments(&expression.left, comments);
            collect_conditional_expr_comments(&expression.right, comments);
        }
        ConditionalExpr::Unary(expression) => {
            collect_conditional_expr_comments(&expression.expr, comments);
        }
        ConditionalExpr::Parenthesized(expression) => {
            collect_conditional_expr_comments(&expression.expr, comments);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_comments(word, comments);
        }
        ConditionalExpr::Pattern(pattern) => collect_pattern_comments(pattern, comments),
        ConditionalExpr::VarRef(reference) => {
            if let Some(subscript) = &reference.subscript
                && let Some(expression) = &subscript.arithmetic_ast
            {
                collect_arithmetic_expr_comments(expression, comments);
            }
        }
    }
}

fn collect_arithmetic_expr_comments(
    expression: &shuck_ast::ArithmeticExprNode,
    comments: &mut Vec<Comment>,
) {
    match &expression.kind {
        shuck_ast::ArithmeticExpr::ShellWord(word) => collect_word_comments(word, comments),
        shuck_ast::ArithmeticExpr::Indexed { index, .. }
        | shuck_ast::ArithmeticExpr::Parenthesized { expression: index }
        | shuck_ast::ArithmeticExpr::Unary { expr: index, .. }
        | shuck_ast::ArithmeticExpr::Postfix { expr: index, .. } => {
            collect_arithmetic_expr_comments(index, comments);
        }
        shuck_ast::ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_expr_comments(left, comments);
            collect_arithmetic_expr_comments(right, comments);
        }
        shuck_ast::ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_expr_comments(condition, comments);
            collect_arithmetic_expr_comments(then_expr, comments);
            collect_arithmetic_expr_comments(else_expr, comments);
        }
        shuck_ast::ArithmeticExpr::Assignment { target, value, .. } => {
            if let shuck_ast::ArithmeticLvalue::Indexed { index, .. } = target {
                collect_arithmetic_expr_comments(index, comments);
            }
            collect_arithmetic_expr_comments(value, comments);
        }
        shuck_ast::ArithmeticExpr::Number(_) | shuck_ast::ArithmeticExpr::Variable(_) => {}
    }
}

fn collect_word_comments(word: &Word, comments: &mut Vec<Comment>) {
    collect_word_part_comments(&word.parts, comments);
}

fn collect_word_part_comments(parts: &[WordPartNode], comments: &mut Vec<Comment>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_word_part_comments(parts, comments),
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                collect_stmt_seq_comments(body, comments);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression) = expression_ast {
                    collect_arithmetic_expr_comments(expression, comments);
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. } => {}
        }
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
