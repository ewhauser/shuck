#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactContext {
    Expansion(ExpansionContext),
    CaseSubject,
    ArithmeticCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactHostKind {
    Direct,
    AssignmentTargetSubscript,
    DeclarationNameSubscript,
    ArrayKeySubscript,
    ConditionalVarRefSubscript,
}

#[derive(Debug)]
pub struct WordNode<'a> {
    key: FactSpan,
    word: &'a Word,
    analysis: ExpansionAnalysis,
    derived: OnceCell<WordNodeDerived>,
}

#[derive(Debug)]
pub(crate) struct WordNodeDerived {
    static_text: Option<Box<str>>,
    starts_with_extglob: bool,
    has_literal_affixes: bool,
    contains_shell_quoting_literals: bool,
    active_expansion_spans: Box<[Span]>,
    scalar_expansion_spans: Box<[Span]>,
    unquoted_scalar_expansion_spans: Box<[Span]>,
    array_expansion_spans: Box<[Span]>,
    all_elements_array_expansion_spans: Box<[Span]>,
    direct_all_elements_array_expansion_spans: Box<[Span]>,
    unquoted_all_elements_array_expansion_spans: Box<[Span]>,
    unquoted_array_expansion_spans: Box<[Span]>,
    command_substitution_spans: Box<[Span]>,
    unquoted_command_substitution_spans: Box<[Span]>,
    double_quoted_expansion_spans: Box<[Span]>,
    unquoted_literal_between_double_quoted_segments_spans: Box<[Span]>,
}

#[derive(Debug)]
pub struct WordOccurrence {
    node_id: WordNodeId,
    command_id: CommandId,
    nested_word_command: bool,
    context: WordFactContext,
    host_kind: WordFactHostKind,
    runtime_literal: RuntimeLiteralAnalysis,
    operand_class: Option<TestOperandClass>,
    array_assignment_split_scalar_expansion_spans: OnceCell<Box<[Span]>>,
}

#[derive(Clone, Copy)]
pub struct WordOccurrenceRef<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
    id: WordOccurrenceId,
}

pub struct WordOccurrenceIter<'facts, 'a> {
    inner: Box<dyn Iterator<Item = WordOccurrenceRef<'facts, 'a>> + 'facts>,
}

impl<'facts, 'a> WordOccurrenceIter<'facts, 'a> {
    pub fn iter(self) -> Self {
        self
    }
}

impl<'facts, 'a> Iterator for WordOccurrenceIter<'facts, 'a> {
    type Item = WordOccurrenceRef<'facts, 'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'facts, 'a> WordOccurrenceRef<'facts, 'a> {
    fn occurrence(self) -> &'facts WordOccurrence {
        self.facts.word_occurrence(self.id)
    }

    fn node(self) -> &'facts WordNode<'a> {
        self.facts.word_node(self.occurrence().node_id)
    }

    fn derived(self) -> &'facts WordNodeDerived {
        self.facts.word_node_derived(self.occurrence().node_id)
    }

    fn word(self) -> &'a Word {
        self.node().word
    }

    pub fn key(self) -> FactSpan {
        self.node().key
    }

    pub fn span(self) -> Span {
        self.word().span
    }

    pub fn command_id(self) -> CommandId {
        self.occurrence().command_id
    }

    pub fn is_nested_word_command(self) -> bool {
        self.occurrence().nested_word_command
    }

    pub fn context(self) -> WordFactContext {
        self.occurrence().context
    }

    pub fn expansion_context(self) -> Option<ExpansionContext> {
        match self.context() {
            WordFactContext::Expansion(context) => Some(context),
            WordFactContext::CaseSubject => None,
            WordFactContext::ArithmeticCommand => None,
        }
    }

    pub fn is_case_subject(self) -> bool {
        self.context() == WordFactContext::CaseSubject
    }

    pub fn is_arithmetic_command(self) -> bool {
        self.context() == WordFactContext::ArithmeticCommand
    }

    pub fn host_kind(self) -> WordFactHostKind {
        self.occurrence().host_kind
    }

    pub fn analysis(self) -> ExpansionAnalysis {
        self.node().analysis
    }

    pub fn runtime_literal(self) -> RuntimeLiteralAnalysis {
        self.occurrence().runtime_literal
    }

    pub fn classification(self) -> WordClassification {
        word_classification_from_analysis(self.analysis())
    }

    pub fn operand_class(self) -> Option<TestOperandClass> {
        self.occurrence().operand_class
    }

    pub fn static_text(self) -> Option<&'facts str> {
        self.derived().static_text.as_deref()
    }

    pub fn is_plain_scalar_reference(self) -> bool {
        word_is_plain_scalar_reference(self.word())
    }

    pub fn is_direct_numeric_expansion(self) -> bool {
        word_is_direct_numeric_expansion(self.word())
    }

    pub fn starts_with_extglob(self) -> bool {
        self.derived().starts_with_extglob
    }

    pub fn has_literal_affixes(self) -> bool {
        self.derived().has_literal_affixes
    }

    pub fn contains_shell_quoting_literals(self) -> bool {
        self.derived().contains_shell_quoting_literals
    }

    pub fn active_expansion_spans(self) -> &'facts [Span] {
        &self.derived().active_expansion_spans
    }

    pub fn scalar_expansion_spans(self) -> &'facts [Span] {
        &self.derived().scalar_expansion_spans
    }

    pub fn unquoted_scalar_expansion_spans(self) -> &'facts [Span] {
        &self.derived().unquoted_scalar_expansion_spans
    }

    pub fn array_assignment_split_scalar_expansion_spans(self) -> &'facts [Span] {
        self.occurrence()
            .array_assignment_split_scalar_expansion_spans
            .get_or_init(|| self.facts.compute_array_assignment_split_scalar_expansion_spans(self.id))
            .as_ref()
    }

    pub fn array_expansion_spans(self) -> &'facts [Span] {
        &self.derived().array_expansion_spans
    }

    pub fn all_elements_array_expansion_spans(self) -> &'facts [Span] {
        &self.derived().all_elements_array_expansion_spans
    }

    pub fn direct_all_elements_array_expansion_spans(self) -> &'facts [Span] {
        &self.derived().direct_all_elements_array_expansion_spans
    }

    pub fn unquoted_all_elements_array_expansion_spans(self) -> &'facts [Span] {
        &self.derived().unquoted_all_elements_array_expansion_spans
    }

    pub fn unquoted_array_expansion_spans(self) -> &'facts [Span] {
        &self.derived().unquoted_array_expansion_spans
    }

    pub fn command_substitution_spans(self) -> &'facts [Span] {
        &self.derived().command_substitution_spans
    }

    pub fn unquoted_command_substitution_spans(self) -> &'facts [Span] {
        &self.derived().unquoted_command_substitution_spans
    }

    pub fn double_quoted_expansion_spans(self) -> &'facts [Span] {
        &self.derived().double_quoted_expansion_spans
    }

    pub fn unquoted_literal_between_double_quoted_segments_spans(self) -> &'facts [Span] {
        &self
            .derived()
            .unquoted_literal_between_double_quoted_segments_spans
    }

    pub fn has_single_part(self) -> bool {
        self.word().parts.len() == 1
    }

    pub fn parts_len(self) -> usize {
        self.word().parts.len()
    }

    pub fn parts_with_spans(self) -> impl Iterator<Item = (&'a WordPart, Span)> + 'a {
        self.word().parts_with_spans()
    }

    pub fn has_direct_all_elements_array_expansion_in_source(self, source: &str) -> bool {
        crate::word_has_direct_all_elements_array_expansion_in_source(self.word(), source)
    }

    pub fn has_quoted_all_elements_array_slice(self) -> bool {
        crate::word_has_quoted_all_elements_array_slice(self.word())
    }

    pub fn double_quoted_scalar_affix_span(self) -> Option<Span> {
        crate::double_quoted_scalar_affix_span(self.word())
    }

    pub fn is_pure_positional_at_splat(self) -> bool {
        crate::word_is_pure_positional_at_splat(self.word())
    }

    pub fn quoted_unindexed_bash_source_span_in_source(self, source: &str) -> Option<Span> {
        crate::word_quoted_unindexed_bash_source_span_in_source(self.word(), source)
    }

    pub fn unquoted_glob_pattern_spans(self, source: &str) -> Vec<Span> {
        crate::word_unquoted_glob_pattern_spans(self.word(), source)
    }

    pub fn unquoted_glob_pattern_spans_outside_brace_expansion(self, source: &str) -> Vec<Span> {
        crate::word_unquoted_glob_pattern_spans_outside_brace_expansion(self.word(), source)
    }

    pub fn suspicious_bracket_glob_spans(self, source: &str) -> Vec<Span> {
        crate::word_suspicious_bracket_glob_spans(self.word(), source)
    }

    pub fn standalone_literal_backslash_span(self, source: &str) -> Option<Span> {
        crate::word_standalone_literal_backslash_span(self.word(), source)
    }

    pub fn unquoted_assign_default_spans(self) -> Vec<Span> {
        crate::word_unquoted_assign_default_spans(self.word())
    }

    pub fn use_replacement_spans(self) -> Vec<Span> {
        crate::word_use_replacement_spans(self.word())
    }

    pub fn unquoted_star_parameter_spans(self) -> Vec<Span> {
        crate::word_unquoted_star_parameter_spans(self.word(), self.unquoted_array_expansion_spans())
    }

    pub fn unquoted_star_splat_spans(self) -> Vec<Span> {
        crate::word_unquoted_star_splat_spans(self.word())
    }

    pub fn unquoted_word_after_single_quoted_segment_spans(self, source: &str) -> Vec<Span> {
        crate::word_unquoted_word_after_single_quoted_segment_spans(self.word(), source)
    }

    pub fn unquoted_scalar_between_double_quoted_segments_spans(
        self,
        candidate_spans: &[Span],
    ) -> Vec<Span> {
        crate::word_unquoted_scalar_between_double_quoted_segments_spans(
            self.word(),
            candidate_spans,
        )
    }

    pub fn nested_dynamic_double_quote_spans(self) -> Vec<Span> {
        crate::word_nested_dynamic_double_quote_spans(self.word())
    }

    pub fn folded_positional_at_splat_span_in_source(self, source: &str) -> Option<Span> {
        crate::word_folded_positional_at_splat_span_in_source(self.word(), source)
    }

    pub fn zsh_flag_modifier_spans(self) -> Vec<Span> {
        crate::word_zsh_flag_modifier_spans(self.word())
    }

    pub fn zsh_nested_expansion_spans(self) -> Vec<Span> {
        crate::word_zsh_nested_expansion_spans(self.word())
    }

    pub fn nested_zsh_substitution_spans(self) -> Vec<Span> {
        crate::word_nested_zsh_substitution_spans(self.word())
    }

    pub fn brace_expansion_spans(self) -> Vec<Span> {
        self.word()
            .brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.expands())
            .map(|brace| brace.span)
            .collect()
    }
}


fn build_brace_variable_before_bracket_spans<'a>(
    nodes: &[WordNode<'a>],
    occurrences: &[WordOccurrence],
    source: &str,
) -> Vec<Span> {
    let mut spans = occurrences
        .iter()
        .filter(|fact| fact.host_kind == WordFactHostKind::Direct)
        .filter(|fact| fact.context != WordFactContext::ArithmeticCommand)
        .flat_map(|fact| {
            word_unbraced_variable_before_bracket_spans(occurrence_word(nodes, fact), source)
        })
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

pub(crate) fn occurrence_word<'a>(
    nodes: &[WordNode<'a>],
    occurrence: &WordOccurrence,
) -> &'a Word {
    nodes[occurrence.node_id.index()].word
}

pub(crate) fn occurrence_key(nodes: &[WordNode<'_>], occurrence: &WordOccurrence) -> FactSpan {
    nodes[occurrence.node_id.index()].key
}

pub(crate) fn occurrence_span(nodes: &[WordNode<'_>], occurrence: &WordOccurrence) -> Span {
    occurrence_word(nodes, occurrence).span
}

pub(crate) fn occurrence_analysis(
    nodes: &[WordNode<'_>],
    occurrence: &WordOccurrence,
) -> ExpansionAnalysis {
    nodes[occurrence.node_id.index()].analysis
}

pub(crate) fn word_node_derived<'a>(node: &'a WordNode<'_>, source: &str) -> &'a WordNodeDerived {
    node.derived
        .get_or_init(|| derive_word_fact_data(node.word, source))
}

fn word_is_plain_scalar_reference(word: &Word) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_plain_scalar_reference(&part.kind)
}

fn word_part_is_plain_scalar_reference(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => !matches!(name.as_str(), "@" | "*"),
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_plain_scalar_reference(&part.kind)
        }
        WordPart::Parameter(parameter) => parameter_is_plain_scalar_reference(parameter),
        _ => false,
    }
}

