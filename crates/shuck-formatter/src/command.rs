use shuck_ast::{
    AlwaysCommand, AnonymousFunctionCommand, ArithmeticCommand, ArithmeticForCommand, ArrayElem,
    Assignment, AssignmentValue, BackgroundOperator, BinaryCommand, BinaryOp, BuiltinCommand,
    CaseCommand, CaseItem, CaseTerminator, Command, CompoundCommand, ConditionalBinaryExpr,
    ConditionalCommand, ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr, CoprocCommand,
    DeclClause, DeclOperand, ForCommand, ForSyntax, ForeachCommand, ForeachSyntax, FunctionDef,
    IfCommand, IfSyntax, Redirect, RedirectKind, RepeatCommand, RepeatSyntax, SelectCommand,
    SimpleCommand, SourceText, Span, Stmt, StmtSeq, StmtTerminator, Subscript, TimeCommand,
    UntilCommand, VarRef, WhileCommand,
};
use shuck_format::{
    Document, Format, FormatElement, FormatResult, hard_line_break, indent, space, text, verbatim,
    write,
};

use crate::FormatNodeRule;
use crate::prelude::{AsFormat, ShellFormatter};
use crate::word::{render_pattern_syntax, render_word_syntax};

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCommand;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatStatement;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCompoundCommand;

impl FormatNodeRule<Stmt> for FormatStatement {
    fn fmt(&self, stmt: &Stmt, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let source = formatter.context().source();
        let render_verbatim = {
            let source_map = formatter.context().comments().source_map();
            let options = formatter.context().options();
            should_render_verbatim(stmt, source_map, options)
        };
        if render_verbatim {
            let span = stmt_verbatim_span(stmt, source);
            formatter.context_mut().comments_mut().claim_in_span(span);
            if let Some(document) = verbatim_stmt(stmt, source) {
                return write!(formatter, [document]);
            }
        }

        if stmt.negated {
            write!(formatter, [text("! ")])?;
        }

        match &stmt.command {
            Command::Compound(CompoundCommand::BraceGroup(commands)) => {
                format_brace_group(commands, formatter, Some(stmt_span(stmt).end.offset))?;
            }
            Command::Compound(CompoundCommand::Subshell(commands)) => {
                format_subshell(commands, formatter, Some(stmt_span(stmt).end.offset))?;
            }
            _ => stmt.command.format().fmt(formatter)?,
        }

        if !stmt.redirects.is_empty() {
            write!(formatter, [space()])?;
            format_redirect_list(&stmt.redirects, formatter)?;
        }

        emit_heredocs(&stmt.redirects, formatter)?;

        if let Some(StmtTerminator::Background(operator)) = stmt.terminator {
            write!(
                formatter,
                [text(format!(" {}", render_background_operator(operator)))]
            )?;
        }

        Ok(())
    }
}

impl FormatNodeRule<Command> for FormatCommand {
    fn fmt(&self, command: &Command, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        match command {
            Command::Simple(command) => format_simple_command(command, formatter),
            Command::Builtin(command) => format_builtin_command(command, formatter),
            Command::Decl(command) => format_decl_clause(command, formatter),
            Command::Binary(command) => format_binary_command(command, formatter),
            Command::Compound(compound) => compound.format().fmt(formatter),
            Command::Function(function) => format_function(function, formatter),
            Command::AnonymousFunction(function) => format_anonymous_function(function, formatter),
        }
    }
}

impl FormatNodeRule<CompoundCommand> for FormatCompoundCommand {
    fn fmt(
        &self,
        command: &CompoundCommand,
        formatter: &mut ShellFormatter<'_, '_>,
    ) -> FormatResult<()> {
        match command {
            CompoundCommand::If(command) => format_if(command, formatter),
            CompoundCommand::For(command) => format_for(command, formatter),
            CompoundCommand::Repeat(command) => format_repeat(command, formatter),
            CompoundCommand::Foreach(command) => format_foreach(command, formatter),
            CompoundCommand::ArithmeticFor(command) => format_arithmetic_for(command, formatter),
            CompoundCommand::While(command) => format_while(command, formatter),
            CompoundCommand::Until(command) => format_until(command, formatter),
            CompoundCommand::Case(command) => format_case(command, formatter),
            CompoundCommand::Select(command) => format_select(command, formatter),
            CompoundCommand::Subshell(commands) => format_subshell(commands, formatter, None),
            CompoundCommand::BraceGroup(commands) => format_brace_group(commands, formatter, None),
            CompoundCommand::Arithmetic(command) => format_arithmetic(command, formatter),
            CompoundCommand::Time(command) => format_time(command, formatter),
            CompoundCommand::Conditional(command) => format_conditional(command, formatter),
            CompoundCommand::Coproc(command) => format_coproc(command, formatter),
            CompoundCommand::Always(command) => format_always(command, formatter),
        }
    }
}

pub(crate) fn format_stmt_sequence(
    sequence: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    format_stmt_sequence_with_upper_bound(sequence.as_slice(), formatter, None)
}

fn format_stmt_sequence_with_upper_bound(
    statements: &[Stmt],
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if statements.is_empty() {
        return Ok(());
    }

    let source = formatter.context().source();
    let compact_layout = formatter.context().options().compact_layout();
    let minify = formatter.context().options().minify();
    let attachment_spans = if minify {
        None
    } else {
        let options = formatter.context().options();
        let source_map = formatter.context().comments().source_map();
        Some(
            statements
                .iter()
                .map(|stmt| stmt_attachment_span(stmt, source, source_map, options))
                .collect::<Vec<_>>(),
        )
    };
    let attachments = if minify {
        None
    } else {
        Some(
            formatter
                .context_mut()
                .comments_mut()
                .attach_sequence(attachment_spans.as_deref().unwrap_or(&[]), upper_bound),
        )
    };
    let compact = compact_layout
        && attachments
            .as_ref()
            .is_none_or(|attachment| !attachment.has_comments());

    if attachments
        .as_ref()
        .is_some_and(|value| value.is_ambiguous())
        && let Some(document) = verbatim_stmts(statements, source)
    {
        let span = statements
            .iter()
            .map(|stmt| stmt_verbatim_span(stmt, source))
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default();
        if let Some(attachment) = &attachments
            && let Some(first) = statements.first()
        {
            let leading = attachment
                .leading_for(0)
                .iter()
                .copied()
                .filter(|comment| comment.span().end.offset <= span.start.offset)
                .collect::<Vec<_>>();
            emit_leading_comments(
                &leading,
                stmt_verbatim_span(first, source).start.line,
                formatter,
            )?;
        }
        formatter.context_mut().comments_mut().claim_in_span(span);
        write!(formatter, [document])?;
        if let Some(attachment) = &attachments {
            emit_dangling_comments(attachment.dangling(), formatter)?;
        }
        return Ok(());
    }

    for (index, stmt) in statements.iter().enumerate() {
        if let Some(attachment) = &attachments {
            let next_line = attachment_spans
                .as_ref()
                .and_then(|spans| spans.get(index))
                .map(|span| span.start.line)
                .unwrap_or(stmt_span(stmt).start.line);
            emit_leading_comments(attachment.leading_for(index), next_line, formatter)?;
        }
        stmt.format().fmt(formatter)?;
        if let Some(attachment) = &attachments {
            emit_trailing_comments(attachment.trailing_for(index), formatter)?;
        }
        if index + 1 < statements.len() {
            if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
                if background_has_explicit_line_break(
                    stmt,
                    &statements[index + 1],
                    formatter,
                    attachment_spans
                        .as_ref()
                        .and_then(|spans| spans.get(index + 1))
                        .copied(),
                ) {
                    let current_end = rendered_stmt_end_line(
                        stmt,
                        source,
                        formatter.context().comments().source_map(),
                    );
                    let next_start = attachments
                        .as_ref()
                        .and_then(|attachment| attachment.leading_for(index + 1).first())
                        .map(|comment| comment.line())
                        .unwrap_or(stmt_span(&statements[index + 1]).start.line);
                    write_line_breaks(line_gap_break_count(current_end, next_start), formatter)?;
                } else {
                    write!(formatter, [space()])?;
                }
            } else if compact {
                write!(formatter, [text("; ")])?;
            } else {
                let current_end = rendered_stmt_end_line(
                    stmt,
                    source,
                    formatter.context().comments().source_map(),
                );
                let next_start = attachments
                    .as_ref()
                    .and_then(|attachment| attachment.leading_for(index + 1).first())
                    .map(|comment| comment.line())
                    .unwrap_or(stmt_span(&statements[index + 1]).start.line);
                write_line_breaks(line_gap_break_count(current_end, next_start), formatter)?;
            }
        }
    }

    if let Some(attachment) = &attachments {
        emit_dangling_comments(attachment.dangling(), formatter)?;
    }
    Ok(())
}

