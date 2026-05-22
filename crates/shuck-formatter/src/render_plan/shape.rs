use shuck_ast::{
    Command, CompoundCommand, ForCommand, IfCommand, IfSyntax, RepeatCommand, SelectCommand, Span,
    Stmt, StmtSeq, StmtTerminator, UntilCommand, WhileCommand,
};

use crate::command::{stmt_format_span, stmt_span};
use crate::context::RenderContext;

pub(crate) fn can_inline_body(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    enclosing_span: Span,
) -> bool {
    can_inline_body_with_upper_bound(
        context,
        commands,
        enclosing_span,
        Some(enclosing_span.end.offset),
    )
}

pub(crate) fn can_inline_body_with_upper_bound(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    enclosing_span: Span,
    upper_bound: Option<usize>,
) -> bool {
    let [command] = commands.as_slice() else {
        return false;
    };
    if matches!(command.terminator, Some(StmtTerminator::Background(_)))
        || !can_inline_stmt(context, command)
    {
        return false;
    }

    if context.facts.sequence(commands, upper_bound).has_comments() {
        return false;
    }

    context.options.compact_layout() || stmt_span(command).start.line == enclosing_span.start.line
}

pub(crate) fn can_inline_stmt(context: RenderContext<'_, '_>, stmt: &Stmt) -> bool {
    let stmt_facts = context.facts.stmt(stmt);
    if stmt_facts.preserve_verbatim() || stmt_facts.has_trailing_comment() {
        return false;
    }

    matches!(
        &stmt.command,
        Command::Simple(_)
            | Command::Builtin(_)
            | Command::Decl(_)
            | Command::Function(_)
            | Command::Binary(_)
            | Command::Compound(
                CompoundCommand::Conditional(_)
                    | CompoundCommand::Arithmetic(_)
                    | CompoundCommand::Time(_)
            )
    )
}

pub(crate) fn can_inline_else_branch_close(
    context: RenderContext<'_, '_>,
    command: &IfCommand,
    body: &StmtSeq,
    fi_span: Span,
) -> bool {
    let [stmt] = body.as_slice() else {
        return false;
    };
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        || !can_inline_stmt(context, stmt)
        || context
            .facts
            .sequence(body, Some(fi_span.start.offset))
            .has_comments()
    {
        return false;
    };
    let Some((_, else_offset)) = context
        .facts
        .if_next_branch_region(command, command.elif_branches.len())
    else {
        return false;
    };
    let else_line = context.source_map().line_number_for_offset(else_offset);
    let body_line = stmt_span(stmt).start.line;
    else_line == body_line && body_line == fi_span.start.line
}

pub(crate) fn can_inline_if_chain(
    context: RenderContext<'_, '_>,
    command: &IfCommand,
    fi_span: Span,
) -> bool {
    if command.elif_branches.is_empty() || command.span.start.line != fi_span.end.line {
        return false;
    }

    if !can_inline_body_with_upper_bound(
        context,
        &command.then_branch,
        command.span,
        Some(context.facts.if_branch_upper_bound(command, 0)),
    ) {
        return false;
    }

    for (index, (_, body)) in command.elif_branches.iter().enumerate() {
        if !can_inline_body_with_upper_bound(
            context,
            body,
            command.span,
            Some(context.facts.if_branch_upper_bound(command, index + 1)),
        ) {
            return false;
        }
    }

    command.else_branch.as_ref().is_none_or(|body| {
        can_inline_body_with_upper_bound(context, body, command.span, Some(fi_span.start.offset))
    })
}

