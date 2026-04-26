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

fn populate_substitution_fact_ranges<'a>(
    commands: &mut [CommandFact<'a>],
    fact_store: &mut FactStore<'a>,
    _command_ids_by_span: &CommandLookupIndex,
    command_child_index: &CommandChildIndex,
    arena_file: &ArenaFile,
    source: &str,
) {
    for index in 0..commands.len() {
        let substitutions = {
            let command_facts = CommandFacts::new(commands, fact_store, arena_file);
            let fact = command_facts
                .get(index)
                .expect("command index should resolve while populating substitution facts");
            build_command_substitution_facts(
                fact,
                command_facts,
                command_child_index,
                source,
            )
        };
        commands[index].substitution_facts = fact_store.substitution_facts.push_many(substitutions);
    }
}

fn build_command_substitution_facts<'a>(
    fact: CommandFactRef<'_, 'a>,
    commands: CommandFacts<'_, 'a>,
    command_child_index: &CommandChildIndex,
    source: &str,
) -> Vec<SubstitutionFact> {
    let relationships = CommandRelationshipContext::new(commands.commands, command_child_index);
    let context = ArenaSubstitutionFactBuildContext {
        commands,
        command_relationships: relationships,
        host_command_id: fact.id(),
        arena_file: fact.arena_file,
        source,
    };
    let mut substitutions = Vec::new();
    let mut substitution_index = FxHashMap::default();

    if let Some(command) = fact.arena_command() {
        for word_id in command.word_ids() {
            collect_or_update_arena_word_substitution_facts(
                fact.arena_file.store.word(*word_id),
                SubstitutionHostKind::Other,
                context,
                &mut substitutions,
                &mut substitution_index,
            );
        }
        visit_arena_command_argument_words_for_substitutions(
            command,
            source,
            &mut |word| {
                collect_or_update_arena_word_substitution_facts(
                    word,
                    SubstitutionHostKind::CommandArgument,
                    context,
                    &mut substitutions,
                    &mut substitution_index,
                );
            },
        );
        visit_arena_declaration_assignment_words_for_substitutions(command, &mut |word| {
            collect_or_update_arena_word_substitution_facts(
                word,
                SubstitutionHostKind::DeclarationAssignmentValue,
                context,
                &mut substitutions,
                &mut substitution_index,
            );
        });
        visit_arena_command_subscript_words_for_substitutions(command, &mut |host_kind, word| {
            collect_or_update_arena_word_substitution_facts(
                word,
                host_kind,
                context,
                &mut substitutions,
                &mut substitution_index,
            );
        });
    }

    if let Some(stmt) = fact.arena_stmt() {
        for redirect in stmt.redirects() {
            match &redirect.target {
                RedirectTargetNode::Word(word_id) => {
                    let host_kind = if redirect.kind == RedirectKind::HereString {
                        SubstitutionHostKind::HereStringOperand
                    } else {
                        SubstitutionHostKind::Other
                    };
                    collect_or_update_arena_word_substitution_facts(
                        fact.arena_file.store.word(*word_id),
                        host_kind,
                        context,
                        &mut substitutions,
                        &mut substitution_index,
                    );
                }
                RedirectTargetNode::Heredoc(heredoc) => {
                    collect_or_update_arena_heredoc_body_substitution_facts(
                        &heredoc.body,
                        fact.arena_file,
                        SubstitutionHostKind::Other,
                        context,
                        &mut substitutions,
                        &mut substitution_index,
                    );
                }
            }
        }
    }

    substitutions
}

fn visit_arena_command_argument_words_for_substitutions(
    command: CommandView<'_>,
    source: &str,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    let store = command.store();
    match command.kind() {
        ArenaFileCommandKind::Simple => {
            let command = command.simple().expect("simple command view");
            if static_word_text_arena(command.name(), source).as_deref() == Some("trap") {
                return;
            }
            for word in command.args() {
                visitor(word);
            }
        }
        ArenaFileCommandKind::Builtin => {
            let command = command.builtin().expect("builtin command view");
            if let Some(word) = command.primary() {
                visitor(word);
            }
            for word in command.extra_args() {
                visitor(word);
            }
        }
        ArenaFileCommandKind::Decl => {
            let command = command.decl().expect("decl command view");
            for operand in command.operands() {
                if let DeclOperandNode::Dynamic(word) = operand {
                    visitor(store.word(*word));
                }
            }
        }
        ArenaFileCommandKind::Function => {
            let command = command.function().expect("function command view");
            for entry in command.entries() {
                visitor(store.word(entry.word));
            }
        }
        ArenaFileCommandKind::AnonymousFunction => {
            let command = command
                .anonymous_function()
                .expect("anonymous function command view");
            for word in command.args() {
                visitor(word);
            }
        }
        ArenaFileCommandKind::Binary | ArenaFileCommandKind::Compound => {}
    }
}

fn visit_arena_declaration_assignment_words_for_substitutions(
    command: CommandView<'_>,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    let Some(command) = command.decl() else {
        return;
    };

    for operand in command.operands() {
        let DeclOperandNode::Assignment(assignment) = operand else {
            continue;
        };
        if let AssignmentValueNode::Scalar(word) = assignment.value {
            visitor(command.store().word(word));
        }
    }
}

