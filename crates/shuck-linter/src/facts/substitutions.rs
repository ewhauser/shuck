#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionKind {
    Command,
    ProcessInput,
    ProcessOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionOutputIntent {
    Captured,
    Discarded,
    Rerouted,
    Mixed,
}

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
    body_processed_ls_pipeline_spans: Box<[Span]>,
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

    pub fn body_processed_ls_pipeline_spans(&self) -> &[Span] {
        &self.body_processed_ls_pipeline_spans
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

#[cfg_attr(shuck_profiling, inline(never))]
fn populate_substitution_fact_ranges<'a>(
    commands: &mut [CommandFact<'a>],
    fact_store: &mut FactStore<'a>,
    command_fact_indices_by_id: &[Option<usize>],
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
) {
    for index in 0..commands.len() {
        let substitutions = {
            let command_facts =
                CommandFacts::new(commands, fact_store, command_fact_indices_by_id);
            let fact = command_facts
                .get(index)
                .expect("command index should resolve while populating substitution facts");
            build_command_substitution_facts(
                fact,
                command_facts,
                command_ids_by_span,
                command_child_index,
                semantic,
                source,
            )
        };
        commands[index].substitution_facts = fact_store.substitution_facts.push_many(substitutions);
    }
}

fn build_command_substitution_facts<'a>(
    fact: CommandFactRef<'_, 'a>,
    commands: CommandFacts<'_, 'a>,
    command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
) -> Vec<SubstitutionFact> {
    let mut substitutions = Vec::new();
    let mut substitution_index = FxHashMap::default();
    let context = SubstitutionFactBuildContext {
        commands,
        command_relationships: CommandRelationshipContext::new(
            commands.commands,
            commands.indices_by_id,
            command_ids_by_span,
            command_child_index,
        ),
        host_command_id: fact.id(),
        semantic,
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

    substitutions
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
    commands: CommandFacts<'b, 'a>,
    command_relationships: CommandRelationshipContext<'b, 'a>,
    host_command_id: CommandId,
    semantic: &'b LinterSemanticArtifacts<'a>,
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
            context.command_relationships,
            context.host_command_id,
            context.semantic,
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
            body_processed_ls_pipeline_spans: body_facts.body_processed_ls_pipeline_spans,
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
    body_processed_ls_pipeline_spans: Box<[Span]>,
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
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
) -> SubstitutionBodyFacts {
    let mut body_has_commands = false;
    let mut body_contains_ls = false;
    let mut bash_file_slurp = false;
    let mut body_command_count = 0;
    semantic.for_each_command_visit_in_body(body, false, |visit| {
        body_has_commands = true;
        body_command_count += 1;
        if body_command_count == 1 {
            bash_file_slurp = is_bash_file_slurp_command(visit.command, visit.redirects, source);
        } else {
            bash_file_slurp = false;
        }
        if let Some(fact) = command_relationships.fact_for_stmt(visit.stmt) {
            body_contains_ls |= fact.literal_name() == Some("ls") && fact.wrappers().is_empty();
        }
    });
    let redirect_summary =
        summarize_stmt_seq_redirects(body, parent_id, commands, command_relationships, source);
    let body_processed_ls_pipeline_spans = substitution_body_processed_ls_pipeline_spans(
        body,
        parent_id,
        commands,
        command_relationships,
        semantic,
        source,
    );

    SubstitutionBodyFacts {
        stdout_intent: redirect_summary.stdout_intent,
        terminal_stdout_intent: redirect_summary.terminal_stdout_intent,
        has_stdout_redirect: redirect_summary.has_stdout_redirect,
        stdout_redirect_spans: redirect_summary.stdout_redirect_spans.into_boxed_slice(),
        stdout_dev_null_redirect_spans: redirect_summary
            .stdout_dev_null_redirect_spans
            .into_boxed_slice(),
        body_contains_ls,
        body_processed_ls_pipeline_spans: body_processed_ls_pipeline_spans.into_boxed_slice(),
        body_contains_echo: substitution_body_contains_echo(body, source),
        body_contains_grep: substitution_body_contains_grep(
            body,
            commands,
            command_relationships,
            parent_id,
            source,
        ),
        body_has_multiple_statements: body.stmts.len() > 1,
        body_is_negated: matches!(body.stmts.as_slice(), [stmt] if stmt.negated),
        body_is_pgrep_lookup: substitution_body_is_pgrep_lookup(
            body,
            commands,
            command_relationships.command_ids_by_span,
        ),
        body_is_seq_utility: substitution_body_is_seq_utility(
            body,
            commands,
            command_relationships.command_ids_by_span,
        ),
        body_has_commands,
        bash_file_slurp,
    }
}

fn summarize_stmt_seq_redirects<'a>(
    body: &'a StmtSeq,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
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
        let stmt_summary =
            summarize_stmt_redirects(stmt, parent_id, commands, command_relationships, source);
        summary = merge_redirect_summaries(summary, stmt_summary, saw_stmt);
        saw_stmt = true;
    }

    summary
}