pub(crate) fn then_branch_starts_with_inline_if(
    context: RenderContext<'_, '_>,
    command: &IfCommand,
    then_span: Span,
    fi_span: Span,
) -> bool {
    if command.span.start.line != fi_span.end.line {
        return false;
    }
    let [stmt] = command.then_branch.as_slice() else {
        return false;
    };
    if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
        return false;
    }
    let Command::Compound(CompoundCommand::If(inner)) = &stmt.command else {
        return false;
    };
    matches!(inner.syntax, IfSyntax::ThenFi { .. })
        && then_span.end.line == inner.span.start.line
        && !context
            .facts
            .sequence(
                &command.then_branch,
                Some(context.facts.if_branch_upper_bound(command, 0)),
            )
            .has_comments()
}

pub(crate) fn can_inline_group(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    open_char: char,
) -> bool {
    let [command] = commands.as_slice() else {
        return false;
    };

    can_inline_stmt(context, command)
        && can_inline_body(context, commands, stmt_span(command))
        && (stmt_span(command).start.line == stmt_span(command).end.line
            || group_delimiters_attach_to_wrapped_body(context, commands, open_char))
}

pub(crate) fn group_has_inline_source_shape(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    open_char: char,
) -> bool {
    context.facts.group_was_inline_in_source(commands)
        || group_delimiters_attach_to_wrapped_body(context, commands, open_char)
}

pub(crate) fn group_delimiters_attach_to_wrapped_body(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    _open_char: char,
) -> bool {
    let (Some(first), Some(last)) = (commands.first(), commands.last()) else {
        return false;
    };
    let Some(group_span) = context
        .facts
        .sequence(commands, None)
        .group_attachment_span()
    else {
        return false;
    };

    group_span.start.line == stmt_format_span(first).start.line
        && group_span.end.line == stmt_format_span(last).end.line
}

pub(crate) fn can_inline_source_line_subshell(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    upper_bound: Option<usize>,
) -> bool {
    let [stmt] = commands.as_slice() else {
        return false;
    };
    if context.facts.sequence(commands, upper_bound).has_comments()
        || context.facts.stmt(stmt).preserve_verbatim()
        || context.facts.stmt(stmt).has_trailing_comment()
    {
        return false;
    }
    if commands.span.start.line != commands.span.end.line {
        return false;
    }

    true
}

pub(crate) fn can_format_multiline_subshell_inline(
    context: RenderContext<'_, '_>,
    commands: &StmtSeq,
    upper_bound: Option<usize>,
) -> bool {
    let [stmt] = commands.as_slice() else {
        return false;
    };
    if context
        .facts
        .sequence(commands, upper_bound)
        .group_open_suffix_span()
        .is_some()
        || context.facts.sequence(commands, upper_bound).has_comments()
    {
        return false;
    }
    let Some(group_span) = context
        .facts
        .sequence(commands, upper_bound)
        .group_attachment_span()
    else {
        return false;
    };
    let group_source = group_span.slice(context.source);
    if !group_source.contains('\n')
        || group_source.contains("\\\n")
        || group_source.contains("\\\r\n")
    {
        return false;
    }

    let first_start = stmt_span(stmt).start.offset.min(context.source.len());
    let open_end = group_span.start.offset.saturating_add('('.len_utf8());
    if context
        .source
        .get(open_end..first_start)
        .is_none_or(|between| between.contains('\n'))
    {
        return false;
    }

    let close_offset =
        group_close_offset(context.source, group_span, upper_bound, ')', ')'.len_utf8());
    let stmt_end = stmt_span(stmt)
        .end
        .offset
        .min(close_offset)
        .min(context.source.len());
    context
        .source
        .get(stmt_end..close_offset)
        .is_some_and(|between| !between.contains('\n'))
}

pub(crate) fn body_starts_with_inline_do_brace_group(
    context: RenderContext<'_, '_>,
    body: &StmtSeq,
) -> bool {
    let Some(CompoundCommand::BraceGroup(commands)) = single_unadorned_compound_stmt(body) else {
        return false;
    };
    let Some(group_span) = context
        .facts
        .sequence(commands, None)
        .group_attachment_span()
    else {
        return false;
    };
    source_line_before_offset_ends_with_do(context.source, group_span.start.offset)
}

