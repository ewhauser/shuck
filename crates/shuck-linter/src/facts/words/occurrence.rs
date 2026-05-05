#[derive(Debug)]
pub struct WordNode<'a> {
    pub(super) key: FactSpan,
    pub(super) word: &'a Word,
    pub(super) analysis: ExpansionAnalysis,
    pub(super) derived: WordNodeDerived<'a>,
}

#[derive(Debug)]
pub(super) struct WordNodeDerived<'a> {
    pub(super) static_text: Option<&'a str>,
    pub(super) trailing_literal_char: Option<char>,
    pub(super) starts_with_extglob: bool,
    pub(super) has_literal_affixes: bool,
    pub(super) contains_shell_quoting_literals: bool,
    pub(super) active_expansion_spans: IdRange<Span>,
    pub(super) scalar_expansion_spans: IdRange<Span>,
    pub(super) unquoted_scalar_expansion_spans: IdRange<Span>,
    pub(super) array_expansion_spans: IdRange<Span>,
    pub(super) all_elements_array_expansion_spans: IdRange<Span>,
    pub(super) direct_all_elements_array_expansion_spans: IdRange<Span>,
    pub(super) unquoted_all_elements_array_expansion_spans: IdRange<Span>,
    pub(super) unquoted_array_expansion_spans: IdRange<Span>,
    pub(super) command_substitution_spans: IdRange<Span>,
    pub(super) unquoted_command_substitution_spans: IdRange<Span>,
    pub(super) unquoted_dollar_paren_command_substitution_spans: IdRange<Span>,
    pub(super) double_quoted_expansion_spans: IdRange<Span>,
    pub(super) unquoted_literal_between_double_quoted_segments_spans: IdRange<Span>,
}

#[derive(Debug)]
pub struct WordOccurrence {
    pub(super) node_id: WordNodeId,
    pub(super) command_id: CommandId,
    pub(super) nested_word_command: bool,
    pub(super) context: WordFactContext,
    pub(super) host_kind: WordFactHostKind,
    pub(super) runtime_literal: RuntimeLiteralAnalysis,
    pub(super) operand_class: Option<TestOperandClass>,
    pub(super) enclosing_expansion_context: Option<ExpansionContext>,
    pub(super) split_sensitive_unquoted_command_substitution_spans: IdRange<Span>,
    pub(super) array_assignment_split_scalar_expansion_spans: IdRange<Span>,
}

#[derive(Clone, Copy)]
pub struct WordOccurrenceRef<'facts, 'a> {
    pub(super) facts: &'facts LinterFacts<'a>,
    pub(super) id: WordOccurrenceId,
}

pub struct WordOccurrenceIter<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
    source: WordOccurrenceIterSource<'facts>,
    filter: WordOccurrenceFilter,
}

pub(super) enum WordOccurrenceIterSource<'facts> {
    All { next: usize },
    Ids(std::slice::Iter<'facts, WordOccurrenceId>),
}

#[derive(Clone, Copy)]
pub(super) enum WordOccurrenceFilter {
    Any,
    NonArithmetic,
    ArithmeticCommand,
    Expansion(ExpansionContext),
    CaseSubject,
}

impl<'facts, 'a> WordOccurrenceIter<'facts, 'a> {
    pub(super) fn all(facts: &'facts LinterFacts<'a>, filter: WordOccurrenceFilter) -> Self {
        Self {
            facts,
            source: WordOccurrenceIterSource::All { next: 0 },
            filter,
        }
    }

    pub(super) fn ids(
        facts: &'facts LinterFacts<'a>,
        ids: &'facts [WordOccurrenceId],
        filter: WordOccurrenceFilter,
    ) -> Self {
        Self {
            facts,
            source: WordOccurrenceIterSource::Ids(ids.iter()),
            filter,
        }
    }

    pub fn iter(self) -> Self {
        self
    }

    fn accepts(&self, id: WordOccurrenceId) -> bool {
        let occurrence = self.facts.word_occurrence(id);
        match self.filter {
            WordOccurrenceFilter::Any => true,
            WordOccurrenceFilter::NonArithmetic => {
                occurrence.context != WordFactContext::ArithmeticCommand
            }
            WordOccurrenceFilter::ArithmeticCommand => {
                occurrence.context == WordFactContext::ArithmeticCommand
            }
            WordOccurrenceFilter::Expansion(context) => {
                occurrence.context == WordFactContext::Expansion(context)
            }
            WordOccurrenceFilter::CaseSubject => self.facts.word_occurrence_ref(id).is_case_subject(),
        }
    }
}

impl<'facts, 'a> Iterator for WordOccurrenceIter<'facts, 'a> {
    type Item = WordOccurrenceRef<'facts, 'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let id = match &mut self.source {
                WordOccurrenceIterSource::All { next } => {
                    let id = WordOccurrenceId::new(*next);
                    *next += 1;
                    (id.index() < self.facts.word_occurrences.len()).then_some(id)
                }
                WordOccurrenceIterSource::Ids(ids) => ids.next().copied(),
            }?;

            if self.accepts(id) {
                return Some(self.facts.word_occurrence_ref(id));
            }
        }
    }
}