fn parameter_is_plain_scalar_reference(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.subscript.is_none() && !matches!(reference.name.as_str(), "@" | "*") =>
        {
            true
        }
        ParameterExpansionSyntax::Zsh(syntax)
            if syntax.length_prefix.is_none()
                && syntax.operation.is_none()
                && syntax.modifiers.is_empty()
                && matches!(
                    &syntax.target,
                    ZshExpansionTarget::Reference(reference)
                        if reference.subscript.is_none()
                            && !matches!(reference.name.as_str(), "@" | "*")
                ) =>
        {
            true
        }
        _ => false,
    }
}

fn word_is_direct_numeric_expansion(word: &Word) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_direct_numeric_expansion(&part.kind)
}

fn word_part_is_direct_numeric_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_direct_numeric_expansion(&part.kind)
        }
        WordPart::Length(_) | WordPart::ArrayLength(_) => true,
        WordPart::Parameter(parameter) => parameter_is_direct_numeric_expansion(parameter),
        _ => false,
    }
}

fn parameter_is_direct_numeric_expansion(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { .. }) => true,
        ParameterExpansionSyntax::Zsh(syntax) => syntax.length_prefix.is_some(),
        _ => false,
    }
}

fn build_function_in_alias_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| alias_definition_word_groups_for_command(fact, source).into_iter())
        .filter_map(|definition_words| function_in_alias_definition_span(definition_words, source))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

fn build_alias_definition_expansion_spans(
    commands: &[CommandFact<'_>],
    nodes: &[WordNode<'_>],
    occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| alias_definition_word_groups_for_command(fact, source).into_iter())
        .filter_map(|definition_words| {
            definition_words
                .iter()
                .flat_map(|candidate| {
                    word_index
                        .get(&FactSpan::new(candidate.span))
                        .into_iter()
                        .flat_map(|indices| indices.iter().copied())
                        .map(|id| &occurrences[id.index()])
                        .filter(move |fact| {
                            fact.context
                                == WordFactContext::Expansion(ExpansionContext::CommandArgument)
                                && occurrence_span(nodes, fact) == candidate.span
                        })
                })
                .flat_map(|fact| {
                    word_node_derived(&nodes[fact.node_id.index()], source)
                        .active_expansion_spans
                        .iter()
                        .copied()
                })
                .min_by_key(|span| (span.start.offset, span.end.offset))
        })
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

fn alias_definition_word_groups_for_command<'a>(
    command: &'a CommandFact<'a>,
    source: &str,
) -> Vec<&'a [&'a Word]> {
    let body_args = command.body_args();
    let mut definition_words = Vec::new();
    let mut index = 0usize;

    while let Some(word) = body_args.get(index).copied() {
        if !word_contains_literal_equals(word, source) {
            index += 1;
            continue;
        }

        let mut last_word = word;
        let mut definition_len = 1usize;
        while word_ends_with_literal_equals(last_word, source)
            && let Some(next_word) = body_args.get(index + definition_len).copied()
            && last_word.span.end.offset == next_word.span.start.offset
        {
            last_word = next_word;
            definition_len += 1;
        }

        definition_words.push(&body_args[index..index + definition_len]);
        index += definition_len;
    }

    definition_words
}

fn word_contains_literal_equals(word: &Word, source: &str) -> bool {
    word_chars_outside_expansions(word, source).any(|(_, ch)| ch == '=')
}

fn word_ends_with_literal_equals(word: &Word, source: &str) -> bool {
    word_chars_outside_expansions(word, source)
        .last()
        .is_some_and(|(_, ch)| ch == '=')
}

fn word_chars_outside_expansions<'a>(
    word: &'a Word,
    source: &'a str,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let text = word.span.slice(source);
    let mut excluded = expansion_part_spans(word);
    excluded.sort_by_key(|span| span.start.offset);
    let mut excluded = excluded.into_iter().peekable();

    text.char_indices().filter(move |(offset, _)| {
        let absolute_offset = word.span.start.offset + offset;
        while matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.end.offset
        ) {
            excluded.next();
        }

        !matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.start.offset && absolute_offset < span.end.offset
        )
    })
}

fn function_in_alias_definition_span(words: &[&Word], source: &str) -> Option<Span> {
    let definition = static_alias_definition_text(words, source)?;
    let (_, value) = definition.split_once('=')?;
    let end = words.last()?.span.end;
    contains_function_definition(value).then(|| Span::from_positions(words[0].span.start, end))
}

fn static_alias_definition_text(words: &[&Word], source: &str) -> Option<String> {
    let mut text = String::new();
    for word in words {
        text.push_str(&static_word_text(word, source)?);
    }
    Some(text)
}

fn contains_function_definition(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_keyword(value, index, "function")
            && precedes_definition_start(value, index)
            && is_definition_after_function_keyword(value, index + "function".len())
        {
            return true;
        }
        if is_identifier_start(bytes[index])
            && precedes_definition_start(value, index)
            && is_definition_after_name(value, index, bytes.len())
        {
            return true;
        }
        index += 1;
    }
    false
}

fn starts_with_keyword(text: &str, index: usize, keyword: &str) -> bool {
    let tail = &text[index..];
    if !tail.starts_with(keyword) {
        return false;
    }
    let before_ok = index == 0 || !is_identifier_char(text.as_bytes()[index - 1]);
    let after_index = index + keyword.len();
    let after_ok = after_index >= text.len() || !is_identifier_char(text.as_bytes()[after_index]);
    before_ok && after_ok
}

fn precedes_definition_start(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }

    let bytes = text.as_bytes();
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }

    cursor == 0 || matches!(bytes[cursor - 1], b';' | b'|' | b'&' | b'(' | b'{' | b'\n')
}

fn is_definition_after_function_keyword(text: &str, mut index: usize) -> bool {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let Some(end) = parse_identifier(text, index) else {
        return false;
    };
    is_definition_suffix(text, end, false)
}

fn is_definition_after_name(text: &str, index: usize, len: usize) -> bool {
    let Some(end) = parse_identifier(text, index) else {
        return false;
    };
    if end >= len {
        return false;
    }
    is_definition_suffix(text, end, true)
}

fn is_definition_suffix(text: &str, mut index: usize, require_parens: bool) -> bool {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let has_parens = bytes
        .get(index..)
        .is_some_and(|rest| rest.starts_with(b"()"));
    if require_parens && !has_parens {
        return false;
    }

    if has_parens {
        index += 2;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
    }

    bytes.get(index) == Some(&b'{')
}

fn parse_identifier(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let first = bytes.get(index).copied()?;
    if !is_identifier_start(first) {
        return None;
    }
    let mut end = index + 1;
    while let Some(byte) = bytes.get(end) {
        if !is_identifier_char(*byte) {
            break;
        }
        end += 1;
    }
    Some(end)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn build_echo_backslash_escape_word_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("echo") && fact.wrappers().is_empty())
        .filter(|fact| !echo_uses_escape_interpreting_flag(fact))
        .flat_map(|fact| fact.body_args().iter().copied())
        .filter(|word| word_contains_echo_backslash_escape(word, source))
        .map(|word| word.span)
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans
}

fn echo_uses_escape_interpreting_flag(command: &CommandFact<'_>) -> bool {
    command
        .options()
        .echo()
        .is_some_and(|echo| echo.uses_escape_interpreting_flag())
}

fn word_contains_echo_backslash_escape(word: &Word, source: &str) -> bool {
    word_parts_contain_echo_backslash_escape(&word.parts, source, false)
}

fn word_parts_contain_echo_backslash_escape(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    parts
        .iter()
        .enumerate()
        .any(|(index, part)| match &part.kind {
            WordPart::Literal(text) => {
                let core_text = if in_double_quotes {
                    text.as_str(source, part.span)
                } else {
                    part.span.slice(source)
                };
                let rendered_text = text.as_str(source, part.span);
                text_contains_echo_backslash_escape(core_text, echo_escape_is_core_family)
                    || (in_double_quotes
                        && text_contains_echo_backslash_escape(
                            rendered_text,
                            echo_escape_is_quote_like,
                        ))
                    || text_contains_echo_double_backslash(rendered_text)
                    || literal_double_backslash_touches_double_quoted_fragment(
                        parts,
                        index,
                        rendered_text,
                    )
            }
            WordPart::SingleQuoted { value, .. } => {
                text_contains_echo_backslash_escape(value.slice(source), echo_escape_is_core_family)
            }
            WordPart::DoubleQuoted { parts, .. } => {
                word_parts_contain_echo_backslash_escape(parts, source, true)
            }
            _ => false,
        })
}

fn echo_escape_is_core_family(byte: u8) -> bool {
    matches!(
        byte,
        b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'x' | b'0'..=b'9'
    )
}

fn echo_escape_is_quote_like(byte: u8) -> bool {
    matches!(byte, b'`' | b'\'')
}

fn literal_double_backslash_touches_double_quoted_fragment(
    parts: &[WordPartNode],
    index: usize,
    rendered_text: &str,
) -> bool {
    (trailing_backslash_count(rendered_text) >= 2
        && parts
            .get(index + 1)
            .is_some_and(|part| matches!(part.kind, WordPart::DoubleQuoted { .. })))
        || (leading_backslash_count(rendered_text) >= 2
            && index
                .checked_sub(1)
                .and_then(|prev| parts.get(prev))
                .is_some_and(|part| matches!(part.kind, WordPart::DoubleQuoted { .. })))
}

fn leading_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .take_while(|byte| **byte == b'\\')
        .count()
}

fn trailing_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
}

fn text_contains_echo_double_backslash(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        if index.saturating_sub(run_start) >= 2
            && bytes.get(index).is_some_and(|next| *next != b'"')
        {
            return true;
        }
    }

    false
}

fn text_contains_echo_backslash_escape(text: &str, is_sensitive: fn(u8) -> bool) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        let Some(&escaped_byte) = bytes.get(index) else {
            continue;
        };

        if index > run_start && is_sensitive(escaped_byte) {
            return true;
        }
    }

    false
}

fn build_echo_to_sed_substitution_spans<'a>(
    commands: &[CommandFact<'a>],
    pipelines: &[PipelineFact<'a>],
    backticks: &[BacktickFragmentFact],
    nodes: &[WordNode<'a>],
    occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut pipeline_sed_command_ids = FxHashSet::default();

    for pipeline in pipelines {
        if let Some(span) =
            sc2001_like_pipeline_span(
                commands,
                pipeline,
                backticks,
                nodes,
                occurrences,
                word_index,
                source,
            )
        {
            spans.push(span);
            if let Some(last_segment) = pipeline.last_segment() {
                pipeline_sed_command_ids.insert(last_segment.command_id());
            }
        }
    }

    spans.extend(commands.iter().filter_map(|command| {
        (!pipeline_sed_command_ids.contains(&command.id()))
            .then(|| sc2001_like_here_string_span(command, backticks, source))
            .flatten()
    }));

    sort_and_dedup_spans(&mut spans);
    spans
}

fn sc2001_like_pipeline_span<'a>(
    commands: &[CommandFact<'a>],
    pipeline: &PipelineFact<'a>,
    backticks: &[BacktickFragmentFact],
    nodes: &[WordNode<'a>],
    occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Option<Span> {
    let [left_segment, right_segment] = pipeline.segments() else {
        return None;
    };

    let left = command_fact(commands, left_segment.command_id());
    let right = command_fact(commands, right_segment.command_id());

    if !command_is_plain_named(left, "echo") || !command_is_plain_named(right, "sed") {
        return None;
    }

    if left
        .options()
        .echo()
        .and_then(|echo| echo.portability_flag_word())
        .is_some()
    {
        return None;
    }

    if !command_has_sc2001_like_sed_script(right, backticks, source) {
        return None;
    }

    let [argument] = left.body_args() else {
        return None;
    };

    let word_fact = word_occurrence_with_context(
        nodes,
        occurrences,
        word_index,
        argument.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    )?;

    if occurrence_static_text(nodes, word_fact, source).is_some() {
        return None;
    }

    let derived = word_node_derived(&nodes[word_fact.node_id.index()], source);
    if derived.scalar_expansion_spans.is_empty()
        && derived.array_expansion_spans.is_empty()
        && derived.command_substitution_spans.is_empty()
    {
        return None;
    }

    if derived.has_literal_affixes
        && !word_occurrence_is_pure_quoted_dynamic(nodes, word_fact, source)
    {
        return None;
    }

    Some(pipeline_span_with_shellcheck_tail(
        commands, pipeline, source,
    ))
}

fn sc2001_like_here_string_span(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
) -> Option<Span> {
    if !command_is_plain_named(command, "sed") {
        return None;
    }

    if !command_has_sc2001_like_sed_script(command, backticks, source) {
        return None;
    }

    let mut here_strings = command
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::HereString);
    here_strings.next()?;
    if here_strings.next().is_some() {
        return None;
    }

    command_span_with_redirects_and_shellcheck_tail(command, source)
}

fn command_is_plain_named(command: &CommandFact<'_>, name: &str) -> bool {
    command.effective_name_is(name) && command.wrappers().is_empty()
}

fn command_has_sc2001_like_sed_script(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
) -> bool {
    command
        .options()
        .sed()
        .is_some_and(|sed| sed.has_single_substitution_script())
        || (command_is_inside_backtick_fragment(command, backticks)
            && sed_has_single_substitution_script(
                command.body_args(),
                source,
                SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes,
            ))
}

fn command_is_inside_backtick_fragment(
    command: &CommandFact<'_>,
    backticks: &[BacktickFragmentFact],
) -> bool {
    let span = command.span();
    backticks.iter().any(|fragment| {
        let fragment_span = fragment.span();
        fragment_span.start.offset <= span.start.offset
            && fragment_span.end.offset >= span.end.offset
    })
}

fn word_occurrence_with_context<'a>(
    nodes: &[WordNode<'a>],
    occurrences: &'a [WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    span: Span,
    context: WordFactContext,
) -> Option<&'a WordOccurrence> {
    word_index
        .get(&FactSpan::new(span))
        .into_iter()
        .flat_map(|indices| indices.iter().copied())
        .map(|id| &occurrences[id.index()])
        .find(|fact| occurrence_span(nodes, fact) == span && fact.context == context)
}

pub(crate) fn occurrence_static_text<'a>(
    nodes: &'a [WordNode<'a>],
    occurrence: &WordOccurrence,
    source: &str,
) -> Option<&'a str> {
    word_node_derived(&nodes[occurrence.node_id.index()], source)
        .static_text
        .as_deref()
}

