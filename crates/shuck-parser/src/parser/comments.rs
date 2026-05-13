use super::*;

impl<'a> Parser<'a> {
    pub(super) fn comment_start(comment: Comment) -> usize {
        usize::from(comment.range.start())
    }

    pub(super) fn is_inline_comment(source: &str, stmt: &Stmt, comment: Comment) -> bool {
        let comment_start = Self::comment_start(comment);
        if comment_start < stmt.span.end.offset {
            return false;
        }
        source
            .get(stmt.span.end.offset..comment_start)
            .is_some_and(|gap| !gap.contains('\n'))
    }

    pub(super) fn take_comments_before(
        comments: &mut VecDeque<Comment>,
        end_offset: usize,
    ) -> VecDeque<Comment> {
        let mut taken = VecDeque::new();
        while comments
            .front()
            .is_some_and(|comment| Self::comment_start(*comment) < end_offset)
        {
            let Some(comment) = comments.pop_front() else {
                unreachable!("front comment should exist while draining");
            };
            taken.push_back(comment);
        }
        taken
    }

    pub(super) fn attach_comments_to_file(&self, file: &mut File) {
        let mut comments = self.comments.iter().copied().collect::<VecDeque<_>>();
        Self::attach_comments_to_stmt_seq_with_source(self.input, &mut file.body, &mut comments);
        file.body.trailing_comments.extend(comments);
    }

    pub(super) fn attach_comments_to_stmt_seq_with_source(
        source: &str,
        sequence: &mut StmtSeq,
        comments: &mut VecDeque<Comment>,
    ) {
        if sequence.stmts.is_empty() {
            sequence
                .trailing_comments
                .extend(Self::take_comments_before(
                    comments,
                    sequence.span.end.offset,
                ));
            return;
        }

        for (index, stmt) in sequence.stmts.iter_mut().enumerate() {
            let leading = Self::take_comments_before(comments, stmt.span.start.offset);
            if index == 0 {
                sequence.leading_comments.extend(leading);
            } else {
                stmt.leading_comments.extend(leading);
            }

            let mut nested = Self::take_comments_before(comments, stmt.span.end.offset);
            Self::attach_comments_to_stmt_with_source(source, stmt, &mut nested);
            if !nested.is_empty() {
                stmt.leading_comments.extend(nested);
            }

            if stmt.inline_comment.is_none()
                && comments
                    .front()
                    .is_some_and(|comment| Self::is_inline_comment(source, stmt, *comment))
            {
                stmt.inline_comment = comments.pop_front();
            }
        }

        sequence
            .trailing_comments
            .extend(Self::take_comments_before(
                comments,
                sequence.span.end.offset,
            ));
    }

    pub(super) fn attach_comments_to_stmt_with_source(
        source: &str,
        stmt: &mut Stmt,
        comments: &mut VecDeque<Comment>,
    ) {
        match &mut stmt.command {
            AstCommand::Binary(binary) => {
                let mut left_comments =
                    Self::take_comments_before(comments, binary.left.span.end.offset);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    binary.left.as_mut(),
                    &mut left_comments,
                );
                if !left_comments.is_empty() {
                    binary.left.leading_comments.extend(left_comments);
                }

                let mut right_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    binary.right.as_mut(),
                    &mut right_comments,
                );
                if !right_comments.is_empty() {
                    binary.right.leading_comments.extend(right_comments);
                }
            }
            AstCommand::Compound(compound) => {
                Self::attach_comments_to_compound_with_source(source, compound, comments);
            }
            AstCommand::Function(function) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    function.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    function.body.leading_comments.extend(body_comments);
                }
            }
            AstCommand::AnonymousFunction(function) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    function.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    function.body.leading_comments.extend(body_comments);
                }
            }
            AstCommand::Simple(_) | AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
        }
    }

    pub(super) fn attach_comments_to_compound_with_source(
        source: &str,
        command: &mut CompoundCommand,
        comments: &mut VecDeque<Comment>,
    ) {
        match command {
            CompoundCommand::If(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut then_branch =
                    Self::take_comments_before(comments, command.then_branch.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.then_branch,
                    &mut then_branch,
                );
                command.then_branch.trailing_comments.extend(then_branch);

                for (condition_seq, body_seq) in &mut command.elif_branches {
                    let mut elif_condition =
                        Self::take_comments_before(comments, condition_seq.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        condition_seq,
                        &mut elif_condition,
                    );
                    condition_seq.trailing_comments.extend(elif_condition);

                    let mut elif_body =
                        Self::take_comments_before(comments, body_seq.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(source, body_seq, &mut elif_body);
                    body_seq.trailing_comments.extend(elif_body);
                }

                if let Some(else_branch) = &mut command.else_branch {
                    let mut else_comments = std::mem::take(comments);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        else_branch,
                        &mut else_comments,
                    );
                    else_branch.trailing_comments.extend(else_comments);
                }
            }
            CompoundCommand::For(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Repeat(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Foreach(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::ArithmeticFor(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::While(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Until(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Case(command) => {
                for case in &mut command.cases {
                    let mut body_comments =
                        Self::take_comments_before(comments, case.body.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        &mut case.body,
                        &mut body_comments,
                    );
                    case.body.trailing_comments.extend(body_comments);
                }
            }
            CompoundCommand::Select(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(source, body, &mut body_comments);
                body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Always(command) => {
                let mut body_comments =
                    Self::take_comments_before(comments, command.body.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);

                let mut always_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.always_body,
                    &mut always_comments,
                );
                command
                    .always_body
                    .trailing_comments
                    .extend(always_comments);
            }
            CompoundCommand::Time(command) => {
                if let Some(inner) = &mut command.command {
                    let mut inner_comments = std::mem::take(comments);
                    Self::attach_comments_to_stmt_with_source(
                        source,
                        inner.as_mut(),
                        &mut inner_comments,
                    );
                    if !inner_comments.is_empty() {
                        inner.leading_comments.extend(inner_comments);
                    }
                }
            }
            CompoundCommand::Coproc(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    command.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    command.body.leading_comments.extend(body_comments);
                }
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
        }
    }
}