impl<'facts, 'a> WordOccurrenceRef<'facts, 'a> {
    fn occurrence(self) -> &'facts WordOccurrence {
        self.facts.word_occurrence(self.id)
    }

    fn node(self) -> &'facts WordNode<'a> {
        self.facts.word_node(self.occurrence().node_id)
    }

    fn derived(self) -> &'facts WordNodeDerived<'a> {
        self.facts.word_node_derived(self.occurrence().node_id)
    }

    pub(super) fn word(self) -> &'a Word {
        self.node().word
    }

    pub fn key(self) -> FactSpan {
        self.node().key
    }

    pub fn span(self) -> Span {
        self.word().span
    }

    pub fn single_double_quoted_replacement(self, source: &str) -> Box<str> {
        rewrite_word_as_single_double_quoted_string(self.word(), source, None)
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

    pub fn host_expansion_context(self) -> Option<ExpansionContext> {
        self.expansion_context()
            .or(self.occurrence().enclosing_expansion_context)
    }

    pub fn is_case_subject(self) -> bool {
        self.context() == WordFactContext::CaseSubject
    }

    pub fn is_arithmetic_command(self) -> bool {
        self.context() == WordFactContext::ArithmeticCommand
    }

    pub fn part_is_inside_backtick_escaped_double_quotes(
        self,
        part_span: Span,
        source: &str,
    ) -> bool {
        let Some(backtick_span) =
            self.facts
                .backtick_substitution_spans()
                .iter()
                .copied()
                .find(|span| {
                    span.start.offset <= part_span.start.offset
                        && span.end.offset >= part_span.end.offset
                })
        else {
            return false;
        };

        let mut index = backtick_span.start.offset.saturating_add('`'.len_utf8());
        let limit = part_span.start.offset.min(
            backtick_span
                .end
                .offset
                .saturating_sub('`'.len_utf8()),
        );
        let mut in_single_quote = false;
        let mut in_escaped_double_quote = false;

        while index < limit {
            let Some(ch) = source[index..].chars().next() else {
                break;
            };
            let ch_len = ch.len_utf8();

            match ch {
                '\'' if !in_escaped_double_quote => {
                    in_single_quote = !in_single_quote;
                    index += ch_len;
                }
                '\\' if !in_single_quote => {
                    let next_index = index + ch_len;
                    let Some(escaped) = source[next_index..].chars().next() else {
                        break;
                    };
                    if escaped == '"' {
                        in_escaped_double_quote = !in_escaped_double_quote;
                    }
                    index = next_index + escaped.len_utf8();
                }
                _ => {
                    index += ch_len;
                }
            }
        }

        in_escaped_double_quote
    }

    pub fn host_kind(self) -> WordFactHostKind {
        self.occurrence().host_kind
    }

    pub fn analysis(self) -> ExpansionAnalysis {
        self.node().analysis
    }

    pub fn can_expand_to_multiple_fields_at_runtime(self, locator: Locator<'_>) -> bool {
        let analysis = self.analysis();
        let runtime_hazards = self.runtime_literal().hazards;

        runtime_hazards.pathname_matching
            || runtime_hazards.brace_fanout
            || analysis.hazards.pathname_matching
            || analysis.hazards.brace_fanout
            || analysis.array_valued
            || analysis.can_expand_to_multiple_fields
            || self.has_direct_all_elements_array_expansion_in_source(locator)
    }

    pub fn is_single_for_list_item(self, locator: Locator<'_>) -> bool {
        if self.context() != WordFactContext::Expansion(ExpansionContext::ForList) {
            return false;
        }

        let analysis = self.analysis();
        if analysis.quote == WordQuote::FullyQuoted
            && analysis.literalness == WordLiteralness::Expanded
            && self.double_quoted_scalar_affix_span().is_none()
        {
            return false;
        }

        !self.can_expand_to_multiple_fields_at_runtime(locator)
    }

    pub fn runtime_literal(self) -> RuntimeLiteralAnalysis {
        self.occurrence().runtime_literal
    }

    pub fn glob_failure_behavior(self) -> GlobFailureBehavior {
        self.runtime_literal().glob_failure_behavior
    }

    pub fn glob_dot_behavior(self) -> GlobDotBehavior {
        self.runtime_literal().glob_dot_behavior
    }

    pub fn glob_pattern_behavior(self) -> GlobPatternBehavior {
        self.runtime_literal().glob_pattern_behavior
    }

    pub fn classification(self) -> WordClassification {
        word_classification_from_analysis(self.analysis())
    }

    pub fn operand_class(self) -> Option<TestOperandClass> {
        self.occurrence().operand_class
    }

    pub fn static_text(self) -> Option<Cow<'a, str>> {
        self.static_text_from_source(self.facts.source)
    }

    pub fn static_text_cow(self, source: &'a str) -> Option<Cow<'a, str>> {
        self.static_text_from_source(source)
    }

    fn static_text_from_source(self, source: &'a str) -> Option<Cow<'a, str>> {
        self.derived()
            .static_text
            .map(Cow::Borrowed)
            .or_else(|| static_word_text(self.word(), source))
    }

    pub fn trailing_literal_char(self) -> Option<char> {
        self.derived().trailing_literal_char
    }

    pub fn contains_template_placeholder(self, source: &str) -> bool {
        contains_template_placeholder_text_in_word(self.span().slice(source))
    }

    pub fn has_suspicious_quoted_command_trailer(self, source: &str) -> bool {
        quoted_command_name_has_suspicious_ending(
            self.span().slice(source),
            self.trailing_literal_char(),
        )
    }

    pub fn has_hash_suffix(self, source: &str) -> bool {
        let text = self.span().slice(source);
        text != "#" && text.ends_with('#')
    }

    pub fn is_plain_scalar_reference(self) -> bool {
        word_is_plain_scalar_reference(self.word())
    }

    pub fn is_plain_parameter_reference(self) -> bool {
        word_is_plain_parameter_reference(self.word())
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
        self.facts.fact_store.word_spans(self.derived().active_expansion_spans)
    }

    pub fn expansion_span_is_zsh_force_glob_parameter(self, span: Span) -> bool {
        shuck_ast::word_span_is_zsh_force_glob_parameter(self.word(), span)
    }

    pub fn scalar_expansion_spans(self) -> &'facts [Span] {
        self.facts.fact_store.word_spans(self.derived().scalar_expansion_spans)
    }

    pub fn unquoted_scalar_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_scalar_expansion_spans)
    }

    pub fn array_assignment_split_scalar_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.occurrence().array_assignment_split_scalar_expansion_spans)
    }

    pub fn array_expansion_spans(self) -> &'facts [Span] {
        self.facts.fact_store.word_spans(self.derived().array_expansion_spans)
    }

    pub fn all_elements_array_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().all_elements_array_expansion_spans)
    }

    pub fn direct_all_elements_array_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().direct_all_elements_array_expansion_spans)
    }

    pub fn unquoted_all_elements_array_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_all_elements_array_expansion_spans)
    }

    pub fn unquoted_array_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_array_expansion_spans)
    }

    pub fn command_substitution_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().command_substitution_spans)
    }

    pub fn unquoted_command_substitution_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_command_substitution_spans)
    }

    pub fn split_sensitive_unquoted_command_substitution_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.occurrence().split_sensitive_unquoted_command_substitution_spans)
    }

    pub fn unquoted_dollar_paren_command_substitution_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_dollar_paren_command_substitution_spans)
    }

    pub fn double_quoted_expansion_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().double_quoted_expansion_spans)
    }

    pub fn single_quoted_equivalent_if_plain_double_quoted(self, source: &str) -> Option<String> {
        single_quoted_equivalent_if_plain_double_quoted_word(self.word(), source)
    }

    pub fn unquoted_literal_between_double_quoted_segments_spans(self) -> &'facts [Span] {
        self.facts
            .fact_store
            .word_spans(self.derived().unquoted_literal_between_double_quoted_segments_spans)
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

    pub fn diagnostic_part_span(
        self,
        part: &WordPart,
        part_span: Span,
        locator: Locator<'_>,
    ) -> Span {
        let source = locator.source();
        let adjusted = match part {
            WordPart::Variable(name) => {
                let expected = format!("${}", name.as_str());
                if part_span.slice(source) == expected {
                    part_span
                } else {
                    let search_start = part_span.start.offset.saturating_sub(1);
                    let search_end = (part_span.end.offset + 1).min(source.len());
                    source
                        .get(search_start..search_end)
                        .and_then(|window| window.find(&expected))
                        .map_or(part_span, |relative_start| {
                            let start_offset = search_start + relative_start;
                            let end_offset = start_offset + expected.len();
                            let start = Position::new().advanced_by(&source[..start_offset]);
                            let end = Position::new().advanced_by(&source[..end_offset]);
                            Span::from_positions(start, end)
                        })
                }
            }
            WordPart::Parameter(_) | WordPart::ParameterExpansion { .. } => {
                shellcheck_parameter_span_inside_escaped_quotes(part_span, source)
                    .unwrap_or(part_span)
            }
            _ => return part_span,
        };

        word_spans::shellcheck_collapsed_backtick_part_span(
            adjusted,
            locator,
            self.facts.backtick_substitution_spans(),
        )
    }

    pub fn has_direct_all_elements_array_expansion_in_source(self, locator: Locator<'_>) -> bool {
        word_spans::word_has_direct_all_elements_array_expansion_in_source(self.word(), locator)
    }

    pub fn has_quoted_all_elements_array_slice(self) -> bool {
        word_spans::word_has_quoted_all_elements_array_slice(self.word())
    }

    pub fn double_quoted_scalar_affix_span(self) -> Option<Span> {
        word_spans::double_quoted_scalar_affix_span(self.word())
    }

    pub fn is_pure_positional_at_splat(self) -> bool {
        word_spans::word_is_pure_positional_at_splat(self.word())
    }

    pub fn quoted_unindexed_bash_source_span_in_source(self, source: &str) -> Option<Span> {
        word_spans::word_quoted_unindexed_bash_source_span_in_source(self.word(), source)
    }

    pub fn unquoted_glob_pattern_spans(self, source: &str) -> Vec<Span> {
        word_spans::word_unquoted_glob_pattern_spans(self.word(), source)
    }

    pub fn has_literal_glob_syntax(self, source: &str) -> bool {
        !self.unquoted_glob_pattern_spans(source).is_empty()
            || self
                .parts_with_spans()
                .any(|(part, _)| matches!(part, WordPart::ZshQualifiedGlob(_)))
    }

    pub fn active_literal_glob_spans(self, source: &str) -> Vec<Span> {
        let runtime = self.runtime_literal();
        word_spans::word_active_glob_pattern_spans(
            self.word(),
            source,
            runtime.pathname_expansion_behavior,
            runtime.glob_pattern_behavior,
        )
    }

    pub fn unquoted_glob_pattern_spans_outside_brace_expansion(self, source: &str) -> Vec<Span> {
        word_spans::word_unquoted_glob_pattern_spans_outside_brace_expansion(self.word(), source)
    }

    pub fn active_glob_spans_outside_brace_expansion(self, source: &str) -> Vec<Span> {
        let runtime = self.runtime_literal();
        word_spans::word_active_glob_pattern_spans_outside_brace_expansion(
            self.word(),
            source,
            runtime.pathname_expansion_behavior,
            runtime.glob_pattern_behavior,
        )
    }

    pub fn starts_with_active_glob_operator(self, source: &str) -> bool {
        let runtime = self.runtime_literal();
        if word_spans::word_starts_with_active_glob_group_operator(
            self.word(),
            source,
            runtime.pathname_expansion_behavior,
            runtime.glob_pattern_behavior,
        ) {
            return true;
        }

        self.facts
            .command(self.command_id())
            .shell_behavior()
            .shell_dialect()
            != shuck_semantic::ShellDialect::Zsh
            && self.starts_with_extglob()
    }

    pub fn suspicious_bracket_glob_spans(self, source: &str) -> Vec<Span> {
        let mut spans = word_spans::word_suspicious_bracket_glob_spans(self.word(), source);
        if self
            .facts
            .semantic
            .shell_behavior_at(self.span().start.offset)
            .brace_character_classes()
            .can_expand()
        {
            spans.extend(word_spans::word_suspicious_brace_character_class_spans(
                self.word(),
                source,
            ));
        }
        spans
    }

    pub fn standalone_literal_backslash_span(self, source: &str) -> Option<Span> {
        word_spans::word_standalone_literal_backslash_span(self.word(), source)
    }

    pub fn unquoted_assign_default_spans(self) -> Vec<Span> {
        word_spans::word_unquoted_assign_default_spans(self.word())
    }

    pub fn use_replacement_spans(self) -> Vec<Span> {
        word_spans::word_use_replacement_spans(self.word())
    }

    pub fn unquoted_star_parameter_spans(self) -> Vec<Span> {
        word_spans::word_unquoted_star_parameter_spans(
            self.word(),
            self.unquoted_array_expansion_spans(),
        )
    }

    pub fn unquoted_star_splat_spans(self) -> Vec<Span> {
        word_spans::word_unquoted_star_splat_spans(self.word())
    }

    pub fn unquoted_word_after_single_quoted_segment_spans(self, source: &str) -> Vec<Span> {
        word_spans::word_unquoted_word_after_single_quoted_segment_spans(self.word(), source)
    }

    pub fn unquoted_scalar_between_double_quoted_segments_spans(
        self,
        candidate_spans: &[Span],
    ) -> Vec<Span> {
        word_spans::word_unquoted_scalar_between_double_quoted_segments_spans(
            self.word(),
            candidate_spans,
        )
    }

    pub fn nested_dynamic_double_quote_spans(self) -> Vec<Span> {
        word_spans::word_nested_dynamic_double_quote_spans(self.word())
    }

    pub fn folded_positional_at_splat_span_in_source(self, source: &str) -> Option<Span> {
        word_spans::word_folded_positional_at_splat_span_in_source(self.word(), source)
    }

    pub fn folded_all_elements_array_span_in_source(self, locator: Locator<'_>) -> Option<Span> {
        word_spans::word_folded_all_elements_array_span_in_source(self.word(), locator)
    }

    pub fn zsh_flag_modifier_spans(self) -> Vec<Span> {
        word_spans::word_zsh_flag_modifier_spans(self.word())
    }

    pub fn zsh_nested_expansion_spans(self) -> Vec<Span> {
        word_spans::word_zsh_nested_expansion_spans(self.word())
    }

    pub fn nested_zsh_substitution_spans(self) -> Vec<Span> {
        word_spans::word_nested_zsh_substitution_spans(self.word())
    }

    pub fn brace_expansion_spans(self) -> Vec<Span> {
        self.word()
            .brace_syntax()
            .iter()
            .copied()
            .filter(|brace| self.brace_syntax_expands(*brace))
            .map(|brace| brace.span)
            .collect()
    }

    fn brace_syntax_expands(self, brace: shuck_ast::BraceSyntax) -> bool {
        if !matches!(brace.quote_context, BraceQuoteContext::Unquoted) {
            return false;
        }

        match brace.expansion_kind() {
            Some(
                shuck_ast::BraceExpansionKind::CommaList
                | shuck_ast::BraceExpansionKind::Sequence,
            ) => true,
            Some(shuck_ast::BraceExpansionKind::CharacterClass) => self
                .facts
                .semantic
                .shell_behavior_at(brace.span.start.offset)
                .brace_character_classes()
                .can_expand(),
            None => false,
        }
    }
}

