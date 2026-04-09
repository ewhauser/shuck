use super::*;

pub(super) fn build_substitution_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Vec<Box<[SubstitutionFact]>> {
    commands
        .iter()
        .map(|fact| build_command_substitution_facts(fact, commands, command_ids_by_span, source))
        .collect()
}

fn build_command_substitution_facts<'a>(
    fact: &CommandFact<'a>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Box<[SubstitutionFact]> {
    let mut substitutions = Vec::new();
    let mut substitution_index = FxHashMap::default();

    visit_command_words_for_substitutions(fact.command(), fact.redirects(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::Other,
            commands,
            command_ids_by_span,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_argument_words_for_substitutions(fact.command(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::CommandArgument,
            commands,
            command_ids_by_span,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_declaration_assignment_words_for_substitutions(fact.command(), &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::DeclarationAssignmentValue,
            commands,
            command_ids_by_span,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_subscript_words_for_substitutions(fact.command(), source, &mut |kind, word| {
        collect_or_update_word_substitution_facts(
            word,
            kind,
            commands,
            command_ids_by_span,
            source,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    substitutions.into_boxed_slice()
}

fn collect_or_update_word_substitution_facts<'a>(
    word: &Word,
    host_kind: SubstitutionHostKind,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    collect_word_substitution_occurrences(&word.parts, false, &mut occurrences);

    for occurrence in occurrences {
        let key = FactSpan::new(occurrence.span);
        if let Some(&index) = substitution_index.get(&key) {
            substitutions[index].host_word_span = word.span;
            substitutions[index].host_kind = host_kind;
            substitutions[index].unquoted_in_host = occurrence.unquoted_in_host;
            continue;
        }

        let (stdout_intent, has_stdout_redirect) =
            classify_substitution_body(occurrence.body, commands, command_ids_by_span, source);
        substitution_index.insert(key, substitutions.len());
        substitutions.push(SubstitutionFact {
            span: occurrence.span,
            kind: occurrence.kind,
            stdout_intent,
            has_stdout_redirect,
            host_word_span: word.span,
            host_kind,
            unquoted_in_host: occurrence.unquoted_in_host,
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct SubstitutionOccurrence<'a> {
    body: &'a StmtSeq,
    span: Span,
    kind: CommandSubstitutionKind,
    unquoted_in_host: bool,
}

fn collect_word_substitution_occurrences<'a>(
    parts: &'a [WordPartNode],
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_substitution_occurrences(parts, true, occurrences);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                visit_arithmetic_words_in_expression(expression_ast.as_ref(), quoted, occurrences);
            }
            WordPart::CommandSubstitution { body, .. } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    unquoted_in_host: !quoted,
                });
            }
            WordPart::ProcessSubstitution { body, is_input } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: if *is_input {
                        CommandSubstitutionKind::ProcessInput
                    } else {
                        CommandSubstitutionKind::ProcessOutput
                    },
                    unquoted_in_host: !quoted,
                });
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn visit_arithmetic_words_in_expression<'a>(
    expression: Option<&'a ArithmeticExprNode>,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    let Some(expression) = expression else {
        return;
    };

    collect_arithmetic_word_substitution_occurrences(expression, quoted, occurrences);
}

fn collect_arithmetic_word_substitution_occurrences<'a>(
    expression: &'a ArithmeticExprNode,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            collect_arithmetic_word_substitution_occurrences(index, quoted, occurrences);
        }
        ArithmeticExpr::ShellWord(word) => {
            collect_word_substitution_occurrences(&word.parts, quoted, occurrences);
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_arithmetic_word_substitution_occurrences(expression, quoted, occurrences);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_arithmetic_word_substitution_occurrences(expr, quoted, occurrences);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_word_substitution_occurrences(left, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(right, quoted, occurrences);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_word_substitution_occurrences(condition, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(then_expr, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(else_expr, quoted, occurrences);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_substitution_occurrences(target, quoted, occurrences);
            collect_arithmetic_word_substitution_occurrences(value, quoted, occurrences);
        }
    }
}

fn collect_arithmetic_lvalue_substitution_occurrences<'a>(
    target: &'a ArithmeticLvalue,
    quoted: bool,
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            collect_arithmetic_word_substitution_occurrences(index, quoted, occurrences);
        }
    }
}