fn summarize_stmt_redirects<'a>(
    stmt: &'a Stmt,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> RedirectSummary {
    match &stmt.command {
        Command::Binary(binary) => match binary.op {
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                let child_parent_id = command_relationships
                    .child_or_lookup_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                summarize_stmt_redirects(
                    &binary.right,
                    child_parent_id,
                    commands,
                    command_relationships,
                    source,
                )
            }
            BinaryOp::And | BinaryOp::Or => {
                let child_parent_id = command_relationships
                    .child_or_lookup_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                let left = summarize_stmt_redirects(
                    &binary.left,
                    child_parent_id,
                    commands,
                    command_relationships,
                    source,
                );
                let right = summarize_stmt_redirects(
                    &binary.right,
                    child_parent_id,
                    commands,
                    command_relationships,
                    source,
                );
                merge_redirect_summaries(left, right, true)
            }
        },
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
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Time(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Always(_),
        ) => summarize_command_redirects(
            stmt,
            parent_id,
            commands,
            command_relationships,
            source,
        ),
    }
}

fn summarize_command_redirects<'a>(
    stmt: &Stmt,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> RedirectSummary {
    if let Some(id) = command_relationships
        .child_id_for_command(parent_id, &stmt.command)
        .or_else(|| command_relationships.id_for_command(&stmt.command))
    {
        summarize_redirect_facts(command_fact_ref(commands, id).redirect_facts(), source)
    } else {
        let redirect_facts = build_redirect_facts(&stmt.redirects, None, source, None);
        summarize_redirect_facts(&redirect_facts, source)
    }
}

fn summarize_redirect_facts(redirects: &[RedirectFact<'_>], source: &str) -> RedirectSummary {
    let state = classify_redirect_facts(redirects);
    RedirectSummary {
        stdout_intent: state.stdout_intent,
        terminal_stdout_intent: state.stdout_intent,
        has_stdout_redirect: state.has_stdout_redirect,
        stdout_redirect_spans: stdout_redirect_spans_for_fix(redirects, source),
        stdout_dev_null_redirect_spans: stdout_dev_null_redirect_spans_for_fix(redirects, source),
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

fn stdout_redirect_spans_for_fix(redirects: &[RedirectFact<'_>], source: &str) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| redirect_matches_substitution_warning(redirect, source))
        .map(|redirect| redirect.redirect().span)
        .collect()
}

fn stdout_dev_null_redirect_spans_for_fix(
    redirects: &[RedirectFact<'_>],
    source: &str,
) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| {
            redirect_targets_stdout_dev_null_for_substitution_warning(redirect, source)
        })
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

fn redirect_targets_stdout_dev_null_for_substitution_warning(
    redirect: &RedirectFact<'_>,
    source: &str,
) -> bool {
    redirect_matches_substitution_warning(redirect, source)
        && redirect_targets_stdout_dev_null(redirect)
}

fn redirect_matches_substitution_warning(redirect: &RedirectFact<'_>, source: &str) -> bool {
    match redirect.redirect().kind {
        RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
            redirect.redirect().fd.unwrap_or(1) == 1
        }
        RedirectKind::DupOutput => {
            redirect.redirect().fd.unwrap_or(1) == 1
                && redirect.redirect().span.slice(source).trim_start().starts_with(">&")
                && dup_stdout_target_redirects_capture_away(redirect, source)
        }
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::OutputBoth
        | RedirectKind::DupInput => false,
    }
}

fn dup_stdout_target_redirects_capture_away(redirect: &RedirectFact<'_>, source: &str) -> bool {
    let Some(target) = redirect.target_span().map(|span| span.slice(source).trim()) else {
        return false;
    };

    target == "-" || (target.chars().all(|ch| ch.is_ascii_digit()) && target != "1")
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

    if !normalized
        .body_name_word()
        .is_some_and(|word| command_name_word_matches_source(word, source, "echo"))
    {
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

fn command_name_word_matches_source(word: &Word, source: &str, name: &str) -> bool {
    static_command_name_text(word, source).is_some_and(|decoded| decoded == name)
        && source_span_static_command_name(word.span, source).as_deref() == Some(name)
}

fn source_span_static_command_name(span: Span, source: &str) -> Option<String> {
    let mut chars = span.slice(source).trim().chars().peekable();
    let mut decoded = String::new();
    let mut quote = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    decoded.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    append_backslash_escaped_char(&mut chars, &mut decoded)?;
                } else if matches!(ch, '$' | '`') {
                    return None;
                } else {
                    decoded.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => append_backslash_escaped_char(&mut chars, &mut decoded)?,
                '$' | '`' | '(' | ')' | ';' | '&' | '|' | '<' | '>' => return None,
                ch if ch.is_whitespace() => return None,
                _ => decoded.push(ch),
            },
            Some(_) => unreachable!("quote state only stores shell quote delimiters"),
        }
    }

    quote.is_none().then_some(decoded).filter(|text| !text.is_empty())
}

fn append_backslash_escaped_char(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    decoded: &mut String,
) -> Option<()> {
    match chars.next()? {
        '\n' => {}
        '\r' if chars.next_if_eq(&'\n').is_some() => {}
        ch => decoded.push(ch),
    }
    Some(())
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

fn substitution_body_processed_ls_pipeline_spans<'a>(
    body: &'a StmtSeq,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    semantic.for_each_command_visit_in_body(body, true, |visit| {
        collect_processed_ls_pipeline_spans_in_stmt(
            visit.stmt,
            parent_id,
            commands,
            command_relationships,
            source,
            &mut spans,
        );
    });
    spans
}

