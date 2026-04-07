use shuck_ast::{
    ArithmeticCommand, ArithmeticForCommand, ArrayElem, Assignment, AssignmentValue, BreakCommand,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandList, CommandListItem,
    CompoundCommand, ConditionalBinaryExpr, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ContinueCommand, CoprocCommand, DeclClause,
    DeclOperand, ExitCommand, ForCommand, FunctionDef, IfCommand, ListOperator, Pipeline, Redirect,
    RedirectKind, ReturnCommand, SelectCommand, SimpleCommand, SourceText, Span, Subscript,
    TimeCommand, UntilCommand, VarRef, WhileCommand,
};
use shuck_format::{
    Document, Format, FormatElement, FormatResult, hard_line_break, indent, space, text, verbatim,
    write,
};

use crate::FormatNodeRule;
use crate::prelude::{AsFormat, ShellFormatter};

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCommand;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCompoundCommand;

impl FormatNodeRule<Command> for FormatCommand {
    fn fmt(&self, command: &Command, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let source = formatter.context().source();
        let render_verbatim = {
            let source_map = formatter.context().comments().source_map();
            let options = formatter.context().options();
            should_render_verbatim(command, source_map, options)
        };
        if render_verbatim {
            let span = command_verbatim_span(command, source);
            formatter.context_mut().comments_mut().claim_in_span(span);
            if let Some(document) = verbatim_command(command, source) {
                return write!(formatter, [document]);
            }
        }

        match command {
            Command::Simple(command) => format_simple_command(command, formatter),
            Command::Builtin(command) => format_builtin_command(command, formatter),
            Command::Decl(command) => format_decl_clause(command, formatter),
            Command::Pipeline(pipeline) => format_pipeline(pipeline, formatter),
            Command::List(list) => format_command_list(list, formatter),
            Command::Compound(compound, redirects) => {
                compound.format().fmt(formatter)?;
                format_redirect_list(redirects, formatter)?;
                emit_heredocs(redirects, formatter)
            }
            Command::Function(function) => format_function(function, formatter),
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
        }
    }
}

pub(crate) fn format_command_sequence(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    format_command_sequence_with_upper_bound(commands, formatter, None)
}