fn classify_substitution_body<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> (SubstitutionOutputIntent, bool) {
    let mut stdout_intent: Option<SubstitutionOutputIntent> = None;
    let mut has_stdout_redirect = false;

    for visit in query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
    ) {
        let state = if let Some(id) = command_id_for_command(visit.command, command_ids_by_span) {
            classify_redirect_facts(command_fact(commands, id).redirect_facts())
        } else {
            let redirect_facts = build_redirect_facts(visit.redirects, source);
            classify_redirect_facts(&redirect_facts)
        };

        has_stdout_redirect |= state.has_stdout_redirect;
        stdout_intent = Some(match stdout_intent {
            Some(current) if current == state.stdout_intent => current,
            Some(_) => SubstitutionOutputIntent::Mixed,
            None => state.stdout_intent,
        });
    }

    (
        stdout_intent.unwrap_or(SubstitutionOutputIntent::Captured),
        has_stdout_redirect,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputSink {
    Captured,
    DevNull,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedirectState {
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
}

fn classify_redirect_facts(redirects: &[RedirectFact<'_>]) -> RedirectState {
    let mut fds = FxHashMap::from_iter([(1, OutputSink::Captured), (2, OutputSink::Other)]);
    let mut has_stdout_redirect = false;

    for redirect in redirects {
        match redirect.redirect().kind {
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                let sink = redirect_file_sink(redirect);
                let fd = redirect.redirect().fd.unwrap_or(1);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::OutputBoth => {
                let sink = redirect_file_sink(redirect);
                has_stdout_redirect = true;
                fds.insert(1, sink);
                fds.insert(2, sink);
            }
            RedirectKind::DupOutput => {
                let fd = redirect.redirect().fd.unwrap_or(1);
                let sink = redirect_dup_output_sink(redirect, &fds);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => {}
        }
    }

    let stdout_sink = *fds.get(&1).unwrap_or(&OutputSink::Other);
    let stderr_sink = *fds.get(&2).unwrap_or(&OutputSink::Other);
    let stdout_intent = if matches!(stdout_sink, OutputSink::Captured)
        || matches!(stderr_sink, OutputSink::Captured)
    {
        SubstitutionOutputIntent::Captured
    } else if matches!(stdout_sink, OutputSink::DevNull) {
        SubstitutionOutputIntent::Discarded
    } else {
        SubstitutionOutputIntent::Rerouted
    };

    RedirectState {
        stdout_intent,
        has_stdout_redirect,
    }
}

fn redirect_file_sink(redirect: &RedirectFact<'_>) -> OutputSink {
    match redirect.analysis() {
        Some(analysis) if analysis.is_definitely_dev_null() => OutputSink::DevNull,
        Some(_) => OutputSink::Other,
        None => OutputSink::Other,
    }
}

fn redirect_dup_output_sink(
    redirect: &RedirectFact<'_>,
    fds: &FxHashMap<i32, OutputSink>,
) -> OutputSink {
    let Some(fd) = redirect
        .analysis()
        .and_then(|analysis| analysis.numeric_descriptor_target)
    else {
        return OutputSink::Other;
    };

    *fds.get(&fd).unwrap_or(&OutputSink::Other)
}

fn visit_command_words_for_substitutions(
    command: &Command,
    redirects: &[Redirect],
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        Command::Simple(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            visitor(&command.name);
            visit_words_for_substitutions(&command.args, visitor);
        }
        Command::Builtin(command) => {
            visit_builtin_words_for_substitutions(command, source, visitor)
        }
        Command::Decl(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            for operand in &command.operands {
                visit_decl_operand_words_for_substitutions(operand, source, visitor);
            }
        }
        Command::Binary(_) => {}
        Command::Function(function) => {
            for entry in &function.header.entries {
                visitor(&entry.word);
            }
        }
        Command::AnonymousFunction(function) => {
            visit_words_for_substitutions(&function.args, visitor);
        }
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    visit_words_for_substitutions(words, visitor);
                }
            }
            CompoundCommand::Repeat(command) => visitor(&command.count),
            CompoundCommand::Foreach(command) => {
                visit_words_for_substitutions(&command.words, visitor)
            }
            CompoundCommand::Case(command) => {
                visitor(&command.word);
                for case in &command.cases {
                    visit_patterns_for_substitutions(&case.patterns, visitor);
                }
            }
            CompoundCommand::Select(command) => {
                visit_words_for_substitutions(&command.words, visitor)
            }
            CompoundCommand::Conditional(command) => {
                visit_conditional_words_for_substitutions(&command.expression, source, visitor);
            }
            CompoundCommand::If(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Always(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Time(_)
            | CompoundCommand::Coproc(_) => {}
        },
    }

    for redirect in redirects {
        visitor(redirect_scan_word(redirect));
    }
}

fn visit_command_argument_words_for_substitutions(
    command: &Command,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        Command::Simple(command) => {
            if static_word_text(&command.name, source).as_deref() == Some("trap") {
                return;
            }
            visit_words_for_substitutions(&command.args, visitor);
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                visit_words_for_substitutions(&command.extra_args, visitor);
            }
        },
        Command::Decl(command) => {
            for operand in &command.operands {
                if let DeclOperand::Dynamic(word) = operand {
                    visitor(word);
                }
            }
        }
        Command::Binary(_) | Command::Compound(_) => {}
        Command::Function(function) => {
            for entry in &function.header.entries {
                visitor(&entry.word);
            }
        }
        Command::AnonymousFunction(function) => {
            visit_words_for_substitutions(&function.args, visitor);
        }
    }
}