fn format_simple_command(
    command: &SimpleCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    if command.args.is_empty()
        && command.assignments.len() == 1
        && render_word_syntax(&command.name, source, formatter.context().options()).is_empty()
        && multiline_compound_assignment_lines(&command.assignments[0], source).is_some()
    {
        return format_standalone_multiline_compound_assignment(
            &command.assignments[0],
            formatter,
        );
    }

    let has_name =
        !render_word_syntax(&command.name, source, formatter.context().options()).is_empty();
    let mut first = true;

    for assignment in &command.assignments {
        if !first {
            write!(formatter, [space()])?;
        }
        write!(
            formatter,
            [text(render_assignment(
                assignment,
                source,
                formatter.context().options(),
            ))]
        )?;
        first = false;
    }

    if has_name {
        if !first {
            write!(formatter, [space()])?;
        }
        command.name.format().fmt(formatter)?;
        first = false;
    }

    for argument in &command.args {
        if !first {
            write!(formatter, [space()])?;
        }
        argument.format().fmt(formatter)?;
        first = false;
    }

    Ok(())
}

fn format_builtin_command(
    command: &BuiltinCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    match command {
        BuiltinCommand::Break(command) => format_builtin_like(
            "break",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
            formatter,
        ),
        BuiltinCommand::Continue(command) => format_builtin_like(
            "continue",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
            formatter,
        ),
        BuiltinCommand::Return(command) => format_builtin_like(
            "return",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
            formatter,
        ),
        BuiltinCommand::Exit(command) => format_builtin_like(
            "exit",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
            formatter,
        ),
    }
}

fn format_builtin_like(
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let options = formatter.context().options();
    let mut pieces = Vec::new();
    for assignment in assignments {
        pieces.push(render_assignment(assignment, source, options));
    }
    pieces.push(name.to_string());
    if let Some(primary) = primary {
        pieces.push(render_word_syntax(primary, source, options));
    }
    for argument in extra_args {
        pieces.push(render_word_syntax(argument, source, options));
    }

    write!(formatter, [text(pieces.join(" "))])
}

fn format_decl_clause(
    command: &DeclClause,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let options = formatter.context().options().clone();
    let mut first = true;

    for assignment in &command.assignments {
        if !first {
            write!(formatter, [space()])?;
        }
        write!(
            formatter,
            [text(render_assignment(assignment, source, &options))]
        )?;
        first = false;
    }

    if !first {
        write!(formatter, [space()])?;
    }
    write!(formatter, [text(command.variant.to_string())])?;
    first = false;

    for operand in &command.operands {
        if !first {
            write!(formatter, [space()])?;
        }
        format_decl_operand(operand, formatter)?;
        first = false;
    }

    Ok(())
}

fn format_decl_operand(
    operand: &DeclOperand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.format().fmt(formatter),
        DeclOperand::Name(name) => write!(formatter, [text(render_var_ref(name, source))]),
        DeclOperand::Assignment(assignment) => {
            write!(
                formatter,
                [text(render_assignment(
                    assignment,
                    source,
                    formatter.context().options(),
                ))]
            )
        }
    }
}

fn format_binary_command(
    command: &BinaryCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    match command.op {
        BinaryOp::Pipe | BinaryOp::PipeAll => format_pipeline(command, formatter),
        BinaryOp::And | BinaryOp::Or => format_command_list(command, formatter),
    }
}

fn format_pipeline(
    pipeline: &BinaryCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let mut statements = Vec::new();
    let mut operators = Vec::new();
    collect_pipeline(pipeline, &mut statements, &mut operators);

    let multiline = formatter.context().options().binary_next_line()
        && statements.len() > 1
        && pipeline_has_explicit_line_break(pipeline, formatter.context().source());
    for (index, stmt) in statements.iter().enumerate() {
        if index > 0 {
            let operator = operators
                .get(index - 1)
                .map(|(operator, _)| binary_operator(operator))
                .unwrap_or("|");
            if multiline {
                write!(formatter, [text(" \\"), hard_line_break()])?;
                let command_document =
                    format_into_document(formatter, |nested| stmt.format().fmt(nested))?;
                let mut indented = Document::new();
                indented.push(text(format!("{operator} ")));
                indented.extend(command_document);
                write!(formatter, [indent(indented)])?;
                continue;
            }
            write!(formatter, [text(format!(" {operator} "))])?;
        }
        if !multiline || index == 0 {
            stmt.format().fmt(formatter)?;
        }
    }
    Ok(())
}

fn pipeline_has_explicit_line_break(pipeline: &BinaryCommand, source: &str) -> bool {
    let mut statements = Vec::new();
    let mut operators = Vec::new();
    collect_pipeline(pipeline, &mut statements, &mut operators);

    let mut previous_end = match statements.first() {
        Some(stmt) => stmt_span(stmt).end.offset,
        None => return false,
    };

    for stmt in statements.iter().skip(1) {
        let next_start = stmt_span(stmt).start.offset;
        let Some(between) = source.get(previous_end..next_start) else {
            previous_end = stmt_span(stmt).end.offset;
            continue;
        };
        if between.contains('\n') {
            return true;
        }
        previous_end = stmt_span(stmt).end.offset;
    }

    false
}

fn format_command_list(
    list: &BinaryCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let mut rest = Vec::new();
    let first = collect_command_list_first(list, &mut rest);

    first.format().fmt(formatter)?;
    for item in &rest {
        format_list_item(item, formatter)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct BinaryListItem<'a> {
    operator: BinaryOp,
    operator_span: Span,
    stmt: &'a Stmt,
}

fn format_list_item(
    item: &BinaryListItem<'_>,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    if list_item_has_explicit_line_break(item, formatter) {
        write!(
            formatter,
            [text(list_item_multiline_separator(item.operator))]
        )?;
        let command_document =
            format_into_document(formatter, |nested| item.stmt.format().fmt(nested))?;
        return write!(formatter, [hard_line_break(), indent(command_document)]);
    }

    write!(formatter, [text(list_item_inline_separator(item.operator))])?;
    item.stmt.format().fmt(formatter)
}

fn list_item_inline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " && ",
        BinaryOp::Or => " || ",
        BinaryOp::Pipe | BinaryOp::PipeAll => "; ",
    }
}

fn list_item_multiline_separator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::And => " &&",
        BinaryOp::Or => " ||",
        BinaryOp::Pipe | BinaryOp::PipeAll => ";",
    }
}

fn list_item_has_explicit_line_break(
    item: &BinaryListItem<'_>,
    formatter: &ShellFormatter<'_, '_>,
) -> bool {
    let source = formatter.context().source();
    let options = formatter.context().options();
    let source_map = formatter.context().comments().source_map();
    let command_start = stmt_attachment_span(item.stmt, source, source_map, options)
        .start
        .offset;
    source
        .get(item.operator_span.end.offset..command_start)
        .is_some_and(|between| between.contains('\n'))
}

fn collect_pipeline<'a>(
    command: &'a BinaryCommand,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    collect_pipeline_stmt(&command.left, statements, operators);
    operators.push((command.op, command.op_span));
    collect_pipeline_stmt(&command.right, statements, operators);
}

fn collect_pipeline_stmt<'a>(
    stmt: &'a Stmt,
    statements: &mut Vec<&'a Stmt>,
    operators: &mut Vec<(BinaryOp, Span)>,
) {
    if let Command::Binary(binary) = &stmt.command
        && stmt.redirects.is_empty()
        && !stmt.negated
        && stmt.terminator.is_none()
        && matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll)
    {
        collect_pipeline(binary, statements, operators);
    } else {
        statements.push(stmt);
    }
}

fn collect_command_list_first<'a>(
    command: &'a BinaryCommand,
    rest: &mut Vec<BinaryListItem<'a>>,
) -> &'a Stmt {
    if let Command::Binary(left_binary) = &command.left.command
        && command.left.redirects.is_empty()
        && !command.left.negated
        && command.left.terminator.is_none()
        && matches!(left_binary.op, BinaryOp::And | BinaryOp::Or)
    {
        let first = collect_command_list_first(left_binary, rest);
        rest.push(BinaryListItem {
            operator: command.op,
            operator_span: command.op_span,
            stmt: &command.right,
        });
        return first;
    }

    let first = command.left.as_ref();
    rest.push(BinaryListItem {
        operator: command.op,
        operator_span: command.op_span,
        stmt: &command.right,
    });
    first
}

fn format_if(command: &IfCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    match command.syntax {
        IfSyntax::ThenFi { .. } => format_then_fi_if(command, formatter),
        IfSyntax::Brace { .. } => format_brace_if(command, formatter),
    }
}

