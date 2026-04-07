use shuck_ast::{Command, Comment, CompoundCommand, File, Stmt, StmtSeq};

pub(crate) fn flatten_comments(file: &File) -> Vec<Comment> {
    let mut comments = Vec::new();
    collect_stmt_seq_comments(&file.body, &mut comments);
    comments
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
    match &stmt.command {
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
            for case in &command.cases {
                collect_stmt_seq_comments(&case.body, comments);
            }
        }
        CompoundCommand::Select(command) => collect_stmt_seq_comments(&command.body, comments),
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
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}