fn visit_declaration_assignment_words_for_substitutions(
    command: &Command,
    visitor: &mut impl FnMut(&Word),
) {
    let Command::Decl(command) = command else {
        return;
    };

    for operand in &command.operands {
        let DeclOperand::Assignment(assignment) = operand else {
            continue;
        };

        if let AssignmentValue::Scalar(word) = &assignment.value {
            visitor(word);
        }
    }
}

fn visit_command_subscript_words_for_substitutions(
    command: &Command,
    source: &str,
    visitor: &mut impl FnMut(SubstitutionHostKind, &Word),
) {
    for assignment in query::command_assignments(command) {
        query::visit_var_ref_subscript_words_with_source(&assignment.target, source, &mut |word| {
            visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
        });

        if let AssignmentValue::Compound(array) = &assignment.value {
            for element in &array.elements {
                if let shuck_ast::ArrayElem::Keyed { key, .. }
                | shuck_ast::ArrayElem::KeyedAppend { key, .. } = element
                {
                    query::visit_subscript_words(Some(key), source, &mut |word| {
                        visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                    });
                }
            }
        }
    }

    for operand in query::declaration_operands(command) {
        match operand {
            DeclOperand::Name(reference) => {
                query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
                    visitor(SubstitutionHostKind::DeclarationNameSubscript, word);
                });
            }
            DeclOperand::Assignment(assignment) => {
                query::visit_var_ref_subscript_words_with_source(
                    &assignment.target,
                    source,
                    &mut |word| {
                        visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
                    },
                );

                if let AssignmentValue::Compound(array) = &assignment.value {
                    for element in &array.elements {
                        if let shuck_ast::ArrayElem::Keyed { key, .. }
                        | shuck_ast::ArrayElem::KeyedAppend { key, .. } = element
                        {
                            query::visit_subscript_words(Some(key), source, &mut |word| {
                                visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                            });
                        }
                    }
                }
            }
            DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
        }
    }
}