pub(crate) fn body_starts_with_inline_do_if(
    context: RenderContext<'_, '_>,
    body: &StmtSeq,
) -> bool {
    let Some(CompoundCommand::If(command)) = single_unadorned_compound_stmt(body) else {
        return false;
    };
    if !matches!(command.syntax, IfSyntax::ThenFi { .. }) {
        return false;
    }
    source_line_before_offset_ends_with_do(context.source, command.span.start.offset)
}

pub(crate) fn inline_do_brace_group_done_separator(
    context: RenderContext<'_, '_>,
    body: &StmtSeq,
    enclosing_span: Span,
) -> &'static str {
    let [stmt] = body.as_slice() else {
        return "; ";
    };
    let Command::Compound(CompoundCommand::BraceGroup(commands)) = &stmt.command else {
        return "; ";
    };
    let Some(group_span) = context
        .facts
        .sequence(commands, None)
        .group_attachment_span()
    else {
        return "; ";
    };
    let between = context
        .source
        .get(group_span.end.offset..enclosing_span.end.offset)
        .unwrap_or_default()
        .trim_start_matches([' ', '\t', '\r']);
    if between.starts_with(';') {
        return "; ";
    }
    if brace_group_last_stmt_allows_done_without_semicolon(commands) {
        " "
    } else {
        "; "
    }
}

fn single_unadorned_compound_stmt(body: &StmtSeq) -> Option<&CompoundCommand> {
    let [stmt] = body.as_slice() else {
        return None;
    };
    if stmt.negated || !stmt.redirects.is_empty() || stmt.terminator.is_some() {
        return None;
    }
    match &stmt.command {
        Command::Compound(command) => Some(command),
        _ => None,
    }
}

fn source_line_before_offset_ends_with_do(source: &str, offset: usize) -> bool {
    let line_start = source[..offset]
        .rfind('\n')
        .map_or(0, |offset| offset.saturating_add(1));
    source[line_start..offset]
        .trim_end_matches([' ', '\t', '\r'])
        .ends_with("do")
}

fn brace_group_last_stmt_allows_done_without_semicolon(commands: &StmtSeq) -> bool {
    let Some(last) = commands.last() else {
        return false;
    };
    command_allows_done_without_semicolon(&last.command)
}

fn command_allows_done_without_semicolon(command: &Command) -> bool {
    match command {
        Command::Compound(command) => compound_allows_done_without_semicolon(command),
        Command::Binary(binary) => command_allows_done_without_semicolon(&binary.right.command),
        _ => false,
    }
}

fn compound_allows_done_without_semicolon(command: &CompoundCommand) -> bool {
    match command {
        CompoundCommand::Case(_) => true,
        CompoundCommand::BraceGroup(commands)
        | CompoundCommand::For(ForCommand { body: commands, .. })
        | CompoundCommand::Repeat(RepeatCommand { body: commands, .. })
        | CompoundCommand::While(WhileCommand { body: commands, .. })
        | CompoundCommand::Until(UntilCommand { body: commands, .. })
        | CompoundCommand::Select(SelectCommand { body: commands, .. }) => {
            brace_group_last_stmt_allows_done_without_semicolon(commands)
        }
        CompoundCommand::ArithmeticFor(command) => {
            brace_group_last_stmt_allows_done_without_semicolon(&command.body)
        }
        _ => false,
    }
}

fn group_close_offset(
    source: &str,
    span: Span,
    upper_bound: Option<usize>,
    close_char: char,
    close_len: usize,
) -> usize {
    let fallback = span.end.offset.saturating_sub(close_len);
    let search_end = upper_bound
        .map(|offset| offset.saturating_add(close_len))
        .unwrap_or(span.end.offset)
        .min(source.len())
        .max(span.start.offset);
    source
        .get(span.start.offset..search_end)
        .and_then(|text| text.rfind(close_char))
        .map_or(fallback, |offset| span.start.offset + offset)
}
