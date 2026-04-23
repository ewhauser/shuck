#[derive(Debug, Clone, Copy)]
pub struct LoopHeaderWordFact<'a> {
    word: &'a Word,
    classification: WordClassification,
    has_all_elements_array_expansion: bool,
    has_unquoted_command_substitution: bool,
    contains_line_oriented_substitution: bool,
    contains_ls_substitution: bool,
    contains_find_substitution: bool,
}

impl<'a> LoopHeaderWordFact<'a> {
    pub fn word(&self) -> &'a Word {
        self.word
    }

    pub fn span(&self) -> Span {
        self.word.span
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
pub struct ForHeaderFact<'a> {
    command: &'a ForCommand,
    command_id: CommandId,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> ForHeaderFact<'a> {
    pub fn command(&self) -> &'a ForCommand {
        self.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact<'a>] {
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
pub struct SelectHeaderFact<'a> {
    command: &'a SelectCommand,
    command_id: CommandId,
    nested_word_command: bool,
    words: Box<[LoopHeaderWordFact<'a>]>,
}

impl<'a> SelectHeaderFact<'a> {
    pub fn command(&self) -> &'a SelectCommand {
        self.command
    }

    pub fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub fn span(&self) -> Span {
        self.command.span
    }

    pub fn is_nested_word_command(&self) -> bool {
        self.nested_word_command
    }

    pub fn words(&self) -> &[LoopHeaderWordFact<'a>] {
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
                has_all_elements_array_expansion:
                    word_spans::word_has_all_elements_array_expansion_syntax(word)
                        || !word_spans::all_elements_array_expansion_part_spans(word, source)
                            .is_empty(),
                has_unquoted_command_substitution: classification.has_command_substitution()
                    && !word_spans::unquoted_command_substitution_part_spans(word).is_empty(),
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