fn visit_assignments_for_substitutions(
    assignments: &[shuck_ast::Assignment],
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    for assignment in assignments {
        query::visit_var_ref_subscript_words_with_source(&assignment.target, source, visitor);

        match &assignment.value {
            AssignmentValue::Scalar(word) => visitor(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        shuck_ast::ArrayElem::Sequential(word) => visitor(word),
                        shuck_ast::ArrayElem::Keyed { key, value }
                        | shuck_ast::ArrayElem::KeyedAppend { key, value } => {
                            query::visit_subscript_words(Some(key), source, visitor);
                            visitor(value);
                        }
                    }
                }
            }
        }
    }
}

fn visit_builtin_words_for_substitutions(
    command: &BuiltinCommand,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match command {
        BuiltinCommand::Break(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.depth {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Continue(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.depth {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Return(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.code {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
        BuiltinCommand::Exit(command) => {
            visit_assignments_for_substitutions(&command.assignments, source, visitor);
            if let Some(word) = &command.code {
                visitor(word);
            }
            visit_words_for_substitutions(&command.extra_args, visitor);
        }
    }
}

fn visit_decl_operand_words_for_substitutions(
    operand: &DeclOperand,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visitor(word),
        DeclOperand::Name(reference) => {
            query::visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
        DeclOperand::Assignment(assignment) => {
            visit_assignments_for_substitutions(std::slice::from_ref(assignment), source, visitor);
        }
    }
}

fn visit_words_for_substitutions(words: &[Word], visitor: &mut impl FnMut(&Word)) {
    for word in words {
        visitor(word);
    }
}

fn visit_patterns_for_substitutions(patterns: &[Pattern], visitor: &mut impl FnMut(&Word)) {
    for pattern in patterns {
        visit_pattern_for_substitutions(pattern, visitor);
    }
}

fn visit_pattern_for_substitutions(pattern: &Pattern, visitor: &mut impl FnMut(&Word)) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                visit_patterns_for_substitutions(patterns, visitor)
            }
            PatternPart::Word(word) => visitor(word),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn visit_conditional_words_for_substitutions(
    expression: &ConditionalExpr,
    source: &str,
    visitor: &mut impl FnMut(&Word),
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            visit_conditional_words_for_substitutions(&expr.left, source, visitor);
            visit_conditional_words_for_substitutions(&expr.right, source, visitor);
        }
        ConditionalExpr::Unary(expr) => {
            visit_conditional_words_for_substitutions(&expr.expr, source, visitor);
        }
        ConditionalExpr::Parenthesized(expr) => {
            visit_conditional_words_for_substitutions(&expr.expr, source, visitor);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => visitor(word),
        ConditionalExpr::Pattern(pattern) => visit_pattern_for_substitutions(pattern, visitor),
        ConditionalExpr::VarRef(reference) => {
            query::visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
    }
}

fn redirect_scan_word(redirect: &Redirect) -> &Word {
    match redirect.word_target() {
        Some(word) => word,
        None => &redirect.heredoc().expect("expected heredoc redirect").body,
    }
}

pub(super) fn build_for_header_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Vec<ForHeaderFact<'a>> {
    commands
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::For(command)) = fact.command() else {
                return None;
            };

            Some(ForHeaderFact {
                command,
                command_id: fact.id(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_loop_header_word_facts(
                    command.words.iter().flat_map(|words| words.iter()),
                    commands,
                    command_ids_by_span,
                    source,
                ),
            })
        })
        .collect()
}

pub(super) fn build_select_header_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Vec<SelectHeaderFact<'a>> {
    commands
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::Select(command)) = fact.command() else {
                return None;
            };

            Some(SelectHeaderFact {
                command,
                command_id: fact.id(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_loop_header_word_facts(
                    command.words.iter(),
                    commands,
                    command_ids_by_span,
                    source,
                ),
            })
        })
        .collect()
}

