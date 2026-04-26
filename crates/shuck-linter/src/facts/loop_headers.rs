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
                words: build_arena_loop_header_word_facts(header_words, arena_file, source),
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
                    arena_file,
                    source,
                ),
            })
        })
        .collect()
}

fn build_arena_loop_header_word_facts(
    word_ids: &[WordId],
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
                    arena_word_has_double_quoted_scalar_only_expansion(word),
                has_quoted_star_splat: arena_word_has_quoted_star_splat(word, source),
                comparable_name_uses: Box::new([]),
                contains_line_oriented_substitution: arena_word_contains_command_name(
                    word,
                    &["cat", "grep", "sed", "awk", "cut", "sort", "uniq"],
                    source,
                ),
                contains_ls_substitution: arena_word_contains_command_name(word, &["ls"], source),
                contains_find_substitution: arena_word_contains_command_name(
                    word,
                    &["find"],
                    source,
                ),
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
        quote: if has_double_quote {
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
        matches!(
            part,
            WordPartArena::ArrayAccess(reference)
                | WordPartArena::ArrayLength(reference)
                | WordPartArena::ArrayIndices(reference)
                if reference
                    .subscript
                    .as_deref()
                    .is_some_and(|subscript| matches!(subscript.text.slice(source), "@" | "*"))
        )
    })
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

fn arena_word_has_double_quoted_scalar_only_expansion(word: WordView<'_>) -> bool {
    arena_word_parts_any(word.store(), word.parts(), |part| {
        matches!(part, WordPartArena::DoubleQuoted { .. })
    })
}

fn arena_word_has_quoted_star_splat(word: WordView<'_>, source: &str) -> bool {
    word.span().slice(source).contains("\"$*\"") || word.span().slice(source).contains("\"${*}\"")
}

fn arena_word_contains_command_name(word: WordView<'_>, names: &[&str], source: &str) -> bool {
    fn visit(store: &AstStore, parts: &[WordPartArenaNode], names: &[&str], source: &str) -> bool {
        parts.iter().any(|part| match &part.kind {
            WordPartArena::CommandSubstitution { body, .. }
            | WordPartArena::ProcessSubstitution { body, .. } => store
                .stmt_seq(*body)
                .stmts()
                .any(|stmt| stmt.command().words().next().and_then(|word| {
                    static_word_text_arena(word, source).map(|text| text.into_owned())
                }).is_some_and(|text| names.contains(&text.as_str()))),
            WordPartArena::DoubleQuoted { parts, .. } => {
                visit(store, store.word_parts(*parts), names, source)
            }
            _ => false,
        })
    }
    visit(word.store(), word.parts(), names, source)
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


fn build_loop_header_word_facts<'a>(
    words: impl IntoIterator<Item = &'a Word>,
    commands: &[CommandFact<'a>],
    command_ids_by_span: &CommandLookupIndex,
    arena_word_ids_by_span: &FxHashMap<FactSpan, WordId>,
    source: &str,
) -> Box<[LoopHeaderWordFact]> {
    words
        .into_iter()
        .map(|word| {
            let classification = classify_word(word, source);
            LoopHeaderWordFact {
                word_id: arena_word_ids_by_span
                    .get(&FactSpan::new(word.span))
                    .copied()
                    .unwrap_or_else(|| panic!("arena word id missing for loop header {:?}", word.span)),
                span: word.span,
                classification,
                has_all_elements_array_expansion:
                    word_spans::word_has_all_elements_array_expansion_syntax(word)
                        || !word_spans::all_elements_array_expansion_part_spans(word, source)
                            .is_empty(),
                has_unquoted_command_substitution: classification.has_command_substitution()
                    && !word_spans::unquoted_command_substitution_part_spans(word).is_empty(),
                has_double_quoted_scalar_only_expansion:
                    !word_spans::word_double_quoted_scalar_only_expansion_spans(word).is_empty(),
                has_quoted_star_splat: !word_spans::word_quoted_star_splat_spans(word).is_empty(),
                comparable_name_uses: comparable_name_uses(word, source),
                contains_line_oriented_substitution: word_contains_line_oriented_substitution(
                    word,
                    commands,
                    command_ids_by_span,
                ),
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