fn collect_processed_ls_pipeline_spans_in_stmt<'a>(
    stmt: &'a Stmt,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Command::Binary(binary) = &stmt.command else {
        return;
    };
    if !matches!(binary.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
        return;
    }

    let mut segments = Vec::new();
    let mut operators = Vec::new();
    collect_pipeline_parts(binary, &mut segments, &mut operators);

    for (index, pair) in segments.windows(2).enumerate() {
        let pipeline_id = command_relationships
            .child_or_lookup_fact(parent_id, stmt)
            .map_or(parent_id, CommandFact::id);
        if stmt_is_raw_ls(pair[0], pipeline_id, commands, command_relationships, source)
            && !stmt_static_utility_name_is(
                pair[1],
                pipeline_id,
                commands,
                command_relationships,
                source,
                "grep",
            )
            && !stmt_static_utility_name_is(
                pair[1],
                pipeline_id,
                commands,
                command_relationships,
                source,
                "xargs",
            )
        {
            spans.push(ls_command_span_before_pipe(
                pair[0],
                operators[index],
                pipeline_id,
                commands,
                command_relationships,
                source,
            ));
        }
    }
}

fn collect_pipeline_parts<'a>(
    command: &'a BinaryCommand,
    segments: &mut Vec<&'a Stmt>,
    operators: &mut Vec<Span>,
) {
    match &command.left.command {
        Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_parts(left, segments, operators);
        }
        _ => segments.push(&command.left),
    }

    operators.push(command.op_span);

    match &command.right.command {
        Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_parts(right, segments, operators);
        }
        _ => segments.push(&command.right),
    }
}

fn stmt_is_raw_ls<'a>(
    stmt: &'a Stmt,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> bool {
    if let Some(fact) = command_relationships
        .child_id_for_command(parent_id, &stmt.command)
        .or_else(|| command_relationships.id_for_command(&stmt.command))
        .map(|id| command_fact_ref(commands, id))
    {
        return fact.literal_name() == Some("ls") && fact.wrappers().is_empty();
    }

    let normalized = command::normalize_command(&stmt.command, source);
    normalized.literal_name.as_deref() == Some("ls") && normalized.wrappers.is_empty()
}

fn stmt_static_utility_name_is<'a>(
    stmt: &'a Stmt,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
    name: &str,
) -> bool {
    if let Some(fact) = command_relationships
        .child_id_for_command(parent_id, &stmt.command)
        .or_else(|| command_relationships.id_for_command(&stmt.command))
        .map(|id| command_fact_ref(commands, id))
    {
        return fact.static_utility_name() == Some(name);
    }

    command::normalize_command(&stmt.command, source).effective_or_literal_name() == Some(name)
}

fn ls_command_span_before_pipe<'a>(
    stmt: &'a Stmt,
    operator_span: Span,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> Span {
    let start = command_relationships
        .child_id_for_command(parent_id, &stmt.command)
        .or_else(|| command_relationships.id_for_command(&stmt.command))
        .map(|id| command_fact_ref(commands, id))
        .and_then(|fact| fact.shellcheck_command_span(source))
        .unwrap_or_else(|| command::normalize_command(&stmt.command, source).body_span)
        .start;
    let span = Span {
        start,
        end: operator_span.start,
    };
    let trimmed = span.slice(source).trim_end();
    Span {
        start,
        end: start.advanced_by(trimmed),
    }
}