fn build_loop_header_word_facts<'a>(
    words: impl IntoIterator<Item = &'a Word>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Box<[LoopHeaderWordFact<'a>]> {
    words
        .into_iter()
        .map(|word| {
            let classification = classify_word(word, source);
            LoopHeaderWordFact {
                word,
                classification,
                has_unquoted_command_substitution: classification.has_command_substitution()
                    && !span::unquoted_command_substitution_part_spans(word).is_empty(),
                contains_ls_substitution: word_contains_command_substitution_named(
                    word,
                    "ls",
                    commands,
                    command_ids_by_span,
                ),
                contains_find_substitution: word_contains_find_substitution(
                    word,
                    commands,
                    command_ids_by_span,
                ),
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

pub(super) fn build_pipeline_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Vec<PipelineFact<'a>> {
    let mut nested_pipeline_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
            continue;
        }

        if matches!(
            &command.left.command,
            Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) && let Some(id) = command_id_for_command(&command.left.command, command_ids_by_span)
        {
            nested_pipeline_commands.insert(id);
        }
        if matches!(
            &command.right.command,
            Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll)
        ) && let Some(id) = command_id_for_command(&command.right.command, command_ids_by_span)
        {
            nested_pipeline_commands.insert(id);
        }
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll)
                || nested_pipeline_commands.contains(&fact.id())
            {
                return None;
            }

            let segments = query::pipeline_segments(fact.command())?;
            Some(PipelineFact {
                key: fact.key(),
                command,
                segments: segments
                    .into_iter()
                    .map(|stmt| build_pipeline_segment_fact(stmt, commands, command_ids_by_span))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
        })
        .collect()
}

fn build_pipeline_segment_fact<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> PipelineSegmentFact<'a> {
    let fact = command_fact_for_stmt(stmt, commands, command_ids_by_span)
        .expect("pipeline segment should have a corresponding command fact");

    PipelineSegmentFact {
        stmt,
        command_id: fact.id(),
        literal_name: fact
            .literal_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
        effective_name: fact
            .effective_name()
            .map(str::to_owned)
            .map(String::into_boxed_str),
    }
}

pub(super) fn build_list_facts<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Vec<ListFact<'a>> {
    let mut nested_list_commands = FxHashSet::default();

    for fact in commands {
        let Command::Binary(command) = fact.command() else {
            continue;
        };
        if !matches!(command.op, BinaryOp::And | BinaryOp::Or) {
            continue;
        }

        if matches!(&command.left.command, Command::Binary(left) if matches!(left.op, BinaryOp::And | BinaryOp::Or))
            && let Some(id) = command_id_for_command(&command.left.command, command_ids_by_span)
        {
            nested_list_commands.insert(id);
        }
        if matches!(&command.right.command, Command::Binary(right) if matches!(right.op, BinaryOp::And | BinaryOp::Or))
            && let Some(id) = command_id_for_command(&command.right.command, command_ids_by_span)
        {
            nested_list_commands.insert(id);
        }
    }

    commands
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::And | BinaryOp::Or)
                || nested_list_commands.contains(&fact.id())
            {
                return None;
            }

            let mut operators = Vec::new();
            collect_short_circuit_operators(command, &mut operators);
            let mixed_short_circuit_span = mixed_short_circuit_operator_span(&operators);

            Some(ListFact {
                key: fact.key(),
                command,
                operators: operators.into_boxed_slice(),
                mixed_short_circuit_span,
            })
        })
        .collect()
}

pub(super) fn build_single_test_subshell_spans<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|fact| single_test_subshell_span(fact, commands, command_ids_by_span, source))
        .collect()
}

pub(super) fn build_subshell_test_group_spans<'a>(
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|fact| subshell_test_group_span(fact, commands, command_ids_by_span, source))
        .collect()
}

