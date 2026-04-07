use shuck_ast::{
    ArithmeticCommand, ArithmeticForCommand, Assignment, AssignmentValue, BreakCommand,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandList, CommandListItem,
    CompoundCommand, ConditionalBinaryExpr, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ContinueCommand, CoprocCommand, DeclClause,
    DeclOperand, ExitCommand, ForCommand, FunctionDef, IfCommand, ListOperator, Pipeline,
    Redirect, RedirectKind, ReturnCommand, SelectCommand, SimpleCommand, Span, TimeCommand,
    UntilCommand, WhileCommand,
};
use shuck_format::{
    Document, Format, FormatElement, FormatResult, hard_line_break, indent, space, text,
    verbatim, write,
};

use crate::FormatNodeRule;
use crate::prelude::{AsFormat, ShellFormatter};

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCommand;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatCompoundCommand;

impl FormatNodeRule<Command> for FormatCommand {
    fn fmt(&self, command: &Command, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        if formatter.context().options().keep_padding()
            && let Some(document) = verbatim_command(command, formatter.context().source())
        {
            return write!(formatter, [document]);
        }

        if has_heredoc(command)
            && command_has_trailing_comment(command, formatter.context().source())
            && let Some(document) = verbatim_command(command, formatter.context().source())
        {
            return write!(formatter, [document]);
        }

        match command {
            Command::Simple(command) => format_simple_command(command, formatter),
            Command::Builtin(command) => format_builtin_command(command, formatter),
            Command::Decl(command) => {
                if command.redirects.iter().any(is_heredoc) {
                    let source = formatter.context().source();
                    let span = merge_redirect_heredoc_spans(command.span, &command.redirects, source);
                    if span.end.offset <= source.len() {
                        return write!(formatter, [verbatim(span.slice(source))]);
                    }
                }
                let source = formatter.context().source();
                let rendered = source
                    .get(command.span.start.offset..command.span.end.offset)
                    .unwrap_or_default()
                    .to_string();
                write!(formatter, [text(rendered)])
            }
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
            CompoundCommand::Subshell(commands) => {
                format_block("(", ")", commands, formatter, false)
            }
            CompoundCommand::BraceGroup(commands) => {
                format_block("{", "}", commands, formatter, false)
            }
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
    if commands.is_empty() {
        return Ok(());
    }

    let compact = formatter.context().options().compact_layout();
    let attachments = if formatter.context().options().minify() {
        None
    } else {
        let spans = commands.iter().map(command_format_span).collect::<Vec<_>>();
        Some(
            formatter
                .context_mut()
                .comments_mut()
                .attach_sequence(&spans, None),
        )
    };

    if attachments.as_ref().is_some_and(|value| value.is_ambiguous()) {
        if let Some(document) = verbatim_commands(commands, formatter.context().source()) {
            return write!(formatter, [document]);
        }
    }

    for (index, command) in commands.iter().enumerate() {
        if let Some(attachment) = &attachments {
            emit_attached_comments(attachment.leading_for(index), formatter, false)?;
        } else {
            emit_leading_comments(command_start_line(command), formatter)?;
        }
        command.format().fmt(formatter)?;
        if let Some(attachment) = &attachments {
            emit_attached_comments(attachment.trailing_for(index), formatter, true)?;
        } else {
            emit_inline_comments(command_end_line(command), formatter)?;
        }
        if index + 1 < commands.len() {
            if compact {
                write!(formatter, [text("; ")])?;
            } else {
                write!(formatter, [hard_line_break()])?;
            }
        }
    }

    if let Some(attachment) = &attachments {
        for comment in attachment.dangling() {
            write!(formatter, [hard_line_break(), text(comment.text().to_string())])?;
        }
    }
    Ok(())
}

fn format_simple_command(
    command: &SimpleCommand,
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

    if !command.name.parts.is_empty() {
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
        pieces.push(primary.render(source));
    }
    for argument in extra_args {
        pieces.push(argument.render(source));
    }

    write!(formatter, [text(pieces.join(" "))])?;
    if !redirects.is_empty() {
        write!(formatter, [space()])?;
        format_redirect_list(redirects, formatter)?;
    }
    emit_heredocs(redirects, formatter)
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
                let command_document = format_into_document(formatter, |nested| {
                    command.format().fmt(nested)
                })?;
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

fn format_command_list(list: &CommandList, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
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
    let separator = match item.operator {
        ListOperator::And => " && ",
        ListOperator::Or => " || ",
        ListOperator::Semicolon => "; ",
        ListOperator::Background => " & ",
    };
    write!(formatter, [text(separator)])?;
    item.command.format().fmt(formatter)
}

fn format_if(command: &IfCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    write!(formatter, [text("if ")])?;
    format_inline_commands(&command.condition, formatter)?;
    if command.elif_branches.is_empty()
        && command.else_branch.is_none()
        && can_inline_sequence(&command.then_branch, formatter)
    {
        write!(formatter, [text("; then ")])?;
        format_inline_commands(&command.then_branch, formatter)?;
        return write!(formatter, [text("; fi")]);
    }
    write!(formatter, [text("; then")])?;
    format_body(&command.then_branch, formatter)?;
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
        format_body(body, formatter)?;
    }
    if let Some(body) = &command.else_branch {
        if formatter.context().options().compact_layout() {
            write!(formatter, [text("; else")])?;
        } else {
            write!(formatter, [hard_line_break(), text("else")])?;
        }
        format_body(body, formatter)?;
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
    if can_inline_sequence(&command.body, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body(&command.body, formatter)?;
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
    if can_inline_sequence(&command.body, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body(&command.body, formatter)?;
    finish_block("done", formatter)
}

fn format_while(
    command: &WhileCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("while ")])?;
    format_inline_commands(&command.condition, formatter)?;
    if can_inline_sequence(&command.body, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body(&command.body, formatter)?;
    finish_block("done", formatter)
}

fn format_until(
    command: &UntilCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    write!(formatter, [text("until ")])?;
    format_inline_commands(&command.condition, formatter)?;
    if can_inline_sequence(&command.body, formatter) {
        write!(formatter, [text("; do ")])?;
        format_inline_commands(&command.body, formatter)?;
        return write!(formatter, [text("; done")]);
    }
    write!(formatter, [text("; do")])?;
    format_body(&command.body, formatter)?;
    finish_block("done", formatter)
}

fn format_case(command: &CaseCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    write!(
        formatter,
        [
            text("case "),
            text(command.word.render(formatter.context().source())),
            text(" in")
        ]
    )?;

    if formatter.context().options().compact_layout() {
        for item in &command.cases {
            write!(formatter, [text(" ")])?;
            format_case_item(item, formatter)?;
        }
        write!(formatter, [text(" esac")])
    } else {
        for item in &command.cases {
            write!(formatter, [hard_line_break()])?;
            format_case_item(item, formatter)?;
        }
        write!(formatter, [hard_line_break(), text("esac")])
    }
}

fn format_case_item(item: &CaseItem, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
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
        pattern.push_str(&word.render(source));
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
        format_command_sequence(&item.commands, formatter)?;
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
            format_command_sequence(&item.commands, nested)
        })?;
        write!(formatter, [hard_line_break(), indent_levels(commands_document, base_indent + 1)])?;

        let terminator = Document::from_element(text(case_terminator(item.terminator)));
        write!(
            formatter,
            [hard_line_break(), indent_levels(terminator, base_indent + 1)]
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
    if leading_space {
        write!(formatter, [space()])?;
    }
    write!(formatter, [text(open)])?;
    format_body(commands, formatter)?;
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
    format_body(&command.body, formatter)?;
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

fn format_conditional(command: &ConditionalCommand, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
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
    function.body.format().fmt(formatter)
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

fn format_body(commands: &[Command], formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if commands.is_empty() {
        return Ok(());
    }

    if formatter.context().options().compact_layout() {
        write!(formatter, [space()])?;
        format_command_sequence(commands, formatter)
    } else {
        let body = format_into_document(formatter, |nested| {
            format_command_sequence(commands, nested)
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

fn format_redirect_list(redirects: &[Redirect], formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    for (index, redirect) in redirects.iter().enumerate() {
        if index > 0 {
            write!(formatter, [space()])?;
        }
        redirect.format().fmt(formatter)?;
    }
    Ok(())
}

fn emit_heredocs(redirects: &[Redirect], formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    let source = formatter.context().source();
    for redirect in redirects {
        let Some(heredoc) = redirect.heredoc() else {
            continue;
        };
        write!(
            formatter,
            [verbatim(render_heredoc_tail(
                heredoc.body.span,
                &heredoc.delimiter.raw.render(source),
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
    if assignment.span.end.offset <= source.len() {
        return assignment.span.slice(source).to_string();
    }

    let mut rendered = assignment.name.to_string();
    if let Some(index) = &assignment.index {
        rendered.push('[');
        rendered.push_str(index.slice(source));
        rendered.push(']');
    }
    if assignment.append {
        rendered.push_str("+=");
    } else {
        rendered.push('=');
    }
    match &assignment.value {
        AssignmentValue::Scalar(value) => rendered.push_str(&value.render(source)),
        AssignmentValue::Array(values) => {
            rendered.push('(');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    rendered.push(' ');
                }
                rendered.push_str(&value.render(source));
            }
            rendered.push(')');
        }
    }
    rendered
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
        Command::Simple(command) => merge_redirect_heredoc_spans(command.span, &command.redirects, source),
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
        Command::Decl(command) => merge_redirect_heredoc_spans(command.span, &command.redirects, source),
        Command::Pipeline(command) => command
            .commands
            .iter()
            .map(|command| command_verbatim_span(command, source))
            .reduce(|left, right| left.merge(right))
            .unwrap_or(command.span),
        Command::List(command) => command
            .rest
            .iter()
            .fold(command_verbatim_span(&command.first, source), |span, item| {
                span.merge(command_verbatim_span(&item.command, source))
            }),
        Command::Compound(command, redirects) => merge_redirect_heredoc_spans(
            compound_verbatim_span(command, source),
            redirects,
            source,
        ),
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
            let span = merge_command_sequence_verbatim_span(command.span, &command.condition, source);
            merge_command_sequence_verbatim_span(span, &command.body, source)
        }
        CompoundCommand::Until(command) => {
            let span = merge_command_sequence_verbatim_span(command.span, &command.condition, source);
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
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            commands
                .iter()
                .map(|command| command_verbatim_span(command, source))
                .reduce(|left, right| left.merge(right))
                .unwrap_or_default()
        }
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command
            .command
            .as_ref()
            .map(|inner| command.span.merge(command_verbatim_span(inner, source)))
            .unwrap_or(command.span),
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => {
            command.span.merge(command_verbatim_span(&command.body, source))
        }
    }
}

fn merge_command_sequence_verbatim_span(mut span: Span, commands: &[Command], source: &str) -> Span {
    for command in commands {
        span = merge_non_empty_span(span, command_verbatim_span(command, source));
    }
    span
}

fn command_start_line(command: &Command) -> usize {
    command_format_span(command).start.line
}

fn command_end_line(command: &Command) -> usize {
    let span = command_format_span(command);
    if span.end.column == 1 && span.end.line > span.start.line {
        span.end.line - 1
    } else {
        span.end.line
    }
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
        Command::Function(command) => command
            .name_span
            .merge(command_format_span(&command.body)),
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
    if span == Span::new() { command.span } else { span }
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

fn emit_leading_comments(line: usize, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if formatter.context().options().minify() {
        return Ok(());
    }

    let comments = formatter
        .context_mut()
        .comments_mut()
        .take_leading_before(line);
    for comment in comments {
        write!(
            formatter,
            [text(comment.text().to_string()), hard_line_break()]
        )?;
    }
    Ok(())
}

fn emit_inline_comments(line: usize, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
    if formatter.context().options().minify() {
        return Ok(());
    }

    let comments = formatter
        .context_mut()
        .comments_mut()
        .take_inline_for_line(line);
    for comment in comments {
        write!(formatter, [text("  "), text(comment.text().to_string())])?;
    }
    Ok(())
}

fn emit_attached_comments(
    comments: &[crate::comments::SourceComment<'_>],
    formatter: &mut ShellFormatter<'_, '_>,
    inline: bool,
) -> FormatResult<()> {
    for comment in comments {
        if inline {
            write!(formatter, [text("  "), text(comment.text().to_string())])?;
        } else {
            write!(formatter, [text(comment.text().to_string()), hard_line_break()])?;
        }
    }
    Ok(())
}

fn can_inline_sequence(commands: &[Command], formatter: &ShellFormatter<'_, '_>) -> bool {
    matches!(commands, [command] if can_inline_command(command, formatter))
}

fn can_inline_command(command: &Command, formatter: &ShellFormatter<'_, '_>) -> bool {
    if has_heredoc(command) || command_has_trailing_comment(command, formatter.context().source()) {
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

fn command_has_trailing_comment(command: &Command, source: &str) -> bool {
    let raw = command_span(command);
    let formatted = command_format_span(command);
    raw.end.offset > formatted.end.offset
        && raw.start.offset <= source.len()
        && raw.end.offset <= source.len()
        && source[formatted.end.offset..raw.end.offset].contains('#')
}

fn format_conditional_expr(
    expression: &ConditionalExpr,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    match expression {
        ConditionalExpr::Binary(expr) => format_conditional_binary(expr, formatter),
        ConditionalExpr::Unary(expr) => format_conditional_unary(expr, formatter),
        ConditionalExpr::Parenthesized(expr) => format_conditional_paren(expr, formatter),
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => word.format().fmt(formatter),
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