fn substitution_body_contains_grep<'a>(
    body: &'a StmtSeq,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
) -> bool {
    substitution_body_terminal_command_fact(body, commands, command_relationships, parent_id)
        .is_some_and(|fact| command_fact_is_grep_family(fact, source))
}

fn substitution_body_terminal_command_fact<'facts, 'a>(
    body: &'a StmtSeq,
    commands: CommandFacts<'facts, 'a>,
    command_relationships: CommandRelationshipContext<'facts, 'a>,
    parent_id: CommandId,
) -> Option<CommandFactRef<'facts, 'a>> {
    let [stmt] = body.stmts.as_slice() else {
        return None;
    };
    let mut stmt = stmt;
    let mut current_parent_id = parent_id;

    loop {
        let fact = command_relationships
            .child_id_for_command(current_parent_id, &stmt.command)
            .or_else(|| command_relationships.id_for_command(&stmt.command))
            .map(|id| command_fact_ref(commands, id))?;

        match &stmt.command {
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => return Some(fact),
            Command::Binary(binary) => match binary.op {
                BinaryOp::Pipe | BinaryOp::PipeAll => {
                    current_parent_id = fact.id();
                    stmt = &binary.right;
                }
                BinaryOp::And | BinaryOp::Or => return None,
            },
            Command::Compound(CompoundCommand::Subshell(body))
            | Command::Compound(CompoundCommand::BraceGroup(body)) => {
                let [inner] = body.stmts.as_slice() else {
                    return None;
                };
                current_parent_id = fact.id();
                stmt = inner;
            }
            Command::Compound(CompoundCommand::Time(command)) => {
                let inner = command.command.as_deref()?;
                current_parent_id = fact.id();
                stmt = inner;
            }
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
            )
            | Command::Function(_)
            | Command::AnonymousFunction(_) => return None,
        }
    }
}

fn command_fact_is_grep_family(fact: CommandFactRef<'_, '_>, source: &str) -> bool {
    if fact
        .effective_or_literal_name()
        .is_some_and(command_name_is_grep_family)
    {
        return true;
    }

    fact.body_name_word().is_some_and(|word| {
        let text = word.span.slice(source).trim_start_matches('\\');
        let name = text.rsplit('/').next().unwrap_or(text);
        command_name_is_grep_family(name)
    })
}

fn command_name_is_grep_family(name: &str) -> bool {
    matches!(name, "grep" | "egrep" | "fgrep")
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

fn visit_command_argument_words_for_substitutions<'a>(
    command: &'a Command,
    source: &str,
    visitor: &mut impl FnMut(&'a Word),
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
    for assignment in command_assignments(command) {
        visit_var_ref_subscript_words_with_source(&assignment.target, source, &mut |word| {
            visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
        });

        if let AssignmentValue::Compound(array) = &assignment.value {
            for element in &array.elements {
                if let shuck_ast::ArrayElem::Keyed { key, .. }
                | shuck_ast::ArrayElem::KeyedAppend { key, .. } = element
                {
                    visit_subscript_words(Some(key), source, &mut |word| {
                        visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                    });
                }
            }
        }
    }

    for operand in declaration_operands(command) {
        match operand {
            DeclOperand::Name(reference) => {
                visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
                    visitor(SubstitutionHostKind::DeclarationNameSubscript, word);
                });
            }
            DeclOperand::Assignment(assignment) => {
                visit_var_ref_subscript_words_with_source(
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
                            visit_subscript_words(Some(key), source, &mut |word| {
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
        visit_var_ref_subscript_words_with_source(&assignment.target, source, visitor);

        match &assignment.value {
            AssignmentValue::Scalar(word) => visitor(word),
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        shuck_ast::ArrayElem::Sequential(word) => visitor(word),
                        shuck_ast::ArrayElem::Keyed { key, value }
                        | shuck_ast::ArrayElem::KeyedAppend { key, value } => {
                            visit_subscript_words(Some(key), source, visitor);
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
            visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
        DeclOperand::Assignment(assignment) => {
            visit_assignments_for_substitutions(std::slice::from_ref(assignment), source, visitor);
        }
    }
}

fn visit_words_for_substitutions<'a>(words: &'a [Word], visitor: &mut impl FnMut(&'a Word)) {
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
            visit_var_ref_subscript_words_with_source(reference, source, visitor);
        }
    }
}