pub(super) fn shellcheck_parameter_span_inside_escaped_quotes(span: Span, source: &str) -> Option<Span> {
    if span.start.line != span.end.line {
        return None;
    }

    let search_start = offset_for_line_column(
        source,
        span.start.line,
        span.start.column.saturating_sub(2).max(1),
    )?;
    let search_end = offset_for_line_column(
        source,
        span.end.line,
        span.end.column.saturating_add(3),
    )
    .or_else(|| line_end_offset(source, span.end.line))?;
    let window = source.get(search_start..search_end)?;
    let relative_dollar = window.find('$')?;
    let start_offset = search_start + relative_dollar;
    let start = Position::new().advanced_by(&source[..start_offset]);
    if start.line != span.start.line
        || start.column < span.start.column
        || start.column > span.start.column.saturating_add(2)
    {
        return None;
    }

    let span_start_offset = offset_for_line_column(source, span.start.line, span.start.column)?;
    let prefix = source.get(span_start_offset..start_offset)?;
    if !prefix.contains('"') && !prefix.contains('\\') {
        return None;
    }

    let end_offset = parameter_expansion_end_offset(source, start_offset)?;
    let end = Position::new().advanced_by(&source[..end_offset]);
    if end.line != span.end.line
        || end.column < span.end.column
        || end.column > span.end.column.saturating_add(3)
    {
        return None;
    }

    if start.column == span.start.column && end.column == span.end.column {
        return None;
    }

    Some(Span::from_positions(start, end))
}

pub(super) fn offset_for_line_column(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }

    let mut current_line = 1usize;
    let mut line_start = 0usize;
    for (offset, ch) in source.char_indices() {
        if current_line == line {
            return offset_for_column_in_line(source, line_start, column);
        }
        if ch == '\n' {
            current_line += 1;
            line_start = offset + ch.len_utf8();
        }
    }

    (current_line == line).then(|| offset_for_column_in_line(source, line_start, column))?
}

pub(super) fn offset_for_column_in_line(source: &str, line_start: usize, column: usize) -> Option<usize> {
    let mut current_column = 1usize;
    for (relative_offset, ch) in source.get(line_start..)?.char_indices() {
        if ch == '\n' {
            break;
        }
        if current_column == column {
            return Some(line_start + relative_offset);
        }
        current_column += 1;
    }

    (current_column == column).then_some(
        line_start
            + source
                .get(line_start..)?
                .find('\n')
                .unwrap_or(source.len() - line_start),
    )
}

pub(super) fn line_end_offset(source: &str, line: usize) -> Option<usize> {
    let line_start = offset_for_line_column(source, line, 1)?;
    Some(
        line_start
            + source
                .get(line_start..)?
                .find('\n')
                .unwrap_or(source.len() - line_start),
    )
}

pub(super) fn parameter_expansion_end_offset(source: &str, dollar_offset: usize) -> Option<usize> {
    let after_dollar = dollar_offset + '$'.len_utf8();
    let bytes = source.as_bytes();
    if bytes.get(after_dollar) == Some(&b'{') {
        let relative_end = source.get(after_dollar..)?.find('}')?;
        return Some(after_dollar + relative_end + '}'.len_utf8());
    }

    let first = source.get(after_dollar..)?.chars().next()?;
    if matches!(first, '@' | '*' | '#' | '?' | '$' | '!' | '-' | '0'..='9') {
        return Some(after_dollar + first.len_utf8());
    }

    let mut end = after_dollar;
    for ch in source.get(after_dollar..)?.chars() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    (end > after_dollar).then_some(end)
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_brace_variable_before_bracket_spans<'a>(
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

pub(super) fn contains_template_placeholder_text_in_word(text: &str) -> bool {
    let Some(start) = text.find("{{") else {
        return false;
    };
    text[start + 2..].contains("}}")
}

pub(super) fn occurrence_word<'a>(nodes: &[WordNode<'a>], occurrence: &WordOccurrence) -> &'a Word {
    nodes[occurrence.node_id.index()].word
}