fn format_then_fi_if(
    command: &IfCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    write!(formatter, [text("if ")])?;
    format_inline_stmts(&command.condition, formatter)?;
    if command.elif_branches.is_empty()
        && command.else_branch.is_none()
        && can_inline_body(&command.then_branch, command.span, formatter)
    {
        write!(formatter, [text("; then ")])?;
        format_inline_stmts(&command.then_branch, formatter)?;
        return write!(formatter, [text("; fi")]);
    }
    write!(formatter, [text("; then")])?;
    format_body_with_upper_bound(
        &command.then_branch,
        formatter,
        Some(if_branch_upper_bound(command, 0, source)),
    )?;
    for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
        if formatter.context().options().compact_layout() {
            write!(formatter, [text("; elif ")])?;
            format_inline_stmts(condition, formatter)?;
            write!(formatter, [text("; then")])?;
        } else {
            write!(formatter, [hard_line_break(), text("elif ")])?;
            format_inline_stmts(condition, formatter)?;
            write!(formatter, [text("; then")])?;
        }
        format_body_with_upper_bound(
            body,
            formatter,
            Some(if_branch_upper_bound(command, index + 1, source)),
        )?;
    }
    if let Some(body) = &command.else_branch {
        if formatter.context().options().compact_layout() {
            write!(formatter, [text("; else")])?;
        } else {
            write!(formatter, [hard_line_break(), text("else")])?;
        }
        format_body_with_upper_bound(body, formatter, Some(command.span.end.offset))?;
    }
    if formatter.context().options().compact_layout() {
        write!(formatter, [text("; fi")])
    } else {
        write!(formatter, [hard_line_break(), text("fi")])
    }
}

fn format_brace_if(
    command: &IfCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    write!(formatter, [text("if ")])?;
    format_inline_stmts(&command.condition, formatter)?;
    write!(formatter, [space()])?;
    format_brace_group(
        &command.then_branch,
        formatter,
        Some(if_branch_upper_bound(command, 0, source)),
    )?;

    for (index, (condition, body)) in command.elif_branches.iter().enumerate() {
        write!(formatter, [text(" elif ")])?;
        format_inline_stmts(condition, formatter)?;
        write!(formatter, [space()])?;
        format_brace_group(
            body,
            formatter,
            Some(if_branch_upper_bound(command, index + 1, source)),
        )?;
    }

    if let Some(body) = &command.else_branch {
        write!(formatter, [text(" else ")])?;
        format_brace_group(body, formatter, Some(command.span.end.offset))?;
    }

    Ok(())
}

fn if_branch_upper_bound(command: &IfCommand, branch_index: usize, source: &str) -> usize {
    let current_branch_end = if branch_index == 0 {
        command.then_branch.span.end.offset
    } else {
        command
            .elif_branches
            .get(branch_index - 1)
            .map(|(_, body)| body.span.end.offset)
            .unwrap_or(command.then_branch.span.end.offset)
    };

    if let Some((condition, _)) = command.elif_branches.get(branch_index) {
        branch_keyword_offset(source, current_branch_end, condition.span.start.offset, "elif")
            .unwrap_or(condition.span.start.offset)
    } else if let Some(body) = &command.else_branch {
        branch_keyword_offset(source, current_branch_end, body.span.start.offset, "else")
            .unwrap_or(body.span.start.offset)
    } else {
        command.span.end.offset
    }
}

fn branch_keyword_offset(source: &str, start: usize, end: usize, keyword: &str) -> Option<usize> {
    let start = start.min(end).min(source.len());
    let end = end.min(source.len());
    source[start..end].rfind(keyword).map(|offset| start + offset)
}

fn format_for(command: &ForCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    write!(formatter, [text("for ")])?;
    for (index, target) in command.targets.iter().enumerate() {
        if index > 0 {
            write!(formatter, [space()])?;
        }
        write!(formatter, [text(target.name.to_string())])?;
    }

    match command.syntax {
        ForSyntax::InDoDone { .. } => {
            if let Some(words) = &command.words {
                write!(formatter, [text(" in")])?;
                for word in words {
                    write!(formatter, [space()])?;
                    word.format().fmt(formatter)?;
                }
            }
            if can_inline_body(&command.body, command.span, formatter) {
                write!(formatter, [text("; do ")])?;
                format_inline_stmts(&command.body, formatter)?;
                write!(formatter, [text("; done")])
            } else {
                write!(formatter, [text("; do")])?;
                format_body_with_upper_bound(
                    &command.body,
                    formatter,
                    Some(command.span.end.offset),
                )?;
                finish_block("done", formatter)
            }
        }
        ForSyntax::ParenDoDone { .. } => {
            write!(formatter, [text(" (")])?;
            for (index, word) in command
                .words
                .iter()
                .flat_map(|words| words.iter())
                .enumerate()
            {
                if index > 0 {
                    write!(formatter, [space()])?;
                }
                word.format().fmt(formatter)?;
            }
            if can_inline_body(&command.body, command.span, formatter) {
                write!(formatter, [text("); do ")])?;
                format_inline_stmts(&command.body, formatter)?;
                write!(formatter, [text("; done")])
            } else {
                write!(formatter, [text("); do")])?;
                format_body_with_upper_bound(
                    &command.body,
                    formatter,
                    Some(command.span.end.offset),
                )?;
                finish_block("done", formatter)
            }
        }
        ForSyntax::InBrace { .. } => {
            if let Some(words) = &command.words {
                write!(formatter, [text(" in")])?;
                for word in words {
                    write!(formatter, [space()])?;
                    word.format().fmt(formatter)?;
                }
            }
            write!(formatter, [text("; ")])?;
            format_brace_group(&command.body, formatter, Some(command.span.end.offset))
        }
        ForSyntax::ParenBrace { .. } => {
            write!(formatter, [text(" (")])?;
            for (index, word) in command
                .words
                .iter()
                .flat_map(|words| words.iter())
                .enumerate()
            {
                if index > 0 {
                    write!(formatter, [space()])?;
                }
                word.format().fmt(formatter)?;
            }
            write!(formatter, [text("); ")])?;
            format_brace_group(&command.body, formatter, Some(command.span.end.offset))
        }
    }
}

fn format_repeat(
    command: &RepeatCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("repeat ")])?;
    command.count.format().fmt(formatter)?;
    match command.syntax {
        RepeatSyntax::DoDone { .. } => {
            if can_inline_body(&command.body, command.span, formatter) {
                write!(formatter, [text("; do ")])?;
                format_inline_stmts(&command.body, formatter)?;
                write!(formatter, [text("; done")])
            } else {
                write!(formatter, [text("; do")])?;
                format_body_with_upper_bound(
                    &command.body,
                    formatter,
                    Some(command.span.end.offset),
                )?;
                finish_block("done", formatter)
            }
        }
        RepeatSyntax::Brace { .. } => {
            write!(formatter, [space()])?;
            format_brace_group(&command.body, formatter, Some(command.span.end.offset))
        }
    }
}

fn format_foreach(
    command: &ForeachCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text(format!("foreach {}", command.variable))])?;
    match command.syntax {
        ForeachSyntax::ParenBrace { .. } => {
            write!(formatter, [text(" (")])?;
            for (index, word) in command.words.iter().enumerate() {
                if index > 0 {
                    write!(formatter, [space()])?;
                }
                word.format().fmt(formatter)?;
            }
            write!(formatter, [text(") ")])?;
            format_brace_group(&command.body, formatter, Some(command.span.end.offset))
        }
        ForeachSyntax::InDoDone { .. } => {
            write!(formatter, [text(" in ")])?;
            for (index, word) in command.words.iter().enumerate() {
                if index > 0 {
                    write!(formatter, [space()])?;
                }
                word.format().fmt(formatter)?;
            }
            if can_inline_body(&command.body, command.span, formatter) {
                write!(formatter, [text("; do ")])?;
                format_inline_stmts(&command.body, formatter)?;
                write!(formatter, [text("; done")])
            } else {
                write!(formatter, [text("; do")])?;
                format_body_with_upper_bound(
                    &command.body,
                    formatter,
                    Some(command.span.end.offset),
                )?;
                finish_block("done", formatter)
            }
        }
    }
}

fn format_select(
    command: &SelectCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(
        formatter,
        [text(format!("select {} in ", command.variable))]
    )?;
    for (index, word) in command.words.iter().enumerate() {
        if index > 0 {
            write!(formatter, [space()])?;
        }
        word.format().fmt(formatter)?;
    }
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_stmts(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body_with_upper_bound(&command.body, formatter, Some(command.span.end.offset))?;
    finish_block("done", formatter)
}

fn format_while(
    command: &WhileCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("while ")])?;
    format_inline_stmts(&command.condition, formatter)?;
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_stmts(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body_with_upper_bound(&command.body, formatter, Some(command.span.end.offset))?;
    finish_block("done", formatter)
}