fn single_test_subshell_span<'a>(
    fact: &CommandFact<'a>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Option<Span> {
    let condition = match fact.command() {
        Command::Compound(CompoundCommand::If(command)) => &command.condition,
        Command::Compound(CompoundCommand::While(command)) => &command.condition,
        Command::Compound(CompoundCommand::Until(command)) => &command.condition,
        _ => return None,
    };

    let [stmt] = condition.as_slice() else {
        return None;
    };
    if stmt.negated {
        return None;
    }

    let condition_fact = command_fact_for_stmt(stmt, commands, command_ids_by_span)?;
    let Command::Compound(CompoundCommand::Subshell(body)) = condition_fact.command() else {
        return None;
    };

    let [body_stmt] = body.as_slice() else {
        return None;
    };
    if body_stmt.negated {
        return None;
    }

    let body_fact = command_fact_for_stmt(body_stmt, commands, command_ids_by_span)?;
    if !is_test_like_command(body_fact) {
        return None;
    }

    Some(subshell_anchor_span(stmt.span, source))
}

fn is_test_like_command(fact: &CommandFact<'_>) -> bool {
    fact.wrappers()
        .iter()
        .all(|wrapper| matches!(wrapper, WrapperKind::Command | WrapperKind::Builtin))
        && (fact.effective_name_is("test")
            || fact.effective_name_is("[")
            || matches!(
                fact.command(),
                Command::Compound(CompoundCommand::Conditional(_))
            ))
}

fn subshell_test_group_span<'a>(
    fact: &CommandFact<'a>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> Option<Span> {
    let Command::Compound(CompoundCommand::Subshell(body)) = fact.command() else {
        return None;
    };

    if !subshell_body_contains_grouped_tests(body, commands, command_ids_by_span) {
        return None;
    }

    Some(subshell_anchor_span(fact.span(), source))
}

fn subshell_body_contains_grouped_tests<'a>(
    body: &StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    subshell_body_analysis(body, commands, command_ids_by_span)
        .is_some_and(|analysis| analysis.has_grouping && analysis.test_count > 0)
}

#[derive(Debug, Default, Clone, Copy)]
struct GroupedTestAnalysis {
    test_count: usize,
    has_grouping: bool,
}

fn subshell_stmt_analysis<'a>(
    stmt: &Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<GroupedTestAnalysis> {
    subshell_command_analysis(&stmt.command, commands, command_ids_by_span)
}

fn subshell_command_analysis<'a>(
    command: &Command,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<GroupedTestAnalysis> {
    match command {
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Compound(CompoundCommand::Conditional(_)) => {
            if let Some(id) = command_id_for_command(command, command_ids_by_span) {
                let fact = command_fact(commands, id);
                if is_test_like_command(fact) {
                    return Some(GroupedTestAnalysis {
                        test_count: 1,
                        has_grouping: false,
                    });
                }
            }
            None
        }
        Command::Compound(CompoundCommand::BraceGroup(body)) => {
            let inner = subshell_body_analysis(body, commands, command_ids_by_span)?;
            Some(GroupedTestAnalysis {
                test_count: inner.test_count,
                has_grouping: true,
            })
        }
        Command::Compound(CompoundCommand::Subshell(body)) => {
            let inner = subshell_body_analysis(body, commands, command_ids_by_span)?;
            Some(GroupedTestAnalysis {
                test_count: inner.test_count,
                has_grouping: inner.has_grouping,
            })
        }
        Command::Binary(binary) if matches!(binary.op, BinaryOp::And | BinaryOp::Or) => {
            let left = subshell_stmt_analysis(&binary.left, commands, command_ids_by_span)?;
            let right = subshell_stmt_analysis(&binary.right, commands, command_ids_by_span)?;
            Some(GroupedTestAnalysis {
                test_count: left.test_count + right.test_count,
                has_grouping: true,
            })
        }
        _ => None,
    }
}

fn subshell_body_analysis<'a>(
    body: &StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> Option<GroupedTestAnalysis> {
    let mut analysis = GroupedTestAnalysis::default();

    if body.stmts.len() > 1 {
        analysis.has_grouping = true;
    }

    for stmt in &body.stmts {
        let stmt_analysis = subshell_stmt_analysis(stmt, commands, command_ids_by_span)?;
        analysis.test_count += stmt_analysis.test_count;
        analysis.has_grouping |= stmt_analysis.has_grouping;
    }

    Some(analysis)
}

