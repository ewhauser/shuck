use super::*;

pub(crate) fn stmt_verbatim_span_with_source_map(stmt: &Stmt, source_map: &SourceMap<'_>) -> Span {
    stmt_verbatim_span_impl(stmt, source_map)
}

pub(super) fn stmt_verbatim_span_impl(stmt: &Stmt, source_map: &SourceMap<'_>) -> Span {
    let source = source_map.source();
    let command_span = if let Command::Simple(command) = &stmt.command
        && simple_command_uses_synthetic_words(command, source)
    {
        synthetic_simple_command_verbatim_span(command, source_map)
    } else {
        command_verbatim_span(&stmt.command, source_map)
    };
    let mut span = command_span;
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
        if let Some(heredoc) = redirect.heredoc() {
            span = span.merge(extend_heredoc_body_span(heredoc.body.span, source));
        }
    }
    if stmt.negated {
        span = merge_non_empty_span(stmt.span, span);
    }
    non_empty_or_stmt_span(stmt, merge_stmt_background_span(stmt, span))
}

fn command_verbatim_span(command: &Command, source_map: &SourceMap<'_>) -> Span {
    match command {
        Command::Simple(command) => command.span,
        Command::Builtin(command) => builtin_like_parts(command).0,
        Command::Decl(command) => command.span,
        Command::Binary(command) => stmt_verbatim_span_impl(&command.left, source_map)
            .merge(stmt_verbatim_span_impl(&command.right, source_map)),
        Command::Compound(command) => compound_verbatim_span(command, source_map),
        Command::Function(command) => function_header_span(command)
            .merge(function_body_verbatim_span(&command.body, source_map)),
        Command::AnonymousFunction(command) => anonymous_function_header_span(command)
            .merge(function_body_verbatim_span(&command.body, source_map))
            .merge(words_span(&command.args)),
    }
}

fn function_body_verbatim_span(body: &Stmt, source_map: &SourceMap<'_>) -> Span {
    let mut span = stmt_verbatim_span_impl(body, source_map);
    if let Some(group_span) = command_group_attachment_span(&body.command, source_map) {
        span = merge_non_empty_span(span, group_span);
    }
    span
}

pub(crate) fn stmt_span(stmt: &Stmt) -> Span {
    merge_stmt_background_span(stmt, merge_stmt_redirect_spans(stmt, stmt.span))
}

pub(super) fn complete_stmt_span(stmt: &Stmt, span: Span) -> Span {
    non_empty_or_stmt_span(
        stmt,
        merge_stmt_background_span(stmt, merge_stmt_redirect_spans(stmt, span)),
    )
}

fn merge_stmt_redirect_spans(stmt: &Stmt, mut span: Span) -> Span {
    for redirect in &stmt.redirects {
        span = merge_non_empty_span(span, redirect.span);
    }
    span
}

fn merge_stmt_background_span(stmt: &Stmt, mut span: Span) -> Span {
    if matches!(stmt.terminator, Some(StmtTerminator::Background(_)))
        && let Some(terminator_span) = stmt.terminator_span
    {
        span = merge_non_empty_span(span, terminator_span);
    }
    span
}