fn visit_arena_command_subscript_words_for_substitutions(
    command: CommandView<'_>,
    visitor: &mut impl FnMut(SubstitutionHostKind, WordView<'_>),
) {
    let store = command.store();
    for assignment in arena_command_assignments(command) {
        visit_arena_var_ref_subscript_words(&assignment.target, store, &mut |word| {
            visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
        });
        visit_arena_array_key_subscript_words(&assignment.value, store, visitor);
    }

    for operand in arena_declaration_operands(command) {
        match operand {
            DeclOperandNode::Name(reference) => {
                visit_arena_var_ref_subscript_words(reference, store, &mut |word| {
                    visitor(SubstitutionHostKind::DeclarationNameSubscript, word);
                });
            }
            DeclOperandNode::Assignment(assignment) => {
                visit_arena_var_ref_subscript_words(&assignment.target, store, &mut |word| {
                    visitor(SubstitutionHostKind::AssignmentTargetSubscript, word);
                });
                visit_arena_array_key_subscript_words(&assignment.value, store, visitor);
            }
            DeclOperandNode::Flag(_) | DeclOperandNode::Dynamic(_) => {}
        }
    }
}

fn visit_arena_array_key_subscript_words(
    value: &AssignmentValueNode,
    store: &AstStore,
    visitor: &mut impl FnMut(SubstitutionHostKind, WordView<'_>),
) {
    let AssignmentValueNode::Compound(array) = value else {
        return;
    };

    for element in store.array_elems(array.elements) {
        match element {
            ArrayElemNode::Keyed { key, .. } | ArrayElemNode::KeyedAppend { key, .. } => {
                visit_arena_subscript_words(key, store, &mut |word| {
                    visitor(SubstitutionHostKind::ArrayKeySubscript, word);
                });
            }
            ArrayElemNode::Sequential(_) => {}
        }
    }
}

fn visit_arena_var_ref_subscript_words(
    reference: &VarRefNode,
    store: &AstStore,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    if let Some(subscript) = reference.subscript.as_deref() {
        visit_arena_subscript_words(subscript, store, visitor);
    }
}

fn visit_arena_subscript_words(
    subscript: &SubscriptNode,
    store: &AstStore,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    if matches!(subscript.kind, shuck_ast::SubscriptKind::Selector(_)) {
        return;
    }
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        visit_arena_arithmetic_words(expression, store, visitor);
        return;
    }
    if let Some(word) = subscript.word_ast {
        visitor(store.word(word));
    }
}

fn visit_arena_arithmetic_words(
    expression: &ArithmeticExprArenaNode,
    store: &AstStore,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    match &expression.kind {
        ArithmeticExprArena::Number(_) | ArithmeticExprArena::Variable(_) => {}
        ArithmeticExprArena::Indexed { index, .. } => {
            visit_arena_arithmetic_words(index, store, visitor);
        }
        ArithmeticExprArena::ShellWord(word) => visitor(store.word(*word)),
        ArithmeticExprArena::Parenthesized { expression } => {
            visit_arena_arithmetic_words(expression, store, visitor);
        }
        ArithmeticExprArena::Unary { expr, .. } | ArithmeticExprArena::Postfix { expr, .. } => {
            visit_arena_arithmetic_words(expr, store, visitor);
        }
        ArithmeticExprArena::Binary { left, right, .. } => {
            visit_arena_arithmetic_words(left, store, visitor);
            visit_arena_arithmetic_words(right, store, visitor);
        }
        ArithmeticExprArena::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arena_arithmetic_words(condition, store, visitor);
            visit_arena_arithmetic_words(then_expr, store, visitor);
            visit_arena_arithmetic_words(else_expr, store, visitor);
        }
        ArithmeticExprArena::Assignment { target, value, .. } => {
            visit_arena_arithmetic_lvalue_words(target, store, visitor);
            visit_arena_arithmetic_words(value, store, visitor);
        }
    }
}

fn visit_arena_arithmetic_lvalue_words(
    target: &ArithmeticLvalueArena,
    store: &AstStore,
    visitor: &mut impl FnMut(WordView<'_>),
) {
    match target {
        ArithmeticLvalueArena::Variable(_) => {}
        ArithmeticLvalueArena::Indexed { index, .. } => {
            visit_arena_arithmetic_words(index, store, visitor);
        }
    }
}

#[derive(Clone, Copy)]
struct ArenaSubstitutionFactBuildContext<'facts, 'a> {
    commands: CommandFacts<'facts, 'a>,
    command_relationships: CommandRelationshipContext<'facts, 'a>,
    host_command_id: CommandId,
    arena_file: &'facts ArenaFile,
    source: &'facts str,
}

fn collect_or_update_arena_word_substitution_facts<'a>(
    word: WordView<'_>,
    host_kind: SubstitutionHostKind,
    context: ArenaSubstitutionFactBuildContext<'_, 'a>,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    collect_arena_word_substitution_occurrences(
        word.store(),
        word.parts(),
        false,
        &mut occurrences,
    );
    collect_or_update_arena_substitution_facts_from_occurrences(
        word.span(),
        host_kind,
        occurrences,
        context,
        substitutions,
        substitution_index,
    );
}