pub(crate) fn word_occurrence_is_pure_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    source: &str,
) -> bool {
    let word = occurrence_word(nodes, fact);
    !span::word_double_quoted_scalar_only_expansion_spans(word).is_empty()
        || !span::word_quoted_all_elements_array_slice_spans(word).is_empty()
        || word_occurrence_is_double_quoted_command_substitution_only(nodes, fact, source)
        || word_occurrence_is_backtick_escaped_double_quoted_dynamic(nodes, fact, source)
}

fn build_unquoted_literal_between_double_quoted_segments_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    let nested_fragment_parts = mixed_quote_word_parts_inside_nested_shell_fragments(word, source);

    let mut spans = word
        .parts
        .windows(3)
        .enumerate()
        .filter_map(|(window_index, window)| {
            let [left, middle, right] = window else {
                return None;
            };
            let WordPart::DoubleQuoted {
                parts: left_inner, ..
            } = &left.kind
            else {
                return None;
            };
            let WordPart::Literal(text) = &middle.kind else {
                return None;
            };
            let WordPart::DoubleQuoted {
                parts: right_inner, ..
            } = &right.kind
            else {
                return None;
            };

            let neighbor_has_literal =
                mixed_quote_double_quoted_parts_contain_literal_content(left_inner)
                    || mixed_quote_double_quoted_parts_contain_literal_content(right_inner);
            let middle_is_nested = nested_fragment_parts
                .get(window_index + 1)
                .copied()
                .unwrap_or(false);
            (neighbor_has_literal
                && !middle_is_nested
                && mixed_quote_literal_is_warnable_between_double_quotes(
                    text.as_str(source, middle.span),
                ))
            .then_some(middle.span)
        })
        .collect::<Vec<_>>();

    if let Some(span) = mixed_quote_trailing_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }

    spans
}

fn mixed_quote_double_quoted_parts_contain_literal_content(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => {
            mixed_quote_double_quoted_parts_contain_literal_content(parts)
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
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

fn mixed_quote_literal_is_warnable_between_double_quotes(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    if text == "\"" {
        return true;
    }

    if matches!(text, "\\\n" | "\\\r\n") {
        return true;
    }

    if text == "/,/" {
        return true;
    }

    if text.chars().all(|ch| matches!(ch, '\\' | '"')) && text.contains('\\') {
        return true;
    }

    if text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return !text.chars().any(char::is_whitespace);
    }

    if text.chars().all(|ch| ch == ':') {
        return text.len() > 1;
    }

    text.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '+' | '-' | '%' | ':')
    })
}

fn mixed_quote_word_parts_inside_nested_shell_fragments(word: &Word, source: &str) -> Vec<bool> {
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;
    let mut nested = Vec::with_capacity(word.parts.len());

    for part in &word.parts {
        nested.push(command_depth > 0 || parameter_depth > 0);

        let (command_delta, parameter_delta) =
            mixed_quote_shell_fragment_balance_delta_for_part(part, source);
        command_depth += command_delta;
        parameter_depth += parameter_delta;
        command_depth = command_depth.max(0);
        parameter_depth = parameter_depth.max(0);
    }

    nested
}

fn mixed_quote_shell_fragment_balance_delta_for_part(
    part: &WordPartNode,
    source: &str,
) -> (i32, i32) {
    match &part.kind {
        WordPart::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        } => {
            let text = part.span.slice(source);
            let body = text
                .strip_prefix('`')
                .and_then(|text| text.strip_suffix('`'))
                .unwrap_or(text);
            mixed_quote_shell_fragment_balance_delta(body, true)
        }
        WordPart::ProcessSubstitution { .. } => {
            mixed_quote_shell_fragment_balance_delta(part.span.slice(source), true)
        }
        _ => mixed_quote_shell_fragment_balance_delta(part.span.slice(source), false),
    }
}

fn mixed_quote_shell_fragment_balance_delta(
    text: &str,
    allow_top_level_command_comments: bool,
) -> (i32, i32) {
    let mut command_delta = 0i32;
    let mut parameter_delta = 0i32;
    let mut chars = text.chars().peekable();
    let mut escaped = false;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut previous_char = None;

    while let Some(ch) = chars.next() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                previous_char = Some(ch);
            }
            continue;
        }

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            previous_char = Some(ch);
            continue;
        }

        if escaped {
            escaped = false;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '"' {
            in_double_quotes = !in_double_quotes;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\\' {
            escaped = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_delta,
                allow_top_level_command_comments,
                previous_char,
            )
        {
            in_comment = true;
            continue;
        }

        if ch == '$' {
            match chars.peek().copied() {
                Some('(') => {
                    command_delta += 1;
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some('{') => {
                    parameter_delta += 1;
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        match ch {
            ')' => command_delta -= 1,
            '}' => parameter_delta -= 1,
            _ => {}
        }

        previous_char = Some(ch);
    }

    (command_delta, parameter_delta)
}

fn mixed_quote_shell_comment_can_start(
    command_depth: i32,
    allow_top_level_command_comments: bool,
    previous_char: Option<char>,
) -> bool {
    (command_depth > 0 || allow_top_level_command_comments)
        && previous_char.is_none_or(|ch| {
            ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
        })
}

fn mixed_quote_trailing_line_join_between_double_quotes_span(
    word: &Word,
    source: &str,
) -> Option<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return None;
    }

    let text = word.span.slice(source);
    let (prefix, suffix) = if let Some(prefix) = text.strip_suffix("\\\n") {
        (prefix, "\\\n")
    } else if let Some(prefix) = text.strip_suffix("\\\r\n") {
        (prefix, "\\\r\n")
    } else {
        return None;
    };

    if !source[word.span.end.offset..].starts_with('"') {
        return None;
    }

    let start = word.span.start.advanced_by(prefix);
    Some(Span::from_positions(start, start.advanced_by(suffix)))
}


pub(crate) fn word_occurrence_is_double_quoted_command_substitution_only(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()], source);
    let [command_substitution] = derived.command_substitution_spans.as_ref() else {
        return false;
    };

    if !derived.scalar_expansion_spans.is_empty() || !derived.array_expansion_spans.is_empty() {
        return false;
    }

    let word_text = occurrence_span(nodes, fact).slice(source);
    word_text.len() == command_substitution.slice(source).len() + 2
        && word_text.starts_with('"')
        && word_text.ends_with('"')
        && &word_text[1..word_text.len() - 1] == command_substitution.slice(source)
}

pub(crate) fn word_occurrence_is_backtick_escaped_double_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()], source);
    let word_text = occurrence_span(nodes, fact).slice(source);
    if !word_text.starts_with("\\\"") || !word_text.ends_with("\\\"") {
        return false;
    }

    let inner = &word_text[2..word_text.len() - 2];
    match (
        derived.scalar_expansion_spans.as_ref(),
        derived.array_expansion_spans.as_ref(),
        derived.command_substitution_spans.as_ref(),
    ) {
        ([scalar], [], []) => inner == scalar.slice(source),
        ([], [array], []) => inner == array.slice(source),
        ([], [], [command_substitution]) => inner == command_substitution.slice(source),
        _ => false,
    }
}