fn format_until(
    command: &UntilCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("until ")])?;
    format_inline_stmts(&command.condition, formatter)?;
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_stmts(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body_with_upper_bound(&command.body, formatter, Some(command.span.end.offset))?;
    finish_block("done", formatter)
}

fn format_case(command: &CaseCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    write!(
        formatter,
        [
            text("case "),
            text(render_word_syntax(
                &command.word,
                formatter.context().source(),
                formatter.context().options(),
            )),
            text(" in")
        ]
    )?;

    if formatter.context().options().compact_layout() {
        for item in &command.cases {
            write!(formatter, [text(" ")])?;
            format_case_item(item, formatter, Some(command.span.end.offset))?;
        }
        write!(formatter, [text(" esac")])
    } else {
        for item in &command.cases {
            write!(formatter, [hard_line_break()])?;
            format_case_item(item, formatter, Some(command.span.end.offset))?;
        }
        write!(formatter, [hard_line_break(), text("esac")])
    }
}

fn format_case_item(
    item: &CaseItem,
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let base_indent = usize::from(
        !formatter.context().options().compact_layout()
            && formatter.context().options().switch_case_indent(),
    );
    let mut pattern = String::new();
    for (index, word) in item.patterns.iter().enumerate() {
        if index > 0 {
            pattern.push_str(" | ");
        }
        pattern.push_str(&render_pattern_syntax(
            word,
            source,
            formatter.context().options(),
        ));
    }
    pattern.push(')');
    if base_indent > 0 {
        write_case_prefix(base_indent, formatter)?;
    }
    write!(formatter, [text(pattern)])?;

    if item.body.is_empty() {
        write!(
            formatter,
            [text(format!(" {}", case_terminator(item.terminator)))]
        )
    } else if formatter.context().options().compact_layout() {
        write!(formatter, [space()])?;
        format_stmt_sequence_with_upper_bound(item.body.as_slice(), formatter, upper_bound)?;
        write!(
            formatter,
            [text(format!("; {}", case_terminator(item.terminator)))]
        )
    } else {
        if base_indent == 0 && item.body.len() == 1 && case_item_was_inline_in_source(item) {
            write!(formatter, [space()])?;
            item.body[0].format().fmt(formatter)?;
            write!(formatter, [space(), text(case_terminator(item.terminator))])?;
            return Ok(());
        }

        let commands_document = format_into_document(formatter, |nested| {
            format_stmt_sequence_with_upper_bound(item.body.as_slice(), nested, upper_bound)
        })?;
        write!(
            formatter,
            [
                hard_line_break(),
                indent_levels(commands_document, base_indent + 1)
            ]
        )?;

        let terminator = Document::from_element(text(case_terminator(item.terminator)));
        write!(
            formatter,
            [
                hard_line_break(),
                indent_levels(terminator, base_indent + 1)
            ]
        )
    }
}

fn format_brace_group(
    commands: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if group_open_suffix(commands.as_slice(), formatter.context().source(), '{').is_none()
        && group_was_inline_in_source(commands.as_slice(), formatter.context().source(), '{', '}')
        && can_inline_group(commands, formatter)
    {
        write!(formatter, [text("{ ")])?;
        format_inline_stmts(commands, formatter)?;
        return write!(formatter, [text("; }")]);
    }

    format_group_with_upper_bound("{", "}", '{', commands, formatter, false, upper_bound)
}

fn format_subshell(
    commands: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if group_open_suffix(commands.as_slice(), formatter.context().source(), '(').is_none()
        && group_was_inline_in_source(commands.as_slice(), formatter.context().source(), '(', ')')
        && can_inline_group(commands, formatter)
    {
        write!(formatter, [text("(")])?;
        format_inline_stmts(commands, formatter)?;
        return write!(formatter, [text(")")]);
    }

    format_group_with_upper_bound("(", ")", '(', commands, formatter, false, upper_bound)
}

fn format_arithmetic(
    command: &ArithmeticCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let rendered = source
        .get(command.span.start.offset..command.span.end.offset)
        .unwrap_or_default()
        .to_string();
    write!(formatter, [text(rendered)])
}

fn format_arithmetic_for(
    command: &ArithmeticForCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let header = format!(
        "for (({};{};{})); do",
        slice_span(source, command.init_span),
        command
            .condition_span
            .map(|span| span.slice(source))
            .unwrap_or(""),
        command
            .step_span
            .map(|span| span.slice(source))
            .unwrap_or(""),
    );
    write!(formatter, [text(header)])?;
    format_body_with_upper_bound(&command.body, formatter, Some(command.span.end.offset))?;
    finish_block("done", formatter)
}

fn format_time(command: &TimeCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if command.posix_format {
        write!(formatter, [text("time -p")])?;
    } else {
        write!(formatter, [text("time")])?;
    }
    if let Some(command) = &command.command {
        write!(formatter, [space()])?;
        command.format().fmt(formatter)?;
    }
    Ok(())
}

fn format_conditional(
    command: &ConditionalCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("[[ ")])?;
    format_conditional_expr(&command.expression, formatter)?;
    write!(formatter, [text(" ]]")])
}

fn format_coproc(
    command: &CoprocCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("coproc")])?;
    if command.name.as_str() != "COPROC" || command.name_span.is_some() {
        write!(
            formatter,
            [space(), text(command.name.as_str().to_string())]
        )?;
    }
    write!(formatter, [space()])?;
    command.body.format().fmt(formatter)
}

fn format_always(
    command: &AlwaysCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    format_brace_group(&command.body, formatter, Some(command.span.end.offset))?;
    write!(formatter, [text(" always ")])?;
    format_brace_group(
        &command.always_body,
        formatter,
        Some(command.span.end.offset),
    )
}

fn format_function(
    function: &FunctionDef,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    format_named_function_header(function, formatter)?;
    if formatter.context().options().function_next_line() {
        write!(formatter, [hard_line_break()])?;
    } else {
        write!(formatter, [space()])?;
    }
    format_function_body(function.body.as_ref(), function.span.end.offset, formatter)
}

fn format_anonymous_function(
    function: &AnonymousFunctionCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let header = match function.surface {
        shuck_ast::AnonymousFunctionSurface::FunctionKeyword { .. } => "function".to_string(),
        shuck_ast::AnonymousFunctionSurface::Parens { .. } => "()".to_string(),
    };
    write!(formatter, [text(header)])?;
    if formatter.context().options().function_next_line() {
        write!(formatter, [hard_line_break()])?;
    } else {
        write!(formatter, [space()])?;
    }
    format_function_body(function.body.as_ref(), function.span.end.offset, formatter)?;
    if !function.args.is_empty() {
        let rendered_args = {
            let source = formatter.context().source();
            let options = formatter.context().options();
            function
                .args
                .iter()
                .map(|argument| render_word_syntax(argument, source, options))
                .collect::<Vec<_>>()
        };
        for argument in rendered_args {
            write!(formatter, [space(), text(argument)])?;
        }
    }
    Ok(())
}

fn format_named_function_header(
    function: &FunctionDef,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let rendered_entries = {
        let options = formatter.context().options();
        function
            .header
            .entries
            .iter()
            .map(|entry| render_word_syntax(&entry.word, source, options))
            .collect::<Vec<_>>()
    };
    let classic_single_name = function.header.entries.len() == 1
        && function.header.entries[0].static_name.is_some()
        && function.header.entries[0]
            .static_name
            .as_ref()
            .is_some_and(|name| name.as_str() == rendered_entries[0]);

    if classic_single_name {
        if function.uses_function_keyword() {
            write!(formatter, [text("function ".to_string())])?;
        }
        let name = function.header.entries[0]
            .static_name
            .as_ref()
            .expect("classic function header should have a static name");
        write!(formatter, [text(name.to_string())])?;
        if function.has_trailing_parens() {
            write!(formatter, [text("()".to_string())])?;
        }
        return Ok(());
    }

    if function.uses_function_keyword() {
        write!(formatter, [text("function".to_string())])?;
        if !function.header.entries.is_empty() {
            write!(formatter, [space()])?;
        }
    }
    for (index, rendered) in rendered_entries.iter().enumerate() {
        if index > 0 {
            write!(formatter, [space()])?;
        }
        write!(formatter, [text(rendered.clone())])?;
    }
    if function.has_trailing_parens() {
        write!(formatter, [text("()".to_string())])?;
    }
    Ok(())
}