fn collect_or_update_arena_heredoc_body_substitution_facts<'a>(
    body: &shuck_ast::HeredocBodyNode,
    arena_file: &ArenaFile,
    host_kind: SubstitutionHostKind,
    context: ArenaSubstitutionFactBuildContext<'_, 'a>,
    substitutions: &mut Vec<SubstitutionFact>,
    substitution_index: &mut FxHashMap<FactSpan, usize>,
) {
    let mut occurrences = Vec::new();
    for part in arena_file.store.heredoc_body_parts(body.parts) {
        match &part.kind {
            ArenaHeredocBodyPart::CommandSubstitution { body, syntax } => {
                occurrences.push(ArenaSubstitutionOccurrence {
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    command_syntax: Some(*syntax),
                    body: *body,
                    unquoted_in_host: true,
                });
            }
            ArenaHeredocBodyPart::ArithmeticExpansion {
                expression_word_ast, ..
            } => collect_arena_word_substitution_occurrences(
                &arena_file.store,
                arena_file.store.word(*expression_word_ast).parts(),
                false,
                &mut occurrences,
            ),
            ArenaHeredocBodyPart::Parameter(_) => {}
            ArenaHeredocBodyPart::Literal(_) | ArenaHeredocBodyPart::Variable(_) => {}
        }
    }
    collect_or_update_arena_substitution_facts_from_occurrences(
        body.span,
        host_kind,
        occurrences,
        context,
        substitutions,
        substitution_index,
    );
}

fn collect_or_update_arena_substitution_facts_from_occurrences<'a>(
    host_span: Span,
    host_kind: SubstitutionHostKind,
    occurrences: Vec<ArenaSubstitutionOccurrence>,
    context: ArenaSubstitutionFactBuildContext<'_, 'a>,
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

        let body_facts = classify_arena_substitution_body(
            occurrence.body,
            context.arena_file,
            context.commands,
            context.command_relationships,
            context.host_command_id,
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

#[derive(Clone, Copy)]
struct ArenaSubstitutionOccurrence {
    span: Span,
    kind: CommandSubstitutionKind,
    command_syntax: Option<CommandSubstitutionSyntax>,
    body: shuck_ast::StmtSeqId,
    unquoted_in_host: bool,
}

fn collect_arena_word_substitution_occurrences(
    store: &AstStore,
    parts: &[WordPartArenaNode],
    quoted: bool,
    occurrences: &mut Vec<ArenaSubstitutionOccurrence>,
) {
    for part in parts {
        match &part.kind {
            WordPartArena::CommandSubstitution { body, syntax } => {
                occurrences.push(ArenaSubstitutionOccurrence {
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                    command_syntax: Some(*syntax),
                    body: *body,
                    unquoted_in_host: !quoted,
                });
            }
            WordPartArena::ProcessSubstitution { body, is_input } => {
                occurrences.push(ArenaSubstitutionOccurrence {
                    span: part.span,
                    kind: if *is_input {
                        CommandSubstitutionKind::ProcessInput
                    } else {
                        CommandSubstitutionKind::ProcessOutput
                    },
                    command_syntax: None,
                    body: *body,
                    unquoted_in_host: !quoted,
                });
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                collect_arena_word_substitution_occurrences(
                    store,
                    store.word_parts(*parts),
                    true,
                    occurrences,
                );
            }
            WordPartArena::ArithmeticExpansion {
                expression_word_ast, ..
            } => {
                // The arithmetic expression word was lowered into the same arena.
                // It is visited by command.word_ids(); avoid duplicating it here.
                let _ = expression_word_ast;
            }
            WordPartArena::Parameter(_) => {}
            WordPartArena::ParameterExpansion {
                operand_word_ast, ..
            }
            | WordPartArena::IndirectExpansion {
                operand_word_ast, ..
            } => {
                let _ = operand_word_ast;
            }
            WordPartArena::Substring { .. }
            | WordPartArena::ArraySlice { .. }
            | WordPartArena::Literal(_)
            | WordPartArena::Variable(_)
            | WordPartArena::SingleQuoted { .. }
            | WordPartArena::Length(_)
            | WordPartArena::ArrayAccess(_)
            | WordPartArena::ArrayLength(_)
            | WordPartArena::ArrayIndices(_)
            | WordPartArena::PrefixMatch { .. }
            | WordPartArena::Transformation { .. }
            | WordPartArena::ZshQualifiedGlob(_) => {}
        }
    }
}

fn classify_arena_substitution_body<'a>(
    body: shuck_ast::StmtSeqId,
    arena_file: &ArenaFile,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    parent_id: CommandId,
    source: &str,
) -> SubstitutionBodyFacts {
    if commands.get(parent_id.index()).is_none() {
        return empty_substitution_body_facts();
    }
    let body = arena_file.store.stmt_seq(body);
    let stmt_count = body.stmts().len();
    let visits = iter_arena_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
    )
    .collect::<Vec<_>>();
    let redirect_summary =
        summarize_arena_stmt_seq_redirects(body, parent_id, commands, command_relationships, source);

    SubstitutionBodyFacts {
        stdout_intent: redirect_summary.stdout_intent,
        terminal_stdout_intent: redirect_summary.terminal_stdout_intent,
        has_stdout_redirect: redirect_summary.has_stdout_redirect,
        stdout_redirect_spans: redirect_summary.stdout_redirect_spans.into_boxed_slice(),
        stdout_dev_null_redirect_spans: redirect_summary
            .stdout_dev_null_redirect_spans
            .into_boxed_slice(),
        body_contains_ls: arena_substitution_body_contains_ls(body, parent_id, command_relationships),
        body_processed_ls_pipeline_spans: arena_substitution_body_processed_ls_pipeline_spans(
            body,
            parent_id,
            commands,
            command_relationships,
            source,
        )
        .into_boxed_slice(),
        body_contains_echo: arena_substitution_body_contains_echo(body, source),
        body_contains_grep: arena_substitution_body_contains_grep(body, source),
        body_has_multiple_statements: stmt_count > 1,
        body_is_negated: matches!(body.stmts().next(), Some(stmt) if stmt_count == 1 && stmt.negated()),
        body_is_pgrep_lookup: arena_substitution_body_is_simple_command_named(
            body,
            commands,
            command_relationships,
            "pgrep",
        ),
        body_is_seq_utility: arena_substitution_body_is_simple_command_named(
            body,
            commands,
            command_relationships,
            "seq",
        ),
        body_has_commands: !visits.is_empty(),
        bash_file_slurp: matches!(
            visits.as_slice(),
            [visit] if is_arena_bash_file_slurp_command(visit.command, visit.redirects, source)
        ),
    }
}

