use shuck_ast::{Command, Comment, CompoundCommand, File, Span, Stmt, StmtSeq};

pub(crate) fn flatten_comments(file: &File) -> Vec<Comment> {
    let mut comments = Vec::new();
    collect_stmt_seq_comments(&file.body, &mut comments);
    let heredoc_body_spans = heredoc_body_spans(file);
    comments.retain(|comment| {
        let offset = usize::from(comment.range.start());
        !heredoc_body_spans
            .iter()
            .any(|span| span_contains_offset(*span, offset))
    });
    comments
}

pub(crate) fn heredoc_body_spans(file: &File) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_stmt_seq_heredoc_body_spans(&file.body, &mut spans);
    spans
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
        Command::AnonymousFunction(function) => collect_stmt_comments(&function.body, comments),
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_stmt_seq_heredoc_body_spans(sequence: &StmtSeq, spans: &mut Vec<Span>) {
    for stmt in sequence.iter() {
        collect_stmt_heredoc_body_spans(stmt, spans);
    }
}

fn collect_stmt_heredoc_body_spans(stmt: &Stmt, spans: &mut Vec<Span>) {
    for redirect in &stmt.redirects {
        if let Some(heredoc) = redirect.heredoc() {
            spans.push(heredoc.body.span);
        }
    }
    match &stmt.command {
        Command::Binary(command) => {
            collect_stmt_heredoc_body_spans(&command.left, spans);
            collect_stmt_heredoc_body_spans(&command.right, spans);
        }
        Command::Compound(command) => collect_compound_heredoc_body_spans(command, spans),
        Command::Function(function) => collect_stmt_heredoc_body_spans(&function.body, spans),
        Command::AnonymousFunction(function) => {
            collect_stmt_heredoc_body_spans(&function.body, spans);
        }
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
    }
}

fn collect_compound_heredoc_body_spans(command: &CompoundCommand, spans: &mut Vec<Span>) {
    match command {
        CompoundCommand::If(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.condition, spans);
            collect_stmt_seq_heredoc_body_spans(&command.then_branch, spans);
            for (condition, body) in &command.elif_branches {
                collect_stmt_seq_heredoc_body_spans(condition, spans);
                collect_stmt_seq_heredoc_body_spans(body, spans);
            }
            if let Some(body) = &command.else_branch {
                collect_stmt_seq_heredoc_body_spans(body, spans);
            }
        }
        CompoundCommand::For(command) => collect_stmt_seq_heredoc_body_spans(&command.body, spans),
        CompoundCommand::Repeat(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::Foreach(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::ArithmeticFor(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::While(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.condition, spans);
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::Until(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.condition, spans);
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::Case(command) => {
            for case in &command.cases {
                collect_stmt_seq_heredoc_body_spans(&case.body, spans);
            }
        }
        CompoundCommand::Select(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
        }
        CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
            collect_stmt_seq_heredoc_body_spans(body, spans);
        }
        CompoundCommand::Always(command) => {
            collect_stmt_seq_heredoc_body_spans(&command.body, spans);
            collect_stmt_seq_heredoc_body_spans(&command.always_body, spans);
        }
        CompoundCommand::Time(command) => {
            if let Some(inner) = &command.command {
                collect_stmt_heredoc_body_spans(inner, spans);
            }
        }
        CompoundCommand::Coproc(command) => collect_stmt_heredoc_body_spans(&command.body, spans),
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
    }
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset < span.end.offset
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
        CompoundCommand::Repeat(command) => collect_stmt_seq_comments(&command.body, comments),
        CompoundCommand::Foreach(command) => collect_stmt_seq_comments(&command.body, comments),
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