fn format_function_body(
    body: &Stmt,
    upper_bound: usize,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    match body {
        Stmt {
            command: Command::Compound(CompoundCommand::BraceGroup(commands)),
            negated: false,
            redirects,
            terminator: None,
            ..
        } if redirects.is_empty() => {
            if !formatter.context().options().function_next_line()
                && group_was_inline_in_source(
                    commands.as_slice(),
                    formatter.context().source(),
                    '{',
                    '}',
                )
                && can_inline_group(commands, formatter)
            {
                write!(formatter, [text("{ ".to_string())])?;
                format_inline_stmts(commands, formatter)?;
                write!(formatter, [text("; }".to_string())])
            } else {
                format_brace_group(commands, formatter, Some(upper_bound))
            }
        }
        Stmt {
            command: Command::Compound(CompoundCommand::Subshell(commands)),
            negated: false,
            redirects,
            terminator: None,
            ..
        } if redirects.is_empty() => {
            if !formatter.context().options().function_next_line()
                && group_was_inline_in_source(
                    commands.as_slice(),
                    formatter.context().source(),
                    '(',
                    ')',
                )
                && can_inline_group(commands, formatter)
            {
                write!(formatter, [text("(".to_string())])?;
                format_inline_stmts(commands, formatter)?;
                write!(formatter, [text(")".to_string())])
            } else {
                format_subshell(commands, formatter, Some(upper_bound))
            }
        }
        _ => body.format().fmt(formatter),
    }
}

fn format_inline_stmts(
    commands: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for (index, stmt) in commands.iter().enumerate() {
        if index > 0 {
            if matches!(
                commands[index - 1].terminator,
                Some(StmtTerminator::Background(_))
            ) {
                write!(formatter, [space()])?;
            } else {
                write!(formatter, [text("; ")])?;
            }
        }
        stmt.format().fmt(formatter)?;
    }
    Ok(())
}

fn format_body_with_upper_bound(
    commands: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if commands.is_empty() {
        return Ok(());
    }

    if formatter.context().options().compact_layout() {
        write!(formatter, [space()])?;
        format_stmt_sequence_with_upper_bound(commands.as_slice(), formatter, upper_bound)
    } else {
        let body = format_into_document(formatter, |nested| {
            format_stmt_sequence_with_upper_bound(commands.as_slice(), nested, upper_bound)
        })?;
        write!(formatter, [hard_line_break(), indent(body)])
    }
}

fn finish_block(close: &str, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if formatter.context().options().compact_layout() {
        write!(formatter, [text(format!("; {close}"))])
    } else {
        write!(formatter, [hard_line_break(), text(close.to_string())])
    }
}

fn format_redirect_list(
    redirects: &[Redirect],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for (index, redirect) in redirects.iter().enumerate() {
        if index > 0 {
            write!(formatter, [space()])?;
        }
        redirect.format().fmt(formatter)?;
    }
    Ok(())
}

fn emit_heredocs(
    redirects: &[Redirect],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    for redirect in redirects {
        let Some(heredoc) = redirect.heredoc() else {
            continue;
        };
        let mut delimiter = String::new();
        heredoc
            .delimiter
            .raw
            .render_syntax_to_buf(source, &mut delimiter);
        write!(
            formatter,
            [verbatim(render_heredoc_tail(
                heredoc.body.span,
                &delimiter,
                source,
            ))]
        )?;
    }
    Ok(())
}

fn render_heredoc_tail(body_span: Span, delimiter: &str, source: &str) -> String {
    let mut rendered = String::new();
    rendered.push('\n');
    rendered.push_str(body_span.slice(source));
    rendered.push_str(delimiter);
    rendered
}

fn render_assignment(
    assignment: &Assignment,
    source: &str,
    options: &crate::options::ResolvedShellFormatOptions,
) -> String {
    let mut rendered = render_assignment_head(assignment, source);
    match &assignment.value {
        AssignmentValue::Scalar(value) => {
            rendered.push_str(&render_word_syntax(value, source, options));
        }
        AssignmentValue::Compound(array) => {
            rendered.push('(');
            for (index, value) in array.elements.iter().enumerate() {
                if index > 0 {
                    rendered.push(' ');
                }
                rendered.push_str(&render_array_elem(value, source, options));
            }
            rendered.push(')');
        }
    }
    trim_unescaped_trailing_whitespace(&rendered).to_string()
}

fn render_array_elem(
    element: &ArrayElem,
    source: &str,
    options: &crate::options::ResolvedShellFormatOptions,
) -> String {
    match element {
        ArrayElem::Sequential(word) => render_word_syntax(word, source, options),
        ArrayElem::Keyed { key, value } => {
            format!(
                "[{}]={}",
                render_subscript(key, source),
                render_word_syntax(value, source, options)
            )
        }
        ArrayElem::KeyedAppend { key, value } => {
            format!(
                "[{}]+={}",
                render_subscript(key, source),
                render_word_syntax(value, source, options)
            )
        }
    }
}

fn render_assignment_head(assignment: &Assignment, source: &str) -> String {
    let mut rendered = assignment.target.name.to_string();
    if let Some(index) = &assignment.target.subscript {
        rendered.push('[');
        rendered.push_str(&render_subscript(index, source));
        rendered.push(']');
    }
    if assignment.append {
        rendered.push_str("+=");
    } else {
        rendered.push('=');
    }
    rendered
}

fn format_standalone_multiline_compound_assignment(
    assignment: &Assignment,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let Some(lines) = multiline_compound_assignment_lines(assignment, source) else {
        return write!(
            formatter,
            [text(render_assignment(
                assignment,
                source,
                formatter.context().options(),
            ))]
        );
    };

    write!(
        formatter,
        [text(render_assignment_head(assignment, source)), text("(".to_string())]
    )?;

    let mut body = Document::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            body.push(hard_line_break());
        }
        body.push(text(line.clone()));
    }

    write!(
        formatter,
        [hard_line_break(), indent(body), hard_line_break(), text(")".to_string())]
    )
}

fn multiline_compound_assignment_lines(assignment: &Assignment, source: &str) -> Option<Vec<String>> {
    let AssignmentValue::Compound(_) = &assignment.value else {
        return None;
    };

    let slice = assignment.span.slice(source);
    if !slice.contains('\n') {
        return None;
    }

    let open = slice.find('(')?;
    let close = slice.rfind(')')?;
    if close <= open {
        return None;
    }

    let lines = slice[open + 1..close]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    (!lines.is_empty()).then_some(lines)
}

fn render_var_ref(reference: &VarRef, source: &str) -> String {
    let mut rendered = reference.name.to_string();
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        rendered.push_str(&render_subscript(subscript, source));
        rendered.push(']');
    }
    rendered
}

fn render_subscript(subscript: &Subscript, source: &str) -> String {
    if let Some(selector) = subscript.selector() {
        return selector.as_char().to_string();
    }

    render_source_text(subscript.syntax_source_text(), source)
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text.as_bytes()[..whitespace_start]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}

fn render_source_text(text: &SourceText, source: &str) -> String {
    if text.is_source_backed() && text.span().end.offset > source.len() {
        String::new()
    } else {
        text.slice(source).to_string()
    }
}

fn has_heredoc(stmt: &Stmt) -> bool {
    stmt.redirects.iter().any(is_heredoc)
        || match &stmt.command {
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => false,
            Command::Binary(command) => has_heredoc(&command.left) || has_heredoc(&command.right),
            Command::Compound(command) => compound_has_heredoc(command),
            Command::Function(function) => has_heredoc(&function.body),
            Command::AnonymousFunction(function) => has_heredoc(&function.body),
        }
}

fn compound_has_heredoc(command: &CompoundCommand) -> bool {
    match command {
        CompoundCommand::If(command) => {
            stmt_seq_has_heredoc(&command.condition)
                || stmt_seq_has_heredoc(&command.then_branch)
                || command.elif_branches.iter().any(|(condition, body)| {
                    stmt_seq_has_heredoc(condition) || stmt_seq_has_heredoc(body)
                })
                || command
                    .else_branch
                    .as_ref()
                    .is_some_and(stmt_seq_has_heredoc)
        }
        CompoundCommand::For(command) => stmt_seq_has_heredoc(&command.body),
        CompoundCommand::Repeat(command) => stmt_seq_has_heredoc(&command.body),
        CompoundCommand::Foreach(command) => stmt_seq_has_heredoc(&command.body),
        CompoundCommand::ArithmeticFor(command) => stmt_seq_has_heredoc(&command.body),
        CompoundCommand::While(command) => {
            stmt_seq_has_heredoc(&command.condition) || stmt_seq_has_heredoc(&command.body)
        }
        CompoundCommand::Until(command) => {
            stmt_seq_has_heredoc(&command.condition) || stmt_seq_has_heredoc(&command.body)
        }
        CompoundCommand::Case(command) => command
            .cases
            .iter()
            .any(|item| stmt_seq_has_heredoc(&item.body)),
        CompoundCommand::Select(command) => stmt_seq_has_heredoc(&command.body),
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            stmt_seq_has_heredoc(commands)
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => false,
        CompoundCommand::Time(command) => command.command.as_deref().is_some_and(has_heredoc),
        CompoundCommand::Coproc(command) => has_heredoc(&command.body),
        CompoundCommand::Always(command) => {
            stmt_seq_has_heredoc(&command.body) || stmt_seq_has_heredoc(&command.always_body)
        }
    }
}

