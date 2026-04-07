use shuck_ast::{
    ArithmeticCommand, ArithmeticForCommand, Assignment, AssignmentValue, BreakCommand,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandList, CommandListItem,
    CompoundCommand, ConditionalCommand, ContinueCommand, CoprocCommand, ExitCommand, ForCommand,
    FunctionDef, IfCommand, ListOperator, Pipeline, RedirectKind, RedirectTarget, ReturnCommand,
    SelectCommand, SimpleCommand, Span, TimeCommand, UntilCommand, WhileCommand,
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
        if formatter.context().options().keep_padding()
            && let Some(document) = verbatim_command(command, formatter.context().source())
        {
            return write!(formatter, [document]);
        }

        if has_heredoc(command)
            && let Some(document) = verbatim_command(command, formatter.context().source())
        {
            return write!(formatter, [document]);
        }

        match command {
            Command::Simple(command) => format_simple_command(command, formatter),
            Command::Builtin(command) => format_builtin_command(command, formatter),
            Command::Decl(command) => {
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
                format_redirect_list(redirects, formatter)
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
    let compact = formatter.context().options().compact_layout();
    for (index, command) in commands.iter().enumerate() {
        emit_leading_comments(command_start_line(command), formatter)?;
        command.format().fmt(formatter)?;
        emit_inline_comments(command_end_line(command), formatter)?;
        if index + 1 < commands.len() {
            if compact {
                write!(formatter, [text("; ")])?;
            } else {
                write!(formatter, [hard_line_break()])?;
            }
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
    Ok(())
}

fn format_pipeline(
    pipeline: &Pipeline,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    if pipeline.negated {
        write!(formatter, [text("! ")])?;
    }

    let multiline = formatter.context().options().binary_next_line() && pipeline.commands.len() > 1;
    for (index, command) in pipeline.commands.iter().enumerate() {
        if index > 0 {
            if multiline {
                write!(formatter, [text(" \\"), hard_line_break()])?;
                let indented = Document::from_elements(vec![
                    text("| "),
                    text(command_summary(command, formatter.context().source())),
                ]);
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
            write!(
                formatter,
                [text(command_summary(&item.commands[0], source))]
            )?;
            write!(formatter, [space(), text(case_terminator(item.terminator))])?;
            return Ok(());
        }

        for command in &item.commands {
            write!(formatter, [hard_line_break()])?;
            write_case_prefix(base_indent + 1, formatter)?;
            write!(formatter, [text(command_summary(command, source))])?;
        }
        write!(formatter, [hard_line_break()])?;
        write_case_prefix(base_indent + 1, formatter)?;
        write!(formatter, [text(case_terminator(item.terminator))])
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

fn format_conditional(
    command: &ConditionalCommand,
    formatter: &mut ShellFormatter<'_, '_>,
) -> FormatResult<()> {
    let source = formatter.context().source();
    let rendered = source
        .get(command.span.start.offset..command.span.end.offset)
        .unwrap_or_default()
        .to_string();
    write!(formatter, [text(rendered)])
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
        write!(formatter, [hard_line_break()])?;
        let mut body = Document::new();
        body.push(verbatim(commands_summary(
            commands,
            formatter.context().source(),
        )));
        write!(formatter, [indent(body)])
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
    redirects: &[shuck_ast::Redirect],
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

fn command_verbatim_span(command: &Command, source: &str) -> Span {
    let mut span = command_span(command);
    if let Command::Simple(simple) = command {
        for redirect in &simple.redirects {
            if is_heredoc(redirect)
                && let RedirectTarget::Heredoc(heredoc) = &redirect.target
            {
                span = span.merge(extend_heredoc_target_span(heredoc.body.span, source));
            }
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

fn command_start_line(command: &Command) -> usize {
    command_span(command).start.line
}

fn command_end_line(command: &Command) -> usize {
    let span = command_span(command);
    if span.end.column == 1 && span.end.line > span.start.line {
        span.end.line - 1
    } else {
        span.end.line
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

fn command_summary(command: &Command, source: &str) -> String {
    let span = command_span(command);
    if span.end.offset <= source.len() {
        let mut rendered = span.slice(source).trim_end().replace('\n', " ");
        for suffix in [";;&", ";&", ";;"] {
            if rendered.ends_with(suffix) {
                rendered.truncate(rendered.len().saturating_sub(suffix.len()));
                rendered = rendered.trim_end().to_string();
                break;
            }
        }
        rendered
    } else {
        String::new()
    }
}

fn commands_summary(commands: &[Command], source: &str) -> String {
    let mut rendered = String::new();
    for (index, command) in commands.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&command_summary(command, source));
    }
    rendered
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

fn extend_heredoc_target_span(span: Span, source: &str) -> Span {
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