fn empty_substitution_body_facts() -> SubstitutionBodyFacts {
    SubstitutionBodyFacts {
        stdout_intent: SubstitutionOutputIntent::Captured,
        terminal_stdout_intent: SubstitutionOutputIntent::Captured,
        has_stdout_redirect: false,
        stdout_redirect_spans: Box::new([]),
        stdout_dev_null_redirect_spans: Box::new([]),
        body_contains_ls: false,
        body_processed_ls_pipeline_spans: Box::new([]),
        body_contains_echo: false,
        body_contains_grep: false,
        body_has_multiple_statements: false,
        body_is_negated: false,
        body_is_pgrep_lookup: false,
        body_is_seq_utility: false,
        body_has_commands: false,
        bash_file_slurp: false,
    }
}

fn summarize_arena_stmt_redirects(stmt: StmtView<'_>, source: &str) -> RedirectSummary {
    summarize_arena_redirect_nodes(stmt.redirects(), stmt.command().store(), source)
}

fn summarize_arena_stmt_seq_redirects<'a>(
    body: StmtSeqView<'a>,
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

    for stmt in body.stmts() {
        let stmt_summary =
            summarize_arena_stmt_redirects_deep(stmt, parent_id, commands, command_relationships, source);
        summary = merge_redirect_summaries(summary, stmt_summary, saw_stmt);
        saw_stmt = true;
    }

    summary
}

#[allow(clippy::only_used_in_recursion)]
fn summarize_arena_stmt_redirects_deep<'a>(
    stmt: StmtView<'a>,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> RedirectSummary {
    let command = stmt.command();
    if let Some(binary) = command.binary() {
        match binary.op() {
            BinaryOp::Pipe | BinaryOp::PipeAll => {
                let child_parent_id = command_relationships
                    .child_or_lookup_arena_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                return single_arena_stmt(binary.right())
                    .map(|right| {
                        summarize_arena_stmt_redirects_deep(
                            right,
                            child_parent_id,
                            commands,
                            command_relationships,
                            source,
                        )
                    })
                    .unwrap_or_else(empty_redirect_summary);
            }
            BinaryOp::And | BinaryOp::Or => {
                let child_parent_id = command_relationships
                    .child_or_lookup_arena_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                let left = single_arena_stmt(binary.left())
                    .map(|left| {
                        summarize_arena_stmt_redirects_deep(
                            left,
                            child_parent_id,
                            commands,
                            command_relationships,
                            source,
                        )
                    })
                    .unwrap_or_else(empty_redirect_summary);
                let right = single_arena_stmt(binary.right())
                    .map(|right| {
                        summarize_arena_stmt_redirects_deep(
                            right,
                            child_parent_id,
                            commands,
                            command_relationships,
                            source,
                        )
                    })
                    .unwrap_or_else(empty_redirect_summary);
                return merge_redirect_summaries(left, right, true);
            }
        }
    }

    summarize_arena_stmt_redirects(stmt, source)
}

fn empty_redirect_summary() -> RedirectSummary {
    RedirectSummary {
        stdout_intent: SubstitutionOutputIntent::Captured,
        terminal_stdout_intent: SubstitutionOutputIntent::Captured,
        has_stdout_redirect: false,
        stdout_redirect_spans: Vec::new(),
        stdout_dev_null_redirect_spans: Vec::new(),
    }
}

fn summarize_arena_redirect_nodes(
    redirects: &[RedirectNode],
    store: &AstStore,
    source: &str,
) -> RedirectSummary {
    let state = classify_arena_redirect_nodes(redirects, store, source);
    RedirectSummary {
        stdout_intent: state.stdout_intent,
        terminal_stdout_intent: state.stdout_intent,
        has_stdout_redirect: state.has_stdout_redirect,
        stdout_redirect_spans: arena_stdout_redirect_spans_for_fix(redirects, source),
        stdout_dev_null_redirect_spans: arena_stdout_dev_null_redirect_spans_for_fix(
            redirects, store, source,
        ),
    }
}

fn arena_redirect_targets_stdout_dev_null(
    redirect: &RedirectNode,
    store: &AstStore,
    source: &str,
) -> bool {
    let RedirectTargetNode::Word(word_id) = redirect.target else {
        return false;
    };
    static_word_text_arena(store.word(word_id), source).as_deref() == Some("/dev/null")
}