pub(super) fn occurrence_key(nodes: &[WordNode<'_>], occurrence: &WordOccurrence) -> FactSpan {
    nodes[occurrence.node_id.index()].key
}

pub(super) fn occurrence_span(nodes: &[WordNode<'_>], occurrence: &WordOccurrence) -> Span {
    occurrence_word(nodes, occurrence).span
}

pub(super) fn occurrence_analysis(
    nodes: &[WordNode<'_>],
    occurrence: &WordOccurrence,
) -> ExpansionAnalysis {
    nodes[occurrence.node_id.index()].analysis
}

pub(super) fn word_node_derived<'node, 'word>(
    node: &'node WordNode<'word>,
) -> &'node WordNodeDerived<'word> {
    &node.derived
}

pub(super) fn word_is_plain_scalar_reference(word: &Word) -> bool {
    word_is_plain_reference(word, false)
}

pub(super) fn word_is_plain_parameter_reference(word: &Word) -> bool {
    word_is_plain_reference(word, true)
}

pub(super) fn word_is_plain_reference(word: &Word, allow_all_elements_parameters: bool) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_plain_reference(&part.kind, allow_all_elements_parameters)
}

pub(super) fn word_part_is_plain_reference(part: &WordPart, allow_all_elements_parameters: bool) -> bool {
    match part {
        WordPart::Variable(name) => {
            allow_all_elements_parameters || !matches!(name.as_str(), "@" | "*")
        }
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return false;
            };
            word_part_is_plain_reference(&part.kind, allow_all_elements_parameters)
        }
        WordPart::Parameter(parameter) => {
            parameter_is_plain_reference(parameter, allow_all_elements_parameters)
        }
        _ => false,
    }
}

pub(super) fn parameter_is_plain_reference(
    parameter: &ParameterExpansion,
    allow_all_elements_parameters: bool,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
            if reference.subscript.is_none()
                && (allow_all_elements_parameters
                    || !matches!(reference.name.as_str(), "@" | "*")) =>
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
                            && (allow_all_elements_parameters
                                || !matches!(reference.name.as_str(), "@" | "*"))
                ) =>
        {
            true
        }
        _ => false,
    }
}

pub(super) fn word_is_direct_numeric_expansion(word: &Word) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_direct_numeric_expansion(&part.kind)
}

pub(super) fn word_part_is_direct_numeric_expansion(part: &WordPart) -> bool {
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

pub(super) fn parameter_is_direct_numeric_expansion(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length { .. }) => true,
        ParameterExpansionSyntax::Zsh(syntax) => syntax.length_prefix.is_some(),
        _ => false,
    }
}


pub(super) fn build_unquoted_command_argument_use_offsets(
    semantic: &SemanticModel,
    nodes: &[WordNode<'_>],
    occurrences: &[WordOccurrence],
) -> FxHashMap<Name, Vec<usize>> {
    let unquoted_command_argument_word_spans = occurrences
        .iter()
        .filter(|fact| {
            fact.context == WordFactContext::Expansion(ExpansionContext::CommandArgument)
        })
        .filter(|fact| occurrence_analysis(nodes, fact).quote == WordQuote::Unquoted)
        .map(|fact| occurrence_span(nodes, fact))
        .collect::<Vec<_>>();
    if unquoted_command_argument_word_spans.is_empty() {
        return FxHashMap::default();
    }

    let mut offsets_by_name = FxHashMap::<Name, Vec<usize>>::default();
    for word_span in unquoted_command_argument_word_spans {
        for reference in semantic.references_in_span(word_span) {
            if matches!(
                reference.kind,
                shuck_semantic::ReferenceKind::DeclarationName
            ) {
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

struct ZshArrayFanoutContext<'a, 'flow> {
    semantic: &'a SemanticModel,
    semantic_analysis: &'a SemanticAnalysis<'a>,
    value_flow: &'flow SemanticValueFlow<'flow, 'a>,
    scope: ScopeId,
    options: Option<&'a ZshOptionState>,
    array_like_capable_names: &'a FxHashSet<Name>,
}

fn apply_zsh_array_fanout(
    word: &Word,
    context: ZshArrayFanoutContext<'_, '_>,
    analysis: &mut ExpansionAnalysis,
) {
    if context.semantic.shell_profile().dialect != shuck_parser::parser::ShellDialect::Zsh
        || zsh_unindexed_array_fanout_is_disabled(context.options)
        || !word_has_unquoted_visible_array_reference(
            word,
            context.semantic,
            context.semantic_analysis,
            context.value_flow,
            context.scope,
            context.array_like_capable_names,
        )
    {
        return;
    }

    analysis.array_valued = true;
    analysis.can_expand_to_multiple_fields = true;
    if !matches!(analysis.value_shape, ExpansionValueShape::Unknown) {
        analysis.value_shape = ExpansionValueShape::MultiField;
    }
}

fn zsh_unindexed_array_fanout_is_disabled(options: Option<&ZshOptionState>) -> bool {
    matches!(options.map(|options| options.ksh_arrays), Some(OptionValue::On))
}

fn word_has_unquoted_visible_array_reference(
    word: &Word,
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    value_flow: &SemanticValueFlow<'_, '_>,
    scope: ScopeId,
    array_like_capable_names: &FxHashSet<Name>,
) -> bool {
    parts_have_unquoted_visible_array_reference(
        &word.parts,
        semantic,
        semantic_analysis,
        value_flow,
        scope,
        false,
        array_like_capable_names,
    )
}

fn parts_have_unquoted_visible_array_reference(
    parts: &[WordPartNode],
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    value_flow: &SemanticValueFlow<'_, '_>,
    scope: ScopeId,
    in_double_quotes: bool,
    array_like_capable_names: &FxHashSet<Name>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if parts_have_unquoted_visible_array_reference(
                    parts,
                    semantic,
                    semantic_analysis,
                    value_flow,
                    scope,
                    true,
                    array_like_capable_names,
                ) {
                    return true;
                }
            }
            WordPart::Variable(name) if !in_double_quotes => {
                if visible_name_is_array_like(
                    name,
                    part.span,
                    semantic,
                    semantic_analysis,
                    value_flow,
                    scope,
                    array_like_capable_names,
                ) {
                    return true;
                }
            }
            WordPart::Parameter(parameter) if !in_double_quotes => {
                if zsh_parameter_targets_visible_array(
                    parameter,
                    semantic,
                    semantic_analysis,
                    value_flow,
                    scope,
                    array_like_capable_names,
                ) {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn zsh_parameter_targets_visible_array(
    parameter: &ParameterExpansion,
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    value_flow: &SemanticValueFlow<'_, '_>,
    scope: ScopeId,
    array_like_capable_names: &FxHashSet<Name>,
) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
        | ParameterExpansionSyntax::Zsh(ZshParameterExpansion {
            target: ZshExpansionTarget::Reference(reference),
            length_prefix: None,
            ..
        }) if reference.subscript.is_none() => visible_name_is_array_like(
            &reference.name,
            reference.name_span,
            semantic,
            semantic_analysis,
            value_flow,
            scope,
            array_like_capable_names,
        ),
        _ => false,
    }
}

fn visible_name_is_array_like(
    name: &Name,
    span: Span,
    semantic: &SemanticModel,
    semantic_analysis: &SemanticAnalysis<'_>,
    value_flow: &SemanticValueFlow<'_, '_>,
    scope: ScopeId,
    array_like_capable_names: &FxHashSet<Name>,
) -> bool {
    if !array_like_capable_names.contains(name) {
        return false;
    }
    let synthetic_use_block = if semantic_analysis.reference_id_for_name_at(name, span).is_none() {
        Some(semantic_analysis.flow_entry_block_for_binding_scopes(&[scope], span.start.offset))
    } else {
        None
    };
    value_flow
        .reaching_value_bindings_for_name_with_synthetic_use_block(
            name,
            span,
            synthetic_use_block,
        )
        .into_iter()
        .any(|binding_id| binding_is_array_like(semantic.binding(binding_id)))
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

#[cfg_attr(shuck_profiling, inline(never))]
#[allow(clippy::too_many_arguments)]
pub(super) fn build_word_facts_for_command<'a>(
    visit: CommandVisit<'a>,
    source: &'a str,
    locator: Locator<'a>,
    semantic: &'a SemanticModel,
    context: WordFactCommandContext,
    normalized: &NormalizedCommand<'a>,
    command_shell_behavior: ShellBehaviorAt<'a>,
    outputs: WordFactOutputs<'_, 'a>,
) {
    let mut collector = WordFactCollector::new(
        source,
        locator,
        semantic,
        context.command_id,
        context.nested_word_command,
        context.scope,
        normalized,
        command_shell_behavior,
        outputs,
    );
    collector.collect_command(visit.command, visit.redirects);
}

#[derive(Clone, Copy)]
pub(super) struct WordFactCommandContext {
    pub(super) command_id: CommandId,
    pub(super) nested_word_command: bool,
    pub(super) scope: ScopeId,
}

pub(super) struct WordFactOutputs<'out, 'a> {
    pub(super) word_nodes: &'out mut Vec<WordNode<'a>>,
    pub(super) word_spans: &'out mut ListArena<Span>,
    pub(super) word_span_scratch: &'out mut Vec<Span>,
    pub(super) word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    pub(super) word_occurrences: &'out mut Vec<WordOccurrence>,
    pub(super) pending_arithmetic_word_occurrences:
        &'out mut Vec<PendingArithmeticWordOccurrence>,
    pub(super) compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    pub(super) array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    pub(super) seen_word_occurrences: &'out mut FxHashSet<WordOccurrenceSeenKey>,
    pub(super) seen_pending_arithmetic_word_occurrences:
        &'out mut FxHashSet<PendingArithmeticSeenKey>,
    pub(super) assoc_binding_visibility_memo:
        &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    pub(super) array_like_capable_names: &'out FxHashSet<Name>,
    pub(super) semantic_analysis: &'out SemanticAnalysis<'a>,
    pub(super) case_pattern_expansions: &'out mut Vec<CasePatternExpansionFact>,
    pub(super) pattern_literal_spans: &'out mut Vec<Span>,
    pub(super) arithmetic: &'out mut ArithmeticFactSummary,
    pub(super) surface: &'out mut SurfaceFragmentSink<'a>,
}

pub(super) struct PendingArithmeticWordOccurrence {
    pub(super) node_id: WordNodeId,
    pub(super) command_id: CommandId,
    pub(super) nested_word_command: bool,
    pub(super) host_kind: WordFactHostKind,
    pub(super) enclosing_expansion_context: ExpansionContext,
}

pub(super) type WordOccurrenceSeenKey = (FactSpan, WordFactContext, WordFactHostKind);
pub(super) type PendingArithmeticSeenKey = (FactSpan, ExpansionContext, WordFactHostKind);

pub(super) fn derive_word_fact_data<'a>(
    word: &'a Word,
    locator: Locator<'a>,
    span_store: &mut ListArena<Span>,
    scratch: &mut Vec<Span>,
) -> WordNodeDerived<'a> {
    let source = locator.source();
    let may_have_runtime_expansion_spans = word_may_have_runtime_expansion_spans(word);
    let may_have_command_substitution_spans = word_may_have_command_substitution_spans(word);
    let may_have_mixed_quote_spans =
        word_may_have_unquoted_literal_between_double_quoted_segments_spans(word, source);

    WordNodeDerived {
        static_text: borrowed_static_word_text(word, source),
        trailing_literal_char: word_trailing_literal_char(word, source),
        starts_with_extglob: word_spans::word_starts_with_extglob(word, source),
        has_literal_affixes: word_has_literal_affixes(word),
        contains_shell_quoting_literals: word_contains_shell_quoting_literals(word, source),
        active_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans || word.has_active_brace_expansion(),
            |spans| {
                word_spans::collect_active_expansion_spans_in_source(word, locator, spans);
            },
        ),
        scalar_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_scalar_expansion_part_spans(word, spans);
            },
        ),
        unquoted_scalar_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_unquoted_scalar_expansion_part_spans(word, spans);
            },
        ),
        array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_array_expansion_part_spans(word, spans);
            },
        ),
        all_elements_array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_all_elements_array_expansion_part_spans(word, locator, spans);
            },
        ),
        direct_all_elements_array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_direct_all_elements_array_expansion_part_spans(
                    word, locator, spans,
                );
            },
        ),
        unquoted_all_elements_array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_unquoted_all_elements_array_expansion_part_spans(
                    word, source, spans,
                );
            },
        ),
        unquoted_array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_unquoted_array_expansion_part_spans(word, spans);
            },
        ),
        command_substitution_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_command_substitution_spans,
            |spans| {
                word_spans::collect_command_substitution_part_spans_in_source(word, locator, spans);
            },
        ),
        unquoted_command_substitution_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_command_substitution_spans,
            |spans| {
                word_spans::collect_unquoted_command_substitution_part_spans_in_source(
                    word, locator, spans,
                );
            },
        ),
        unquoted_dollar_paren_command_substitution_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_command_substitution_spans,
            |spans| {
                word_spans::collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
                    word, locator, spans,
                );
            },
        ),
        double_quoted_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                collect_double_quoted_expansion_part_spans(word, spans);
            },
        ),
        unquoted_literal_between_double_quoted_segments_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_mixed_quote_spans,
            |spans| {
                collect_unquoted_literal_between_double_quoted_segments_spans(word, source, spans);
            },
        ),
    }
}