fn build_unquoted_command_argument_use_offsets(
    semantic: &SemanticModel,
    nodes: &[WordNode<'_>],
    occurrences: &[WordOccurrence],
) -> FxHashMap<Name, Vec<usize>> {
    let unquoted_command_argument_word_spans = occurrences
        .iter()
        .filter(|fact| fact.context == WordFactContext::Expansion(ExpansionContext::CommandArgument))
        .filter(|fact| occurrence_analysis(nodes, fact).quote == WordQuote::Unquoted)
        .map(|fact| occurrence_span(nodes, fact))
        .collect::<Vec<_>>();
    if unquoted_command_argument_word_spans.is_empty() {
        return FxHashMap::default();
    }

    let references = semantic.references();
    let mut reference_indices = references
        .iter()
        .enumerate()
        .filter(|(_, reference)| {
            !matches!(
                reference.kind,
                shuck_semantic::ReferenceKind::DeclarationName
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    reference_indices.sort_unstable_by_key(|&index| references[index].span.start.offset);

    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();
    for word_span in unquoted_command_argument_word_spans {
        let first_reference = reference_indices
            .partition_point(|&index| references[index].span.start.offset < word_span.start.offset);
        for &index in &reference_indices[first_reference..] {
            let reference = &references[index];
            if reference.span.start.offset > word_span.end.offset {
                break;
            }
            if !contains_span(word_span, reference.span) {
                continue;
            }

            offsets_by_name
                .entry(reference.name.clone())
                .or_default()
                .push(word_span.start.offset);
        }
    }

    for offsets in offsets_by_name.values_mut() {
        offsets.sort_unstable();
        offsets.dedup();
    }

    offsets_by_name
}

fn build_word_facts_for_command<'a>(
    visit: CommandVisit<'a>,
    source: &'a str,
    semantic: &'a SemanticModel,
    context: WordFactCommandContext,
    normalized: &NormalizedCommand<'a>,
    command_zsh_options: Option<ZshOptionState>,
    outputs: WordFactOutputs<'_, 'a>,
) {
    let mut collector = WordFactCollector::new(
        source,
        semantic,
        context.command_id,
        context.nested_word_command,
        normalized,
        command_zsh_options,
        outputs,
    );
    collector.collect_command(visit.command, visit.redirects);
}

#[cfg(feature = "benchmarking")]
pub(crate) fn benchmark_collect_word_facts(
    file: &File,
    source: &str,
    semantic: &SemanticModel,
) -> usize {
    let mut word_nodes = Vec::new();
    let mut word_node_ids_by_span = FxHashMap::default();
    let mut word_occurrences = Vec::new();
    let mut compound_assignment_value_word_spans = FxHashSet::default();
    let mut array_assignment_split_word_ids = Vec::new();
    let mut assoc_binding_visibility_memo = FxHashMap::default();
    let mut case_pattern_expansion_spans = Vec::new();
    let mut pattern_literal_spans = Vec::new();
    let mut arithmetic_summary = ArithmeticFactSummary::default();
    let mut surface_fragments = SurfaceFragmentSink::new(source);

    for (next_command_id, traversed) in query::iter_commands_with_context(
        &file.body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    )
    .enumerate()
    {
        let visit = traversed.visit;
        let normalized = command::normalize_command(visit.command, source);
        let command_zsh_options = effective_command_zsh_options(
            semantic,
            command_span(visit.command).start.offset,
            &normalized,
        );
        build_word_facts_for_command(
            visit,
            source,
            semantic,
            WordFactCommandContext {
                command_id: CommandId::new(next_command_id),
                nested_word_command: traversed.context.nested_word_command,
            },
            &normalized,
            command_zsh_options,
            WordFactOutputs {
                word_nodes: &mut word_nodes,
                word_node_ids_by_span: &mut word_node_ids_by_span,
                word_occurrences: &mut word_occurrences,
                compound_assignment_value_word_spans: &mut compound_assignment_value_word_spans,
                array_assignment_split_word_ids: &mut array_assignment_split_word_ids,
                assoc_binding_visibility_memo: &mut assoc_binding_visibility_memo,
                case_pattern_expansion_spans: &mut case_pattern_expansion_spans,
                pattern_literal_spans: &mut pattern_literal_spans,
                arithmetic: &mut arithmetic_summary,
                surface: &mut surface_fragments,
            },
        );
    }

    let surface_fragments = surface_fragments.finish();

    word_occurrences.len()
        + word_nodes.len()
        + compound_assignment_value_word_spans.len()
        + array_assignment_split_word_ids.len()
        + case_pattern_expansion_spans.len()
        + pattern_literal_spans.len()
        + arithmetic_summary.array_index_arithmetic_spans.len()
        + arithmetic_summary.arithmetic_score_line_spans.len()
        + arithmetic_summary.dollar_in_arithmetic_spans.len()
        + arithmetic_summary.arithmetic_command_substitution_spans.len()
        + surface_fragments.single_quoted.len()
        + surface_fragments.backticks.len()
        + surface_fragments.pattern_charclass_spans.len()
        + surface_fragments.substring_expansions.len()
        + surface_fragments.case_modifications.len()
        + surface_fragments.replacement_expansions.len()
}

#[derive(Clone, Copy)]
struct WordFactCommandContext {
    command_id: CommandId,
    nested_word_command: bool,
}

struct WordFactOutputs<'out, 'a> {
    word_nodes: &'out mut Vec<WordNode<'a>>,
    word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    word_occurrences: &'out mut Vec<WordOccurrence>,
    compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    assoc_binding_visibility_memo:
        &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    case_pattern_expansion_spans: &'out mut Vec<Span>,
    pattern_literal_spans: &'out mut Vec<Span>,
    arithmetic: &'out mut ArithmeticFactSummary,
    surface: &'out mut SurfaceFragmentSink<'a>,
}

fn derive_word_fact_data(word: &Word, source: &str) -> WordNodeDerived {
    WordNodeDerived {
        static_text: static_word_text(word, source)
            .map(|text| text.into_owned().into_boxed_str()),
        starts_with_extglob: span::word_starts_with_extglob(word, source),
        has_literal_affixes: word_has_literal_affixes(word),
        contains_shell_quoting_literals: word_contains_shell_quoting_literals(word, source),
        active_expansion_spans: span::active_expansion_spans_in_source(word, source)
            .into_boxed_slice(),
        scalar_expansion_spans: span::scalar_expansion_part_spans(word, source).into_boxed_slice(),
        unquoted_scalar_expansion_spans: span::unquoted_scalar_expansion_part_spans(word, source)
            .into_boxed_slice(),
        array_expansion_spans: span::array_expansion_part_spans(word, source).into_boxed_slice(),
        all_elements_array_expansion_spans: span::all_elements_array_expansion_part_spans(
            word, source,
        )
        .into_boxed_slice(),
        direct_all_elements_array_expansion_spans:
            span::direct_all_elements_array_expansion_part_spans(word, source).into_boxed_slice(),
        unquoted_all_elements_array_expansion_spans:
            span::unquoted_all_elements_array_expansion_part_spans(word, source)
                .into_boxed_slice(),
        unquoted_array_expansion_spans: span::unquoted_array_expansion_part_spans(word, source)
            .into_boxed_slice(),
        command_substitution_spans: span::command_substitution_part_spans_in_source(word, source)
            .into_boxed_slice(),
        unquoted_command_substitution_spans:
            span::unquoted_command_substitution_part_spans_in_source(word, source)
                .into_boxed_slice(),
        double_quoted_expansion_spans: double_quoted_expansion_part_spans(word).into_boxed_slice(),
        unquoted_literal_between_double_quoted_segments_spans:
            build_unquoted_literal_between_double_quoted_segments_spans(word, source)
                .into_boxed_slice(),
    }
}

struct WordFactCollector<'out, 'a, 'norm> {
    source: &'a str,
    semantic: &'a SemanticModel,
    command_id: CommandId,
    nested_word_command: bool,
    surface_command_name: Option<&'norm str>,
    command_zsh_options: Option<ZshOptionState>,
    word_nodes: &'out mut Vec<WordNode<'a>>,
    word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    word_occurrences: &'out mut Vec<WordOccurrence>,
    array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    assoc_binding_visibility_memo:
        &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    seen: FxHashSet<(FactSpan, WordFactContext, WordFactHostKind)>,
    compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    case_pattern_expansion_spans: &'out mut Vec<Span>,
    pattern_literal_spans: &'out mut Vec<Span>,
    arithmetic: &'out mut ArithmeticFactSummary,
    surface: &'out mut SurfaceFragmentSink<'a>,
}

impl<'out, 'a, 'norm> WordFactCollector<'out, 'a, 'norm> {
    fn new(
        source: &'a str,
        semantic: &'a SemanticModel,
        command_id: CommandId,
        nested_word_command: bool,
        normalized: &'norm NormalizedCommand<'a>,
        command_zsh_options: Option<ZshOptionState>,
        outputs: WordFactOutputs<'out, 'a>,
    ) -> Self {
        Self {
            source,
            semantic,
            command_id,
            nested_word_command,
            surface_command_name: normalized.effective_or_literal_name(),
            command_zsh_options,
            word_nodes: outputs.word_nodes,
            word_node_ids_by_span: outputs.word_node_ids_by_span,
            word_occurrences: outputs.word_occurrences,
            array_assignment_split_word_ids: outputs.array_assignment_split_word_ids,
            assoc_binding_visibility_memo: outputs.assoc_binding_visibility_memo,
            seen: FxHashSet::default(),
            compound_assignment_value_word_spans: outputs.compound_assignment_value_word_spans,
            case_pattern_expansion_spans: outputs.case_pattern_expansion_spans,
            pattern_literal_spans: outputs.pattern_literal_spans,
            arithmetic: outputs.arithmetic,
            surface: outputs.surface,
        }
    }

    fn surface_context(&self) -> SurfaceScanContext<'norm> {
        SurfaceScanContext::new(self.surface_command_name, self.nested_word_command)
    }

    fn collect_surface_only_word(
        &mut self,
        word: &Word,
        surface_context: SurfaceScanContext<'_>,
    ) -> bool {
        self.surface.collect_word(word, surface_context)
    }

    fn collect_command(&mut self, command: &'a Command, redirects: &'a [Redirect]) {
        self.collect_command_name_context_word(command);
        self.collect_argument_context_words(command);
        self.collect_expansion_assignment_value_words(command);
        let surface_context = self.surface_context();

        if let Command::Compound(command) = command {
            match command {
                CompoundCommand::For(command) => {
                    if let Some(words) = &command.words {
                        for word in words {
                            self.push_word_with_surface(
                                word,
                                WordFactContext::Expansion(ExpansionContext::ForList),
                                WordFactHostKind::Direct,
                                surface_context,
                            );
                        }
                    }
                }
                CompoundCommand::Repeat(command) => {
                    self.push_word_with_surface(
                        &command.count,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        WordFactHostKind::Direct,
                        surface_context,
                    );
                }
                CompoundCommand::Foreach(command) => {
                    for word in &command.words {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::ForList),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                }
                CompoundCommand::Select(command) => {
                    for word in &command.words {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::SelectList),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                }
                CompoundCommand::Case(command) => {
                    self.push_word_with_surface(
                        &command.word,
                        WordFactContext::CaseSubject,
                        WordFactHostKind::Direct,
                        surface_context,
                    );
                    for case in &command.cases {
                        for pattern in &case.patterns {
                            let pattern_context = surface_context.with_pattern_charclass_scan();
                            self.surface.collect_pattern_structure(pattern, pattern_context);
                            self.collect_case_pattern_expansion_spans(pattern);
                            self.collect_pattern_context_words(
                                pattern,
                                WordFactContext::Expansion(ExpansionContext::CasePattern),
                                WordFactHostKind::Direct,
                                Some(pattern_context),
                            );
                        }
                    }
                }
                CompoundCommand::Conditional(command) => {
                    self.collect_conditional_expansion_words(
                        &command.expression,
                        SurfaceScanContext::new(None, self.nested_word_command),
                    );
                }
                CompoundCommand::Arithmetic(command) => {
                    if let Some(expression) = &command.expr_ast {
                        collect_arithmetic_command_spans(
                            expression,
                            self.source,
                            &mut self.arithmetic.dollar_in_arithmetic_spans,
                            &mut self.arithmetic.arithmetic_command_substitution_spans,
                        );
                    }
                }
                CompoundCommand::ArithmeticFor(command) => {
                    for expression in [
                        command.init_ast.as_ref(),
                        command.condition_ast.as_ref(),
                        command.step_ast.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        collect_arithmetic_command_spans(
                            expression,
                            self.source,
                            &mut self.arithmetic.dollar_in_arithmetic_spans,
                            &mut self.arithmetic.arithmetic_command_substitution_spans,
                        );
                    }
                }
                CompoundCommand::If(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Subshell(_)
                | CompoundCommand::BraceGroup(_)
                | CompoundCommand::Always(_)
                | CompoundCommand::Coproc(_)
                | CompoundCommand::Time(_) => {}
            }
        }

        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => {
                    let Some(context) = ExpansionContext::from_redirect_kind(redirect.kind) else {
                        continue;
                    };
                    let word_context = if redirect.kind == RedirectKind::HereString {
                        surface_context
                    } else {
                        surface_context.without_command_name()
                    };
                    self.push_word_with_surface(
                        word,
                        WordFactContext::Expansion(context),
                        WordFactHostKind::Direct,
                        word_context,
                    );
                }
                None => {
                    let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                    if heredoc.delimiter.expands_body {
                        self.surface.collect_heredoc_body(
                            &heredoc.body,
                            surface_context.without_open_double_quote_scan(),
                        );
                    }
                }
            }
        }

        if let Some(action) = trap_action_word(command, self.source) {
            self.push_word(
                action,
                WordFactContext::Expansion(ExpansionContext::TrapAction),
                WordFactHostKind::Direct,
            );
        }
    }

    fn collect_command_name_context_word(&mut self, command: &'a Command) {
        let surface_context = self.surface_context();
        match command {
            Command::Simple(command) => {
                if static_word_text(&command.name, self.source).is_none() {
                    self.push_word_with_surface(
                        &command.name,
                        WordFactContext::Expansion(ExpansionContext::CommandName),
                        WordFactHostKind::Direct,
                        surface_context,
                    );
                } else {
                    self.collect_surface_only_word(&command.name, surface_context);
                }
            }
            Command::Function(function) => {
                for entry in &function.header.entries {
                    if static_word_text(&entry.word, self.source).is_none() {
                        self.push_word_with_surface(
                            &entry.word,
                            WordFactContext::Expansion(ExpansionContext::CommandName),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    } else {
                        self.collect_surface_only_word(&entry.word, surface_context);
                    }
                }
            }
            Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::AnonymousFunction(_) => {}
        }
    }

    fn collect_argument_context_words(&mut self, command: &'a Command) {
        match command {
            Command::Simple(command) => {
                let surface_context = self.surface_context();
                let surface_command_name = surface_context.command_name();
                let trap_command =
                    static_word_text(&command.name, self.source).as_deref() == Some("trap");
                let variable_set_operand =
                    surface::simple_command_variable_set_operand(command, self.source);
                let mut saw_open_double_quote = false;
                if surface_command_name == Some("unset") {
                    for word in &command.args {
                        self.surface.record_unset_array_target_word(word);
                    }
                }
                if matches!(surface_command_name, Some("echo" | "printf")) {
                    self.surface
                        .collect_split_suspect_closing_quote_fragment_in_words(&command.args);
                }
                for word in &command.args {
                    let base_surface_word_context = if variable_set_operand
                        .is_some_and(|operand| std::ptr::eq(word, operand))
                    {
                        surface_context.variable_set_operand()
                    } else {
                        surface_context
                    };
                    let surface_word_context = if saw_open_double_quote
                        && !surface::word_has_reopened_double_quote_window(
                            word,
                            self.source,
                            surface_command_name,
                        ) {
                        base_surface_word_context.without_open_double_quote_scan()
                    } else {
                        base_surface_word_context
                    };
                    if trap_command {
                        saw_open_double_quote |=
                            self.collect_surface_only_word(word, surface_word_context);
                    } else {
                        if surface_command_name == Some("eval") {
                            collect_wrapped_arithmetic_spans_in_word(
                                word,
                                self.source,
                                &mut self.arithmetic.dollar_in_arithmetic_spans,
                                &mut self.arithmetic.arithmetic_command_substitution_spans,
                            );
                        }
                        let (_, opened) = self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                            surface_word_context,
                        );
                        saw_open_double_quote |= opened;
                    }
                }
            }
            Command::Builtin(command) => match command {
                BuiltinCommand::Break(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.depth {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        surface_context,
                    );
                }
                BuiltinCommand::Continue(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.depth {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        surface_context,
                    );
                }
                BuiltinCommand::Return(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.code {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        surface_context,
                    );
                }
                BuiltinCommand::Exit(command) => {
                    let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                    if let Some(word) = &command.code {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(ExpansionContext::CommandArgument),
                            WordFactHostKind::Direct,
                            surface_context,
                        );
                    }
                    self.collect_words_with_context(
                        &command.extra_args,
                        WordFactContext::Expansion(ExpansionContext::CommandArgument),
                        surface_context,
                    );
                }
            },
            Command::Decl(command) => {
                let surface_context = SurfaceScanContext::new(None, self.nested_word_command);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) => {
                            self.collect_surface_only_word(word, surface_context);
                        }
                        DeclOperand::Dynamic(word) => {
                            self.push_word_with_surface(
                                word,
                                WordFactContext::Expansion(ExpansionContext::CommandArgument),
                                WordFactHostKind::Direct,
                                surface_context,
                            );
                        }
                        DeclOperand::Name(_) | DeclOperand::Assignment(_) => {}
                    }
                }
            }
            Command::Binary(_) | Command::Compound(_) | Command::Function(_) => {}
            Command::AnonymousFunction(function) => {
                self.collect_words_with_context(
                    &function.args,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                    SurfaceScanContext::new(None, self.nested_word_command),
                );
            }
        }
    }

    fn collect_expansion_assignment_value_words(&mut self, command: &'a Command) {
        for assignment in query::command_assignments(command) {
            self.collect_expansion_assignment_words(
                assignment,
                WordFactContext::Expansion(ExpansionContext::AssignmentValue),
            );
        }

        for operand in query::declaration_operands(command) {
            match operand {
                DeclOperand::Name(reference) => {
                    self.surface.record_var_ref_subscript(reference);
                    let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
                        Some(&reference.name),
                        Some(reference.name_span),
                        reference.subscript.as_ref(),
                    );
                    query::visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| {
                            let surface_context =
                                SurfaceScanContext::new(None, self.nested_word_command);
                            collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                                &word.parts,
                                self.source,
                                &mut self.arithmetic.dollar_in_arithmetic_spans,
                            );
                            if indexed_semantics {
                                self.collect_array_index_arithmetic_spans(word);
                                self.collect_dollar_prefixed_indexed_subscript_spans(word);
                            }
                            self.push_word_with_surface(
                                word,
                                WordFactContext::Expansion(
                                    ExpansionContext::DeclarationAssignmentValue,
                                ),
                                WordFactHostKind::DeclarationNameSubscript,
                                surface_context,
                            );
                        },
                    );
                }
                DeclOperand::Assignment(assignment) => {
                    self.collect_expansion_assignment_words(
                        assignment,
                        WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue),
                    );
                }
                DeclOperand::Flag(_) | DeclOperand::Dynamic(_) => {}
            }
        }
    }

    fn collect_expansion_assignment_words(
        &mut self,
        assignment: &'a Assignment,
        context: WordFactContext,
    ) {
        let surface_context = SurfaceScanContext::new(None, self.nested_word_command)
            .with_assignment_target(assignment.target.name.as_str());
        self.surface.record_var_ref_subscript(&assignment.target);
        let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
            Some(&assignment.target.name),
            Some(assignment.target.name_span),
            assignment.target.subscript.as_ref(),
        );
        query::visit_var_ref_subscript_words_with_source(
            &assignment.target,
            self.source,
            &mut |word| {
                collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                    &word.parts,
                    self.source,
                    &mut self.arithmetic.dollar_in_arithmetic_spans,
                );
                if indexed_semantics {
                    self.collect_array_index_arithmetic_spans(word);
                    self.collect_dollar_prefixed_indexed_subscript_spans(word);
                }
                self.push_word_with_surface(
                    word,
                    context,
                    WordFactHostKind::AssignmentTargetSubscript,
                    surface_context,
                );
            },
        );

        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.push_word_with_surface(word, context, WordFactHostKind::Direct, surface_context);
            }
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => {
                            self.compound_assignment_value_word_spans
                                .insert(FactSpan::new(word.span));
                            if let (Some(index), _) = self.push_word_with_surface(
                                word,
                                context,
                                WordFactHostKind::Direct,
                                surface_context,
                            )
                            {
                                self.array_assignment_split_word_ids.push(index);
                            }
                        }
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            self.surface.record_subscript(Some(key));
                            let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
                                Some(&assignment.target.name),
                                Some(assignment.target.name_span),
                                Some(key),
                            );
                            query::visit_subscript_words(Some(key), self.source, &mut |word| {
                                collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                                    &word.parts,
                                    self.source,
                                    &mut self.arithmetic.dollar_in_arithmetic_spans,
                                );
                                if indexed_semantics {
                                    self.collect_dollar_prefixed_indexed_subscript_spans(word);
                                }
                                self.push_word_with_surface(
                                    word,
                                    context,
                                    WordFactHostKind::ArrayKeySubscript,
                                    surface_context,
                                );
                            });
                            self.compound_assignment_value_word_spans
                                .insert(FactSpan::new(value.span));
                            self.push_word_with_surface(
                                value,
                                context,
                                WordFactHostKind::Direct,
                                surface_context,
                            );
                        }
                    }
                }
            }
        }
    }

    fn collect_words_with_context(
        &mut self,
        words: &'a [Word],
        context: WordFactContext,
        surface_context: SurfaceScanContext<'_>,
    ) {
        for word in words {
            self.push_word_with_surface(word, context, WordFactHostKind::Direct, surface_context);
        }
    }

    fn collect_pattern_context_words(
        &mut self,
        pattern: &'a Pattern,
        context: WordFactContext,
        host_kind: WordFactHostKind,
        surface_context: Option<SurfaceScanContext<'_>>,
    ) {
        let is_case_pattern = matches!(
            context,
            WordFactContext::Expansion(ExpansionContext::CasePattern)
        );
        if is_case_pattern && !pattern_contains_word_or_group(pattern) {
            self.pattern_literal_spans.push(pattern.span);
        }
        for (part, _span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_pattern_context_words(
                            pattern,
                            context,
                            host_kind,
                            surface_context,
                        );
                    }
                }
                PatternPart::Word(word) => {
                    if let Some(surface_context) = surface_context {
                        self.push_word_with_surface(word, context, host_kind, surface_context);
                    } else {
                        self.push_word(word, context, host_kind);
                    }
                }
                PatternPart::Literal(_) | PatternPart::CharClass(_) if is_case_pattern => {}
                PatternPart::AnyString | PatternPart::AnyChar => {}
                PatternPart::Literal(_) | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn collect_case_pattern_expansion_spans(&mut self, pattern: &Pattern) {
        if pattern_has_glob_structure(pattern, self.source) {
            return;
        }

        if pattern_is_arithmetic_only(pattern) {
            return;
        }

        let expanded_word_spans = pattern
            .parts
            .iter()
            .filter_map(|part| match &part.kind {
                PatternPart::Word(word) => {
                    let analysis =
                        analyze_word(word, self.source, self.command_zsh_options.as_ref());
                    (analysis.literalness == WordLiteralness::Expanded
                        && analysis.quote != WordQuote::FullyQuoted)
                        .then_some(word.span)
                }
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_)
                | PatternPart::Group { .. } => None,
            })
            .collect::<Vec<_>>();

        if expanded_word_spans.is_empty() {
            return;
        }

        if pattern.parts.len() > 1 {
            self.case_pattern_expansion_spans.push(pattern.span);
        } else {
            self.case_pattern_expansion_spans
                .extend(expanded_word_spans);
        }
    }

    fn collect_zsh_qualified_glob_context_words(
        &mut self,
        glob: &'a ZshQualifiedGlob,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_pattern_context_words(pattern, context, host_kind, None);
            }
        }
    }

    fn collect_conditional_expansion_words(
        &mut self,
        expression: &'a ConditionalExpr,
        surface_context: SurfaceScanContext<'_>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.collect_conditional_expansion_words(&expr.left, surface_context);
                self.collect_conditional_expansion_words(&expr.right, surface_context);
            }
            ConditionalExpr::Unary(expr) => self.collect_conditional_expansion_words(
                &expr.expr,
                if expr.op == ConditionalUnaryOp::VariableSet {
                    surface_context.variable_set_operand()
                } else {
                    surface_context
                },
            ),
            ConditionalExpr::Parenthesized(expr) => {
                self.collect_conditional_expansion_words(&expr.expr, surface_context)
            }
            ConditionalExpr::Word(word) => {
                self.push_word_with_surface(
                    word,
                    WordFactContext::Expansion(ExpansionContext::StringTestOperand),
                    WordFactHostKind::Direct,
                    surface_context,
                );
            }
            ConditionalExpr::Regex(word) => {
                self.push_word_with_surface(
                    word,
                    WordFactContext::Expansion(ExpansionContext::RegexOperand),
                    WordFactHostKind::Direct,
                    surface_context,
                );
            }
            ConditionalExpr::Pattern(pattern) => {
                let pattern_context = surface_context.with_pattern_charclass_scan();
                self.surface.collect_pattern_structure(pattern, pattern_context);
                self.collect_pattern_context_words(
                    pattern,
                    WordFactContext::Expansion(ExpansionContext::ConditionalPattern),
                    WordFactHostKind::Direct,
                    Some(pattern_context),
                );
            }
            ConditionalExpr::VarRef(reference) => {
                self.surface.record_var_ref_subscript(reference);
                query::visit_var_ref_subscript_words_with_source(
                    reference,
                    self.source,
                    &mut |word| {
                        self.push_word_with_surface(
                            word,
                            WordFactContext::Expansion(
                                ExpansionContext::ConditionalVarRefSubscript,
                            ),
                            WordFactHostKind::ConditionalVarRefSubscript,
                            surface_context,
                        );
                    },
                );
            }
        }
    }

    fn collect_word_parameter_patterns(
        &mut self,
        parts: &'a [WordPartNode],
        host_kind: WordFactHostKind,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::ZshQualifiedGlob(glob) => self.collect_zsh_qualified_glob_context_words(
                    glob,
                    WordFactContext::Expansion(ExpansionContext::ParameterPattern),
                    host_kind,
                ),
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_word_parameter_patterns(parts, host_kind)
                }
                WordPart::Parameter(parameter) => {
                    if let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                        operator,
                        ..
                    }) = &parameter.syntax
                    {
                        self.collect_parameter_operator_patterns(operator, host_kind);
                    }
                }
                WordPart::ParameterExpansion { operator, .. } => {
                    self.collect_parameter_operator_patterns(operator, host_kind)
                }
                WordPart::IndirectExpansion {
                    operator: Some(operator),
                    ..
                } => self.collect_parameter_operator_patterns(operator, host_kind),
                WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { operator: None, .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {}
            }
        }
    }

    fn collect_parameter_operator_patterns(
        &mut self,
        operator: &'a ParameterOp,
        host_kind: WordFactHostKind,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern }
            | ParameterOp::ReplaceFirst { pattern, .. }
            | ParameterOp::ReplaceAll { pattern, .. } => self.collect_pattern_context_words(
                pattern,
                WordFactContext::Expansion(ExpansionContext::ParameterPattern),
                host_kind,
                None,
            ),
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    fn push_word(
        &mut self,
        word: &'a Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) -> Option<WordOccurrenceId> {
        self.push_word_occurrence(word, context, host_kind, None).0
    }

    fn push_word_with_surface(
        &mut self,
        word: &'a Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
        surface_context: SurfaceScanContext<'_>,
    ) -> (Option<WordOccurrenceId>, bool) {
        self.push_word_occurrence(word, context, host_kind, Some(surface_context))
    }

    fn intern_word_node(&mut self, word: &'a Word) -> WordNodeId {
        let key = FactSpan::new(word.span);
        if let Some(id) = self.word_node_ids_by_span.get(&key).copied() {
            return id;
        }

        let id = WordNodeId::new(self.word_nodes.len());
        let analysis = analyze_word(word, self.source, self.command_zsh_options.as_ref());
        self.word_nodes.push(WordNode {
            key,
            word,
            analysis,
            derived: OnceCell::new(),
        });
        self.word_node_ids_by_span.insert(key, id);
        id
    }

    fn push_word_occurrence(
        &mut self,
        word: &'a Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
        surface_context: Option<SurfaceScanContext<'_>>,
    ) -> (Option<WordOccurrenceId>, bool) {
        let opened_double_quote = surface_context
            .map(|surface_context| self.surface.collect_word(word, surface_context))
            .unwrap_or(false);
        let key = FactSpan::new(word.span);
        if !self.seen.insert((key, context, host_kind)) {
            return (None, opened_double_quote);
        }

        self.collect_word_parameter_patterns(&word.parts, host_kind);
        self.collect_arithmetic_summary(word, context, host_kind);

        let node_id = self.intern_word_node(word);
        let analysis = self.word_nodes[node_id.index()].analysis;
        let runtime_literal = match context {
            WordFactContext::Expansion(context) => {
                analyze_literal_runtime(word, self.source, context, self.command_zsh_options.as_ref())
            }
            WordFactContext::CaseSubject | WordFactContext::ArithmeticCommand => {
                RuntimeLiteralAnalysis::default()
            }
        };
        let operand_class = match context {
            WordFactContext::Expansion(context) if word_context_supports_operand_class(context) => {
                Some(
                    if analysis.literalness == WordLiteralness::Expanded
                        || runtime_literal.is_runtime_sensitive()
                    {
                        TestOperandClass::RuntimeSensitive
                    } else {
                        TestOperandClass::FixedLiteral
                    },
                )
            }
            WordFactContext::Expansion(_)
            | WordFactContext::CaseSubject
            | WordFactContext::ArithmeticCommand => None,
        };
        let id = WordOccurrenceId::new(self.word_occurrences.len());
        self.word_occurrences.push(WordOccurrence {
            node_id,
            command_id: self.command_id,
            nested_word_command: self.nested_word_command,
            context,
            host_kind,
            runtime_literal,
            operand_class,
            array_assignment_split_scalar_expansion_spans: OnceCell::new(),
        });
        (Some(id), opened_double_quote)
    }

    fn collect_arithmetic_summary(
        &mut self,
        word: &Word,
        context: WordFactContext,
        host_kind: WordFactHostKind,
    ) {
        if host_kind == WordFactHostKind::Direct
            && matches!(
                context,
                WordFactContext::Expansion(ExpansionContext::AssignmentValue)
                    | WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue)
            )
        {
            self.arithmetic
                .arithmetic_score_line_spans
                .extend(span::parenthesized_arithmetic_expansion_part_spans(word));
        }

        collect_arithmetic_expansion_spans_from_parts(
            &word.parts,
            self.source,
            host_kind == WordFactHostKind::Direct,
            &mut self.arithmetic.dollar_in_arithmetic_spans,
            &mut self.arithmetic.arithmetic_command_substitution_spans,
        );

        if host_kind == WordFactHostKind::Direct
            && word_needs_wrapped_arithmetic_fallback(word, self.source)
        {
            collect_wrapped_arithmetic_spans_in_word(
                word,
                self.source,
                &mut self.arithmetic.dollar_in_arithmetic_spans,
                &mut self.arithmetic.arithmetic_command_substitution_spans,
            );
        }
    }

    fn subscript_uses_index_arithmetic_semantics(
        &mut self,
        owner_name: Option<&Name>,
        owner_name_span: Option<Span>,
        subscript: Option<&Subscript>,
    ) -> bool {
        let Some(subscript) = subscript else {
            return false;
        };
        if subscript.selector().is_some() {
            return false;
        }
        if matches!(
            subscript.interpretation,
            shuck_ast::SubscriptInterpretation::Associative
        ) {
            return false;
        }

        !owner_name
            .is_some_and(|name| self.assoc_binding_visible_for_subscript(name, owner_name_span, subscript))
    }

    fn assoc_binding_visible_for_subscript(
        &mut self,
        owner_name: &Name,
        owner_name_span: Option<Span>,
        subscript: &Subscript,
    ) -> bool {
        let current_scope = self.semantic.scope_at(subscript.span().start.offset);
        let key = (
            owner_name.clone(),
            current_scope,
            owner_name_span.map(FactSpan::new),
        );
        if let Some(result) = self.assoc_binding_visibility_memo.get(&key) {
            return *result;
        }

        let visible = if let Some(binding) =
            self.prior_visible_binding_for_subscript(owner_name, subscript.span())
        {
            binding.attributes.contains(BindingAttributes::ASSOC)
        } else {
            self.assoc_binding_visible_from_named_callers(owner_name, subscript.span())
        };
        self.assoc_binding_visibility_memo.insert(key, visible);
        visible
    }

    fn assoc_binding_visible_from_named_callers(&self, owner_name: &Name, span: Span) -> bool {
        let Some(function_names) = self.named_function_scope_names(span.start.offset) else {
            return false;
        };

        let mut seen = FxHashSet::default();
        let mut worklist = function_names.to_vec();

        while let Some(function_name) = worklist.pop() {
            if !seen.insert(function_name.clone()) {
                continue;
            }

            for call_site in self.semantic.call_sites_for(&function_name) {
                if let Some(binding) =
                    self.visible_binding_for_caller_assoc_lookup(owner_name, call_site.name_span)
                {
                    if binding.attributes.contains(BindingAttributes::ASSOC) {
                        return true;
                    }
                    continue;
                }

                if let Some(caller_names) = self.named_function_scope_names(call_site.name_span.start.offset)
                {
                    worklist.extend(caller_names.iter().cloned());
                }
            }
        }

        false
    }

    fn prior_visible_binding_for_subscript(
        &self,
        owner_name: &Name,
        span: Span,
    ) -> Option<&shuck_semantic::Binding> {
        let current_scope = self.semantic.scope_at(span.start.offset);
        self.semantic
            .visible_binding_for_assoc_lookup(owner_name, current_scope, span)
    }

    fn visible_binding_for_caller_assoc_lookup(
        &self,
        owner_name: &Name,
        span: Span,
    ) -> Option<&shuck_semantic::Binding> {
        let current_scope = self.semantic.scope_at(span.start.offset);
        self.semantic
            .visible_binding_for_assoc_lookup(owner_name, current_scope, span)
    }

    fn named_function_scope_names(&self, offset: usize) -> Option<&[Name]> {
        let scope = self.semantic.scope_at(offset);
        self.semantic
            .ancestor_scopes(scope)
            .find_map(|scope_id| match &self.semantic.scope(scope_id).kind {
                shuck_semantic::ScopeKind::Function(
                    shuck_semantic::FunctionScopeKind::Named(names),
                ) => Some(names.as_slice()),
                _ => None,
            })
    }

    fn collect_array_index_arithmetic_spans(&mut self, word: &Word) {
        self.arithmetic
            .array_index_arithmetic_spans
            .extend(span::arithmetic_expansion_part_spans(word));
    }

    fn collect_dollar_prefixed_indexed_subscript_spans(&mut self, word: &Word) {
        collect_dollar_prefixed_indexed_subscript_word_spans(
            word,
            self.source,
            &mut self.arithmetic.dollar_in_arithmetic_spans,
        );
    }
}

