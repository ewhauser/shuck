#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionHostKind {
    CommandArgument,
    HereStringOperand,
    DeclarationAssignmentValue,
    AssignmentTargetSubscript,
    DeclarationNameSubscript,
    ArrayKeySubscript,
    Other,
}

#[derive(Debug, Clone)]
pub struct SubstitutionFact {
    span: Span,
    kind: CommandSubstitutionKind,
    command_syntax: Option<CommandSubstitutionSyntax>,
    stdout_intent: SubstitutionOutputIntent,
    terminal_stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
    stdout_redirect_spans: Box<[Span]>,
    stdout_dev_null_redirect_spans: Box<[Span]>,
    body_contains_ls: bool,
    body_contains_echo: bool,
    body_contains_grep: bool,
    body_has_multiple_statements: bool,
    body_is_negated: bool,
    body_is_pgrep_lookup: bool,
    body_is_seq_utility: bool,
    body_has_commands: bool,
    bash_file_slurp: bool,
    host_word_span: Span,
    host_kind: SubstitutionHostKind,
    unquoted_in_host: bool,
}

impl SubstitutionFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> CommandSubstitutionKind {
        self.kind
    }

    pub fn command_syntax(&self) -> Option<CommandSubstitutionSyntax> {
        self.command_syntax
    }

    pub fn uses_backtick_syntax(&self) -> bool {
        self.command_syntax == Some(CommandSubstitutionSyntax::Backtick)
    }

    pub fn stdout_intent(&self) -> SubstitutionOutputIntent {
        self.stdout_intent
    }

    pub fn terminal_stdout_intent(&self) -> SubstitutionOutputIntent {
        self.terminal_stdout_intent
    }

    pub fn has_stdout_redirect(&self) -> bool {
        self.has_stdout_redirect
    }

    pub fn stdout_redirect_spans(&self) -> &[Span] {
        &self.stdout_redirect_spans
    }

    pub fn stdout_dev_null_redirect_spans(&self) -> &[Span] {
        &self.stdout_dev_null_redirect_spans
    }

    pub fn body_contains_ls(&self) -> bool {
        self.body_contains_ls
    }

    pub fn body_contains_echo(&self) -> bool {
        self.body_contains_echo
    }

    pub fn body_contains_grep(&self) -> bool {
        self.body_contains_grep
    }

    pub fn body_has_multiple_statements(&self) -> bool {
        self.body_has_multiple_statements
    }
    pub fn body_is_negated(&self) -> bool {
        self.body_is_negated
    }

    pub fn body_is_pgrep_lookup(&self) -> bool {
        self.body_is_pgrep_lookup
    }

    pub fn body_is_seq_utility(&self) -> bool {
        self.body_is_seq_utility
    }

    pub fn body_has_commands(&self) -> bool {
        self.body_has_commands
    }
    pub fn is_bash_file_slurp(&self) -> bool {
        self.bash_file_slurp
    }
    pub fn host_word_span(&self) -> Span {
        self.host_word_span
    }

    pub fn host_kind(&self) -> SubstitutionHostKind {
        self.host_kind
    }

    pub fn unquoted_in_host(&self) -> bool {
        self.unquoted_in_host
    }

    pub fn stdout_is_captured(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Captured
    }

    pub fn stdout_is_discarded(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Discarded
    }

    pub fn stdout_is_rerouted(&self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Rerouted
    }
}

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
    let context = SubstitutionFactBuildContext {
        commands,
        command_ids_by_span,
        source,
    };

    visit_command_words_for_substitutions(fact.command(), fact.redirects(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::Other,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_heredoc_bodies_for_substitutions(fact.redirects(), &mut |body| {
        collect_or_update_heredoc_body_substitution_facts(
            body,
            SubstitutionHostKind::Other,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_argument_words_for_substitutions(fact.command(), source, &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::CommandArgument,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_here_string_words_for_substitutions(fact.redirects(), &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::HereStringOperand,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_declaration_assignment_words_for_substitutions(fact.command(), &mut |word| {
        collect_or_update_word_substitution_facts(
            word,
            SubstitutionHostKind::DeclarationAssignmentValue,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    visit_command_subscript_words_for_substitutions(fact.command(), source, &mut |kind, word| {
        collect_or_update_word_substitution_facts(
            word,
            kind,
            context,
            &mut substitutions,
            &mut substitution_index,
        );
    });

    substitutions.into_boxed_slice()
}

fn collect_or_update_word_substitution_facts<'a>(
    word: &Word,
    host_kind: SubstitutionHostKind,
    context: SubstitutionFactBuildContext<'a, '_>,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    collect_word_substitution_occurrences(&word.parts, false, &mut occurrences);
    collect_or_update_substitution_facts_from_occurrences(
        word.span,
        host_kind,
        occurrences,
        context,
        substitutions,
        substitution_index,
    );
}

fn collect_or_update_heredoc_body_substitution_facts<'a>(
    body: &shuck_ast::HeredocBody,
    host_kind: SubstitutionHostKind,
    context: SubstitutionFactBuildContext<'a, '_>,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    collect_heredoc_body_substitution_occurrences(&body.parts, &mut occurrences);
    collect_or_update_substitution_facts_from_occurrences(
        body.span,
        host_kind,
        occurrences,
        context,
        substitutions,
        substitution_index,
    );
}

#[derive(Clone, Copy)]
struct SubstitutionFactBuildContext<'a, 'b> {
    commands: &'b [CommandFact<'a>],
    command_ids_by_span: &'b CommandLookupIndex,
    source: &'b str,
}

fn collect_or_update_substitution_facts_from_occurrences<'a>(
    host_span: Span,
    host_kind: SubstitutionHostKind,
    occurrences: Vec<SubstitutionOccurrence<'a>>,
    context: SubstitutionFactBuildContext<'a, '_>,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    for occurrence in occurrences {
        let key = FactSpan::new(occurrence.span);
        if let Some(&index) = substitution_index.get(&key) {
            substitutions[index].host_word_span = host_span;
            substitutions[index].host_kind = host_kind;
            substitutions[index].unquoted_in_host = occurrence.unquoted_in_host;
            continue;
        }

        let body_facts = classify_substitution_body(
            occurrence.body,
            context.commands,
            context.command_ids_by_span,
            context.source,
        );
        substitution_index.insert(key, substitutions.len());
        substitutions.push(SubstitutionFact {
            span: occurrence.span,
            kind: occurrence.kind,
            command_syntax: occurrence.command_syntax,
            stdout_intent: body_facts.stdout_intent,
            terminal_stdout_intent: body_facts.terminal_stdout_intent,
            has_stdout_redirect: body_facts.has_stdout_redirect,
            stdout_redirect_spans: body_facts.stdout_redirect_spans,
            stdout_dev_null_redirect_spans: body_facts.stdout_dev_null_redirect_spans,
            body_contains_ls: body_facts.body_contains_ls,
            body_contains_echo: body_facts.body_contains_echo,
            body_contains_grep: body_facts.body_contains_grep,
            body_has_multiple_statements: body_facts.body_has_multiple_statements,
            body_is_negated: body_facts.body_is_negated,
            body_is_pgrep_lookup: body_facts.body_is_pgrep_lookup,
            body_is_seq_utility: body_facts.body_is_seq_utility,
            body_has_commands: body_facts.body_has_commands,
            bash_file_slurp: body_facts.bash_file_slurp,
            host_word_span: host_span,
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
    command_syntax: Option<CommandSubstitutionSyntax>,
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
            WordPart::CommandSubstitution { body, syntax } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    command_syntax: Some(*syntax),
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
                    command_syntax: None,
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

fn collect_heredoc_body_substitution_occurrences<'a>(
    parts: &'a [shuck_ast::HeredocBodyPartNode],
    occurrences: &mut Vec<SubstitutionOccurrence<'a>>,
) {
    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                visit_arithmetic_words_in_expression(expression_ast.as_ref(), false, occurrences);
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, syntax } => {
                occurrences.push(SubstitutionOccurrence {
                    body,
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    command_syntax: Some(*syntax),
                    unquoted_in_host: true,
                });
            }
            shuck_ast::HeredocBodyPart::Literal(_)
            | shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_) => {}
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

#[derive(Debug, Clone)]
struct SubstitutionBodyFacts {
    stdout_intent: SubstitutionOutputIntent,
    terminal_stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
    stdout_redirect_spans: Box<[Span]>,
    stdout_dev_null_redirect_spans: Box<[Span]>,
    body_contains_ls: bool,
    body_contains_echo: bool,
    body_contains_grep: bool,
    body_has_multiple_statements: bool,
    body_is_negated: bool,
    body_is_pgrep_lookup: bool,
    body_is_seq_utility: bool,
    body_has_commands: bool,
    bash_file_slurp: bool,
}

#[derive(Debug, Clone)]
struct RedirectSummary {
    stdout_intent: SubstitutionOutputIntent,
    terminal_stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
    stdout_redirect_spans: Vec<Span>,
    stdout_dev_null_redirect_spans: Vec<Span>,
}

fn classify_substitution_body<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> SubstitutionBodyFacts {
    let visits = query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
    )
    .collect::<Vec<_>>();
    let redirect_summary =
        summarize_stmt_seq_redirects(body, commands, command_ids_by_span, source);

    SubstitutionBodyFacts {
        stdout_intent: redirect_summary.stdout_intent,
        terminal_stdout_intent: redirect_summary.terminal_stdout_intent,
        has_stdout_redirect: redirect_summary.has_stdout_redirect,
        stdout_redirect_spans: redirect_summary.stdout_redirect_spans.into_boxed_slice(),
        stdout_dev_null_redirect_spans: redirect_summary
            .stdout_dev_null_redirect_spans
            .into_boxed_slice(),
        body_contains_ls: substitution_body_contains_ls(body, commands, command_ids_by_span),
        body_contains_echo: substitution_body_contains_echo(body, source),
        body_contains_grep: substitution_body_contains_grep(body, source),
        body_has_multiple_statements: body.stmts.len() > 1,
        body_is_negated: matches!(body.stmts.as_slice(), [stmt] if stmt.negated),
        body_is_pgrep_lookup: substitution_body_is_pgrep_lookup(
            body,
            commands,
            command_ids_by_span,
        ),
        body_is_seq_utility: substitution_body_is_seq_utility(body, commands, command_ids_by_span),
        body_has_commands: !visits.is_empty(),
        bash_file_slurp: matches!(visits.as_slice(), [visit] if is_bash_file_slurp_command(visit.command, visit.redirects, source)),
    }
}

fn summarize_stmt_seq_redirects<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> RedirectSummary {
    let mut summary = RedirectSummary {
        stdout_intent: SubstitutionOutputIntent::Captured,
        terminal_stdout_intent: SubstitutionOutputIntent::Captured,
        has_stdout_redirect: false,
        stdout_redirect_spans: Vec::new(),
        stdout_dev_null_redirect_spans: Vec::new(),
    };
    let mut saw_stmt = false;

    for stmt in &body.stmts {
        let stmt_summary = summarize_stmt_redirects(stmt, commands, command_ids_by_span, source);
        summary = merge_redirect_summaries(summary, stmt_summary, saw_stmt);
        saw_stmt = true;
    }

    summary
}

fn summarize_stmt_redirects<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> RedirectSummary {
    match &stmt.command {
        Command::Binary(binary) => match binary.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                summarize_stmt_redirects(&binary.right, commands, command_ids_by_span, source)
            }
            BinaryOp::And | BinaryOp::Or => {
                let left =
                    summarize_stmt_redirects(&binary.left, commands, command_ids_by_span, source);
                let right =
                    summarize_stmt_redirects(&binary.right, commands, command_ids_by_span, source);
                merge_redirect_summaries(left, right, true)
            }
        },
        Command::Compound(CompoundCommand::Subshell(body))
        | Command::Compound(CompoundCommand::BraceGroup(body)) => {
            summarize_compound_stmt_redirects(
                summarize_stmt_seq_redirects(body, commands, command_ids_by_span, source),
                &stmt.redirects,
                source,
            )
        }
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_deref()
            .map(|inner_stmt| {
                summarize_compound_stmt_redirects(
                    summarize_stmt_redirects(inner_stmt, commands, command_ids_by_span, source),
                    &stmt.redirects,
                    source,
                )
            })
            .unwrap_or_else(default_redirect_summary),
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_)
        | Command::Compound(
            CompoundCommand::If(_)
            | CompoundCommand::For(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Case(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Always(_),
        ) => summarize_command_redirects(&stmt.command, &stmt.redirects, commands, command_ids_by_span, source),
    }
}

fn summarize_compound_stmt_redirects(
    summary: RedirectSummary,
    redirects: &[Redirect],
    source: &str,
) -> RedirectSummary {
    compound_redirects_capture_stderr_to_stdout(redirects, source)
        .then_some(default_redirect_summary())
        .unwrap_or(summary)
}

fn summarize_command_redirects<'a>(
    command: &Command,
    redirects: &[Redirect],
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    source: &str,
) -> RedirectSummary {
    let (state, stdout_redirect_spans, stdout_dev_null_redirect_spans) =
        if let Some(id) = command_id_for_command(command, command_ids_by_span) {
            let redirects = command_fact(commands, id).redirect_facts();
            (
                classify_redirect_facts(redirects),
                stdout_redirect_spans_for_fix(redirects),
                stdout_dev_null_redirect_spans_for_fix(redirects),
            )
        } else {
            let redirect_facts = build_redirect_facts(redirects, source, None);
            (
                classify_redirect_facts(&redirect_facts),
                stdout_redirect_spans_for_fix(&redirect_facts),
                stdout_dev_null_redirect_spans_for_fix(&redirect_facts),
            )
        };

    RedirectSummary {
        stdout_intent: state.stdout_intent,
        terminal_stdout_intent: state.stdout_intent,
        has_stdout_redirect: state.has_stdout_redirect,
        stdout_redirect_spans,
        stdout_dev_null_redirect_spans,
    }
}

fn merge_redirect_summaries(
    mut current: RedirectSummary,
    next: RedirectSummary,
    saw_existing: bool,
) -> RedirectSummary {
    current.has_stdout_redirect |= next.has_stdout_redirect;
    current.stdout_redirect_spans.extend(next.stdout_redirect_spans);
    current
        .stdout_dev_null_redirect_spans
        .extend(next.stdout_dev_null_redirect_spans);
    current.terminal_stdout_intent = next.terminal_stdout_intent;
    current.stdout_intent = if saw_existing {
        if current.stdout_intent == next.stdout_intent {
            current.stdout_intent
        } else {
            SubstitutionOutputIntent::Mixed
        }
    } else {
        next.stdout_intent
    };
    current
}

fn default_redirect_summary() -> RedirectSummary {
    RedirectSummary {
        stdout_intent: SubstitutionOutputIntent::Captured,
        terminal_stdout_intent: SubstitutionOutputIntent::Captured,
        has_stdout_redirect: false,
        stdout_redirect_spans: Vec::new(),
        stdout_dev_null_redirect_spans: Vec::new(),
    }
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

fn stdout_redirect_spans_for_fix(redirects: &[RedirectFact<'_>]) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| redirect_matches_substitution_warning(redirect))
        .map(|redirect| redirect.redirect().span)
        .collect()
}

fn stdout_dev_null_redirect_spans_for_fix(redirects: &[RedirectFact<'_>]) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| redirect_targets_stdout_dev_null_for_substitution_warning(redirect))
        .map(|redirect| redirect.redirect().span)
        .collect()
}

fn redirect_affects_stdout(redirect: &RedirectFact<'_>) -> bool {
    match redirect.redirect().kind {
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::DupOutput => redirect.redirect().fd.unwrap_or(1) == 1,
        RedirectKind::OutputBoth => true,
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupInput => false,
    }
}

fn redirect_targets_stdout_dev_null(redirect: &RedirectFact<'_>) -> bool {
    redirect_affects_stdout(redirect) && redirect_file_sink(redirect) == OutputSink::DevNull
}

fn redirect_targets_stdout_dev_null_for_substitution_warning(redirect: &RedirectFact<'_>) -> bool {
    redirect_matches_substitution_warning(redirect) && redirect_targets_stdout_dev_null(redirect)
}

fn redirect_matches_substitution_warning(redirect: &RedirectFact<'_>) -> bool {
    matches!(
        redirect.redirect().kind,
        RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append
    ) && redirect.redirect().fd.unwrap_or(1) == 1
}

fn compound_redirects_capture_stderr_to_stdout(redirects: &[Redirect], source: &str) -> bool {
    let redirect_facts = build_redirect_facts(redirects, source, None);
    classify_redirect_facts(&redirect_facts).stdout_intent == SubstitutionOutputIntent::Captured
        && redirect_facts.iter().any(is_stderr_to_stdout_dup)
}

fn is_stderr_to_stdout_dup(redirect: &RedirectFact<'_>) -> bool {
    redirect.redirect().kind == RedirectKind::DupOutput
        && redirect.redirect().fd == Some(2)
        && redirect
            .analysis()
            .and_then(|analysis| analysis.numeric_descriptor_target)
            == Some(1)
}

fn substitution_body_contains_echo(body: &StmtSeq, source: &str) -> bool {
    let [stmt] = body.stmts.as_slice() else {
        return false;
    };

    if !matches!(
        stmt.command,
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_)
    ) {
        return false;
    }

    let normalized = command::normalize_command(&stmt.command, source);
    if !normalized.effective_name_is("echo") {
        return false;
    }

    let body_args = normalized.body_args();
    if body_args.first().is_some_and(|word| {
        static_word_text(word, source).is_some_and(|text| text.starts_with('-'))
    }) {
        return false;
    }

    if body_args
        .first()
        .is_some_and(|word| word_has_leading_dynamic_dash_literal(word, source))
    {
        return false;
    }

    if matches!(body_args, [word] if word_is_command_substitution_only(word)) {
        return false;
    }

    body_args
        .iter()
        .all(|word| !word_contains_unquoted_glob_or_brace(word, source))
}

fn word_is_command_substitution_only(word: &Word) -> bool {
    match word.parts.as_slice() {
        [
            WordPartNode {
                kind: WordPart::CommandSubstitution { .. },
                ..
            },
        ] => true,
        [
            WordPartNode {
                kind: WordPart::DoubleQuoted { parts, .. },
                ..
            },
        ] => matches!(
            parts.as_slice(),
            [WordPartNode {
                kind: WordPart::CommandSubstitution { .. },
                ..
            }]
        ),
        _ => false,
    }
}

fn word_has_leading_dynamic_dash_literal(word: &Word, source: &str) -> bool {
    let mut saw_dynamic = false;
    leading_dynamic_dash_literal_in_parts(&word.parts, source, &mut saw_dynamic).unwrap_or(false)
}

fn leading_dynamic_dash_literal_in_parts(
    parts: &[WordPartNode],
    source: &str,
    saw_dynamic: &mut bool,
) -> Option<bool> {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let text = text.as_str(source, part.span);
                if !text.is_empty() {
                    return Some(*saw_dynamic && text.starts_with('-'));
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                if !text.is_empty() {
                    return Some(*saw_dynamic && text.starts_with('-'));
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if let Some(result) =
                    leading_dynamic_dash_literal_in_parts(parts, source, saw_dynamic)
                {
                    return Some(result);
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                *saw_dynamic = true;
            }
        }
    }

    None
}

fn substitution_body_contains_ls<'a>(
    body: &'a StmtSeq,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    body.stmts
        .iter()
        .any(|stmt| stmt_contains_raw_ls(stmt, commands, command_ids_by_span))
}

fn substitution_body_contains_grep(body: &StmtSeq, source: &str) -> bool {
    let [stmt] = body.stmts.as_slice() else {
        return false;
    };

    command_contains_grep_output(&stmt.command, source)
}

fn stmt_contains_raw_ls<'a>(
    stmt: &'a Stmt,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
) -> bool {
    command_fact_for_stmt(stmt, commands, command_ids_by_span)
        .is_some_and(|fact| fact.literal_name() == Some("ls") && fact.wrappers().is_empty())
        || match &stmt.command {
            Command::Binary(binary) => {
                stmt_contains_raw_ls(&binary.left, commands, command_ids_by_span)
                    || stmt_contains_raw_ls(&binary.right, commands, command_ids_by_span)
            }
            Command::Compound(CompoundCommand::Subshell(body))
            | Command::Compound(CompoundCommand::BraceGroup(body)) => body
                .stmts
                .iter()
                .any(|stmt| stmt_contains_raw_ls(stmt, commands, command_ids_by_span)),
            Command::Compound(CompoundCommand::Time(command)) => command
                .command
                .as_deref()
                .is_some_and(|stmt| stmt_contains_raw_ls(stmt, commands, command_ids_by_span)),
            Command::Compound(
                CompoundCommand::If(_)
                | CompoundCommand::For(_)
                | CompoundCommand::Repeat(_)
                | CompoundCommand::Foreach(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Case(_)
                | CompoundCommand::Select(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Conditional(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Always(_),
            ) => false,
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => false,
            Command::Function(_) | Command::AnonymousFunction(_) => false,
        }
}

fn command_contains_grep_output(command: &Command, source: &str) -> bool {
    match command {
        Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {
            command_is_grep_family(command, source)
        }
        Command::Binary(binary) => match binary.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                command_contains_grep_output(&binary.right.command, source)
            }
            BinaryOp::And | BinaryOp::Or => false,
        },
        Command::Compound(CompoundCommand::Subshell(body))
        | Command::Compound(CompoundCommand::BraceGroup(body)) => {
            substitution_body_contains_grep(body, source)
        }
        Command::Compound(CompoundCommand::Time(command)) => command
            .command
            .as_deref()
            .is_some_and(|stmt| command_contains_grep_output(&stmt.command, source)),
        Command::Compound(
            CompoundCommand::If(_)
            | CompoundCommand::For(_)
            | CompoundCommand::Repeat(_)
            | CompoundCommand::Foreach(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Case(_)
            | CompoundCommand::Select(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Always(_),
        ) => false,
        Command::Function(_) | Command::AnonymousFunction(_) => false,
    }
}

fn command_name_is_grep_family(name: &str) -> bool {
    matches!(name, "grep" | "egrep" | "fgrep")
}

fn command_is_grep_family(command: &Command, source: &str) -> bool {
    let normalized = command::normalize_command(command, source);
    if normalized
        .effective_or_literal_name()
        .is_some_and(command_name_is_grep_family)
    {
        return true;
    }

    normalized.body_name_word().is_some_and(|word| {
        let text = word.span.slice(source).trim_start_matches('\\');
        let name = text.rsplit('/').next().unwrap_or(text);
        command_name_is_grep_family(name)
    })
}

fn word_contains_unquoted_glob_or_brace(word: &Word, source: &str) -> bool {
    word_parts_contain_unquoted_glob_or_brace(&word.parts, source, false)
}

fn word_parts_contain_unquoted_glob_or_brace(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if word_parts_contain_unquoted_glob_or_brace(parts, source, true) {
                    return true;
                }
            }
            WordPart::Literal(text) => {
                if !in_double_quotes
                    && text
                        .as_str(source, part.span)
                        .chars()
                        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
                {
                    return true;
                }
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
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
            WordPart::SingleQuoted { .. } => {}
        }
    }

    false
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
        if let Some(word) = redirect.word_target() {
            visitor(word);
        }
    }
}

fn is_bash_file_slurp_command(command: &Command, redirects: &[Redirect], source: &str) -> bool {
    let Command::Simple(command) = command else {
        return false;
    };

    if !command.assignments.is_empty()
        || !command.args.is_empty()
        || !command.name.render(source).is_empty()
    {
        return false;
    }

    matches!(
        redirects,
        [redirect]
            if redirect.kind == RedirectKind::Input
                && redirect.fd_var.is_none()
                && redirect.fd.unwrap_or(0) == 0
    )
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

fn visit_here_string_words_for_substitutions(
    redirects: &[Redirect],
    visitor: &mut impl FnMut(&Word),
) {
    for redirect in redirects {
        if redirect.kind == RedirectKind::HereString {
            let Some(word) = redirect.word_target() else {
                continue;
            };
            visitor(word);
        }
    }
}

fn visit_heredoc_bodies_for_substitutions(
    redirects: &[Redirect],
    visitor: &mut impl FnMut(&shuck_ast::HeredocBody),
) {
    for redirect in redirects {
        let Some(heredoc) = redirect.heredoc() else {
            continue;
        };
        if heredoc.delimiter.expands_body {
            visitor(&heredoc.body);
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