pub(super) fn push_word_span_list(
    span_store: &mut ListArena<Span>,
    scratch: &mut Vec<Span>,
    collect: impl FnOnce(&mut Vec<Span>),
) -> IdRange<Span> {
    scratch.clear();
    collect(scratch);
    span_store.push_many(scratch.drain(..))
}

pub(super) fn push_needed_word_span_list(
    span_store: &mut ListArena<Span>,
    scratch: &mut Vec<Span>,
    needed: bool,
    collect: impl FnOnce(&mut Vec<Span>),
) -> IdRange<Span> {
    if needed {
        push_word_span_list(span_store, scratch, collect)
    } else {
        IdRange::empty()
    }
}

pub(super) fn word_may_have_runtime_expansion_spans(word: &Word) -> bool {
    word_parts_may_have_runtime_expansion_spans(&word.parts)
}

pub(super) fn word_parts_may_have_runtime_expansion_spans(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => word_parts_may_have_runtime_expansion_spans(parts),
        _ => true,
    })
}

pub(super) fn word_may_have_command_substitution_spans(word: &Word) -> bool {
    word_parts_may_have_command_substitution_spans(&word.parts)
}

pub(super) fn word_parts_may_have_command_substitution_spans(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => word_parts_may_have_command_substitution_spans(parts),
        WordPart::CommandSubstitution { .. } => true,
        _ => false,
    })
}

pub(super) fn word_may_have_unquoted_literal_between_double_quoted_segments_spans(
    word: &Word,
    source: &str,
) -> bool {
    let has_reopened_literal = word.parts.windows(3).any(|window| {
        matches!(
            window,
            [
                WordPartNode {
                    kind: WordPart::DoubleQuoted { .. },
                    ..
                },
                WordPartNode {
                    kind: WordPart::Literal(_),
                    ..
                },
                WordPartNode {
                    kind: WordPart::DoubleQuoted { .. },
                    ..
                },
            ]
        )
    });
    if has_reopened_literal {
        return true;
    }

    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return false;
    }

    let text = word.span.slice(source);
    text.contains("\\\n")
        || text.contains("\\\r\n")
        || mixed_quote_following_line_join_suffix_after_word(word, source).is_some()
}

pub(super) fn borrowed_static_word_text<'a>(word: &'a Word, source: &'a str) -> Option<&'a str> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    borrowed_static_word_part_text(part, source)
}

pub(super) fn borrowed_static_word_part_text<'a>(
    part: &'a WordPartNode,
    source: &'a str,
) -> Option<&'a str> {
    match &part.kind {
        WordPart::Literal(text) => Some(text.as_str(source, part.span)),
        WordPart::SingleQuoted { value, .. } => Some(value.slice(source)),
        WordPart::DoubleQuoted { parts, .. } => {
            let [part] = parts.as_slice() else {
                return None;
            };
            borrowed_static_word_part_text(part, source)
        }
        _ => None,
    }
}

pub(super) fn word_trailing_literal_char(word: &Word, source: &str) -> Option<char> {
    trailing_literal_char_in_parts(&word.parts, source)
}

pub(super) fn trailing_literal_char_in_parts(parts: &[WordPartNode], source: &str) -> Option<char> {
    let part = parts.last()?;

    match &part.kind {
        WordPart::Literal(text) => text.as_str(source, part.span).chars().next_back(),
        WordPart::SingleQuoted { value, .. } => value.slice(source).chars().next_back(),
        WordPart::DoubleQuoted { parts, .. } => trailing_literal_char_in_parts(parts, source),
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
        | WordPart::ZshQualifiedGlob(_) => None,
    }
}