fn pattern_has_glob_structure(pattern: &Pattern, source: &str) -> bool {
    pattern.parts_with_spans().any(|(part, span)| match part {
        PatternPart::AnyString | PatternPart::AnyChar | PatternPart::CharClass(_) => true,
        PatternPart::Group { .. } => true,
        PatternPart::Literal(text) => literal_text_has_glob_bracket(text.as_str(source, span)),
        PatternPart::Word(word) => word.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::Literal(text)
                    if literal_text_has_glob_bracket(text.as_str(source, part.span))
            )
        }),
    })
}

fn literal_text_has_glob_bracket(text: &str) -> bool {
    text.contains('[') || text.contains(']')
}

fn pattern_is_arithmetic_only(pattern: &Pattern) -> bool {
    pattern.parts.iter().all(|part| match &part.kind {
        PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => true,
        PatternPart::Word(word) => word_is_arithmetic_only(word),
        PatternPart::CharClass(_) | PatternPart::Group { .. } => false,
    })
}

fn word_is_arithmetic_only(word: &Word) -> bool {
    word.parts.iter().all(word_part_is_arithmetic_only)
}

fn word_part_is_arithmetic_only(part: &WordPartNode) -> bool {
    match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::ArithmeticExpansion { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts.iter().all(word_part_is_arithmetic_only),
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
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
        | WordPart::ZshQualifiedGlob(_) => false,
    }
}