fn non_empty_or_stmt_span(stmt: &Stmt, span: Span) -> Span {
    if span == Span::new() {
        stmt_span(stmt)
    } else {
        span
    }
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
            .reduce(Span::merge)
            .unwrap_or_default(),
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

fn compound_verbatim_span(command: &CompoundCommand, source_map: &SourceMap<'_>) -> Span {
    match command {
        CompoundCommand::Subshell(commands) => {
            group_verbatim_span_impl(commands.as_slice(), source_map, '(', ')')
        }
        CompoundCommand::BraceGroup(commands) => {
            group_verbatim_span_impl(commands.as_slice(), source_map, '{', '}')
        }
        _ => compound_verbatim_span_from_children(command, source_map),
    }
}

fn compound_verbatim_span_from_children(
    command: &CompoundCommand,
    source_map: &SourceMap<'_>,
) -> Span {
    let mut span = compound_span(command);
    for_each_compound_child(command, |child| {
        span = match child {
            CompoundChild::Stmt(stmt) => span.merge(stmt_verbatim_span_impl(stmt, source_map)),
            CompoundChild::Sequence(sequence) => {
                merge_stmt_sequence_verbatim_span(span, sequence, source_map)
            }
        };
    });
    span
}

fn merge_stmt_sequence_verbatim_span(
    mut span: Span,
    commands: &StmtSeq,
    source_map: &SourceMap<'_>,
) -> Span {
    for command in commands.iter() {
        span = merge_non_empty_span(span, stmt_verbatim_span_impl(command, source_map));
    }
    span
}

pub(crate) fn command_format_span(command: &Command) -> Span {
    match command {
        Command::Simple(command) => simple_command_format_span(command),
        Command::Builtin(command) => {
            let (span, name, assignments, primary, extra_args) = builtin_like_parts(command);
            builtin_like_span(span.start, name, assignments, primary, extra_args)
        }
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

pub(crate) fn builtin_like_parts(
    command: &BuiltinCommand,
) -> (
    Span,
    &'static str,
    &[Assignment],
    Option<&shuck_ast::Word>,
    &[shuck_ast::Word],
) {
    match command {
        BuiltinCommand::Break(command) => (
            command.span,
            "break",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Continue(command) => (
            command.span,
            "continue",
            &command.assignments,
            command.depth.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Return(command) => (
            command.span,
            "return",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
        ),
        BuiltinCommand::Exit(command) => (
            command.span,
            "exit",
            &command.assignments,
            command.code.as_ref(),
            &command.extra_args,
        ),
    }
}

pub(crate) fn builtin_like_parts_mut(
    command: &mut BuiltinCommand,
) -> (&mut [Assignment], Option<&mut Word>, &mut [Word]) {
    match command {
        BuiltinCommand::Break(command) => (
            &mut command.assignments,
            command.depth.as_mut(),
            &mut command.extra_args,
        ),
        BuiltinCommand::Continue(command) => (
            &mut command.assignments,
            command.depth.as_mut(),
            &mut command.extra_args,
        ),
        BuiltinCommand::Return(command) => (
            &mut command.assignments,
            command.code.as_mut(),
            &mut command.extra_args,
        ),
        BuiltinCommand::Exit(command) => (
            &mut command.assignments,
            command.code.as_mut(),
            &mut command.extra_args,
        ),
    }
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

pub(crate) fn function_attachment_span(command: &FunctionDef) -> Span {
    function_header_span(command).merge(stmt_span(&command.body))
}

pub(crate) fn anonymous_function_attachment_span(command: &AnonymousFunctionCommand) -> Span {
    anonymous_function_header_span(command)
        .merge(stmt_span(&command.body))
        .merge(words_span(&command.args))
}

pub(crate) fn function_header_span(command: &FunctionDef) -> Span {
    command.header.span()
}

pub(crate) fn anonymous_function_header_span(command: &AnonymousFunctionCommand) -> Span {
    match command.surface {
        shuck_ast::AnonymousFunctionSurface::FunctionKeyword {
            function_keyword_span,
        } => function_keyword_span,
        shuck_ast::AnonymousFunctionSurface::Parens { parens_span } => parens_span,
    }
}

pub(crate) fn words_span(words: &[shuck_ast::Word]) -> Span {
    words.iter().fold(Span::new(), |span, word| {
        merge_non_empty_span(span, word.span)
    })
}

pub(crate) fn compound_format_span(command: &CompoundCommand) -> Span {
    match command {
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter()
            .map(stmt_format_span)
            .reduce(Span::merge)
            .unwrap_or_default(),
        _ => compound_span(command),
    }
}

pub(crate) fn stmt_format_span(stmt: &Stmt) -> Span {
    let span = if stmt.negated {
        stmt.span
    } else {
        command_format_span(&stmt.command)
    };
    complete_stmt_span(stmt, span)
}

pub(crate) fn merge_non_empty_span(current: Span, next: Span) -> Span {
    if current == Span::new() {
        next
    } else if next == Span::new() {
        current
    } else {
        current.merge(next)
    }
}

pub(super) fn span_for_offsets(source_map: &SourceMap<'_>, start: usize, end: usize) -> Span {
    source_map.span_for_offsets(start, end)
}

pub(crate) fn simple_command_uses_synthetic_words(command: &SimpleCommand, source: &str) -> bool {
    word_uses_synthetic_source(&command.name, source)
}

fn synthetic_simple_command_verbatim_span(
    command: &SimpleCommand,
    source_map: &SourceMap<'_>,
) -> Span {
    let source = source_map.source();
    let start = command.span.start.offset.min(source.len());
    let end = command.span.end.offset.min(source.len());
    let Some(raw) = source.get(start..end) else {
        return command.span;
    };
    let leading_padding = raw.len() - raw.trim_start_matches([' ', '\t']).len();
    let candidate = &raw[leading_padding..];
    let command_start = if let Some(operator_len) = candidate
        .starts_with("&&")
        .then_some(2)
        .or_else(|| candidate.starts_with("||").then_some(2))
    {
        let after_operator = leading_padding + operator_len;
        let rest = &raw[after_operator..];
        let operator_padding = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        start + after_operator + operator_padding
    } else {
        start
    };
    let command_end =
        trim_synthetic_simple_command_trailing_case_terminator(source, command_start, end);
    span_for_offsets(source_map, command_start, command_end)
}

fn trim_synthetic_simple_command_trailing_case_terminator(
    source: &str,
    start: usize,
    end: usize,
) -> usize {
    let Some(raw) = source.get(start..end) else {
        return end;
    };
    let trimmed_end = raw.trim_end_matches([' ', '\t', '\r', '\n']).len();
    let trimmed = &raw[..trimmed_end];
    let terminator_len = [";;&", ";;", ";&", ";|"]
        .into_iter()
        .find_map(|terminator| trimmed.ends_with(terminator).then_some(terminator.len()));
    let Some(terminator_len) = terminator_len else {
        return start + trimmed_end;
    };
    let before_terminator = trimmed[..trimmed.len() - terminator_len]
        .trim_end_matches([' ', '\t'])
        .len();
    start + before_terminator
}

fn word_uses_synthetic_source(word: &Word, source: &str) -> bool {
    if !word
        .parts_with_spans()
        .any(|(part, _)| word_part_uses_synthetic_source(part))
    {
        return false;
    }
    let rendered = word.render_syntax(source);
    let raw = source
        .get(word.span.start.offset.min(source.len())..word.span.end.offset.min(source.len()));
    raw.is_none_or(|raw| raw != rendered)
}

fn word_part_uses_synthetic_source(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(text) => !text.is_source_backed(),
        WordPart::SingleQuoted { value, .. } => !value.is_source_backed(),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_uses_synthetic_source(&part.kind)),
        _ => false,
    }
}