fn stmt_seq_has_heredoc(commands: &StmtSeq) -> bool {
    commands.iter().any(has_heredoc)
}

fn is_heredoc(redirect: &shuck_ast::Redirect) -> bool {
    matches!(
        redirect.kind,
        RedirectKind::HereDoc | RedirectKind::HereDocStrip
    )
}

fn verbatim_stmt(stmt: &Stmt, source: &str) -> Option<FormatElement> {
    let span = stmt_verbatim_span(stmt, source);
    (span.end.offset <= source.len()).then(|| verbatim(span.slice(source)))
}

fn verbatim_stmts(statements: &[Stmt], source: &str) -> Option<FormatElement> {
    let span = statements
        .iter()
        .map(|stmt| stmt_verbatim_span(stmt, source))
        .reduce(|left, right| left.merge(right))?;
    (span.end.offset <= source.len()).then(|| verbatim(span.slice(source)))
}

fn stmt_verbatim_span(stmt: &Stmt, source: &str) -> Span {
    let mut span = merge_redirect_heredoc_spans(
        command_verbatim_span(&stmt.command, source),
        &stmt.redirects,
        source,
    );
    if stmt.negated {
        span = merge_non_empty_span(stmt.span, span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    if span == Span::new() {
        stmt_span(stmt)
    } else {
        span
    }
}

fn command_verbatim_span(command: &Command, source: &str) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => command.span,
            BuiltinCommand::Continue(command) => command.span,
            BuiltinCommand::Return(command) => command.span,
            BuiltinCommand::Exit(command) => command.span,
        },
        Command::Decl(command) => command.span,
        Command::Binary(command) => stmt_verbatim_span(&command.left, source)
            .merge(stmt_verbatim_span(&command.right, source)),
        Command::Compound(command) => compound_verbatim_span(command, source),
        Command::Function(command) => {
            function_header_span(command).merge(stmt_verbatim_span(&command.body, source))
        }
        Command::AnonymousFunction(command) => anonymous_function_header_span(command)
            .merge(stmt_verbatim_span(&command.body, source))
            .merge(words_span(&command.args)),
    }
}

fn merge_redirect_heredoc_spans(mut span: Span, redirects: &[Redirect], source: &str) -> Span {
    for redirect in redirects {
        span = merge_non_empty_span(span, redirect.span);
        if let Some(heredoc) = redirect.heredoc() {
            span = span.merge(extend_heredoc_body_span(heredoc.body.span, source));
        }
    }
    span
}