fn classify_arena_redirect_nodes(
    redirects: &[RedirectNode],
    store: &AstStore,
    source: &str,
) -> RedirectState {
    let mut fds = FxHashMap::from_iter([(1, OutputSink::Captured), (2, OutputSink::Other)]);
    let mut has_stdout_redirect = false;

    for redirect in redirects {
        match redirect.kind {
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                let sink = arena_redirect_file_sink(redirect, store, source);
                let fd = redirect.fd.unwrap_or(1);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::OutputBoth => {
                let sink = arena_redirect_file_sink(redirect, store, source);
                has_stdout_redirect = true;
                fds.insert(1, sink);
                fds.insert(2, sink);
            }
            RedirectKind::DupOutput => {
                let fd = redirect.fd.unwrap_or(1);
                let sink = arena_redirect_dup_output_sink(redirect, store, source, &fds);
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

fn arena_stdout_redirect_spans_for_fix(redirects: &[RedirectNode], source: &str) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| arena_redirect_matches_substitution_warning(redirect, source))
        .map(|redirect| redirect.span)
        .collect()
}

fn arena_stdout_dev_null_redirect_spans_for_fix(
    redirects: &[RedirectNode],
    store: &AstStore,
    source: &str,
) -> Vec<Span> {
    redirects
        .iter()
        .filter(|redirect| {
            arena_redirect_matches_substitution_warning(redirect, source)
                && arena_redirect_targets_stdout_dev_null(redirect, store, source)
        })
        .map(|redirect| redirect.span)
        .collect()
}

fn arena_redirect_file_sink(
    redirect: &RedirectNode,
    store: &AstStore,
    source: &str,
) -> OutputSink {
    if arena_redirect_targets_stdout_dev_null(redirect, store, source) {
        OutputSink::DevNull
    } else {
        OutputSink::Other
    }
}

fn arena_redirect_dup_output_sink(
    redirect: &RedirectNode,
    store: &AstStore,
    source: &str,
    fds: &FxHashMap<i32, OutputSink>,
) -> OutputSink {
    let Some(fd) = arena_redirect_static_target_text(redirect, store, source)
        .and_then(|text| text.parse::<i32>().ok())
    else {
        return OutputSink::Other;
    };

    *fds.get(&fd).unwrap_or(&OutputSink::Other)
}

fn arena_redirect_matches_substitution_warning(redirect: &RedirectNode, source: &str) -> bool {
    match redirect.kind {
        RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
            redirect.fd.unwrap_or(1) == 1
        }
        RedirectKind::DupOutput => {
            redirect.fd.unwrap_or(1) == 1
                && redirect.span.slice(source).trim_start().starts_with(">&")
                && arena_dup_stdout_target_redirects_capture_away(redirect, source)
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

fn arena_dup_stdout_target_redirects_capture_away(redirect: &RedirectNode, source: &str) -> bool {
    let Some(target) = arena_redirect_target_source(redirect, source).map(str::trim)
    else {
        return false;
    };

    target == "-" || (target.chars().all(|ch| ch.is_ascii_digit()) && target != "1")
}

fn arena_redirect_target_source<'a>(redirect: &RedirectNode, source: &'a str) -> Option<&'a str> {
    match redirect.target {
        RedirectTargetNode::Word(_) => {
            let text = redirect.span.slice(source);
            let operator_start = text
                .char_indices()
                .find(|(_, ch)| matches!(ch, '<' | '>'))
                .map_or(0, |(index, _)| index);
            let operator_text = &text[operator_start..];
            let target_start = operator_text
                .char_indices()
                .find(|(_, ch)| !matches!(ch, '<' | '>' | '&' | '|'))
                .map_or(operator_text.len(), |(index, _)| index);
            Some(operator_text[target_start..].trim_start())
        }
        RedirectTargetNode::Heredoc(_) => None,
    }
}

fn arena_redirect_static_target_text<'a>(
    redirect: &RedirectNode,
    store: &AstStore,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    let RedirectTargetNode::Word(word_id) = redirect.target else {
        return None;
    };
    static_word_text_arena(store.word(word_id), source)
}

fn arena_substitution_body_contains_ls<'a>(
    body: StmtSeqView<'a>,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> bool {
    body.stmts()
        .any(|stmt| arena_stmt_contains_raw_ls(stmt, parent_id, command_relationships))
}

fn arena_substitution_body_processed_ls_pipeline_spans<'a>(
    body: StmtSeqView<'a>,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    for visit in iter_arena_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        collect_arena_processed_ls_pipeline_spans_in_stmt(
            visit.stmt,
            parent_id,
            commands,
            command_relationships,
            source,
            &mut spans,
        );
    }
    spans
}