fn standalone_variable_name_from_word_parts(parts: &[WordPartNode]) -> Option<&str> {
    let [part] = parts else {
        return None;
    };

    match &part.kind {
        WordPart::Variable(name) => Some(name.as_str()),
        WordPart::Parameter(parameter) => match parameter.bourne() {
            Some(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none() =>
            {
                Some(reference.name.as_str())
            }
            _ => None,
        },
        WordPart::DoubleQuoted { parts, .. } => standalone_variable_name_from_word_parts(parts),
        WordPart::Literal(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::SingleQuoted { .. }
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
        | WordPart::ZshQualifiedGlob(_) => None,
    }
}

fn word_context_supports_operand_class(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::AssignmentValue
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::StringTestOperand
            | ExpansionContext::RegexOperand
            | ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern
    )
}

fn word_has_literal_affixes(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
        )
    })
}

fn word_contains_shell_quoting_literals(word: &Word, source: &str) -> bool {
    word_parts_contain_shell_quoting_literals(&word.parts, source)
}

fn word_parts_contain_shell_quoting_literals(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => text_contains_shell_quoting_literals(
            text.as_str(source, part.span),
            ShellQuotingLiteralTextContext::ShellContinuationAware,
        ),
        WordPart::SingleQuoted { value, .. } => text_contains_shell_quoting_literals(
            value.slice(source),
            ShellQuotingLiteralTextContext::LiteralBackslashNewlines,
        ),
        WordPart::DoubleQuoted { parts, .. } => {
            word_parts_contain_shell_quoting_literals(parts, source)
        }
        _ => false,
    })
}

#[derive(Clone, Copy)]
enum ShellQuotingLiteralTextContext {
    ShellContinuationAware,
    LiteralBackslashNewlines,
}

fn text_contains_shell_quoting_literals(
    text: &str,
    context: ShellQuotingLiteralTextContext,
) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '\\' {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] == '\\' {
            end += 1;
        }
        if chars.get(end).is_some_and(|next| {
            matches!(next, '"' | '\'')
                || (next.is_whitespace()
                    && (matches!(
                        context,
                        ShellQuotingLiteralTextContext::LiteralBackslashNewlines
                    ) || !matches!(next, '\n' | '\r')))
        }) {
            return true;
        }

        index = end;
    }

    false
}