pub(super) struct WordFactCollector<'out, 'a, 'norm> {
    source: &'a str,
    locator: Locator<'a>,
    semantic: &'a SemanticModel,
    command_id: CommandId,
    nested_word_command: bool,
    command_scope: ScopeId,
    surface_command_name: Option<&'norm str>,
    surface_body_arg_start_offset: Option<usize>,
    command_shell_behavior: ShellBehaviorAt<'a>,
    word_nodes: &'out mut Vec<WordNode<'a>>,
    word_spans: &'out mut ListArena<Span>,
    word_span_scratch: &'out mut Vec<Span>,
    word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    word_occurrences: &'out mut Vec<WordOccurrence>,
    pending_arithmetic_word_occurrences: &'out mut Vec<PendingArithmeticWordOccurrence>,
    array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    assoc_binding_visibility_memo: &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    array_like_capable_names: &'out FxHashSet<Name>,
    semantic_analysis: &'out SemanticAnalysis<'a>,
    value_flow: SemanticValueFlow<'out, 'a>,
    seen: &'out mut FxHashSet<WordOccurrenceSeenKey>,
    seen_pending_arithmetic: &'out mut FxHashSet<PendingArithmeticSeenKey>,
    compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    case_pattern_expansions: &'out mut Vec<CasePatternExpansionFact>,
    pattern_literal_spans: &'out mut Vec<Span>,
    arithmetic: &'out mut ArithmeticFactSummary,
    surface: &'out mut SurfaceFragmentSink<'a>,
}

pub(super) fn simple_command_wrapper_target_index(command: &SimpleCommand, source: &str) -> Option<usize> {
    let command_name = static_command_name_text(&command.name, source)?;
    let word_count = 1 + command.args.len();
    match static_command_wrapper_target_index(word_count, 0, command_name.as_ref(), |index| {
        static_word_text(simple_command_word_at(command, index), source)
    }) {
        StaticCommandWrapperTarget::Wrapper { target_index } => target_index,
        StaticCommandWrapperTarget::NotWrapper => None,
    }
}

pub(super) fn simple_command_word_at(command: &SimpleCommand, index: usize) -> &Word {
    if index == 0 {
        &command.name
    } else {
        &command.args[index - 1]
    }
}

fn collect_split_sensitive_unquoted_command_substitution_spans(
    parts: &[WordPartNode],
    source: &str,
    quoted: bool,
    behavior: &ShellBehaviorAt<'_>,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_split_sensitive_unquoted_command_substitution_spans(
                    parts, source, true, behavior, spans,
                );
            }
            WordPart::CommandSubstitution { .. }
                if !quoted
                    && analyze_part(&part.kind, source, quoted, behavior)
                        .can_expand_to_multiple_fields =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