fn collect_arena_processed_ls_pipeline_spans_in_stmt<'a>(
    stmt: StmtView<'a>,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(binary) = stmt.command().binary() else {
        return;
    };
    if !matches!(binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll) {
        return;
    }

    let mut segments = Vec::new();
    let mut operators = Vec::new();
    collect_arena_pipeline_parts(binary, &mut segments, &mut operators);

    for (index, pair) in segments.windows(2).enumerate() {
        let pipeline_id = command_relationships
            .child_or_lookup_arena_fact(parent_id, stmt)
            .map_or(parent_id, CommandFact::id);
        if arena_stmt_is_raw_ls(pair[0], pipeline_id, commands, command_relationships, source)
            && !arena_stmt_static_utility_name_is(
                pair[1],
                pipeline_id,
                commands,
                command_relationships,
                source,
                "grep",
            )
            && !arena_stmt_static_utility_name_is(
                pair[1],
                pipeline_id,
                commands,
                command_relationships,
                source,
                "xargs",
            )
        {
            spans.push(arena_ls_command_span_before_pipe(
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

fn collect_arena_pipeline_parts<'a>(
    command: BinaryCommandView<'a>,
    segments: &mut Vec<StmtView<'a>>,
    operators: &mut Vec<Span>,
) {
    if let Some(left) = single_arena_stmt(command.left()) {
        if let Some(left_binary) = left.command().binary()
            && matches!(left_binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
        {
            collect_arena_pipeline_parts(left_binary, segments, operators);
        } else {
            segments.push(left);
        }
    }

    operators.push(command.op_span());

    if let Some(right) = single_arena_stmt(command.right()) {
        if let Some(right_binary) = right.command().binary()
            && matches!(right_binary.op(), BinaryOp::Pipe | BinaryOp::PipeAll)
        {
            collect_arena_pipeline_parts(right_binary, segments, operators);
        } else {
            segments.push(right);
        }
    }
}

fn arena_stmt_is_raw_ls<'a>(
    stmt: StmtView<'a>,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> bool {
    if let Some(fact) = command_relationships
        .child_or_lookup_arena_fact(parent_id, stmt)
        .map(CommandFact::id)
        .and_then(|id| commands.get(id.index()))
    {
        return fact.literal_name() == Some("ls") && fact.wrappers().is_empty();
    }

    let normalized = command::normalize_arena_command(stmt.command(), source);
    normalized.literal_name.as_deref() == Some("ls") && normalized.wrappers.is_empty()
}

fn arena_stmt_static_utility_name_is<'a>(
    stmt: StmtView<'a>,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
    name: &str,
) -> bool {
    if let Some(fact) = command_relationships
        .child_or_lookup_arena_fact(parent_id, stmt)
        .map(CommandFact::id)
        .and_then(|id| commands.get(id.index()))
    {
        return fact.static_utility_name() == Some(name);
    }

    command::normalize_arena_command(stmt.command(), source).effective_or_literal_name() == Some(name)
}

fn arena_ls_command_span_before_pipe<'a>(
    stmt: StmtView<'a>,
    operator_span: Span,
    parent_id: CommandId,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    source: &str,
) -> Span {
    let start = command_relationships
        .child_or_lookup_arena_fact(parent_id, stmt)
        .map(CommandFact::id)
        .and_then(|id| commands.get(id.index()))
        .and_then(|fact| fact.shellcheck_command_span(source))
        .unwrap_or_else(|| command::normalize_arena_command(stmt.command(), source).body_span)
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

fn arena_substitution_body_contains_echo(body: StmtSeqView<'_>, source: &str) -> bool {
    let Some(stmt) = single_arena_stmt(body) else {
        return false;
    };

    if !matches!(
        stmt.command().kind(),
        ArenaFileCommandKind::Simple | ArenaFileCommandKind::Builtin | ArenaFileCommandKind::Decl
    ) {
        return false;
    }

    let normalized = command::normalize_arena_command(stmt.command(), source);
    let fallback_simple = stmt.command().simple();
    let fallback_echo = fallback_simple.is_some_and(|command| {
        arena_command_name_word_matches_source(command.name(), source, "echo")
    });
    if !normalized.effective_name_is("echo") && !fallback_echo {
        return false;
    }

    if normalized.effective_name_is("echo")
        && !normalized.body_name_word_id().is_some_and(|word_id| {
            arena_command_name_word_matches_source(
                stmt.command().store().word(word_id),
                source,
                "echo",
            )
        })
    {
        return false;
    }

    let fallback_args;
    let body_args = if normalized.effective_name_is("echo") {
        normalized.body_args()
    } else if let Some(command) = fallback_simple {
        fallback_args = command.arg_ids().to_vec();
        &fallback_args
    } else {
        return false;
    };

    if body_args.first().is_some_and(|word_id| {
        static_word_text_arena(stmt.command().store().word(*word_id), source)
            .is_some_and(|text| text.starts_with('-'))
    }) {
        return false;
    }

    if body_args.first().is_some_and(|word_id| {
        arena_word_has_leading_dynamic_dash_literal(stmt.command().store().word(*word_id), source)
    }) {
        return false;
    }

    if matches!(body_args, [word_id] if arena_word_is_command_substitution_only(stmt.command().store().word(*word_id))) {
        return false;
    }

    body_args
        .iter()
        .all(|word_id| !arena_word_contains_unquoted_glob_or_brace(stmt.command().store().word(*word_id), source))
}

fn arena_command_name_word_matches_source(word: WordView<'_>, source: &str, name: &str) -> bool {
    static_command_name_text_arena(word, source).is_some_and(|decoded| decoded == name)
        && source_span_static_command_name(word.span(), source).as_deref() == Some(name)
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

fn arena_word_is_command_substitution_only(word: WordView<'_>) -> bool {
    match word.parts() {
        [WordPartArenaNode {
            kind: WordPartArena::CommandSubstitution { .. },
            ..
        }] => true,
        [WordPartArenaNode {
            kind: WordPartArena::DoubleQuoted { parts, .. },
            ..
        }] => matches!(
            word.store().word_parts(*parts),
            [WordPartArenaNode {
                kind: WordPartArena::CommandSubstitution { .. },
                ..
            }]
        ),
        _ => false,
    }
}

fn arena_word_has_leading_dynamic_dash_literal(word: WordView<'_>, source: &str) -> bool {
    let mut saw_dynamic = false;
    arena_leading_dynamic_dash_literal_in_parts(
        word.parts(),
        word.store(),
        source,
        &mut saw_dynamic,
    )
    .unwrap_or(false)
}

fn arena_leading_dynamic_dash_literal_in_parts(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    saw_dynamic: &mut bool,
) -> Option<bool> {
    for part in parts {
        match &part.kind {
            WordPartArena::Literal(text) => {
                let text = text.as_str(source, part.span);
                if !text.is_empty() {
                    return Some(*saw_dynamic && text.starts_with('-'));
                }
            }
            WordPartArena::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                if !text.is_empty() {
                    return Some(*saw_dynamic && text.starts_with('-'));
                }
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                if let Some(result) = arena_leading_dynamic_dash_literal_in_parts(
                    store.word_parts(*parts),
                    store,
                    source,
                    saw_dynamic,
                ) {
                    return Some(result);
                }
            }
            WordPartArena::Variable(_)
            | WordPartArena::Parameter(_)
            | WordPartArena::CommandSubstitution { .. }
            | WordPartArena::ArithmeticExpansion { .. }
            | WordPartArena::ParameterExpansion { .. }
            | WordPartArena::Length(_)
            | WordPartArena::ArrayAccess(_)
            | WordPartArena::ArrayLength(_)
            | WordPartArena::ArrayIndices(_)
            | WordPartArena::Substring { .. }
            | WordPartArena::ArraySlice { .. }
            | WordPartArena::IndirectExpansion { .. }
            | WordPartArena::PrefixMatch { .. }
            | WordPartArena::ProcessSubstitution { .. }
            | WordPartArena::Transformation { .. }
            | WordPartArena::ZshQualifiedGlob(_) => {
                *saw_dynamic = true;
            }
        }
    }

    None
}

fn arena_substitution_body_contains_grep(body: StmtSeqView<'_>, source: &str) -> bool {
    let Some(stmt) = single_arena_stmt(body) else {
        return false;
    };

    arena_command_contains_grep_output(stmt.command(), source)
}

fn arena_command_contains_grep_output(command: CommandView<'_>, source: &str) -> bool {
    match command.kind() {
        ArenaFileCommandKind::Simple
        | ArenaFileCommandKind::Builtin
        | ArenaFileCommandKind::Decl => arena_command_is_grep_family(command, source),
        ArenaFileCommandKind::Binary => {
            let binary = command.binary().expect("binary command view");
            match binary.op() {
                BinaryOp::Pipe | BinaryOp::PipeAll => single_arena_stmt(binary.right())
                    .is_some_and(|stmt| arena_command_contains_grep_output(stmt.command(), source)),
                BinaryOp::And | BinaryOp::Or => false,
            }
        }
        ArenaFileCommandKind::Compound => {
            let compound = command.compound().expect("compound command view");
            match compound.node() {
                CompoundCommandNode::Subshell(body) | CompoundCommandNode::BraceGroup(body) => {
                    arena_substitution_body_contains_grep(command.store().stmt_seq(*body), source)
                }
                CompoundCommandNode::Time {
                    command: time_body,
                    ..
                } => time_body
                    .as_ref()
                    .and_then(|body| single_arena_stmt(command.store().stmt_seq(*body)))
                    .is_some_and(|stmt| arena_command_contains_grep_output(stmt.command(), source)),
                CompoundCommandNode::If { .. }
                | CompoundCommandNode::For { .. }
                | CompoundCommandNode::Repeat { .. }
                | CompoundCommandNode::Foreach { .. }
                | CompoundCommandNode::ArithmeticFor(_)
                | CompoundCommandNode::While { .. }
                | CompoundCommandNode::Until { .. }
                | CompoundCommandNode::Case { .. }
                | CompoundCommandNode::Select { .. }
                | CompoundCommandNode::Arithmetic(_)
                | CompoundCommandNode::Conditional(_)
                | CompoundCommandNode::Coproc { .. }
                | CompoundCommandNode::Always { .. } => false,
            }
        }
        ArenaFileCommandKind::Function | ArenaFileCommandKind::AnonymousFunction => false,
    }
}

fn arena_command_is_grep_family(command: CommandView<'_>, source: &str) -> bool {
    let normalized = command::normalize_arena_command(command, source);
    if normalized
        .effective_or_literal_name()
        .is_some_and(command_name_is_grep_family)
    {
        return true;
    }

    normalized.body_name_word_id().is_some_and(|word_id| {
        let word = command.store().word(word_id);
        let text = word.span().slice(source).trim_start_matches('\\');
        let name = text.rsplit('/').next().unwrap_or(text);
        command_name_is_grep_family(name)
    })
}

fn arena_word_contains_unquoted_glob_or_brace(word: WordView<'_>, source: &str) -> bool {
    arena_word_parts_contain_unquoted_glob_or_brace(word.parts(), word.store(), source, false)
}

fn arena_word_parts_contain_unquoted_glob_or_brace(
    parts: &[WordPartArenaNode],
    store: &AstStore,
    source: &str,
    in_double_quotes: bool,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPartArena::DoubleQuoted { parts, .. } => {
                if arena_word_parts_contain_unquoted_glob_or_brace(
                    store.word_parts(*parts),
                    store,
                    source,
                    true,
                ) {
                    return true;
                }
            }
            WordPartArena::Literal(text) => {
                if !in_double_quotes
                    && text
                        .as_str(source, part.span)
                        .chars()
                        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
                {
                    return true;
                }
            }
            WordPartArena::CommandSubstitution { .. }
            | WordPartArena::ProcessSubstitution { .. }
            | WordPartArena::ArithmeticExpansion { .. }
            | WordPartArena::Variable(_)
            | WordPartArena::Parameter(_)
            | WordPartArena::ParameterExpansion { .. }
            | WordPartArena::Length(_)
            | WordPartArena::ArrayAccess(_)
            | WordPartArena::ArrayLength(_)
            | WordPartArena::ArrayIndices(_)
            | WordPartArena::Substring { .. }
            | WordPartArena::ArraySlice { .. }
            | WordPartArena::IndirectExpansion { .. }
            | WordPartArena::PrefixMatch { .. }
            | WordPartArena::Transformation { .. }
            | WordPartArena::ZshQualifiedGlob(_) => {}
            WordPartArena::SingleQuoted { .. } => {}
        }
    }

    false
}