fn subshell_anchor_span(span: Span, source: &str) -> Span {
    let Some(open_paren_offset) = leading_open_paren_offset(source, span.start.offset) else {
        return span;
    };

    let end_offset = trim_trailing_whitespace_offset(source, span.end.offset);
    Span::from_positions(
        position_at_offset(source, open_paren_offset),
        position_at_offset(source, end_offset),
    )
}

fn leading_open_paren_offset(source: &str, start_offset: usize) -> Option<usize> {
    for (offset, ch) in source[..start_offset].char_indices().rev() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '(' {
            return Some(offset);
        }

        return None;
    }

    None
}

fn position_at_offset(source: &str, target_offset: usize) -> Position {
    source[..target_offset]
        .chars()
        .fold(Position::new(), |mut position, ch| {
            position.advance(ch);
            position
        })
}

fn trim_trailing_whitespace_offset(source: &str, end_offset: usize) -> usize {
    for (offset, ch) in source[..end_offset].char_indices().rev() {
        if ch.is_whitespace() {
            continue;
        }

        return offset + ch.len_utf8();
    }

    end_offset
}

fn collect_short_circuit_operators(command: &BinaryCommand, operators: &mut Vec<ListOperatorFact>) {
    if let Command::Binary(left) = &command.left.command
        && matches!(left.op, BinaryOp::And | BinaryOp::Or)
    {
        collect_short_circuit_operators(left, operators);
    }

    if matches!(command.op, BinaryOp::And | BinaryOp::Or) {
        operators.push(ListOperatorFact {
            op: command.op,
            span: command.op_span,
        });
    }

    if let Command::Binary(right) = &command.right.command
        && matches!(right.op, BinaryOp::And | BinaryOp::Or)
    {
        collect_short_circuit_operators(right, operators);
    }
}

fn mixed_short_circuit_operator_span(operators: &[ListOperatorFact]) -> Option<Span> {
    let mut previous = operators.first()?;

    for operator in operators.iter().skip(1) {
        if previous.op() != operator.op() {
            return Some(previous.span());
        }

        previous = operator;
    }

    None
}

fn word_contains_find_substitution<'a>(
    word: &'a Word,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    word.parts
        .iter()
        .any(|part| part_contains_find_substitution(&part.kind, commands, command_ids_by_span))
}

fn word_contains_command_substitution_named<'a>(
    word: &'a Word,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    word.parts.iter().any(|part| {
        part_contains_command_substitution_named(&part.kind, name, commands, command_ids_by_span)
    })
}

fn part_contains_command_substitution_named<'a>(
    part: &WordPart,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts.iter().any(|part| {
            part_contains_command_substitution_named(
                &part.kind,
                name,
                commands,
                command_ids_by_span,
            )
        }),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_simple_command_named(body, name, commands, command_ids_by_span)
        }
        _ => false,
    }
}

fn part_contains_find_substitution<'a>(
    part: &WordPart,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| part_contains_find_substitution(&part.kind, commands, command_ids_by_span)),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            substitution_body_is_find(body, commands, command_ids_by_span)
        }
        _ => false,
    }
}

fn substitution_body_is_find<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_effective_name_is(stmt, "find", commands, command_ids_by_span))
}

fn substitution_body_is_simple_command_named<'a>(
    body: &'a StmtSeq,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    matches!(body.as_slice(), [stmt] if stmt_literal_name_is(stmt, name, commands, command_ids_by_span))
}

fn stmt_effective_name_is<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_for_stmt(stmt, commands, command_ids_by_span)
        .map(|fact| fact.effective_name_is(name))
        .unwrap_or(false)
}

fn stmt_literal_name_is<'a>(
    stmt: &'a Stmt,
    name: &str,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_for_stmt(stmt, commands, command_ids_by_span).and_then(CommandFact::literal_name)
        == Some(name)
}