impl<'out, 'a, 'norm> WordFactCollector<'out, 'a, 'norm> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source: &'a str,
        locator: Locator<'a>,
        semantic: &'a SemanticModel,
        command_id: CommandId,
        nested_word_command: bool,
        command_scope: ScopeId,
        normalized: &'norm NormalizedCommand<'a>,
        command_shell_behavior: ShellBehaviorAt<'a>,
        outputs: WordFactOutputs<'out, 'a>,
    ) -> Self {
        let value_flow = outputs.semantic_analysis.value_flow();
        Self {
            source,
            locator,
            semantic,
            command_id,
            nested_word_command,
            command_scope,
            surface_command_name: normalized.effective_or_literal_name(),
            surface_body_arg_start_offset: normalized
                .body_args()
                .first()
                .map(|word| word.span.start.offset),
            command_shell_behavior,
            word_nodes: outputs.word_nodes,
            word_spans: outputs.word_spans,
            word_span_scratch: outputs.word_span_scratch,
            word_node_ids_by_span: outputs.word_node_ids_by_span,
            word_occurrences: outputs.word_occurrences,
            pending_arithmetic_word_occurrences: outputs.pending_arithmetic_word_occurrences,
            array_assignment_split_word_ids: outputs.array_assignment_split_word_ids,
            assoc_binding_visibility_memo: outputs.assoc_binding_visibility_memo,
            array_like_capable_names: outputs.array_like_capable_names,
            semantic_analysis: outputs.semantic_analysis,
            value_flow,
            seen: {
                outputs.seen_word_occurrences.clear();
                outputs.seen_word_occurrences
            },
            seen_pending_arithmetic: {
                outputs.seen_pending_arithmetic_word_occurrences.clear();
                outputs.seen_pending_arithmetic_word_occurrences
            },
            compound_assignment_value_word_spans: outputs.compound_assignment_value_word_spans,
            case_pattern_expansions: outputs.case_pattern_expansions,
            pattern_literal_spans: outputs.pattern_literal_spans,
            arithmetic: outputs.arithmetic,
            surface: outputs.surface,
        }
    }

    fn surface_context(&self) -> SurfaceScanContext<'norm> {
        SurfaceScanContext::new(
            self.surface_command_name,
            self.nested_word_command,
            self.semantic.shell_profile().dialect,
        )
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
                            self.surface
                                .collect_pattern_structure(pattern, pattern_context);
                            self.collect_case_pattern_expansions(pattern);
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
                        SurfaceScanContext::new(
                            None,
                            self.nested_word_command,
                            self.semantic.shell_profile().dialect,
                        ),
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
                        if single_quoted_literal_exempt_here_string(surface_context.command_name())
                        {
                            surface_context.literal_expansion_exempt()
                        } else {
                            surface_context
                        }
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
                    let Some(heredoc) = redirect.heredoc() else {
                        continue;
                    };
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
                if let Some(target_index) =
                    simple_command_wrapper_target_index(command, self.source)
                {
                    let target_word = simple_command_word_at(command, target_index);
                    self.push_word_with_surface(
                        target_word,
                        WordFactContext::Expansion(ExpansionContext::CommandName),
                        WordFactHostKind::CommandWrapperTarget,
                        surface_context,
                    );
                }

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
                let wrapper_target_arg_index =
                    simple_command_wrapper_target_index(command, self.source)
                        .and_then(|index| index.checked_sub(1));
                let body_arg_start = self
                    .surface_body_arg_start_offset
                    .and_then(|offset| {
                        command
                            .args
                            .iter()
                            .position(|word| word.span.start.offset == offset)
                    })
                    .unwrap_or_else(|| wrapper_target_arg_index.map_or(0, |index| index + 1));
                let trap_command =
                    static_word_text(&command.name, self.source).as_deref() == Some("trap");
                let trap_action = trap_command
                    .then(|| trap_action_word_from_simple_command(command, self.source))
                    .flatten();
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
                for (arg_index, word) in command.args.iter().enumerate() {
                    if wrapper_target_arg_index == Some(arg_index) {
                        continue;
                    }
                    let base_surface_word_context = if variable_set_operand
                        .is_some_and(|operand| std::ptr::eq(word, operand))
                    {
                        surface_context.variable_set_operand()
                    } else if trap_action.is_some_and(|action| std::ptr::eq(action, word))
                        || single_quoted_literal_exempt_argument(
                            surface_command_name,
                            self.semantic.shell_profile().dialect,
                            &command.args,
                            arg_index,
                            body_arg_start,
                            word,
                            self.source,
                        )
                    {
                        surface_context.literal_expansion_exempt()
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
                        if !trap_action.is_some_and(|action| std::ptr::eq(action, word)) {
                            self.push_word(
                                word,
                                WordFactContext::Expansion(ExpansionContext::CommandArgument),
                                WordFactHostKind::Direct,
                            );
                        }
                    } else {
                        if surface_command_name == Some("eval") {
                            collect_wrapped_arithmetic_spans_in_word(
                                word,
                                self.source,
                                &mut self.arithmetic.dollar_in_arithmetic_spans,
                                &mut self.arithmetic.arithmetic_command_substitution_spans,
                            );
                        }
                        let word_context = Self::simple_command_argument_expansion_context(
                            surface_command_name,
                            word,
                            self.source,
                        );
                        let (_, opened) = self.push_word_with_surface(
                            word,
                            word_context,
                            WordFactHostKind::Direct,
                            surface_word_context,
                        );
                        saw_open_double_quote |= opened;
                    }
                }
            }
            Command::Builtin(command) => match command {
                BuiltinCommand::Break(command) => {
                    let surface_context = SurfaceScanContext::new(
                        None,
                        self.nested_word_command,
                        self.semantic.shell_profile().dialect,
                    );
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
                    let surface_context = SurfaceScanContext::new(
                        None,
                        self.nested_word_command,
                        self.semantic.shell_profile().dialect,
                    );
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
                    let surface_context = SurfaceScanContext::new(
                        None,
                        self.nested_word_command,
                        self.semantic.shell_profile().dialect,
                    );
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
                    let surface_context = SurfaceScanContext::new(
                        None,
                        self.nested_word_command,
                        self.semantic.shell_profile().dialect,
                    );
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
                let surface_context = SurfaceScanContext::new(
                    None,
                    self.nested_word_command,
                    self.semantic.shell_profile().dialect,
                );
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
                    SurfaceScanContext::new(
                        None,
                        self.nested_word_command,
                        self.semantic.shell_profile().dialect,
                    ),
                );
            }
        }
    }

    fn simple_command_argument_expansion_context(
        command_name: Option<&str>,
        word: &Word,
        source: &str,
    ) -> WordFactContext {
        match command_name {
            Some("let") => WordFactContext::ArithmeticCommand,
            Some("declare" | "export" | "local" | "readonly" | "typeset")
                if Self::simple_assignment_like_word(word, source) =>
            {
                WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue)
            }
            _ => WordFactContext::Expansion(ExpansionContext::CommandArgument),
        }
    }

    fn simple_assignment_like_word(word: &Word, source: &str) -> bool {
        let text = word.span.slice(source);
        let Some((name, _)) = text.split_once('=') else {
            return false;
        };

        is_shell_variable_name(name)
    }

    fn collect_expansion_assignment_value_words(&mut self, command: &'a Command) {
        for assignment in command_assignments(command) {
            self.collect_expansion_assignment_words(
                assignment,
                WordFactContext::Expansion(ExpansionContext::AssignmentValue),
            );
        }

        for operand in declaration_operands(command) {
            match operand {
                DeclOperand::Name(reference) => {
                    let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
                        Some(&reference.name),
                        Some(reference.name_span),
                        reference.subscript.as_deref(),
                    );
                    if !indexed_semantics {
                        self.surface.record_arithmetic_only_suppressed_subscript(
                            reference.subscript.as_deref(),
                        );
                    }
                    visit_var_ref_subscript_words_with_source(
                        reference,
                        self.source,
                        &mut |word| {
                            let surface_context = SurfaceScanContext::new(
                                None,
                                self.nested_word_command,
                                self.semantic.shell_profile().dialect,
                            );
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
        let surface_context = SurfaceScanContext::new(
            None,
            self.nested_word_command,
            self.semantic.shell_profile().dialect,
        )
            .with_assignment_target(assignment.target.name.as_str());
        let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
            Some(&assignment.target.name),
            Some(assignment.target.name_span),
            assignment.target.subscript.as_deref(),
        );
        if !indexed_semantics {
            self.surface
                .record_arithmetic_only_suppressed_subscript(assignment.target.subscript.as_deref());
        }
        visit_var_ref_subscript_words_with_source(&assignment.target, self.source, &mut |word| {
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
        });

        match &assignment.value {
            AssignmentValue::Scalar(word) => {
                self.push_word_with_surface(
                    word,
                    context,
                    WordFactHostKind::Direct,
                    surface_context,
                );
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
                            ) {
                                self.array_assignment_split_word_ids.push(index);
                            }
                        }
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            let indexed_semantics = self.subscript_uses_index_arithmetic_semantics(
                                Some(&assignment.target.name),
                                Some(assignment.target.name_span),
                                Some(key),
                            );
                            if !indexed_semantics {
                                self.surface
                                    .record_arithmetic_only_suppressed_subscript(Some(key));
                            }
                            visit_subscript_words(Some(key), self.source, &mut |word| {
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

    fn collect_case_pattern_expansions(&mut self, pattern: &Pattern) {
        if pattern_has_glob_structure(pattern, self.source) {
            return;
        }

        if pattern_is_arithmetic_only(pattern) {
            return;
        }

        let expanded_words = pattern
            .parts
            .iter()
            .filter_map(|part| match &part.kind {
                PatternPart::Word(word) => {
                    let analysis =
                        analyze_word(word, self.source, Some(&self.command_shell_behavior));
                    (analysis.literalness == WordLiteralness::Expanded
                        && analysis.quote != WordQuote::FullyQuoted)
                        .then_some(word)
                }
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_)
                | PatternPart::Group { .. } => None,
            })
            .collect::<Vec<_>>();

        if expanded_words.is_empty() {
            return;
        }

        if pattern.parts.len() > 1 {
            self.case_pattern_expansions
                .push(CasePatternExpansionFact::new(
                    pattern.span,
                    rewrite_pattern_as_single_double_quoted_string(pattern, self.source),
                ));
        } else {
            self.case_pattern_expansions
                .extend(expanded_words.into_iter().map(|word| {
                    CasePatternExpansionFact::new(
                        word.span,
                        rewrite_word_as_single_double_quoted_string(word, self.source, None),
                    )
                }));
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
                self.surface
                    .collect_pattern_structure(pattern, pattern_context);
                self.collect_pattern_context_words(
                    pattern,
                    WordFactContext::Expansion(ExpansionContext::ConditionalPattern),
                    WordFactHostKind::Direct,
                    Some(pattern_context),
                );
            }
            ConditionalExpr::VarRef(reference) => {
                self.surface
                    .record_arithmetic_only_suppressed_subscript(reference.subscript.as_deref());
                visit_var_ref_subscript_words_with_source(reference, self.source, &mut |word| {
                    self.push_word_with_surface(
                        word,
                        WordFactContext::Expansion(ExpansionContext::ConditionalVarRefSubscript),
                        WordFactHostKind::ConditionalVarRefSubscript,
                        surface_context,
                    );
                });
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
        let mut analysis = analyze_word(word, self.source, Some(&self.command_shell_behavior));
        apply_zsh_array_fanout(
            word,
            ZshArrayFanoutContext {
                semantic: self.semantic,
                semantic_analysis: self.semantic_analysis,
                value_flow: &self.value_flow,
                scope: self.command_scope,
                options: self.command_shell_behavior.zsh_options(),
                array_like_capable_names: self.array_like_capable_names,
            },
            &mut analysis,
        );
        let derived =
            derive_word_fact_data(word, self.locator, self.word_spans, self.word_span_scratch);
        self.word_nodes.push(WordNode {
            key,
            word,
            analysis,
            derived,
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
            WordFactContext::Expansion(context) => analyze_literal_runtime(
                word,
                self.source,
                context,
                Some(&self.command_shell_behavior),
            ),
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
        self.word_span_scratch.clear();
        collect_split_sensitive_unquoted_command_substitution_spans(
            &word.parts,
            self.source,
            false,
            &self.command_shell_behavior,
            self.word_span_scratch,
        );
        word_spans::normalize_command_substitution_spans(self.word_span_scratch, self.locator);
        let split_sensitive_unquoted_command_substitution_spans =
            self.word_spans.push_many(self.word_span_scratch.drain(..));
        let id = WordOccurrenceId::new(self.word_occurrences.len());
        self.word_occurrences.push(WordOccurrence {
            node_id,
            command_id: self.command_id,
            nested_word_command: self.nested_word_command,
            context,
            host_kind,
            runtime_literal,
            operand_class,
            enclosing_expansion_context: None,
            split_sensitive_unquoted_command_substitution_spans,
            array_assignment_split_scalar_expansion_spans: IdRange::empty(),
        });
        if let WordFactContext::Expansion(enclosing_expansion_context) = context {
            self.collect_pending_arithmetic_word_occurrences(
                word,
                enclosing_expansion_context,
                host_kind,
            );
        }
        (Some(id), opened_double_quote)
    }

    fn collect_pending_arithmetic_word_occurrences(
        &mut self,
        word: &'a Word,
        enclosing_expansion_context: ExpansionContext,
        host_kind: WordFactHostKind,
    ) {
        self.collect_pending_arithmetic_word_occurrences_in_parts(
            &word.parts,
            enclosing_expansion_context,
            host_kind,
        );
    }

    fn collect_pending_arithmetic_word_occurrences_in_parts(
        &mut self,
        parts: &'a [WordPartNode],
        enclosing_expansion_context: ExpansionContext,
        host_kind: WordFactHostKind,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self
                    .collect_pending_arithmetic_word_occurrences_in_parts(
                        parts,
                        enclosing_expansion_context,
                        host_kind,
                    ),
                WordPart::ArithmeticExpansion {
                    expression_ast,
                    expression_word_ast,
                    ..
                } => {
                    if let Some(expression) = expression_ast.as_ref() {
                        visit_arithmetic_words(expression, &mut |word| {
                            self.push_pending_arithmetic_word_occurrence(
                                word,
                                enclosing_expansion_context,
                                host_kind,
                            );
                        });
                    } else {
                        self.push_pending_arithmetic_word_occurrence(
                            expression_word_ast,
                            enclosing_expansion_context,
                            host_kind,
                        );
                    }
                }
                WordPart::Parameter(parameter) => self
                    .collect_pending_arithmetic_word_occurrences_in_parameter_expansion(
                        parameter,
                        enclosing_expansion_context,
                        host_kind,
                    ),
                WordPart::ParameterExpansion {
                    operator,
                    operand_word_ast,
                    ..
                }
                | WordPart::IndirectExpansion {
                    operator: Some(operator),
                    operand_word_ast,
                    ..
                } => self.collect_pending_arithmetic_word_occurrences_in_parameter_operator(
                    operator,
                    operand_word_ast.as_deref(),
                    enclosing_expansion_context,
                    host_kind,
                ),
                WordPart::Literal(_)
                | WordPart::SingleQuoted { .. }
                | WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { operator: None, .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. }
                | WordPart::ZshQualifiedGlob(_) => {}
            }
        }
    }

    fn collect_pending_arithmetic_word_occurrences_in_parameter_expansion(
        &mut self,
        parameter: &'a ParameterExpansion,
        enclosing_expansion_context: ExpansionContext,
        host_kind: WordFactHostKind,
    ) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(
                BourneParameterExpansion::Operation {
                    operator,
                    operand_word_ast,
                    ..
                }
                | BourneParameterExpansion::Indirect {
                    operator: Some(operator),
                    operand_word_ast,
                    ..
                },
            ) => self.collect_pending_arithmetic_word_occurrences_in_parameter_operator(
                operator,
                operand_word_ast.as_deref(),
                enclosing_expansion_context,
                host_kind,
            ),
            ParameterExpansionSyntax::Bourne(
                BourneParameterExpansion::Access { .. }
                | BourneParameterExpansion::Length { .. }
                | BourneParameterExpansion::Indices { .. }
                | BourneParameterExpansion::Indirect { operator: None, .. }
                | BourneParameterExpansion::PrefixMatch { .. }
                | BourneParameterExpansion::Slice { .. }
                | BourneParameterExpansion::Transformation { .. },
            ) => {}
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &syntax.target {
                    ZshExpansionTarget::Nested(parameter) => self
                        .collect_pending_arithmetic_word_occurrences_in_parameter_expansion(
                            parameter,
                            enclosing_expansion_context,
                            host_kind,
                        ),
                    ZshExpansionTarget::Word(word) => self
                        .collect_pending_arithmetic_word_occurrences(
                            word,
                            enclosing_expansion_context,
                            host_kind,
                        ),
                    ZshExpansionTarget::Reference(_) | ZshExpansionTarget::Empty => {}
                }

                if let Some(operation) = syntax.operation.as_ref()
                    && let Some(operand_word) = operation.operand_word_ast()
                {
                    self.collect_pending_arithmetic_word_occurrences(
                        operand_word,
                        enclosing_expansion_context,
                        host_kind,
                    );
                }

                if let Some(operation) = syntax.operation.as_ref()
                    && let Some(replacement_word) = operation.replacement_word_ast()
                {
                    self.collect_pending_arithmetic_word_occurrences(
                        replacement_word,
                        enclosing_expansion_context,
                        host_kind,
                    );
                }
            }
        }
    }

    fn collect_pending_arithmetic_word_occurrences_in_parameter_operator(
        &mut self,
        operator: &'a ParameterOp,
        operand_word_ast: Option<&'a Word>,
        enclosing_expansion_context: ExpansionContext,
        host_kind: WordFactHostKind,
    ) {
        if matches!(
            operator,
            ParameterOp::UseDefault
                | ParameterOp::AssignDefault
                | ParameterOp::UseReplacement
                | ParameterOp::Error
        ) && let Some(operand_word) = operand_word_ast
        {
            self.collect_pending_arithmetic_word_occurrences(
                operand_word,
                enclosing_expansion_context,
                host_kind,
            );
        }

        if let Some(replacement_word) = operator.replacement_word_ast() {
            self.collect_pending_arithmetic_word_occurrences(
                replacement_word,
                enclosing_expansion_context,
                host_kind,
            );
        }
    }

    fn push_pending_arithmetic_word_occurrence(
        &mut self,
        word: &'a Word,
        enclosing_expansion_context: ExpansionContext,
        host_kind: WordFactHostKind,
    ) {
        let key = FactSpan::new(word.span);
        if !self
            .seen_pending_arithmetic
            .insert((key, enclosing_expansion_context, host_kind))
        {
            return;
        }

        let node_id = self.intern_word_node(word);
        self.pending_arithmetic_word_occurrences
            .push(PendingArithmeticWordOccurrence {
                node_id,
                command_id: self.command_id,
                nested_word_command: self.nested_word_command,
                host_kind,
                enclosing_expansion_context,
            });
        self.collect_pending_arithmetic_word_occurrences(
            word,
            enclosing_expansion_context,
            host_kind,
        );
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
            self.arithmetic.arithmetic_score_line_spans.extend(
                word_spans::parenthesized_arithmetic_expansion_part_spans(word),
            );
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

        if owner_name.is_some_and(|name| {
            self.assoc_binding_visible_for_subscript(name, owner_name_span, subscript)
        }) {
            return false;
        }
        if owner_name.is_some_and(|name| {
            self.assoc_lookup_binding_blocks_zsh_option_map_fallback(
                name,
                owner_name_span,
                subscript,
            )
        }) {
            return true;
        }

        if self.semantic.shell_profile().dialect == shuck_parser::parser::ShellDialect::Zsh
            && owner_name.is_some_and(|name| {
                super::zsh_option_map_binding_permits_implicit_assoc_key(
                    self.semantic,
                    self.binding_visible_for_subscript(name, owner_name_span, subscript),
                    name,
                    self.source,
                )
                    && super::zsh_option_map_subscript_key(
                        name.as_str(),
                        subscript.syntax_text(self.source),
                    )
            })
        {
            return false;
        }

        true
    }

    fn assoc_binding_visible_for_subscript(
        &mut self,
        owner_name: &Name,
        owner_name_span: Option<Span>,
        subscript: &Subscript,
    ) -> bool {
        let key = (
            owner_name.clone(),
            self.command_scope,
            owner_name_span.map(FactSpan::new),
        );
        if let Some(result) = self.assoc_binding_visibility_memo.get(&key) {
            return *result;
        }

        let lookup_span = owner_name_span.unwrap_or(subscript.span());
        let visible =
            self.semantic
                .assoc_binding_visible_for_lookup(owner_name, self.command_scope, lookup_span);
        self.assoc_binding_visibility_memo.insert(key, visible);
        visible
    }

    fn assoc_lookup_binding_blocks_zsh_option_map_fallback(
        &self,
        owner_name: &Name,
        owner_name_span: Option<Span>,
        subscript: &Subscript,
    ) -> bool {
        let lookup_span = owner_name_span.unwrap_or(subscript.span());
        self.semantic
            .visible_assoc_lookup_binding_for_lookup(owner_name, self.command_scope, lookup_span)
            .is_some_and(|binding| {
                !binding
                    .attributes
                    .contains(shuck_semantic::BindingAttributes::ASSOC)
                    && (!super::zsh_option_map_binding_origin(owner_name, binding, self.source)
                        || super::zsh_option_map_binding_has_prior_assoc_lookup_blocker(
                            self.semantic,
                            owner_name,
                            binding,
                            self.source,
                        ))
            })
    }

    fn binding_visible_for_subscript(
        &self,
        owner_name: &Name,
        owner_name_span: Option<Span>,
        subscript: &Subscript,
    ) -> Option<&shuck_semantic::Binding> {
        let lookup_span = owner_name_span.unwrap_or(subscript.span());
        self.semantic
            .visible_binding_for_lookup(owner_name, self.command_scope, lookup_span)
    }

    fn collect_array_index_arithmetic_spans(&mut self, word: &Word) {
        self.arithmetic
            .array_index_arithmetic_spans
            .extend(word_spans::arithmetic_expansion_part_spans(word));
    }

    fn collect_dollar_prefixed_indexed_subscript_spans(&mut self, word: &Word) {
        collect_dollar_prefixed_indexed_subscript_word_spans(
            word,
            self.source,
            &mut self.arithmetic.dollar_in_arithmetic_spans,
        );
    }
}