fn arena_stmt_contains_raw_ls<'a>(
    stmt: StmtView<'a>,
    parent_id: CommandId,
    command_relationships: CommandRelationshipContext<'_, 'a>,
) -> bool {
    command_relationships
        .child_or_lookup_arena_fact(parent_id, stmt)
        .is_some_and(|fact| fact.literal_name() == Some("ls") && fact.wrappers().is_empty())
        || match stmt.command().kind() {
            ArenaFileCommandKind::Binary => {
                let binary = stmt.command().binary().expect("binary command view");
                let child_parent_id = command_relationships
                    .child_or_lookup_arena_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                single_arena_stmt(binary.left())
                    .is_some_and(|left| {
                        arena_stmt_contains_raw_ls(left, child_parent_id, command_relationships)
                    })
                    || single_arena_stmt(binary.right()).is_some_and(|right| {
                        arena_stmt_contains_raw_ls(right, child_parent_id, command_relationships)
                    })
            }
            ArenaFileCommandKind::Compound => {
                let compound = stmt.command().compound().expect("compound command view");
                let child_parent_id = command_relationships
                    .child_or_lookup_arena_fact(parent_id, stmt)
                    .map_or(parent_id, CommandFact::id);
                match compound.node() {
                    CompoundCommandNode::Subshell(body) | CompoundCommandNode::BraceGroup(body) => {
                        stmt.command()
                            .store()
                            .stmt_seq(*body)
                            .stmts()
                            .any(|stmt| {
                                arena_stmt_contains_raw_ls(
                                    stmt,
                                    child_parent_id,
                                    command_relationships,
                                )
                            })
                    }
                    CompoundCommandNode::Time {
                        command: time_body,
                        ..
                    } => time_body
                        .as_ref()
                        .and_then(|body| single_arena_stmt(stmt.command().store().stmt_seq(*body)))
                        .is_some_and(|stmt| {
                            arena_stmt_contains_raw_ls(
                                stmt,
                                child_parent_id,
                                command_relationships,
                            )
                        }),
                    CompoundCommandNode::If { .. }
                    | CompoundCommandNode::For { .. }
                    | CompoundCommandNode::Repeat { .. }
                    | CompoundCommandNode::Foreach { .. }
                    | CompoundCommandNode::ArithmeticFor(_)
                    | CompoundCommandNode::While { .. }
                    | CompoundCommandNode::Until { .. }
                    | CompoundCommandNode::Case { .. }
                    | CompoundCommandNode::Select { .. }
                    | CompoundCommandNode::Arithmetic(_)
                    | CompoundCommandNode::Conditional(_)
                    | CompoundCommandNode::Coproc { .. }
                    | CompoundCommandNode::Always { .. } => false,
                }
            }
            ArenaFileCommandKind::Simple
            | ArenaFileCommandKind::Builtin
            | ArenaFileCommandKind::Decl
            | ArenaFileCommandKind::Function
            | ArenaFileCommandKind::AnonymousFunction => false,
        }
}

fn arena_substitution_body_is_simple_command_named<'a>(
    body: StmtSeqView<'a>,
    commands: CommandFacts<'_, 'a>,
    command_relationships: CommandRelationshipContext<'_, 'a>,
    name: &str,
) -> bool {
    let Some(stmt) = single_arena_stmt(body) else {
        return false;
    };

    command_relationships
        .fact_for_arena_stmt(stmt)
        .map(CommandFact::id)
        .and_then(|id| commands.get(id.index()))
        .is_some_and(|fact| fact.literal_name() == Some(name))
}

fn is_arena_bash_file_slurp_command(
    command: CommandView<'_>,
    redirects: &[RedirectNode],
    source: &str,
) -> bool {
    let Some(command) = command.simple() else {
        return false;
    };

    if !command.assignments().is_empty()
        || !command.arg_ids().is_empty()
        || !command.name().span().slice(source).is_empty()
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

fn command_name_is_grep_family(name: &str) -> bool {
    matches!(name, "grep" | "egrep" | "fgrep")
}