fn format_command_sequence_with_upper_bound(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if commands.is_empty() {
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
            commands
                .iter()
                .map(|command| command_attachment_span(command, source, source_map, options))
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
        && let Some(document) = verbatim_commands(commands, source)
    {
        let span = commands
            .iter()
            .map(|command| command_verbatim_span(command, source))
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default();
        if let Some(attachment) = &attachments
            && let Some(first) = commands.first()
        {
            let leading = attachment
                .leading_for(0)
                .iter()
                .copied()
                .filter(|comment| comment.span().end.offset <= span.start.offset)
                .collect::<Vec<_>>();
            emit_leading_comments(
                &leading,
                command_verbatim_span(first, source).start.line,
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

    for (index, command) in commands.iter().enumerate() {
        if let Some(attachment) = &attachments {
            let next_line = attachment_spans
                .as_ref()
                .and_then(|spans| spans.get(index))
                .map(|span| span.start.line)
                .unwrap_or(command_span(command).start.line);
            emit_leading_comments(attachment.leading_for(index), next_line, formatter)?;
        }
        command.format().fmt(formatter)?;
        if let Some(attachment) = &attachments {
            emit_trailing_comments(attachment.trailing_for(index), formatter)?;
        }
        if index + 1 < commands.len() {
            if compact {
                write!(formatter, [text("; ")])?;
            } else {
                let current_end = rendered_command_end_line(
                    command,
                    source,
                    formatter.context().comments().source_map(),
                );
                let next_start = attachments
                    .as_ref()
                    .and_then(|attachment| attachment.leading_for(index + 1).first())
                    .map(|comment| comment.line())
                    .unwrap_or(command_span(&commands[index + 1]).start.line);
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
    let has_name = !command.name.render_syntax(source).is_empty();
    let mut first = true;

    for assignment in &command.assignments {
        if !first {
            write!(formatter, [space()])?;
        }
        write!(formatter, [text(render_assignment(assignment, source))])?;
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

    if !command.redirects.is_empty() {
        if !first {
            write!(formatter, [space()])?;
        }
        format_redirect_list(&command.redirects, formatter)?;
    }

    emit_heredocs(&command.redirects, formatter)
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
            &command.redirects,
            formatter,
        ),
        BuiltinCommand::Continue(command) => format_builtin_like(
            "continue",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
            &command.redirects,
            formatter,
        ),
        BuiltinCommand::Return(command) => format_builtin_like(
            "return",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
            &command.redirects,
            formatter,
        ),
        BuiltinCommand::Exit(command) => format_builtin_like(
            "exit",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
            &command.redirects,
            formatter,
        ),
    }
}

fn format_builtin_like(
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
    redirects: &[shuck_ast::Redirect],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let mut pieces = Vec::new();
    for assignment in assignments {
        pieces.push(render_assignment(assignment, source));
    }
    pieces.push(name.to_string());
    if let Some(primary) = primary {
        pieces.push(primary.render_syntax(source));
    }
    for argument in extra_args {
        pieces.push(argument.render_syntax(source));
    }

    write!(formatter, [text(pieces.join(" "))])?;
    if !redirects.is_empty() {
        write!(formatter, [space()])?;
        format_redirect_list(redirects, formatter)?;
    }
    emit_heredocs(redirects, formatter)
}

fn format_decl_clause(
    command: &DeclClause,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let mut first = true;

    for assignment in &command.assignments {
        if !first {
            write!(formatter, [space()])?;
        }
        write!(formatter, [text(render_assignment(assignment, source))])?;
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

    if !command.redirects.is_empty() {
        if !first {
            write!(formatter, [space()])?;
        }
        format_redirect_list(&command.redirects, formatter)?;
    }

    emit_heredocs(&command.redirects, formatter)
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
            write!(formatter, [text(render_assignment(assignment, source))])
        }
    }
}

fn format_pipeline(
    pipeline: &Pipeline,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    if pipeline.negated {
        write!(formatter, [text("! ")])?;
    }

    let multiline = formatter.context().options().binary_next_line()
        && pipeline.commands.len() > 1
        && pipeline_has_explicit_line_break(pipeline, formatter.context().source());
    for (index, command) in pipeline.commands.iter().enumerate() {
        if index > 0 {
            if multiline {
                write!(formatter, [text(" \\"), hard_line_break()])?;
                let command_document =
                    format_into_document(formatter, |nested| command.format().fmt(nested))?;
                let mut indented = Document::new();
                indented.push(text("| "));
                indented.extend(command_document);
                write!(formatter, [indent(indented)])?;
                continue;
            }
            write!(formatter, [text(" | ")])?;
        }
        if !multiline || index == 0 {
            command.format().fmt(formatter)?;
        }
    }
    Ok(())
}

fn pipeline_has_explicit_line_break(pipeline: &Pipeline, source: &str) -> bool {
    let mut previous_end = match pipeline.commands.first() {
        Some(command) => command_span(command).end.offset,
        None => return false,
    };

    for command in pipeline.commands.iter().skip(1) {
        let next_start = command_span(command).start.offset;
        let Some(between) = source.get(previous_end..next_start) else {
            previous_end = command_span(command).end.offset;
            continue;
        };
        if between.contains('\n') {
            return true;
        }
        previous_end = command_span(command).end.offset;
    }

    false
}

fn format_command_list(
    list: &CommandList,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    list.first.format().fmt(formatter)?;
    for item in &list.rest {
        format_list_item(item, formatter)?;
    }
    Ok(())
}

fn format_list_item(
    item: &CommandListItem,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    if list_item_has_explicit_line_break(item, formatter) {
        write!(
            formatter,
            [text(list_item_multiline_separator(item.operator))]
        )?;
        let command_document =
            format_into_document(formatter, |nested| item.command.format().fmt(nested))?;
        return write!(formatter, [hard_line_break(), indent(command_document)]);
    }

    write!(formatter, [text(list_item_inline_separator(item.operator))])?;
    item.command.format().fmt(formatter)
}

fn list_item_inline_separator(operator: ListOperator) -> &'static str {
    match operator {
        ListOperator::And => " && ",
        ListOperator::Or => " || ",
        ListOperator::Semicolon => "; ",
        ListOperator::Background => " & ",
    }
}

fn list_item_multiline_separator(operator: ListOperator) -> &'static str {
    match operator {
        ListOperator::And => " &&",
        ListOperator::Or => " ||",
        ListOperator::Semicolon => ";",
        ListOperator::Background => " &",
    }
}

fn list_item_has_explicit_line_break(
    item: &CommandListItem,
    formatter: &ShellFormatter<'_, '_>,
) -> bool {
    let source = formatter.context().source();
    let options = formatter.context().options();
    let source_map = formatter.context().comments().source_map();
    let command_start = command_attachment_span(&item.command, source, source_map, options)
        .start
        .offset;
    source
        .get(item.operator_span.end.offset..command_start)
        .is_some_and(|between| between.contains('\n'))
}

fn format_if(command: &IfCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    let upper_bound = Some(command.span.end.offset);
    write!(formatter, [text("if ")])?;
    format_inline_commands(&command.condition, formatter)?;
    if command.elif_branches.is_empty()
        && command.else_branch.is_none()
        && can_inline_body(&command.then_branch, command.span, formatter)
    {
        write!(formatter, [text("; then ")])?;
        format_inline_commands(&command.then_branch, formatter)?;
        return write!(formatter, [text("; fi")]);
    }
    write!(formatter, [text("; then")])?;
    format_body_with_upper_bound(&command.then_branch, formatter, upper_bound)?;
    for (condition, body) in &command.elif_branches {
        if formatter.context().options().compact_layout() {
            write!(formatter, [text("; elif ")])?;
            format_inline_commands(condition, formatter)?;
            write!(formatter, [text("; then")])?;
        } else {
            write!(formatter, [hard_line_break(), text("elif ")])?;
            format_inline_commands(condition, formatter)?;
            write!(formatter, [text("; then")])?;
        }
        format_body_with_upper_bound(body, formatter, upper_bound)?;
    }
    if let Some(body) = &command.else_branch {
        if formatter.context().options().compact_layout() {
            write!(formatter, [text("; else")])?;
        } else {
            write!(formatter, [hard_line_break(), text("else")])?;
        }
        format_body_with_upper_bound(body, formatter, upper_bound)?;
    }
    if formatter.context().options().compact_layout() {
        write!(formatter, [text("; fi")])
    } else {
        write!(formatter, [hard_line_break(), text("fi")])
    }
}

fn format_for(command: &ForCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    write!(formatter, [text(format!("for {}", command.variable))])?;
    if let Some(words) = &command.words {
        write!(formatter, [text(" in ")])?;
        for (index, word) in words.iter().enumerate() {
            if index > 0 {
                write!(formatter, [space()])?;
            }
            word.format().fmt(formatter)?;
        }
    }
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body_with_upper_bound(&command.body, formatter, Some(command.span.end.offset))?;
    finish_block("done", formatter)
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
        format_inline_commands(&command.body, formatter)?;
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
    format_inline_commands(&command.condition, formatter)?;
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
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
    format_inline_commands(&command.condition, formatter)?;
    if can_inline_body(&command.body, command.span, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
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
            text(command.word.render_syntax(formatter.context().source())),
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
        pattern.push_str(&word.render_syntax(source));
    }
    pattern.push(')');
    if base_indent > 0 {
        write_case_prefix(base_indent, formatter)?;
    }
    write!(formatter, [text(pattern)])?;

    if item.commands.is_empty() {
        write!(
            formatter,
            [text(format!(" {}", case_terminator(item.terminator)))]
        )
    } else if formatter.context().options().compact_layout() {
        write!(formatter, [space()])?;
        format_command_sequence_with_upper_bound(&item.commands, formatter, upper_bound)?;
        write!(
            formatter,
            [text(format!("; {}", case_terminator(item.terminator)))]
        )
    } else {
        if base_indent == 0 && item.commands.len() == 1 {
            write!(formatter, [space()])?;
            item.commands[0].format().fmt(formatter)?;
            write!(formatter, [space(), text(case_terminator(item.terminator))])?;
            return Ok(());
        }

        let commands_document = format_into_document(formatter, |nested| {
            format_command_sequence_with_upper_bound(&item.commands, nested, upper_bound)
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

fn format_block(
    open: &str,
    close: &str,
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    leading_space: bool,
) -> FormatResult<()> {
    format_block_with_upper_bound(open, close, commands, formatter, leading_space, None)
}

fn format_brace_group(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if group_open_suffix(commands, formatter.context().source(), '{').is_none()
        && can_inline_group(commands, formatter)
    {
        write!(formatter, [text("{ ")])?;
        format_inline_commands(commands, formatter)?;
        return write!(formatter, [text("; }")]);
    }

    format_group_with_upper_bound("{", "}", '{', commands, formatter, false, upper_bound)
}

fn format_subshell(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if group_open_suffix(commands, formatter.context().source(), '(').is_none()
        && can_inline_group(commands, formatter)
    {
        write!(formatter, [text("(")])?;
        format_inline_commands(commands, formatter)?;
        return write!(formatter, [text(")")]);
    }

    format_group_with_upper_bound("(", ")", '(', commands, formatter, false, upper_bound)
}

fn format_block_with_upper_bound(
    open: &str,
    close: &str,
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    leading_space: bool,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if leading_space {
        write!(formatter, [space()])?;
    }
    write!(formatter, [text(open)])?;
    format_body_with_upper_bound(commands, formatter, upper_bound)?;
    finish_block(close, formatter)
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

fn format_function(
    function: &FunctionDef,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text(format!("{}()", function.name))])?;
    if formatter.context().options().function_next_line() {
        write!(formatter, [hard_line_break()])?;
    } else {
        write!(formatter, [space()])?;
    }
    match function.body.as_ref() {
        Command::Compound(CompoundCommand::BraceGroup(commands), redirects)
            if redirects.is_empty() =>
        {
            if !formatter.context().options().function_next_line()
                && can_inline_group(commands, formatter)
            {
                write!(formatter, [text("{ ")])?;
                format_inline_commands(commands, formatter)?;
                return write!(formatter, [text("; }")]);
            }
            format_brace_group(commands, formatter, Some(function.span.end.offset))
        }
        Command::Compound(CompoundCommand::Subshell(commands), redirects)
            if redirects.is_empty() =>
        {
            if !formatter.context().options().function_next_line()
                && can_inline_group(commands, formatter)
            {
                write!(formatter, [text("(")])?;
                format_inline_commands(commands, formatter)?;
                return write!(formatter, [text(")")]);
            }
            format_subshell(commands, formatter, Some(function.span.end.offset))
        }
        _ => function.body.format().fmt(formatter),
    }
}

fn format_inline_commands(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    for (index, command) in commands.iter().enumerate() {
        if index > 0 {
            write!(formatter, [text("; ")])?;
        }
        command.format().fmt(formatter)?;
    }
    Ok(())
}

fn format_body_with_upper_bound(
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if commands.is_empty() {
        return Ok(());
    }

    if formatter.context().options().compact_layout() {
        write!(formatter, [space()])?;
        format_command_sequence_with_upper_bound(commands, formatter, upper_bound)
    } else {
        let body = format_into_document(formatter, |nested| {
            format_command_sequence_with_upper_bound(commands, nested, upper_bound)
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
        write!(
            formatter,
            [verbatim(render_heredoc_tail(
                heredoc.body.span,
                &heredoc.delimiter.raw.render_syntax(source),
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

fn render_assignment(assignment: &Assignment, source: &str) -> String {
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
    match &assignment.value {
        AssignmentValue::Scalar(value) => rendered.push_str(&value.render_syntax(source)),
        AssignmentValue::Compound(array) => {
            rendered.push('(');
            for (index, value) in array.elements.iter().enumerate() {
                if index > 0 {
                    rendered.push(' ');
                }
                rendered.push_str(&render_array_elem(value, source));
            }
            rendered.push(')');
        }
    }
    trim_unescaped_trailing_whitespace(&rendered).to_string()
}

fn render_array_elem(element: &ArrayElem, source: &str) -> String {
    match element {
        ArrayElem::Sequential(word) => word.render_syntax(source),
        ArrayElem::Keyed { key, value } => {
            format!(
                "[{}]={}",
                render_subscript(key, source),
                value.render_syntax(source)
            )
        }
        ArrayElem::KeyedAppend { key, value } => {
            format!(
                "[{}]+={}",
                render_subscript(key, source),
                value.render_syntax(source)
            )
        }
    }
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

        let backslash_count = text[..whitespace_start]
            .as_bytes()
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

fn has_heredoc(command: &Command) -> bool {
    match command {
        Command::Simple(command) => command.redirects.iter().any(is_heredoc),
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(BreakCommand { redirects, .. })
            | BuiltinCommand::Continue(ContinueCommand { redirects, .. })
            | BuiltinCommand::Return(ReturnCommand { redirects, .. })
            | BuiltinCommand::Exit(ExitCommand { redirects, .. }) => {
                redirects.iter().any(is_heredoc)
            }
        },
        Command::Decl(command) => command.redirects.iter().any(is_heredoc),
        Command::Pipeline(pipeline) => pipeline.commands.iter().any(has_heredoc),
        Command::List(list) => {
            has_heredoc(&list.first) || list.rest.iter().any(|item| has_heredoc(&item.command))
        }
        Command::Compound(_, redirects) => redirects.iter().any(is_heredoc),
        Command::Function(function) => has_heredoc(&function.body),
    }
}

fn is_heredoc(redirect: &shuck_ast::Redirect) -> bool {
    matches!(
        redirect.kind,
        RedirectKind::HereDoc | RedirectKind::HereDocStrip
    )
}

fn verbatim_command(command: &Command, source: &str) -> Option<FormatElement> {
    let span = command_verbatim_span(command, source);
    (span.end.offset <= source.len()).then(|| verbatim(span.slice(source)))
}

fn verbatim_commands(commands: &[Command], source: &str) -> Option<FormatElement> {
    let span = commands
        .iter()
        .map(|command| command_verbatim_span(command, source))
        .reduce(|left, right| left.merge(right))?;
    (span.end.offset <= source.len()).then(|| verbatim(span.slice(source)))
}

fn command_verbatim_span(command: &Command, source: &str) -> Span {
    let span = match command {
        Command::Simple(command) => {
            merge_redirect_heredoc_spans(command.span, &command.redirects, source)
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                merge_redirect_heredoc_spans(command.span, &command.redirects, source)
            }
            BuiltinCommand::Continue(command) => {
                merge_redirect_heredoc_spans(command.span, &command.redirects, source)
            }
            BuiltinCommand::Return(command) => {
                merge_redirect_heredoc_spans(command.span, &command.redirects, source)
            }
            BuiltinCommand::Exit(command) => {
                merge_redirect_heredoc_spans(command.span, &command.redirects, source)
            }
        },
        Command::Decl(command) => {
            merge_redirect_heredoc_spans(command.span, &command.redirects, source)
        }
        Command::Pipeline(command) => command
            .commands
            .iter()
            .map(|command| command_verbatim_span(command, source))
            .reduce(|left, right| left.merge(right))
            .unwrap_or(command.span),
        Command::List(command) => command.rest.iter().fold(
            command_verbatim_span(&command.first, source),
            |span, item| span.merge(command_verbatim_span(&item.command, source)),
        ),
        Command::Compound(command, redirects) => {
            merge_redirect_heredoc_spans(compound_verbatim_span(command, source), redirects, source)
        }
        Command::Function(command) => command
            .name_span
            .merge(command_verbatim_span(&command.body, source)),
    };

    if span == Span::new() {
        command_span(command)
    } else {
        span
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

fn command_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => command.span,
            BuiltinCommand::Continue(command) => command.span,
            BuiltinCommand::Return(command) => command.span,
            BuiltinCommand::Exit(command) => command.span,
        },
        Command::Decl(command) => command.span,
        Command::Pipeline(command) => command.span,
        Command::List(command) => command.span,
        Command::Compound(command, redirects) => redirects
            .iter()
            .fold(compound_span(command), |span, redirect| {
                span.merge(redirect.span)
            }),
        Command::Function(command) => command.span,
    }
}

fn compound_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(command_span)
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
    }
}

fn compound_verbatim_span(command: &CompoundCommand, source: &str) -> Span {
    match command {
        CompoundCommand::If(command) => {
            let mut span = command.span;
            span = merge_command_sequence_verbatim_span(span, &command.condition, source);
            span = merge_command_sequence_verbatim_span(span, &command.then_branch, source);
            for (condition, body) in &command.elif_branches {
                span = merge_command_sequence_verbatim_span(span, condition, source);
                span = merge_command_sequence_verbatim_span(span, body, source);
            }
            if let Some(body) = &command.else_branch {
                span = merge_command_sequence_verbatim_span(span, body, source);
            }
            span
        }
        CompoundCommand::For(command) => {
            merge_command_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::ArithmeticFor(command) => {
            merge_command_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::While(command) => {
            let span =
                merge_command_sequence_verbatim_span(command.span, &command.condition, source);
            merge_command_sequence_verbatim_span(span, &command.body, source)
        }
        CompoundCommand::Until(command) => {
            let span =
                merge_command_sequence_verbatim_span(command.span, &command.condition, source);
            merge_command_sequence_verbatim_span(span, &command.body, source)
        }
        CompoundCommand::Case(command) => {
            let mut span = command.span;
            for item in &command.cases {
                span = merge_command_sequence_verbatim_span(span, &item.commands, source);
            }
            span
        }
        CompoundCommand::Select(command) => {
            merge_command_sequence_verbatim_span(command.span, &command.body, source)
        }
        CompoundCommand::Subshell(commands) => group_verbatim_span(commands, source, '(', ')'),
        CompoundCommand::BraceGroup(commands) => group_verbatim_span(commands, source, '{', '}'),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command
            .command
            .as_ref()
            .map(|inner| command.span.merge(command_verbatim_span(inner, source)))
            .unwrap_or(command.span),
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command
            .span
            .merge(command_verbatim_span(&command.body, source)),
    }
}

fn merge_command_sequence_verbatim_span(
    mut span: Span,
    commands: &[Command],
    source: &str,
) -> Span {
    for command in commands {
        span = merge_non_empty_span(span, command_verbatim_span(command, source));
    }
    span
}

fn group_verbatim_span(commands: &[Command], source: &str, open: char, close: char) -> Span {
    let inner = commands
        .iter()
        .map(|command| command_verbatim_span(command, source))
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
    commands: &[Command],
    formatter: &mut ShellFormatter<'_, '_>,
    leading_space: bool,
    upper_bound: Option<usize>,
) -> FormatResult<()> {
    if leading_space {
        write!(formatter, [space()])?;
    }
    write!(formatter, [text(open)])?;
    if let Some((span, suffix)) =
        group_open_suffix(commands, formatter.context().source(), open_char)
    {
        formatter.context_mut().comments_mut().claim_in_span(span);
        write!(formatter, [text(suffix.to_string())])?;
    }
    format_body_with_upper_bound(commands, formatter, upper_bound)?;
    finish_block(close, formatter)
}

fn group_open_suffix<'a>(
    commands: &[Command],
    source: &'a str,
    open: char,
) -> Option<(Span, &'a str)> {
    let first = commands.first()?;
    let first_start = command_span(first).start.offset;
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

fn group_attachment_span(commands: &[Command], source: &str, open: char) -> Option<Span> {
    let first = commands.first()?;
    let last = commands.last()?;
    let open_offset = source[..command_span(first).start.offset].rfind(open)?;
    Some(span_for_offsets(
        source,
        open_offset,
        command_format_span(last).end.offset,
    ))
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
                &command.redirects,
            ),
            BuiltinCommand::Continue(command) => builtin_like_span(
                command.span.start,
                "continue",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
            BuiltinCommand::Return(command) => builtin_like_span(
                command.span.start,
                "return",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
            BuiltinCommand::Exit(command) => builtin_like_span(
                command.span.start,
                "exit",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
        },
        Command::Decl(command) => decl_clause_format_span(command),
        Command::Pipeline(command) => command
            .commands
            .iter()
            .map(command_format_span)
            .reduce(|left, right| left.merge(right))
            .unwrap_or(command.span),
        Command::List(command) => command
            .rest
            .iter()
            .fold(command_format_span(&command.first), |span, item| {
                span.merge(command_format_span(&item.command))
            }),
        Command::Compound(command, redirects) => redirects
            .iter()
            .fold(compound_format_span(command), |span, redirect| {
                span.merge(redirect.span)
            }),
        Command::Function(command) => command.name_span.merge(command_format_span(&command.body)),
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
    for redirect in &command.redirects {
        span = merge_non_empty_span(span, redirect.span);
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
    redirects: &[Redirect],
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
    for redirect in redirects {
        span = merge_non_empty_span(span, redirect.span);
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
    for redirect in &command.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    span
}

fn compound_format_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(command_format_span)
            .reduce(|left, right| left.merge(right))
            .unwrap_or_default(),
        _ => compound_span(command),
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
        write!(formatter, [text("  "), text(comment.text().to_string())])?;
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

fn rendered_command_end_line(
    command: &Command,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
) -> usize {
    let span = match command {
        Command::Function(_) => command_span(command),
        _ if has_heredoc(command) => command_verbatim_span(command, source),
        _ => command_format_span(command),
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
    commands: &[Command],
    enclosing_span: Span,
    formatter: &ShellFormatter<'_, '_>,
) -> bool {
    let [command] = commands else {
        return false;
    };
    if !can_inline_command(command, formatter) {
        return false;
    }

    let has_comments = {
        let source = formatter.context().source();
        let source_map = formatter.context().comments().source_map();
        let options = formatter.context().options();
        let span = command_attachment_span(command, source, source_map, options);
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
        || command_span(command).start.line == enclosing_span.start.line
}

fn can_inline_group(commands: &[Command], formatter: &ShellFormatter<'_, '_>) -> bool {
    let [command] = commands else {
        return false;
    };

    can_inline_command(command, formatter)
        && command_span(command).start.line == command_span(command).end.line
        && can_inline_body(commands, command_span(command), formatter)
}

fn can_inline_command(command: &Command, formatter: &ShellFormatter<'_, '_>) -> bool {
    if has_heredoc(command)
        || command_has_trailing_comment(command, formatter.context().comments().source_map())
    {
        return false;
    }

    matches!(
        command,
        Command::Simple(_)
            | Command::Builtin(_)
            | Command::Decl(_)
            | Command::Pipeline(_)
            | Command::List(_)
            | Command::Compound(
                CompoundCommand::Conditional(_)
                    | CompoundCommand::Arithmetic(_)
                    | CompoundCommand::Time(_),
                _
            )
    )
}

fn command_has_trailing_comment(
    command: &Command,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let raw = command_span(command);
    let formatted = command_format_span(command);
    raw.end.offset > formatted.end.offset
        && source_map.contains_comment_between(formatted.end.offset, raw.end.offset)
}

fn should_render_verbatim(
    command: &Command,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> bool {
    (options.keep_padding() && command_has_alignment_sensitive_padding(command, source_map))
        || (has_heredoc(command) && command_has_trailing_comment(command, source_map))
}

fn command_attachment_span(
    command: &Command,
    source: &str,
    source_map: &crate::comments::SourceMap<'_>,
    options: &crate::options::ResolvedShellFormatOptions,
) -> Span {
    if should_render_verbatim(command, source_map, options) {
        command_verbatim_span(command, source)
    } else if let Command::Compound(CompoundCommand::BraceGroup(commands), redirects) = command {
        redirects.iter().fold(
            group_attachment_span(commands, source, '{')
                .unwrap_or_else(|| command_format_span(command)),
            |span, redirect| span.merge(redirect.span),
        )
    } else if let Command::Compound(CompoundCommand::Subshell(commands), redirects) = command {
        redirects.iter().fold(
            group_attachment_span(commands, source, '(')
                .unwrap_or_else(|| command_format_span(command)),
            |span, redirect| span.merge(redirect.span),
        )
    } else {
        command_format_span(command)
    }
}

fn command_has_alignment_sensitive_padding(
    command: &Command,
    source_map: &crate::comments::SourceMap<'_>,
) -> bool {
    let mut spans = command_token_spans(command);
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
            spans.extend(command.redirects.iter().map(|redirect| redirect.span));
            spans
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => builtin_like_token_spans(
                command.span.start,
                "break",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
            BuiltinCommand::Continue(command) => builtin_like_token_spans(
                command.span.start,
                "continue",
                &command.assignments,
                command.depth.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
            BuiltinCommand::Return(command) => builtin_like_token_spans(
                command.span.start,
                "return",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
                &command.redirects,
            ),
            BuiltinCommand::Exit(command) => builtin_like_token_spans(
                command.span.start,
                "exit",
                &command.assignments,
                command.code.as_ref(),
                &command.extra_args,
                &command.redirects,
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
            spans.extend(command.redirects.iter().map(|redirect| redirect.span));
            spans
        }
        Command::Compound(command, redirects) => {
            let mut spans = vec![compound_format_span(command)];
            spans.extend(redirects.iter().map(|redirect| redirect.span));
            spans
        }
        Command::Function(command) => vec![command.name_span, command_format_span(&command.body)],
        _ => vec![command_format_span(command)],
    }
}

fn builtin_like_token_spans(
    start: shuck_ast::Position,
    name: &str,
    assignments: &[Assignment],
    primary: Option<&shuck_ast::Word>,
    extra_args: &[shuck_ast::Word],
    redirects: &[Redirect],
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
    spans.extend(redirects.iter().map(|redirect| redirect.span));
    spans
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
        ConditionalExpr::Pattern(pattern) => {
            write!(
                formatter,
                [text(pattern.render_syntax(formatter.context().source()))]
            )
        }
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

    use shuck_parser::parser::{Parser, ShellDialect};

    #[test]
    fn parsed_standalone_assignment_renders_without_trailing_space() {
        let source = "x=1\n";
        let parsed = Parser::with_dialect(source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let Command::Simple(command) = &parsed.script.commands[0] else {
            panic!("expected a simple command");
        };

        assert_eq!(render_assignment(&command.assignments[0], source), "x=1");
        assert!(command.args.is_empty());
        assert!(command.redirects.is_empty());
        assert!(!command.name.parts.is_empty());
        assert!(command.name.render_syntax(source).is_empty());
    }
}