fn is_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn is_scannable_simple_arithmetic_subscript_text(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && (is_shell_variable_name(trimmed) || trimmed.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_simple_arithmetic_reference_subscript(subscript: &Subscript, source: &str) -> bool {
    subscript.selector().is_none()
        && !subscript.syntax_text(source).contains('$')
        && matches!(
            subscript.arithmetic_ast.as_ref().map(|expr| &expr.kind),
            Some(ArithmeticExpr::Variable(_) | ArithmeticExpr::Number(_))
        )
}

fn is_arithmetic_variable_reference_word(word: &Word, source: &str) -> bool {
    matches!(word.parts.as_slice(), [part] if match &part.kind {
        WordPart::Variable(name) => is_shell_variable_name(name.as_str()),
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if is_shell_variable_name(reference.name.as_str())
                    && reference
                        .subscript
                        .as_ref()
                        .is_none_or(|subscript| {
                            is_simple_arithmetic_reference_subscript(subscript, source)
                        })
        ),
        _ => false,
    })
}

fn collect_arithmetic_command_spans(
    expression: &ArithmeticExprNode,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    query::visit_arithmetic_words(expression, &mut |word| {
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            true,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

fn collect_slice_arithmetic_expression_spans(
    expression: &ArithmeticExprNode,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    query::visit_arithmetic_words(expression, &mut |word| {
        collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
            &word.parts,
            source,
            dollar_spans,
        );
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            false,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

fn collect_arithmetic_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    if !text.slice(source).contains('$') {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

fn collect_dollar_prefixed_arithmetic_variable_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }

        let Some(next) = bytes.get(index + 1).copied() else {
            break;
        };

        let match_end = if next == b'{' {
            let name_start = index + 2;
            let Some(first) = bytes.get(name_start).copied() else {
                index += 1;
                continue;
            };
            if !(first == b'_' || first.is_ascii_alphabetic()) {
                index += 1;
                continue;
            }

            let mut name_end = name_start + 1;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }

            match bytes.get(name_end).copied() {
                Some(b'}') => name_end + 1,
                Some(b'[') => {
                    let subscript_start = name_end + 1;
                    let Some(subscript_end_rel) = text[subscript_start..].find(']') else {
                        index += 1;
                        continue;
                    };
                    let subscript_end = subscript_start + subscript_end_rel;
                    if bytes.get(subscript_end + 1) != Some(&b'}')
                        || !is_scannable_simple_arithmetic_subscript_text(
                            &text[subscript_start..subscript_end],
                        )
                    {
                        index += 1;
                        continue;
                    }

                    subscript_end + 2
                }
                _ => {
                    index += 1;
                    continue;
                }
            }
        } else if next == b'_' || next.is_ascii_alphabetic() {
            let mut name_end = index + 2;
            while let Some(byte) = bytes.get(name_end).copied() {
                if byte == b'_' || byte.is_ascii_alphanumeric() {
                    name_end += 1;
                } else {
                    break;
                }
            }
            name_end
        } else {
            index += 1;
            continue;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push(Span::from_positions(start, end));
        index = match_end;
    }
}

fn collect_dollar_prefixed_indexed_subscript_word_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in &word.parts {
        match &part.kind {
            WordPart::Variable(name) if is_shell_variable_name(name.as_str()) => {
                spans.push(part.span);
            }
            WordPart::Variable(_) => {}
            WordPart::Parameter(parameter) => {
                if matches!(
                    parameter.bourne(),
                    Some(BourneParameterExpansion::Access { reference })
                        if is_shell_variable_name(reference.name.as_str())
                            && reference
                                .subscript
                                .as_ref()
                                .is_none_or(|subscript| {
                                    is_simple_arithmetic_reference_subscript(subscript, source)
                                })
                ) {
                    spans.push(part.span);
                }
            }
            WordPart::Literal(_)
            | WordPart::DoubleQuoted { .. }
            | WordPart::SingleQuoted { .. }
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
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_wrapped_arithmetic_spans_in_word(
    word: &Word,
    source: &str,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    let text = word.span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 2 < bytes.len() {
        if bytes[index] != b'$' || bytes[index + 1] != b'(' || bytes[index + 2] != b'(' {
            index += 1;
            continue;
        }

        let mut depth = 1usize;
        let mut cursor = index + 3;
        let mut matched = false;

        while cursor < bytes.len() {
            if cursor + 2 < bytes.len()
                && bytes[cursor] == b'$'
                && bytes[cursor + 1] == b'('
                && bytes[cursor + 2] == b'('
            {
                depth += 1;
                cursor += 3;
                continue;
            }

            match bytes[cursor] {
                b'(' => {
                    depth += 1;
                    cursor += 1;
                }
                b')' => {
                    if depth == 1 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' {
                        let expr_start = index + 3;
                        let expr_end = cursor;
                        let start = word.span.start.advanced_by(&text[..expr_start]);
                        let end = start.advanced_by(&text[expr_start..expr_end]);
                        let expression_span = Span::from_positions(start, end);
                        collect_dollar_prefixed_arithmetic_variable_spans(
                            expression_span,
                            source,
                            dollar_spans,
                        );
                        collect_wrapped_arithmetic_command_substitution_spans(
                            expression_span,
                            source,
                            command_substitution_spans,
                        );
                        index = cursor + 2;
                        matched = true;
                        break;
                    }

                    depth = depth.saturating_sub(1);
                    cursor += 1;
                }
                _ => {
                    cursor += 1;
                }
            }
        }

        if !matched {
            break;
        }
    }
}

fn collect_wrapped_arithmetic_command_substitution_spans(
    span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if !is_unescaped_dollar(bytes, index)
            || bytes[index + 1] != b'('
            || bytes.get(index + 2) == Some(&b'(')
        {
            index += 1;
            continue;
        }

        let Some(end) = find_command_substitution_end(bytes, index) else {
            break;
        };

        let start = span.start.advanced_by(&text[..index]);
        let end_pos = start.advanced_by(&text[index..end]);
        spans.push(Span::from_positions(start, end_pos));
        index = end;
    }
}

fn is_unescaped_dollar(bytes: &[u8], index: usize) -> bool {
    if bytes.get(index) != Some(&b'$') {
        return false;
    }

    let mut backslash_count = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslash_count += 1;
        cursor -= 1;
    }

    backslash_count.is_multiple_of(2)
}

fn find_command_substitution_end(bytes: &[u8], start: usize) -> Option<usize> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut paren_depth = 0usize;
    let mut cursor = start + 2;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(bytes, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 => return Some(cursor + 1),
            b')' => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn find_wrapped_arithmetic_end(bytes: &[u8], start: usize) -> Option<usize> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut paren_depth = 0usize;
    let mut cursor = start + 3;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(bytes, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 && cursor + 1 < bytes.len() && bytes[cursor + 1] == b')' => {
                return Some(cursor + 2);
            }
            b')' if paren_depth > 0 => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn find_process_substitution_end(bytes: &[u8], start: usize) -> Option<usize> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut paren_depth = 0usize;
    let mut cursor = start + 2;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'{'
        {
            cursor = find_runtime_parameter_closing_brace(text, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && matches!(bytes[cursor], b'<' | b'>')
            && bytes[cursor + 1] == b'('
        {
            cursor = find_process_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' => cursor = skip_single_quoted(bytes, cursor + 1)?,
            b'"' => cursor = skip_double_quoted(bytes, cursor + 1)?,
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            b'(' => {
                paren_depth += 1;
                cursor += 1;
            }
            b')' if paren_depth == 0 => return Some(cursor + 1),
            b')' => {
                paren_depth -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn skip_single_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\'' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn skip_double_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;

    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }

        if cursor + 2 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
            && bytes[cursor + 2] == b'('
        {
            cursor = find_wrapped_arithmetic_end(bytes, cursor)?;
            continue;
        }

        if cursor + 1 < bytes.len()
            && is_unescaped_dollar(bytes, cursor)
            && bytes[cursor + 1] == b'('
        {
            cursor = find_command_substitution_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'"' => return Some(cursor + 1),
            b'`' => cursor = skip_backticks(bytes, cursor + 1)?,
            _ => cursor += 1,
        }
    }

    None
}

fn skip_backticks(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
            continue;
        }
        if bytes[cursor] == b'`' {
            return Some(cursor + 1);
        }
        cursor += 1;
    }
    None
}

fn word_needs_wrapped_arithmetic_fallback(word: &Word, source: &str) -> bool {
    parts_need_wrapped_arithmetic_fallback(&word.parts, source)
}

fn parts_need_wrapped_arithmetic_fallback(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            parts_need_wrapped_arithmetic_fallback(parts, source)
        }
        WordPart::Substring {
            offset_ast: None,
            offset,
            ..
        }
        | WordPart::ArraySlice {
            offset_ast: None,
            offset,
            ..
        } => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        WordPart::Parameter(parameter) => {
            parameter_needs_wrapped_arithmetic_fallback(parameter, source)
        }
        _ => false,
    })
}

fn parameter_needs_wrapped_arithmetic_fallback(
    parameter: &ParameterExpansion,
    source: &str,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
            offset_ast: None,
            offset,
            ..
        }) => offset.is_source_backed() && offset.slice(source).starts_with("$(("),
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => {
                parameter_needs_wrapped_arithmetic_fallback(parameter, source)
            }
            ZshExpansionTarget::Word(word) => word_needs_wrapped_arithmetic_fallback(word, source),
            ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => false,
        },
        _ => false,
    }
}

fn collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
    parts: &[WordPartNode],
    source: &str,
    dollar_spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                    parts,
                    source,
                    dollar_spans,
                )
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                let mut ignored_command_substitution_spans = Vec::new();
                if let Some(expression) = expression_ast {
                    query::visit_arithmetic_words(expression, &mut |word| {
                        collect_arithmetic_context_spans_in_word(
                            word,
                            source,
                            true,
                            dollar_spans,
                            &mut ignored_command_substitution_spans,
                        );
                    });
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        true,
                        dollar_spans,
                        &mut ignored_command_substitution_spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_arithmetic_context_spans_in_word(
    word: &Word,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    if collect_dollar_spans && is_arithmetic_variable_reference_word(word, source) {
        dollar_spans.push(word.span);
    }

    for part in &word.parts {
        if let WordPart::CommandSubstitution { .. } = &part.kind {
            command_substitution_spans.push(part.span);
        }
    }

    collect_arithmetic_expansion_spans_from_parts(
        &word.parts,
        source,
        collect_dollar_spans,
        dollar_spans,
        command_substitution_spans,
    );
}

fn collect_arithmetic_spans_in_parameter_operator(
    operator: &ParameterOp,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_expansion_spans_from_parts(
            &replacement_word_ast.parts,
            source,
            collect_dollar_spans,
            dollar_spans,
            command_substitution_spans,
        ),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::RemovePrefixShort { .. }
        | ParameterOp::RemovePrefixLong { .. }
        | ParameterOp::RemoveSuffixShort { .. }
        | ParameterOp::RemoveSuffixLong { .. }
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn collect_arithmetic_expansion_spans_from_parts(
    parts: &[WordPartNode],
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_arithmetic_expansion_spans_from_parts(
                parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    query::visit_arithmetic_words(expression, &mut |word| {
                        collect_arithmetic_context_spans_in_word(
                            word,
                            source,
                            collect_dollar_spans,
                            dollar_spans,
                            command_substitution_spans,
                        );
                    });
                } else {
                    collect_arithmetic_expansion_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        collect_dollar_spans,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Parameter(parameter) => collect_arithmetic_spans_in_parameter_expansion(
                parameter,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &offset_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &length_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_from_parts(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_arithmetic_update_operator_spans_from_parts(parts, source, spans)
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &expression_word_ast.parts,
                        source,
                        spans,
                    );
                }
            }
            WordPart::Parameter(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion(
                    parameter, source, spans,
                )
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
                collect_arithmetic_update_operator_spans_in_parameter_operator(
                    operator, source, spans,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans)
            }
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        spans,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_var_ref(
    reference: &VarRef,
    source: &str,
    spans: &mut Vec<Span>,
) {
    query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_arithmetic_update_operator_spans_from_parts(&word.parts, source, spans);
    });
}

fn collect_arithmetic_update_operator_spans_in_parameter_expansion(
    parameter: &ParameterExpansion,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
                if let Some(operator) = operator.as_ref() {
                    collect_arithmetic_update_operator_spans_in_parameter_operator(
                        operator, source, spans,
                    );
                }
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &operand_word_ast.parts,
                        source,
                        spans,
                    );
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
                collect_arithmetic_update_operator_spans_in_parameter_operator(
                    operator, source, spans,
                );
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &operand_word_ast.parts,
                        source,
                        spans,
                    );
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        spans,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => {
                collect_arithmetic_update_operator_spans_in_var_ref(reference, source, spans);
            }
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion(
                    parameter, source, spans,
                );
            }
            ZshExpansionTarget::Word(word) => {
                collect_arithmetic_update_operator_spans_from_parts(&word.parts, source, spans);
            }
            ZshExpansionTarget::Empty => {}
        },
    }
}

fn collect_arithmetic_update_operator_spans_in_parameter_operator(
    operator: &ParameterOp,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_update_operator_spans_from_parts(
            &replacement_word_ast.parts,
            source,
            spans,
        ),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::RemovePrefixShort { .. }
        | ParameterOp::RemovePrefixLong { .. }
        | ParameterOp::RemoveSuffixShort { .. }
        | ParameterOp::RemoveSuffixLong { .. }
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn collect_arithmetic_spans_in_var_ref(
    reference: &VarRef,
    source: &str,
    _collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    query::visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
            &word.parts,
            source,
            dollar_spans,
        );
        collect_arithmetic_context_spans_in_word(
            word,
            source,
            false,
            dollar_spans,
            command_substitution_spans,
        );
    });
}

