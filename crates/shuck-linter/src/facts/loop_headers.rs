#[derive(Debug, Clone)]
pub struct LoopHeaderWordFact {
    word_id: WordId,
    span: Span,
    classification: WordClassification,
    has_all_elements_array_expansion: bool,
    has_unquoted_command_substitution: bool,
    has_double_quoted_scalar_only_expansion: bool,
    has_quoted_star_splat: bool,
    comparable_name_uses: Box<[ComparableNameUse]>,
    contains_line_oriented_substitution: bool,
    contains_ls_substitution: bool,
    contains_find_substitution: bool,
}

impl LoopHeaderWordFact {
    pub fn word_id(&self) -> WordId {
        self.word_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn word(&self) -> FactWordSpan {
        FactWordSpan { span: self.span }
    }

    pub fn classification(&self) -> WordClassification {
        self.classification
    }

    pub fn has_command_substitution(&self) -> bool {
        self.classification.has_command_substitution()
    }

    pub fn has_all_elements_array_expansion(&self) -> bool {
        self.has_all_elements_array_expansion
    }

    pub fn has_unquoted_command_substitution(&self) -> bool {
        self.has_unquoted_command_substitution
    }

    pub fn has_double_quoted_scalar_only_expansion(&self) -> bool {
        self.has_double_quoted_scalar_only_expansion
    }

    pub fn has_quoted_star_splat(&self) -> bool {
        self.has_quoted_star_splat
    }

    pub(crate) fn comparable_name_uses(&self) -> &[ComparableNameUse] {
        &self.comparable_name_uses
    }

    pub fn contains_line_oriented_substitution(&self) -> bool {
        self.contains_line_oriented_substitution
    }

    pub fn contains_ls_substitution(&self) -> bool {
        self.contains_ls_substitution
    }

    pub fn contains_find_substitution(&self) -> bool {
        self.contains_find_substitution
    }
}

#[derive(Debug, Clone)]
pub struct ForHeaderFact {
    span: Span,
    body_span: Span,
    target_spans: Box<[Span]>,
    command_id: CommandId,
    arena_command_id: Option<AstCommandId>,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact]>,
}

impl ForHeaderFact {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn body_span(&self) -> Span {
        self.body_span
    }

    pub fn target_spans(&self) -> &[Span] {
        &self.target_spans
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact] {
        &self.words
    }

    pub fn has_command_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::has_command_substitution)
    }

    pub fn has_find_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::contains_find_substitution)
    }
}

#[derive(Debug, Clone)]
pub struct SelectHeaderFact {
    span: Span,
    body_span: Span,
    variable_span: Span,
    command_id: CommandId,
    arena_command_id: Option<AstCommandId>,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact]>,
}

impl SelectHeaderFact {
    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn arena_command_id(&self) -> Option<AstCommandId> {
        self.arena_command_id
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn body_span(&self) -> Span {
        self.body_span
    }

    pub fn variable_span(&self) -> Span {
        self.variable_span
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact] {
        &self.words
    }

    pub fn has_command_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::has_command_substitution)
    }

    pub fn has_find_substitution(&self) -> bool {
        self.words
            .iter()
            .any(LoopHeaderWordFact::contains_find_substitution)
    }
}