fn stmt_span(stmt: &Stmt) -> Span {
    let mut span = stmt.span;
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    span
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(stmt_span)
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

fn compound_verbatim_span(command: &CompoundCommand, source: &str) -> Span {
    match command {
        CompoundCommand::If(command) => {
            let mut span = command.span;
            span = merge_stmt_sequence_verbatim_span(span, &command.condition, source);
            span = merge_stmt_sequence_verbatim_span(span, &command.then_branch, source);
            for (condition, body) in &command.elif_branches {
                span = merge_stmt_sequence_verbatim_span(span, condition, source);
                span = merge_stmt_sequence_verbatim_span(span, body, source);
            }
            if let Some(body) = &command.else_branch {
                span = merge_stmt_sequence_verbatim_span(span, body, source);
            }
            span
        }
        CompoundCommand::For(command) => {
            merge_stmt_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::Repeat(command) => {
            merge_stmt_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::Foreach(command) => {
            merge_stmt_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::ArithmeticFor(command) => {
            merge_stmt_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::While(command) => {
            let span = merge_stmt_sequence_verbatim_span(command.span, &command.condition, source);
            merge_stmt_sequence_verbatim_span(span, &command.body, source)
        }
        CompoundCommand::Until(command) => {
            let span = merge_stmt_sequence_verbatim_span(command.span, &command.condition, source);
            merge_stmt_sequence_verbatim_span(span, &command.body, source)
        }
        CompoundCommand::Case(command) => {
            let mut span = command.span;
            for item in &command.cases {
                span = merge_stmt_sequence_verbatim_span(span, &item.body, source);
            }
            span
        }
        CompoundCommand::Select(command) => {
            merge_stmt_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::Subshell(commands) => {
            group_verbatim_span(commands.as_slice(), source, '(', ')')
        }
        CompoundCommand::BraceGroup(commands) => {
            group_verbatim_span(commands.as_slice(), source, '{', '}')
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command
            .command
            .as_ref()
            .map(|inner| command.span.merge(stmt_verbatim_span(inner, source)))
            .unwrap_or(command.span),
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command
            .span
            .merge(stmt_verbatim_span(&command.body, source)),
        CompoundCommand::Always(command) => {
            let span = merge_stmt_sequence_verbatim_span(command.span, &command.body, source);
            merge_stmt_sequence_verbatim_span(span, &command.always_body, source)
        }
    }
}

fn merge_stmt_sequence_verbatim_span(mut span: Span, commands: &StmtSeq, source: &str) -> Span {
    for command in commands.iter() {
        span = merge_non_empty_span(span, stmt_verbatim_span(command, source));
    }
    span
}

fn group_verbatim_span(commands: &[Stmt], source: &str, open: char, close: char) -> Span {
    let inner = commands
        .iter()
        .map(|command| stmt_verbatim_span(command, source))
        .reduce(|left, right| left.merge(right))
        .unwrap_or_default();
    if inner == Span::new() {
        return inner;
    }

    let Some(open_offset) = source[..inner.start.offset].rfind(open) else {
        return inner;
    };
    let wrapper_prefix_start = open_offset + open.len_utf8();
    if !source[wrapper_prefix_start..inner.start.offset].contains('#') {
        return inner;
    }
    let Some(close_offset) = source[inner.end.offset..].find(close) else {
        return inner;
    };

    span_for_offsets(
        source,
        open_offset,
        inner.end.offset + close_offset + close.len_utf8(),
    )
}

fn format_group_with_upper_bound(
    open: &str,
    close: &str,
    open_char: char,
    commands: &StmtSeq,
    formatter: &mut ShellFormatter<'_, '_>,
    leading_space: bool,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if leading_space {
        write!(formatter, [space()])?;
    }
    write!(formatter, [text(open)])?;
    if let Some((span, suffix)) =
        group_open_suffix(commands.as_slice(), formatter.context().source(), open_char)
    {
        formatter.context_mut().comments_mut().claim_in_span(span);
        write!(formatter, [text(suffix.to_string())])?;
    }
    format_body_with_upper_bound(commands, formatter, upper_bound)?;
    finish_block(close, formatter)
}

fn group_open_suffix<'a>(
    commands: &[Stmt],
    source: &'a str,
    open: char,
) -> Option<(Span, &'a str)> {
    let first = commands.first()?;
    let first_start = stmt_span(first).start.offset;
    let open_offset = source[..first_start].rfind(open)?;
    let line_end = source[open_offset..]
        .find('\n')
        .map(|offset| open_offset + offset)
        .unwrap_or(source.len());
    let suffix_start = open_offset + open.len_utf8();
    let suffix = source.get(suffix_start..line_end)?;
    suffix
        .contains('#')
        .then(|| (span_for_offsets(source, suffix_start, line_end), suffix))
}

fn group_attachment_span(commands: &[Stmt], source: &str, open: char, close: char) -> Option<Span> {
    let first = commands.first()?;
    let last = commands.last()?;
    let open_offset = source[..stmt_span(first).start.offset].rfind(open)?;
    let last_end = stmt_span(last).end.offset;
    let end = source[last_end..]
        .find(close)
        .map(|offset| last_end + offset + close.len_utf8())
        .unwrap_or(last_end);
    Some(span_for_offsets(source, open_offset, end))
}

fn group_was_inline_in_source(commands: &[Stmt], source: &str, open: char, close: char) -> bool {
    group_attachment_span(commands, source, open, close)
        .map(|span| !span.slice(source).contains('\n'))
        .unwrap_or(false)
}

fn command_format_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => simple_command_format_span(command),
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => builtin_like_span(
                command.span.start,
                "break",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Continue(command) => builtin_like_span(
                command.span.start,
                "continue",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Return(command) => builtin_like_span(
                command.span.start,
                "return",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Exit(command) => builtin_like_span(
                command.span.start,
                "exit",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
        },
        Command::Decl(command) => decl_clause_format_span(command),
        Command::Binary(command) => {
            stmt_format_span(&command.left).merge(stmt_format_span(&command.right))
        }
        Command::Compound(command) => compound_format_span(command),
        Command::Function(command) => function_attachment_span(command),
        Command::AnonymousFunction(command) => anonymous_function_attachment_span(command),
    }
}

fn simple_command_format_span(command: &SimpleCommand) -> Span {
    let mut span = Span::new();
    for assignment in &command.assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    if !command.name.parts.is_empty() {
        span = merge_non_empty_span(span, command.name.span);
    }
    for argument in &command.args {
        span = merge_non_empty_span(span, argument.span);
    }
    if span == Span::new() {
        command.span
    } else {
        span
    }
}

fn builtin_like_span(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
) -> Span {
    let mut span = Span::from_positions(start, start.advanced_by(name));
    for assignment in assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    if let Some(primary) = primary {
        span = merge_non_empty_span(span, primary.span);
    }
    for argument in extra_args {
        span = merge_non_empty_span(span, argument.span);
    }
    span
}

fn decl_clause_format_span(command: &DeclClause) -> Span {
    let mut span = command.variant_span;
    for assignment in &command.assignments {
        span = merge_non_empty_span(span, assignment.span);
    }
    for operand in &command.operands {
        let operand_span = match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
            DeclOperand::Name(name) => name.span,
            DeclOperand::Assignment(assignment) => assignment.span,
        };
        span = merge_non_empty_span(span, operand_span);
    }
    span
}

fn function_attachment_span(command: &FunctionDef) -> Span {
    function_header_span(command).merge(stmt_span(&command.body))
}

fn anonymous_function_attachment_span(command: &AnonymousFunctionCommand) -> Span {
    anonymous_function_header_span(command)
        .merge(stmt_span(&command.body))
        .merge(words_span(&command.args))
}

fn function_header_span(command: &FunctionDef) -> Span {
    command.header.span()
}

fn anonymous_function_header_span(command: &AnonymousFunctionCommand) -> Span {
    match command.surface {
        shuck_ast::AnonymousFunctionSurface::FunctionKeyword {
            function_keyword_span,
        } => function_keyword_span,
        shuck_ast::AnonymousFunctionSurface::Parens { parens_span } => parens_span,
    }
}

fn words_span(words: &[shuck_ast::Word]) -> Span {
    words.iter().fold(Span::new(), |span, word| {
        merge_non_empty_span(span, word.span)
    })
}

fn compound_format_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(stmt_format_span)
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default(),
        _ => compound_span(command),
    }
}

fn stmt_format_span(stmt: &Stmt) -> Span {
    let mut span = if stmt.negated {
        stmt.span
    } else {
        command_format_span(&stmt.command)
    };
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    if span == Span::new() {
        stmt_span(stmt)
    } else {
        span
    }
}

fn merge_non_empty_span(current: Span, next: Span) -> Span {
    if current == Span::new() {
        next
    } else if next == Span::new() {
        current
    } else {
        current.merge(next)
    }
}

fn span_for_offsets(source: &str, start: usize, end: usize) -> Span {
    crate::comments::SourceMap::new(source).span_for_offsets(start, end)
}

fn emit_leading_comments(
    comments: &[crate::comments::SourceComment<'_>],
    next_line: usize,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for (index, comment) in comments.iter().enumerate() {
        write!(formatter, [text(comment.text().to_string())])?;
        let target_line = comments
            .get(index + 1)
            .map(|next| next.line())
            .unwrap_or(next_line);
        write_line_breaks(line_gap_break_count(comment.line(), target_line), formatter)?;
    }
    Ok(())
}

fn emit_trailing_comments(
    comments: &[crate::comments::SourceComment<'_>],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for comment in comments {
        write!(formatter, [text(" "), text(comment.text().to_string())])?;
    }
    Ok(())
}

fn emit_dangling_comments(
    comments: &[crate::comments::SourceComment<'_>],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for (index, comment) in comments.iter().enumerate() {
        write!(
            formatter,
            [hard_line_break(), text(comment.text().to_string())]
        )?;
        if let Some(next) = comments.get(index + 1) {
            write_line_breaks(line_gap_break_count(comment.line(), next.line()), formatter)?;
        }
    }
    Ok(())
}

fn line_gap_break_count(current_line: usize, next_line: usize) -> usize {
    next_line.saturating_sub(current_line).max(1)
}

fn rendered_stmt_end_line(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    let span = match &stmt.command {
        Command::Function(_) | Command::AnonymousFunction(_) => stmt_span(stmt),
        _ if has_heredoc(stmt) => stmt_verbatim_span(stmt, source),
        _ => stmt_format_span(stmt),
    };
    span_render_end_line(span, source, source_map)
}

fn span_render_end_line(
    span: Span,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    let mut end = span.end.offset.min(source.len());
    while end > span.start.offset
        && source
            .as_bytes()
            .get(end - 1)
            .is_some_and(u8::is_ascii_whitespace)
    {
        end -= 1;
    }

    if end == span.start.offset {
        span.start.line
    } else {
        source_map.line_number_for_offset(end - 1)
    }
}

fn write_line_breaks(count: usize, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    for _ in 0..count {
        write!(formatter, [hard_line_break()])?;
    }
    Ok(())
}

fn can_inline_body(
    commands: &StmtSeq,
    enclosing_span: Span,
    formatter: &ShellFormatter<'_, '_>,
) -> bool {
    let [command] = commands.as_slice() else {
        return false;
    };
    if matches!(command.terminator, Some(StmtTerminator::Background(_)))
        || !can_inline_stmt(command, formatter)
    {
        return false;
    }

    let has_comments = {
        let source = formatter.context().source();
        let source_map = formatter.context().comments().source_map();
        let options = formatter.context().options();
        let span = stmt_attachment_span(command, source, source_map, options);
        formatter
            .context()
            .comments()
            .inspect_sequence(&[span], Some(enclosing_span.end.offset))
            .attachment
            .has_comments()
    };
    if has_comments {
        return false;
    }

    formatter.context().options().compact_layout()
        || stmt_span(command).start.line == enclosing_span.start.line
}

fn can_inline_group(commands: &StmtSeq, formatter: &ShellFormatter<'_, '_>) -> bool {
    let [command] = commands.as_slice() else {
        return false;
    };

    can_inline_stmt(command, formatter)
        && stmt_span(command).start.line == stmt_span(command).end.line
        && can_inline_body(commands, stmt_span(command), formatter)
}

fn can_inline_stmt(stmt: &Stmt, formatter: &ShellFormatter<'_, '_>) -> bool {
    if has_heredoc(stmt)
        || stmt_has_trailing_comment(stmt, formatter.context().comments().source_map())
    {
        return false;
    }

    matches!(
        &stmt.command,
        Command::Simple(_)
            | Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(
                CompoundCommand::Conditional(_)
                    | CompoundCommand::Arithmetic(_)
                    | CompoundCommand::Time(_)
            )
    )
}

fn stmt_has_trailing_comment(stmt: &Stmt, source_map: &crate::comments::SourceMap<'_>) -> bool {
    let raw = stmt_span(stmt);
    let formatted = stmt_format_span(stmt);
    raw.end.offset > formatted.end.offset
        && source_map.contains_comment_between(formatted.end.offset, raw.end.offset)
}

fn should_render_verbatim(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> bool {
    (options.keep_padding() && stmt_has_alignment_sensitive_padding(stmt, source_map))
        || (has_heredoc(stmt) && stmt_has_trailing_comment(stmt, source_map))
}

fn stmt_attachment_span(
    stmt: &Stmt,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    if should_render_verbatim(stmt, source_map, options) {
        stmt_verbatim_span(stmt, source)
    } else if let Command::Function(command) = &stmt.command {
        function_attachment_span(command)
    } else if let Command::AnonymousFunction(command) = &stmt.command {
        anonymous_function_attachment_span(command)
    } else if let Command::Compound(CompoundCommand::BraceGroup(commands)) = &stmt.command {
        stmt.redirects.iter().fold(
            group_attachment_span(commands.as_slice(), source, '{', '}')
                .unwrap_or_else(|| stmt_span(stmt)),
            |span, redirect| span.merge(redirect.span),
        )
    } else if let Command::Compound(CompoundCommand::Subshell(commands)) = &stmt.command {
        stmt.redirects.iter().fold(
            group_attachment_span(commands.as_slice(), source, '(', ')')
                .unwrap_or_else(|| stmt_span(stmt)),
            |span, redirect| span.merge(redirect.span),
        )
    } else {
        stmt_format_span(stmt)
    }
}

fn stmt_has_alignment_sensitive_padding(
    stmt: &Stmt,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let mut spans = stmt_token_spans(stmt);
    spans.retain(|span| span != &Span::new() && span.start.offset < span.end.offset);
    spans.sort_by_key(|span| span.start.offset);
    spans.windows(2).any(|window| {
        let [left, right] = window else {
            return false;
        };
        if right.start.offset <= left.end.offset {
            return false;
        }
        source_map.has_alignment_padding_between(left.end.offset, right.start.offset)
    })
}

fn case_item_was_inline_in_source(item: &CaseItem) -> bool {
    let Some(stmt) = item.body.first() else {
        return false;
    };

    item.patterns
        .last()
        .is_some_and(|pattern| pattern.span.end.line == stmt_span(stmt).start.line)
}

fn command_token_spans(command: &Command) -> Vec<Span> {
    match command {
        Command::Simple(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            if !command.name.parts.is_empty() {
                spans.push(command.name.span);
            }
            spans.extend(command.args.iter().map(|word| word.span));
            spans
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => builtin_like_token_spans(
                command.span.start,
                "break",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Continue(command) => builtin_like_token_spans(
                command.span.start,
                "continue",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Return(command) => builtin_like_token_spans(
                command.span.start,
                "return",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
            BuiltinCommand::Exit(command) => builtin_like_token_spans(
                command.span.start,
                "exit",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
            ),
        },
        Command::Decl(command) => {
            let mut spans = command
                .assignments
                .iter()
                .map(|assignment| assignment.span)
                .collect::<Vec<_>>();
            spans.push(command.variant_span);
            spans.extend(command.operands.iter().map(|operand| match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => word.span,
                DeclOperand::Name(name) => name.span,
                DeclOperand::Assignment(assignment) => assignment.span,
            }));
            spans
        }
        Command::Binary(command) => vec![command.span],
        Command::Compound(command) => vec![compound_format_span(command)],
        Command::Function(command) => vec![
            function_header_span(command),
            stmt_format_span(&command.body),
        ],
        Command::AnonymousFunction(command) => {
            let mut spans = vec![
                anonymous_function_header_span(command),
                stmt_format_span(&command.body),
            ];
            spans.extend(command.args.iter().map(|argument| argument.span));
            spans
        }
    }
}

fn builtin_like_token_spans(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
) -> Vec<Span> {
    let mut spans = assignments
        .iter()
        .map(|assignment| assignment.span)
        .collect::<Vec<_>>();
    spans.push(Span::from_positions(start, start.advanced_by(name)));
    if let Some(primary) = primary {
        spans.push(primary.span);
    }
    spans.extend(extra_args.iter().map(|argument| argument.span));
    spans
}

fn stmt_token_spans(stmt: &Stmt) -> Vec<Span> {
    let mut spans = if stmt.negated {
        vec![Span::from_positions(
            stmt.span.start,
            stmt.span.start.advanced_by("!"),
        )]
    } else {
        Vec::new()
    };
    spans.extend(command_token_spans(&stmt.command));
    spans.extend(stmt.redirects.iter().map(|redirect| redirect.span));
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        spans.push(terminator_span);
    }
    spans
}

fn render_background_operator(operator: BackgroundOperator) -> &'static str {
    match operator {
        BackgroundOperator::Plain => "&",
        BackgroundOperator::Pipe => "&|",
        BackgroundOperator::Bang => "&!",
    }
}

fn background_has_explicit_line_break(
    current: &Stmt,
    next: &Stmt,
    formatter: &ShellFormatter<'_, '_>,
    next_span: Option<Span>,
) -> bool {
    let Some(terminator_span) = current.terminator_span else {
        return false;
    };
    let source = formatter.context().source();
    let options = formatter.context().options();
    let source_map = formatter.context().comments().source_map();
    let next_start = next_span
        .unwrap_or_else(|| stmt_attachment_span(next, source, source_map, options))
        .start
        .offset;
    source
        .get(terminator_span.end.offset..next_start)
        .is_some_and(|between| between.contains('\n'))
}

fn format_conditional_expr(
    expression: &ConditionalExpr,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    match expression {
        ConditionalExpr::Binary(expr) => format_conditional_binary(expr, formatter),
        ConditionalExpr::Unary(expr) => format_conditional_unary(expr, formatter),
        ConditionalExpr::Parenthesized(expr) => format_conditional_paren(expr, formatter),
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => word.format().fmt(formatter),
        ConditionalExpr::Pattern(pattern) => write!(
            formatter,
            [text(render_pattern_syntax(
                pattern,
                formatter.context().source(),
                formatter.context().options(),
            ))]
        ),
        ConditionalExpr::VarRef(reference) => {
            write!(
                formatter,
                [text(render_var_ref(
                    reference,
                    formatter.context().source()
                ))]
            )
        }
    }
}

fn format_conditional_binary(
    expression: &ConditionalBinaryExpr,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    format_conditional_expr(&expression.left, formatter)?;
    write!(formatter, [space(), text(expression.op.as_str()), space()])?;
    format_conditional_expr(&expression.right, formatter)
}

fn format_conditional_unary(
    expression: &ConditionalUnaryExpr,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text(expression.op.as_str()), space()])?;
    format_conditional_expr(&expression.expr, formatter)
}

fn format_conditional_paren(
    expression: &ConditionalParenExpr,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("(")])?;
    format_conditional_expr(&expression.expr, formatter)?;
    write!(formatter, [text(")")])
}

fn format_into_document(
    formatter: &mut ShellFormatter<'_, '_>,
    build: impl FnOnce(&mut ShellFormatter<'_, '_>) -> FormatResult<()>,
) -> FormatResult<Document> {
    let context = formatter.context().clone();
    let mut nested = shuck_format::Formatter::new(context);
    build(&mut nested)?;
    let nested = nested.finish();
    *formatter.context_mut() = nested.context().clone();
    Ok(nested.document().clone())
}

fn indent_levels(mut document: Document, levels: usize) -> Document {
    for _ in 0..levels {
        document = Document::from_element(indent(document));
    }
    document
}

fn case_terminator(terminator: CaseTerminator) -> &'static str {
    match terminator {
        CaseTerminator::Break => ";;",
        CaseTerminator::FallThrough => ";&",
        CaseTerminator::Continue => ";;&",
    }
}

fn binary_operator(operator: &shuck_ast::BinaryOp) -> &'static str {
    match operator {
        shuck_ast::BinaryOp::And => "&&",
        shuck_ast::BinaryOp::Or => "||",
        shuck_ast::BinaryOp::Pipe => "|",
        shuck_ast::BinaryOp::PipeAll => "|&",
    }
}