fn collect_arithmetic_spans_in_parameter_expansion(
    parameter: &ParameterExpansion,
    source: &str,
    collect_dollar_spans: bool,
    dollar_spans: &mut Vec<Span>,
    command_substitution_spans: &mut Vec<Span>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                collect_arithmetic_spans_in_parameter_operator(
                    operator,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arithmetic_spans_in_var_ref(
                    reference,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                );
                if let Some(expression) = offset_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &offset_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &offset_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_slice_arithmetic_expression_spans(
                        expression,
                        source,
                        dollar_spans,
                        command_substitution_spans,
                    );
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_dollar_spans_in_nested_arithmetic_expansions_from_parts(
                        &length_word_ast.parts,
                        source,
                        dollar_spans,
                    );
                    collect_arithmetic_expansion_spans_from_parts(
                        &length_word_ast.parts,
                        source,
                        false,
                        dollar_spans,
                        command_substitution_spans,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => collect_arithmetic_spans_in_var_ref(
                reference,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_spans_in_parameter_expansion(
                    parameter,
                    source,
                    collect_dollar_spans,
                    dollar_spans,
                    command_substitution_spans,
                )
            }
            ZshExpansionTarget::Word(word) => collect_arithmetic_expansion_spans_from_parts(
                &word.parts,
                source,
                collect_dollar_spans,
                dollar_spans,
                command_substitution_spans,
            ),
            ZshExpansionTarget::Empty => {}
        },
    }
}

fn word_classification_from_analysis(analysis: ExpansionAnalysis) -> WordClassification {
    WordClassification {
        quote: analysis.quote,
        literalness: analysis.literalness,
        expansion_kind: match (analysis.has_scalar_expansion(), analysis.array_valued) {
            (false, false) => WordExpansionKind::None,
            (true, false) => WordExpansionKind::Scalar,
            (false, true) => WordExpansionKind::Array,
            (true, true) => WordExpansionKind::Mixed,
        },
        substitution_shape: if analysis.substitution_shape == WordSubstitutionShape::None {
            WordSubstitutionShape::None
        } else if analysis.substitution_shape == WordSubstitutionShape::Plain {
            WordSubstitutionShape::Plain
        } else {
            WordSubstitutionShape::Mixed
        },
    }
}

fn double_quoted_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_expansion_spans(&word.parts, false, &mut spans);
    spans
}

fn collect_double_quoted_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_double_quoted_expansion_spans(parts, true, spans);
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
            | WordPart::ZshQualifiedGlob(_)
                if inside_double_quotes =>
            {
                spans.push(part.span)
            }
            WordPart::Literal(_) => {}
            _ => {}
        }
    }
}


pub fn leading_literal_word_prefix(word: &Word, source: &str) -> String {
    let mut prefix = String::new();
    collect_leading_literal_word_parts(&word.parts, source, &mut prefix);
    prefix
}

fn collect_leading_literal_word_parts(
    parts: &[WordPartNode],
    source: &str,
    prefix: &mut String,
) -> bool {
    for part in parts {
        if !collect_leading_literal_word_part(part, source, prefix) {
            return false;
        }
    }
    true
}

fn collect_leading_literal_word_part(
    part: &WordPartNode,
    source: &str,
    prefix: &mut String,
) -> bool {
    match &part.kind {
        WordPart::Literal(text) => {
            prefix.push_str(text.as_str(source, part.span));
            true
        }
        WordPart::SingleQuoted { value, .. } => {
            prefix.push_str(value.slice(source));
            true
        }
        WordPart::DoubleQuoted { parts, .. } => {
            collect_leading_literal_word_parts(parts, source, prefix)
        }
        _ => false,
    }
}

fn parse_wait_command(args: &[&Word], source: &str) -> WaitCommandFacts {
    let mut option_spans = Vec::new();
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if text.starts_with('-') && text != "-" {
            option_spans.push(word.span);
            index += 1;
            if wait_option_consumes_argument(&text) {
                index += 1;
            }
            continue;
        }

        break;
    }

    WaitCommandFacts {
        option_spans: option_spans.into_boxed_slice(),
    }
}

fn parse_ln_command<'a>(args: &[&'a Word], source: &str) -> Option<LnCommandFacts<'a>> {
    let mut index = 0usize;
    let mut saw_symbolic_flag = false;
    let mut target_directory_mode = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            match long {
                "symbolic" => saw_symbolic_flag = true,
                "target-directory" => {
                    target_directory_mode = true;
                    index += 1;
                    args.get(index)?;
                }
                "suffix" => {
                    index += 1;
                    args.get(index)?;
                }
                "backup"
                | "directory"
                | "force"
                | "interactive"
                | "logical"
                | "no-dereference"
                | "no-target-directory"
                | "physical"
                | "relative"
                | "verbose" => {}
                _ if long.starts_with("target-directory=") => {
                    target_directory_mode = true;
                }
                _ if long.starts_with("suffix=") => {}
                _ => return None,
            }

            index += 1;
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            match flag {
                's' => saw_symbolic_flag = true,
                't' => {
                    target_directory_mode = true;
                    if chars.peek().is_none() {
                        index += 1;
                        args.get(index)?;
                    }
                    break;
                }
                'S' => {
                    if chars.peek().is_none() {
                        index += 1;
                        args.get(index)?;
                    }
                    break;
                }
                'b' | 'd' | 'f' | 'F' | 'i' | 'L' | 'n' | 'P' | 'r' | 'T' | 'v' => {}
                _ => return None,
            }
        }

        index += 1;
    }

    if !saw_symbolic_flag {
        return None;
    }

    let operands = &args[index..];
    if operands.is_empty() {
        return None;
    }

    Some(LnCommandFacts {
        symlink_target_words: if target_directory_mode {
            operands.to_vec().into_boxed_slice()
        } else {
            vec![operands[0]].into_boxed_slice()
        },
    })
}

fn wait_option_consumes_argument(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };
    let Some(p_index) = flags.find('p') else {
        return false;
    };

    p_index + 1 == flags.len()
}

fn parse_mapfile_command(args: &[&Word], source: &str) -> MapfileCommandFacts {
    let mut input_fd = Some(0);
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" || !text.starts_with('-') || text == "-" || text.starts_with("--") {
            break;
        }

        let flags = &text[1..];
        let mut recognized = true;

        for (offset, flag) in flags.char_indices() {
            if !matches!(flag, 't' | 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's') {
                recognized = false;
                break;
            }

            if !mapfile_option_takes_argument(flag) {
                continue;
            }

            let remainder = &flags[offset + flag.len_utf8()..];
            let argument = if remainder.is_empty() {
                index += 1;
                args.get(index)
                    .and_then(|next| static_word_text(next, source))
            } else {
                Some(remainder.into())
            };

            if flag == 'u' {
                input_fd = argument.and_then(|value| value.parse::<i32>().ok());
            }

            break;
        }

        if !recognized {
            break;
        }

        index += 1;
    }

    MapfileCommandFacts { input_fd }
}

fn mapfile_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's')
}

fn parse_xargs_command(args: &[&Word], source: &str) -> XargsCommandFacts {
    let mut uses_null_input = false;
    let mut inline_replace_option_spans = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                break;
            }
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        if let Some(long) = text.strip_prefix("--") {
            if long == "null" {
                uses_null_input = true;
            }

            let consume_next_argument = xargs_long_option_requires_separate_argument(long);
            index += 1;
            if consume_next_argument {
                index += 1;
            }
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == '0' {
                uses_null_input = true;
            }
            if flag == 'i' {
                inline_replace_option_spans.push(word.span);
            }

            match xargs_short_option_argument_style(flag) {
                XargsShortOptionArgumentStyle::None => {}
                XargsShortOptionArgumentStyle::OptionalInlineOnly => break,
                XargsShortOptionArgumentStyle::Required => {
                    if chars.peek().is_none() {
                        consume_next_argument = true;
                    }
                    break;
                }
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    XargsCommandFacts {
        uses_null_input,
        inline_replace_option_spans: inline_replace_option_spans.into_boxed_slice(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XargsShortOptionArgumentStyle {
    None,
    OptionalInlineOnly,
    Required,
}

fn xargs_short_option_argument_style(flag: char) -> XargsShortOptionArgumentStyle {
    match flag {
        'e' | 'i' | 'l' => XargsShortOptionArgumentStyle::OptionalInlineOnly,
        'a' | 'E' | 'I' | 'L' | 'n' | 'P' | 's' | 'd' => XargsShortOptionArgumentStyle::Required,
        _ => XargsShortOptionArgumentStyle::None,
    }
}

fn xargs_long_option_requires_separate_argument(option: &str) -> bool {
    if option.contains('=') {
        return false;
    }

    matches!(
        option,
        "arg-file"
            | "delimiter"
            | "max-args"
            | "max-chars"
            | "max-lines"
            | "max-procs"
            | "process-slot-var"
    )
}

fn parse_expr_command(args: &[&Word], source: &str) -> Option<ExprCommandFacts> {
    let (string_helper_kind, string_helper_span) = expr_string_helper(args, source)
        .map_or((None, None), |(kind, span)| (Some(kind), Some(span)));

    Some(ExprCommandFacts {
        uses_arithmetic_operator: !expr_uses_string_form(args, source),
        string_helper_kind,
        string_helper_span,
    })
}

fn expr_uses_string_form(args: &[&Word], source: &str) -> bool {
    matches!(
        args.first()
            .and_then(|word| static_word_text(word, source))
            .as_deref(),
        Some("length" | "index" | "match" | "substr")
    ) || args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        .is_some_and(|text| matches!(text, ":" | "=" | "!=" | "<" | ">" | "<=" | ">=" | "=="))
}

fn expr_string_helper(args: &[&Word], source: &str) -> Option<(ExprStringHelperKind, Span)> {
    let word = args.first()?;
    let kind = match static_word_text(word, source).as_deref() {
        Some("length") => ExprStringHelperKind::Length,
        Some("index") => ExprStringHelperKind::Index,
        Some("match") => ExprStringHelperKind::Match,
        Some("substr") => ExprStringHelperKind::Substr,
        _ => return None,
    };

    Some((kind, word.span))
}

fn parse_exit_command<'a>(command: &'a Command, source: &str) -> Option<ExitCommandFacts<'a>> {
    let Command::Builtin(BuiltinCommand::Exit(exit)) = command else {
        return None;
    };
    let Some(status_word) = exit.code.as_ref() else {
        return Some(ExitCommandFacts {
            status_word: None,
            is_numeric_literal: false,
            status_is_static: false,
            status_has_literal_content: false,
        });
    };
    let status_text = static_word_text(status_word, source);

    Some(ExitCommandFacts {
        status_word: Some(status_word),
        is_numeric_literal: status_text.as_deref().is_some_and(|text| {
            !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
        }),
        status_is_static: status_text.is_some(),
        status_has_literal_content: word_contains_literal_content(status_word, source),
    })
}

fn word_contains_literal_content(word: &Word, source: &str) -> bool {
    word_parts_contain_literal_content(&word.parts, source)
}

fn word_parts_contain_literal_content(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => !text.as_str(source, part.span).is_empty(),
        WordPart::SingleQuoted { value, .. } => !value.slice(source).is_empty(),
        WordPart::DoubleQuoted { parts, .. } => word_parts_contain_literal_content(parts, source),
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
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

fn detect_sudo_family_invoker(
    command: &Command,
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<SudoFamilyInvoker> {
    let Command::Simple(command) = command else {
        return None;
    };
    let body_start = normalized.body_span.start.offset;
    let scan_all_words = normalized.body_words.is_empty();

    std::iter::once(&command.name)
        .chain(command.args.iter())
        // Unresolved sudo-family wrappers intentionally keep the wrapper marker
        // even when there is no statically known inner command.
        .take_while(|word| scan_all_words || word.span.start.offset < body_start)
        .filter_map(|word| static_word_text(word, source))
        .map(|word| word.strip_prefix('\\').unwrap_or(word.as_ref()).to_owned())
        .filter_map(|word| match word.as_str() {
            "sudo" => Some(SudoFamilyInvoker::Sudo),
            "doas" => Some(SudoFamilyInvoker::Doas),
            "run0" => Some(SudoFamilyInvoker::Run0),
            _ => None,
        })
        .last()
}

fn trap_action_word<'a>(command: &'a Command, source: &str) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    if static_word_text(&command.name, source).as_deref() != Some("trap") {
        return None;
    }

    let mut start = 0usize;

    if let Some(first) = command
        .args
        .first()
        .and_then(|word| static_word_text(word, source))
    {
        match first.as_ref() {
            "-p" | "-l" => return None,
            "--" => start = 1,
            _ => {}
        }
    }

    let action = command.args.get(start)?;
    command.args.get(start + 1)?;
    Some(action)
}