pub(super) fn build_for_header_facts<'a>(
    commands: &[CommandFact<'a>],
    arena_file: &ArenaFile,
    _command_ids_by_span: &CommandLookupIndex,
    _arena_word_ids_by_span: &FxHashMap<FactSpan, WordId>,
    source: &str,
) -> Vec<ForHeaderFact> {
    commands
        .iter()
        .filter_map(|fact| {
            let command = arena_file
                .store
                .command(fact.arena_command_id()?)
                .compound()?;
            let CompoundCommandNode::For {
                targets,
                words,
                body,
                ..
            } = command.node()
            else {
                return None;
            };
            let header_words = words
                .map(|range| arena_file.store.word_ids(range))
                .unwrap_or(&[]);
            Some(ForHeaderFact {
                span: fact.span(),
                body_span: arena_file.store.stmt_seq(*body).span(),
                target_spans: arena_file
                    .store
                    .for_targets(*targets)
                    .iter()
                    .map(|target| target.span)
                    .collect(),
                command_id: fact.id(),
                arena_command_id: fact.arena_command_id(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_arena_loop_header_word_facts(
                    header_words,
                    commands,
                    arena_file,
                    source,
                ),
            })
        })
        .collect()
}

pub(super) fn build_select_header_facts<'a>(
    commands: &[CommandFact<'a>],
    arena_file: &ArenaFile,
    _command_ids_by_span: &CommandLookupIndex,
    _arena_word_ids_by_span: &FxHashMap<FactSpan, WordId>,
    source: &str,
) -> Vec<SelectHeaderFact> {
    commands
        .iter()
        .filter_map(|fact| {
            let command = arena_file
                .store
                .command(fact.arena_command_id()?)
                .compound()?;
            let CompoundCommandNode::Select {
                variable_span,
                words,
                body,
                ..
            } = command.node()
            else {
                return None;
            };
            Some(SelectHeaderFact {
                span: fact.span(),
                body_span: arena_file.store.stmt_seq(*body).span(),
                variable_span: *variable_span,
                command_id: fact.id(),
                arena_command_id: fact.arena_command_id(),
                nested_word_command: fact.is_nested_word_command(),
                words: build_arena_loop_header_word_facts(
                    arena_file.store.word_ids(*words),
                    commands,
                    arena_file,
                    source,
                ),
            })
        })
        .collect()
}

fn build_arena_loop_header_word_facts(
    word_ids: &[WordId],
    commands: &[CommandFact<'_>],
    arena_file: &ArenaFile,
    source: &str,
) -> Box<[LoopHeaderWordFact]> {
    word_ids
        .iter()
        .copied()
        .map(|word_id| {
            let word = arena_file.store.word(word_id);
            let classification = classify_arena_loop_header_word(word);
            LoopHeaderWordFact {
                word_id,
                span: word.span(),
                classification,
                has_all_elements_array_expansion: arena_word_has_all_elements_array_expansion(
                    word, source,
                ),
                has_unquoted_command_substitution: classification.has_command_substitution()
                    && arena_word_has_unquoted_command_substitution(word),
                has_double_quoted_scalar_only_expansion:
                    arena_word_has_double_quoted_scalar_only_expansion(word, source),
                has_quoted_star_splat: arena_word_has_quoted_star_splat(word, source),
                comparable_name_uses: Box::new([]),
                contains_line_oriented_substitution: arena_word_contains_line_oriented_substitution(
                    word,
                    commands,
                    source,
                ),
                contains_ls_substitution: arena_word_contains_command_substitution_named(
                    word, "ls", commands,
                ),
                contains_find_substitution: arena_word_contains_find_substitution(word, commands),
            }
        })
        .collect()
}

fn classify_arena_loop_header_word(word: WordView<'_>) -> WordClassification {
    let mut has_substitution = false;
    let mut has_expansion = false;
    let mut has_double_quote = false;
    classify_arena_loop_header_parts(
        word.store(),
        word.parts(),
        false,
        &mut has_substitution,
        &mut has_expansion,
        &mut has_double_quote,
    );
    WordClassification {
        quote: if arena_loop_header_word_is_fully_quoted(word) {
            WordQuote::FullyQuoted
        } else if has_double_quote {
            WordQuote::Mixed
        } else {
            WordQuote::Unquoted
        },
        literalness: if has_expansion || has_substitution {
            WordLiteralness::Expanded
        } else {
            WordLiteralness::FixedLiteral
        },
        expansion_kind: if has_expansion {
            WordExpansionKind::Scalar
        } else {
            WordExpansionKind::None
        },
        substitution_shape: if has_substitution {
            WordSubstitutionShape::Mixed
        } else {
            WordSubstitutionShape::None
        },
    }
}

fn arena_loop_header_word_is_fully_quoted(word: WordView<'_>) -> bool {
    matches!(
        word.parts(),
        [part]
            if matches!(
                part.kind,
                WordPartArena::SingleQuoted { .. } | WordPartArena::DoubleQuoted { .. }
            )
    )
}

fn classify_arena_loop_header_parts(
    store: &AstStore,
    parts: &[WordPartArenaNode],
    quoted: bool,
    has_substitution: &mut bool,
    has_expansion: &mut bool,
    has_double_quote: &mut bool,
) {
    for part in parts {
        match &part.kind {
            WordPartArena::CommandSubstitution { .. }
            | WordPartArena::ProcessSubstitution { .. } => *has_substitution = true,
            WordPartArena::DoubleQuoted { parts, .. } => {
                *has_double_quote = true;
                classify_arena_loop_header_parts(
                    store,
                    store.word_parts(*parts),
                    true,
                    has_substitution,
                    has_expansion,
                    has_double_quote,
                );
            }
            WordPartArena::Variable(_)
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
            | WordPartArena::ArithmeticExpansion { .. } => *has_expansion = true,
            WordPartArena::Literal(_)
            | WordPartArena::SingleQuoted { .. }
            | WordPartArena::ZshQualifiedGlob(_) => {
                let _ = quoted;
            }
        }
    }
}

fn arena_word_has_all_elements_array_expansion(word: WordView<'_>, source: &str) -> bool {
    arena_word_parts_any(word.store(), word.parts(), |part| {
        arena_word_part_uses_all_elements_array_expansion(part, source)
    })
}

fn arena_word_part_uses_all_elements_array_expansion(
    part: &WordPartArena,
    source: &str,
) -> bool {
    match part {
        WordPartArena::Variable(name) => name.as_str() == "@",
        WordPartArena::ArrayAccess(reference)
        | WordPartArena::ArrayIndices(reference)
        | WordPartArena::ArraySlice { reference, .. } => {
            arena_var_ref_uses_all_elements_at_splat(reference)
        }
        WordPartArena::PrefixMatch {
            kind: PrefixMatchKind::At,
            ..
        } => true,
        WordPartArena::Parameter(parameter) => {
            arena_parameter_might_use_all_elements_array_expansion(parameter, source)
        }
        WordPartArena::ParameterExpansion {
            reference,
            operator,
            ..
        }
        | WordPartArena::IndirectExpansion {
            reference,
            operator: Some(operator),
            ..
        } => {
            !matches!(operator, ParameterOp::UseReplacement)
                && arena_var_ref_uses_all_elements_at_splat(reference)
        }
        WordPartArena::Transformation { reference, .. } => {
            arena_var_ref_uses_all_elements_at_splat(reference)
        }
        WordPartArena::Literal(_)
        | WordPartArena::SingleQuoted { .. }
        | WordPartArena::DoubleQuoted { .. }
        | WordPartArena::CommandSubstitution { .. }
        | WordPartArena::ArithmeticExpansion { .. }
        | WordPartArena::Length(_)
        | WordPartArena::ArrayLength(_)
        | WordPartArena::Substring { .. }
        | WordPartArena::IndirectExpansion { .. }
        | WordPartArena::ProcessSubstitution { .. }
        | WordPartArena::PrefixMatch {
            kind: PrefixMatchKind::Star,
            ..
        }
        | WordPartArena::ZshQualifiedGlob(_) => false,
    }
}

fn arena_var_ref_uses_all_elements_at_splat(reference: &VarRefNode) -> bool {
    reference.name.as_str() == "@"
        || matches!(
            reference
                .subscript
                .as_deref()
                .map(|subscript| &subscript.kind),
            Some(shuck_ast::SubscriptKind::Selector(SubscriptSelector::At))
        )
}

fn arena_parameter_might_use_all_elements_array_expansion(
    parameter: &ParameterExpansionNode,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntaxNode::Bourne(syntax) => match syntax {
            BourneParameterExpansionNode::Access { reference }
            | BourneParameterExpansionNode::Indices { reference }
            | BourneParameterExpansionNode::Slice { reference, .. }
            | BourneParameterExpansionNode::Transformation { reference, .. } => {
                arena_var_ref_uses_all_elements_at_splat(reference)
            }
            BourneParameterExpansionNode::Operation {
                reference,
                operator,
                ..
            } => {
                !matches!(operator, ParameterOp::UseReplacement)
                    && arena_var_ref_uses_all_elements_at_splat(reference)
            }
            BourneParameterExpansionNode::Length { .. }
            | BourneParameterExpansionNode::Indirect { .. } => false,
            BourneParameterExpansionNode::PrefixMatch { kind, .. } => {
                matches!(kind, PrefixMatchKind::At)
            }
        },
        ParameterExpansionSyntaxNode::Zsh(_) => parameter.raw_body.slice(source).contains('@'),
    }
}

fn arena_word_has_unquoted_command_substitution(word: WordView<'_>) -> bool {
    fn parts_any(store: &AstStore, parts: &[WordPartArenaNode], quoted: bool) -> bool {
        parts.iter().any(|part| match &part.kind {
            WordPartArena::CommandSubstitution { .. } | WordPartArena::ProcessSubstitution { .. } => {
                !quoted
            }
            WordPartArena::DoubleQuoted { parts, .. } => parts_any(store, store.word_parts(*parts), true),
            _ => false,
        })
    }
    parts_any(word.store(), word.parts(), false)
}

fn arena_word_has_double_quoted_scalar_only_expansion(word: WordView<'_>, source: &str) -> bool {
    if arena_word_parts_any(word.store(), word.parts(), |part| {
        arena_word_part_is_shell_quoting_transform(part, source)
    }) {
        return false;
    }

    arena_word_parts_any(word.store(), word.parts(), |part| {
        matches!(part, WordPartArena::DoubleQuoted { .. })
    })
}

fn arena_word_part_is_shell_quoting_transform(part: &WordPartArena, source: &str) -> bool {
    match part {
        WordPartArena::Transformation { operator: 'Q', .. } => true,
        WordPartArena::Parameter(parameter) => {
            matches!(
                &parameter.syntax,
                ParameterExpansionSyntaxNode::Bourne(
                    BourneParameterExpansionNode::Transformation { operator: 'Q', .. }
                )
            ) || parameter.raw_body.slice(source).ends_with("@Q")
        }
        _ => false,
    }
}

fn arena_word_has_quoted_star_splat(word: WordView<'_>, source: &str) -> bool {
    word.span().slice(source).contains("\"$*\"") || word.span().slice(source).contains("\"${*}\"")
}

fn arena_word_contains_find_substitution(word: WordView<'_>, commands: &[CommandFact<'_>]) -> bool {
    fn visit(store: &AstStore, parts: &[WordPartArenaNode], commands: &[CommandFact<'_>]) -> bool {
        parts.iter().any(|part| match &part.kind {
            WordPartArena::CommandSubstitution { body, .. }
            | WordPartArena::ProcessSubstitution { body, .. } => {
                arena_substitution_body_is_find(store.stmt_seq(*body), commands)
            }
            WordPartArena::DoubleQuoted { parts, .. } => visit(store, store.word_parts(*parts), commands),
            _ => false,
        })
    }
    visit(word.store(), word.parts(), commands)
}

fn arena_word_contains_line_oriented_substitution(
    word: WordView<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
) -> bool {
    fn visit(store: &AstStore, parts: &[WordPartArenaNode], commands: &[CommandFact<'_>], source: &str) -> bool {
        parts.iter().any(|part| match &part.kind {
            WordPartArena::CommandSubstitution { body, .. } => {
                arena_substitution_body_is_line_oriented(store.stmt_seq(*body), commands, source)
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                visit(store, store.word_parts(*parts), commands, source)
            }
            _ => false,
        })
    }
    visit(word.store(), word.parts(), commands, source)
}

fn arena_word_contains_command_substitution_named(
    word: WordView<'_>,
    name: &str,
    commands: &[CommandFact<'_>],
) -> bool {
    fn visit(
        store: &AstStore,
        parts: &[WordPartArenaNode],
        name: &str,
        commands: &[CommandFact<'_>],
    ) -> bool {
        parts.iter().any(|part| match &part.kind {
            WordPartArena::CommandSubstitution { body, .. }
            | WordPartArena::ProcessSubstitution { body, .. } => {
                arena_loop_substitution_body_is_simple_command_named(
                    store.stmt_seq(*body),
                    name,
                    commands,
                )
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                visit(store, store.word_parts(*parts), name, commands)
            }
            _ => false,
        })
    }
    visit(word.store(), word.parts(), name, commands)
}

fn arena_substitution_body_is_find(body: StmtSeqView<'_>, commands: &[CommandFact<'_>]) -> bool {
    matches!(single_arena_stmt(body), Some(stmt) if arena_stmt_invokes_find(stmt, commands))
}

fn arena_substitution_body_is_line_oriented(
    body: StmtSeqView<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
) -> bool {
    matches!(
        single_arena_stmt(body),
        Some(stmt)
            if arena_command_is_line_oriented_substitution_body(
                stmt.command(),
                commands,
                source,
            )
    )
}

fn arena_loop_substitution_body_is_simple_command_named(
    body: StmtSeqView<'_>,
    name: &str,
    commands: &[CommandFact<'_>],
) -> bool {
    matches!(
        single_arena_stmt(body),
        Some(stmt) if arena_command_fact_for_stmt(stmt, commands).and_then(CommandFact::literal_name) == Some(name)
    )
}

#[allow(clippy::only_used_in_recursion)]
fn arena_command_is_line_oriented_substitution_body(
    command: CommandView<'_>,
    commands: &[CommandFact<'_>],
    source: &str,
) -> bool {
    match command.kind() {
        ArenaFileCommandKind::Simple
        | ArenaFileCommandKind::Builtin
        | ArenaFileCommandKind::Decl => arena_command_fact_for_command(command, commands)
            .is_some_and(arena_command_fact_is_line_oriented_utility),
        ArenaFileCommandKind::Binary => {
            let binary = command.binary().expect("binary command view");
            match binary.op() {
                BinaryOp::Pipe | BinaryOp::PipeAll => {
                    single_arena_stmt(binary.left()).is_some_and(|left| {
                        arena_command_is_line_oriented_substitution_body(
                            left.command(),
                            commands,
                            source,
                        )
                    }) && single_arena_stmt(binary.right()).is_some_and(|right| {
                        arena_command_is_line_oriented_substitution_body(
                            right.command(),
                            commands,
                            source,
                        )
                    })
                }
                BinaryOp::And | BinaryOp::Or => false,
            }
        }
        ArenaFileCommandKind::Compound => {
            let compound = command.compound().expect("compound command view");
            match compound.node() {
                CompoundCommandNode::Time { command: body, .. } => body
                    .as_ref()
                    .and_then(|body| single_arena_stmt(command.store().stmt_seq(*body)))
                    .is_some_and(|stmt| {
                        arena_command_is_line_oriented_substitution_body(
                            stmt.command(),
                            commands,
                            source,
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
                | CompoundCommandNode::Subshell(_)
                | CompoundCommandNode::BraceGroup(_)
                | CompoundCommandNode::Coproc { .. }
                | CompoundCommandNode::Always { .. } => false,
            }
        }
        ArenaFileCommandKind::Function | ArenaFileCommandKind::AnonymousFunction => false,
    }
}

fn arena_command_fact_is_line_oriented_utility(fact: &CommandFact<'_>) -> bool {
    if arena_command_fact_invokes_find(fact) {
        return false;
    }

    fact.effective_or_literal_name().is_some_and(|name| {
        matches!(
            name.rsplit('/').next().unwrap_or(name),
            "cat" | "grep" | "egrep" | "fgrep" | "awk" | "sed" | "cut" | "sort"
        )
    })
}

fn arena_stmt_invokes_find(stmt: StmtView<'_>, commands: &[CommandFact<'_>]) -> bool {
    arena_command_fact_for_stmt(stmt, commands).is_some_and(arena_command_fact_invokes_find)
}

fn arena_command_fact_invokes_find(fact: &CommandFact<'_>) -> bool {
    command_name_matches_basename(fact.literal_name(), "find")
        || command_name_matches_basename(fact.effective_name(), "find")
        || fact.has_wrapper(WrapperKind::FindExec)
        || fact.has_wrapper(WrapperKind::FindExecDir)
}

fn command_name_matches_basename(name: Option<&str>, expected: &str) -> bool {
    name.is_some_and(|name| name == expected || name.rsplit('/').next() == Some(expected))
}

fn arena_command_fact_for_stmt<'a>(
    stmt: StmtView<'_>,
    commands: &'a [CommandFact<'_>],
) -> Option<&'a CommandFact<'a>> {
    arena_command_fact_for_command(stmt.command(), commands)
}

fn arena_command_fact_for_command<'a>(
    command: CommandView<'_>,
    commands: &'a [CommandFact<'_>],
) -> Option<&'a CommandFact<'a>> {
    let id = command.id();
    commands
        .iter()
        .find(|fact| fact.arena_command_id().is_some_and(|candidate| candidate.index() == id.index()))
}

fn arena_word_parts_any(
    store: &AstStore,
    parts: &[WordPartArenaNode],
    pred: impl Fn(&WordPartArena) -> bool + Copy,
) -> bool {
    parts.iter().any(|part| {
        pred(&part.kind)
            || matches!(&part.kind, WordPartArena::DoubleQuoted { parts, .. } if arena_word_parts_any(store, store.word_parts(*parts), pred))
    })
}