fn slice_span(source: &str, span: Option<Span>) -> &str {
    span.and_then(|span| source.get(span.start.offset..span.end.offset))
        .unwrap_or("")
}

fn write_case_prefix(levels: usize, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if levels == 0 {
        return Ok(());
    }

    let prefix = match formatter.context().options().indent_style() {
        shuck_format::IndentStyle::Tab => "\t".repeat(levels),
        shuck_format::IndentStyle::Space => {
            " ".repeat(levels * usize::from(formatter.context().options().indent_width()))
        }
    };
    write!(formatter, [text(prefix)])
}

fn extend_heredoc_body_span(span: Span, source: &str) -> Span {
    let mut end = span.end.offset;
    while end < source.len() {
        let byte = source.as_bytes()[end];
        end += 1;
        if byte == b'\n' {
            break;
        }
    }
    let end_position = span.start.advanced_by(&source[span.start.offset..end]);
    Span::from_positions(span.start, end_position)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ShellFormatOptions;
    use shuck_parser::parser::{Parser, ShellDialect};

    #[test]
    fn parsed_standalone_assignment_renders_without_trailing_space() {
        let source = "x=1\n";
        let parsed = Parser::with_dialect(source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let stmt = &parsed.file.body[0];
        let Command::Simple(command) = &stmt.command else {
            panic!("expected a simple command");
        };

        let options = ShellFormatOptions::default().resolve(source, None);

        assert_eq!(
            render_assignment(&command.assignments[0], source, &options),
            "x=1"
        );
        assert!(command.args.is_empty());
        assert!(stmt.redirects.is_empty());
        assert!(!command.name.parts.is_empty());
        assert!(command.name.render_syntax(source).is_empty());
    }
}
