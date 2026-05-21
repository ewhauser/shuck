use super::*;

pub(crate) fn command_group_commands(command: &Command) -> Option<(&StmtSeq, char)> {
    match command {
        Command::Compound(CompoundCommand::BraceGroup(commands)) => Some((commands, '{')),
        Command::Compound(CompoundCommand::Subshell(commands)) => Some((commands, '(')),
        _ => None,
    }
}

pub(crate) fn branch_open_keyword_start(
    sequence: &StmtSeq,
    source: &str,
    keyword: &str,
) -> Option<usize> {
    let first = sequence.first()?;
    last_uncommented_shell_keyword_before(source, stmt_span(first).start.offset, keyword)
}

pub(crate) fn if_next_branch_region_with_body_end(
    command: &IfCommand,
    branch_index: usize,
    source: &str,
    mut branch_body_end: impl FnMut(&StmtSeq) -> usize,
) -> Option<(usize, usize)> {
    let current_branch_end = if branch_index == 0 {
        branch_body_end(&command.then_branch)
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| branch_body_end(body))
            .unwrap_or_else(|| branch_body_end(&command.then_branch))
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        let keyword = branch_keyword_offset(
            source,
            current_branch_end,
            condition.span.start.offset,
            "elif",
        )
        .unwrap_or(condition.span.start.offset);
        Some((current_branch_end, keyword))
    } else if branch_index == command.elif_branches.len() {
        command.else_branch.as_ref().map(|body| {
            let keyword =
                branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
                    .unwrap_or(body.span.start.offset);
            (current_branch_end, keyword)
        })
    } else {
        None
    }
}

pub(crate) fn collect_pipeline_parts<'a, T>(
    command: &'a BinaryCommand,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<T>,
    operator_for: &impl Fn(&BinaryCommand) -> T,
) {
    collect_pipeline_stmt_parts(command.left.as_ref(), statements, operators, operator_for);
    operators.push(operator_for(command));
    collect_pipeline_stmt_parts(command.right.as_ref(), statements, operators, operator_for);
}

fn collect_pipeline_stmt_parts<'a, T>(
    stmt: &'a Stmt,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<T>,
    operator_for: &impl Fn(&BinaryCommand) -> T,
) {
    if let Some(binary) = stmt_plain_pipeline_binary(stmt) {
        collect_pipeline_parts(binary, statements, operators, operator_for);
    } else {
        statements.push(stmt);
    }
}

fn stmt_plain_pipeline_binary(stmt: &Stmt) -> Option<&BinaryCommand> {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(
            binary.op,
            shuck_ast::BinaryOp::Pipe | shuck_ast::BinaryOp::PipeAll
        )
    {
        Some(binary)
    } else {
        None
    }
}

pub(crate) fn collect_binary_list_first<'a, T>(
    command: &'a BinaryCommand,
    rest: &mut Vec<T>,
    item_for: &impl Fn(&'a BinaryCommand) -> T,
) -> &'a Stmt {
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_binary_list_first(left_binary, rest, item_for);
        rest.push(item_for(command));
        return first;
    }

    let first = command.left.as_ref();
    rest.push(item_for(command));
    first
}
