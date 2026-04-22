use super::*;
use shuck_ast::{
    HeredocBody, HeredocBodyPart, HeredocBodyPartNode, PatternGroupKind, PatternPartNode, Position,
};

#[derive(Debug)]
pub struct SingleQuotedFragmentFact {
    span: Span,
    diagnostic_span: Span,
    dollar_quoted: bool,
    command_name: Option<Box<str>>,
    assignment_target: Option<Box<str>>,
    variable_set_operand: bool,
    literal_backslash_in_single_quotes_span: Option<Span>,
}

impl SingleQuotedFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn diagnostic_span(&self) -> Span {
        self.diagnostic_span
    }

    pub fn dollar_quoted(&self) -> bool {
        self.dollar_quoted
    }

    pub fn command_name(&self) -> Option<&str> {
        self.command_name.as_deref()
    }

    pub fn assignment_target(&self) -> Option<&str> {
        self.assignment_target.as_deref()
    }

    pub fn variable_set_operand(&self) -> bool {
        self.variable_set_operand
    }

    pub fn literal_backslash_in_single_quotes_span(&self) -> Option<Span> {
        self.literal_backslash_in_single_quotes_span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DollarDoubleQuotedFragmentFact {
    span: Span,
}

impl DollarDoubleQuotedFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone)]
pub struct OpenDoubleQuoteFragmentFact {
    span: Span,
    replacement_span: Span,
    replacement: Box<str>,
}

impl OpenDoubleQuoteFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn replacement_span(&self) -> Span {
        self.replacement_span
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

#[derive(Debug, Clone)]
pub struct CasePatternExpansionFact {
    span: Span,
    replacement: Box<str>,
}

impl CasePatternExpansionFact {
    pub(super) fn new(span: Span, replacement: Box<str>) -> Self {
        Self { span, replacement }
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuspectClosingQuoteFragmentFact {
    span: Span,
}

impl SuspectClosingQuoteFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BacktickFragmentFact {
    span: Span,
    empty: bool,
}

impl BacktickFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LegacyArithmeticFragmentFact {
    span: Span,
}

impl LegacyArithmeticFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionalParameterFragmentKind {
    AboveNine,
    General,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionalParameterFragmentFact {
    span: Span,
    kind: PositionalParameterFragmentKind,
    guarded: bool,
}

impl PositionalParameterFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn kind(&self) -> PositionalParameterFragmentKind {
        self.kind
    }

    pub fn is_above_nine(&self) -> bool {
        self.kind == PositionalParameterFragmentKind::AboveNine
    }

    pub fn is_guarded(&self) -> bool {
        self.guarded
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NestedParameterExpansionFragmentFact {
    span: Span,
}

impl NestedParameterExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndirectExpansionFragmentFact {
    span: Span,
    array_keys: bool,
}

impl IndirectExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn array_keys(&self) -> bool {
        self.array_keys
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexedArrayReferenceFragmentFact {
    span: Span,
    plain: bool,
}

impl IndexedArrayReferenceFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn is_plain(&self) -> bool {
        self.plain
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParameterPatternSpecialTargetFragmentFact {
    span: Span,
}

impl ParameterPatternSpecialTargetFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ZshParameterIndexFlagFragmentFact {
    span: Span,
}

impl ZshParameterIndexFlagFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubstringExpansionFragmentFact {
    span: Span,
}

impl SubstringExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CaseModificationFragmentFact {
    span: Span,
}

impl CaseModificationFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReplacementExpansionFragmentFact {
    span: Span,
}

impl ReplacementExpansionFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StarGlobRemovalFragmentFact {
    span: Span,
}

impl StarGlobRemovalFragmentFact {
    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Default)]
pub(super) struct SurfaceFragmentFacts {
    pub(super) single_quoted: Vec<SingleQuotedFragmentFact>,
    pub(super) dollar_double_quoted: Vec<DollarDoubleQuotedFragmentFact>,
    pub(super) open_double_quotes: Vec<OpenDoubleQuoteFragmentFact>,
    pub(super) suspect_closing_quotes: Vec<SuspectClosingQuoteFragmentFact>,
    pub(super) backticks: Vec<BacktickFragmentFact>,
    pub(super) legacy_arithmetic: Vec<LegacyArithmeticFragmentFact>,
    pub(super) positional_parameters: Vec<PositionalParameterFragmentFact>,
    pub(super) positional_parameter_operator_spans: Vec<Span>,
    pub(super) unicode_smart_quote_spans: Vec<Span>,
    pub(super) pattern_exactly_one_extglob_spans: Vec<Span>,
    pub(super) pattern_charclass_spans: Vec<Span>,
    pub(super) parameter_pattern_spans: Vec<Span>,
    pub(super) nested_pattern_charclass_spans: Vec<Span>,
    pub(super) nested_parameter_expansions: Vec<NestedParameterExpansionFragmentFact>,
    pub(super) indirect_expansions: Vec<IndirectExpansionFragmentFact>,
    pub(super) indexed_array_references: Vec<IndexedArrayReferenceFragmentFact>,
    pub(super) parameter_pattern_special_targets: Vec<ParameterPatternSpecialTargetFragmentFact>,
    pub(super) zsh_parameter_index_flags: Vec<ZshParameterIndexFlagFragmentFact>,
    pub(super) substring_expansions: Vec<SubstringExpansionFragmentFact>,
    pub(super) case_modifications: Vec<CaseModificationFragmentFact>,
    pub(super) replacement_expansions: Vec<ReplacementExpansionFragmentFact>,
    pub(super) star_glob_removals: Vec<StarGlobRemovalFragmentFact>,
    pub(super) subscript_spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SurfaceScanContext<'a> {
    command_name: Option<&'a str>,
    assignment_target: Option<&'a str>,
    nested_word_command: bool,
    variable_set_operand: bool,
    guarded_parameter_operand: bool,
    collect_open_double_quotes: bool,
    collect_pattern_charclasses: bool,
}

impl<'a> SurfaceScanContext<'a> {
    pub(super) fn new(command_name: Option<&'a str>, nested_word_command: bool) -> Self {
        Self {
            command_name,
            nested_word_command,
            collect_open_double_quotes: true,
            collect_pattern_charclasses: false,
            ..Self::default()
        }
    }

    pub(super) fn command_name(&self) -> Option<&'a str> {
        self.command_name
    }

    pub(super) fn with_assignment_target(self, assignment_target: &'a str) -> Self {
        Self {
            assignment_target: Some(assignment_target),
            ..self
        }
    }

    pub(super) fn variable_set_operand(self) -> Self {
        Self {
            variable_set_operand: true,
            ..self
        }
    }

    pub(super) fn guarded_parameter_operand(self) -> Self {
        Self {
            guarded_parameter_operand: true,
            ..self
        }
    }

    pub(super) fn without_open_double_quote_scan(self) -> Self {
        Self {
            collect_open_double_quotes: false,
            ..self
        }
    }

    pub(super) fn without_command_name(self) -> Self {
        Self {
            command_name: None,
            ..self
        }
    }

    pub(super) fn with_pattern_charclass_scan(self) -> Self {
        Self {
            collect_pattern_charclasses: true,
            ..self
        }
    }
}

pub(super) struct SurfaceFragmentSink<'a> {
    source: &'a str,
    facts: SurfaceFragmentFacts,
}

impl<'a> SurfaceFragmentSink<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            source,
            facts: SurfaceFragmentFacts::default(),
        }
    }

    pub(super) fn finish(self) -> SurfaceFragmentFacts {
        self.facts
    }

    fn opening_backtick_is_escaped(&self, span: Span) -> bool {
        let source = self.source.as_bytes();
        let start = span.start.offset;
        let Some(fragment) = self.source.get(start..span.end.offset) else {
            return false;
        };
        let Some(first_backtick) = fragment.find('`') else {
            return true;
        };
        if !fragment[..first_backtick].bytes().all(|byte| byte == b'\\') {
            return false;
        }

        let mut backslashes = first_backtick;
        let mut cursor = start;
        while cursor > 0 && source[cursor - 1] == b'\\' {
            backslashes += 1;
            cursor -= 1;
        }

        backslashes % 2 == 1
    }

    fn looks_like_unbraced_positional_above_nine(&self, span: Span) -> bool {
        let fragment = span.slice(self.source);
        let fragment = fragment.strip_prefix('"').unwrap_or(fragment);
        let mut chars = fragment.chars();

        matches!(
            (chars.next(), chars.next(), chars.next()),
            (Some('$'), Some(digit), Some(next))
                if matches!(digit, '1'..='9') && next.is_ascii_digit()
        )
    }

    fn record_array_reference(&mut self, span: Span, plain: bool) {
        if let Some(fragment) = self
            .facts
            .indexed_array_references
            .iter_mut()
            .find(|fragment| fragment.span() == span)
        {
            fragment.plain |= plain;
            return;
        }
        self.facts
            .indexed_array_references
            .push(IndexedArrayReferenceFragmentFact { span, plain });
    }

    fn record_parameter_pattern_special_target(&mut self, operand_span: Span) {
        if self
            .facts
            .parameter_pattern_special_targets
            .iter()
            .any(|fragment| fragment.span() == operand_span)
        {
            return;
        }
        self.facts
            .parameter_pattern_special_targets
            .push(ParameterPatternSpecialTargetFragmentFact { span: operand_span });
    }

    fn record_substring_expansion(&mut self, span: Span) {
        if self
            .facts
            .substring_expansions
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .substring_expansions
            .push(SubstringExpansionFragmentFact { span });
    }

    fn record_zsh_parameter_index_flag(&mut self, span: Span) {
        if self
            .facts
            .zsh_parameter_index_flags
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .zsh_parameter_index_flags
            .push(ZshParameterIndexFlagFragmentFact { span });
    }

    fn record_case_modification(&mut self, span: Span) {
        if self
            .facts
            .case_modifications
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .case_modifications
            .push(CaseModificationFragmentFact { span });
    }

    fn record_replacement_expansion(&mut self, span: Span) {
        if self
            .facts
            .replacement_expansions
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .replacement_expansions
            .push(ReplacementExpansionFragmentFact { span });
    }

    fn record_parameter_pattern(&mut self, span: Span) {
        if self.facts.parameter_pattern_spans.contains(&span) {
            return;
        }
        self.facts.parameter_pattern_spans.push(span);
    }

    fn record_star_glob_removal(&mut self, span: Span) {
        if self
            .facts
            .star_glob_removals
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .star_glob_removals
            .push(StarGlobRemovalFragmentFact { span });
    }

    pub(super) fn collect_word(&mut self, word: &Word, context: SurfaceScanContext<'_>) -> bool {
        let open_double_quote_count = self.facts.open_double_quotes.len();
        collect_unicode_smart_quote_spans_in_word_parts(
            &word.parts,
            self.source,
            false,
            &mut self.facts.unicode_smart_quote_spans,
        );
        for span in zsh_parameter_index_flag_spans_in_word(word.span.slice(self.source), word.span)
        {
            self.record_zsh_parameter_index_flag(span);
        }
        if context.collect_open_double_quotes {
            self.collect_open_double_quote_fragments(
                word,
                context.command_name,
                context.assignment_target,
            );
        }
        self.collect_word_parts(&word.parts, context);
        self.facts.open_double_quotes.len() > open_double_quote_count
    }

    pub(super) fn collect_heredoc_body(
        &mut self,
        body: &HeredocBody,
        context: SurfaceScanContext<'_>,
    ) {
        self.collect_heredoc_body_parts(&body.parts, context);
    }

    pub(super) fn record_unset_array_target_word(&mut self, word: &Word) {
        if word_looks_like_unset_array_target(word, self.source) {
            self.facts.subscript_spans.push(word.span);
        }
    }

    fn collect_open_double_quote_fragments(
        &mut self,
        word: &Word,
        command_name: Option<&str>,
        assignment_target: Option<&str>,
    ) {
        let fragments = suspect_double_quote_spans(
            word,
            self.source,
            command_name,
            assignment_target,
        );
        if fragments.is_empty() {
            return;
        }

        let replacement = rewrite_word_as_single_double_quoted_string(
            word,
            self.source,
            assignment_target,
        );
        for (opening_span, closing_span) in fragments {
            self.facts
                .open_double_quotes
                .push(OpenDoubleQuoteFragmentFact {
                    span: opening_span,
                    replacement_span: word.span,
                    replacement: replacement.clone(),
                });
            self.facts
                .suspect_closing_quotes
                .push(SuspectClosingQuoteFragmentFact { span: closing_span });
        }
    }

    pub(super) fn collect_split_suspect_closing_quote_fragment_in_words(&mut self, words: &[Word]) {
        for (index, word) in words.iter().enumerate() {
            let has_later_words = index + 1 < words.len();
            for span in split_suspect_closing_quote_spans(word, self.source, has_later_words) {
                if self
                    .facts
                    .suspect_closing_quotes
                    .iter()
                    .any(|fragment| fragment.span() == span)
                {
                    continue;
                }
                self.facts
                    .suspect_closing_quotes
                    .push(SuspectClosingQuoteFragmentFact { span });
            }
        }
    }

    fn collect_word_parts(&mut self, parts: &[WordPartNode], context: SurfaceScanContext<'_>) {
        for (index, part) in parts.iter().enumerate() {
            if let WordPart::Variable(name) = &part.kind
                && matches!(
                    name.as_str(),
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                )
                && let Some(next_part) = parts.get(index + 1)
                && let WordPart::Literal(text) = &next_part.kind
                && text
                    .as_str(self.source, next_part.span)
                    .starts_with(|char: char| char.is_ascii_digit())
                && self.looks_like_unbraced_positional_above_nine(part.span.merge(next_part.span))
            {
                self.facts
                    .positional_parameters
                    .push(PositionalParameterFragmentFact {
                        span: part.span.merge(next_part.span),
                        kind: PositionalParameterFragmentKind::AboveNine,
                        guarded: context.guarded_parameter_operand,
                    });
            }

            match &part.kind {
                WordPart::SingleQuoted { dollar, .. } => {
                    self.facts.single_quoted.push(SingleQuotedFragmentFact {
                        span: part.span,
                        diagnostic_span: self.single_quoted_fragment_diagnostic_span(part.span),
                        dollar_quoted: *dollar,
                        command_name: context
                            .command_name
                            .map(str::to_owned)
                            .map(String::into_boxed_str),
                        assignment_target: context
                            .assignment_target
                            .map(str::to_owned)
                            .map(String::into_boxed_str),
                        variable_set_operand: context.variable_set_operand,
                        literal_backslash_in_single_quotes_span:
                            single_quoted_backslash_continuation_span(parts, index, self.source),
                    });
                }
                WordPart::DoubleQuoted { parts, dollar } => {
                    if *dollar {
                        self.facts
                            .dollar_double_quoted
                            .push(DollarDoubleQuotedFragmentFact { span: part.span });
                    }
                    self.collect_word_parts(parts, context);
                }
                WordPart::ZshQualifiedGlob(glob) => self.collect_zsh_qualified_glob(glob, context),
                WordPart::ArithmeticExpansion {
                    expression,
                    syntax: ArithmeticExpansionSyntax::LegacyBracket,
                    expression_word_ast,
                    expression_ast,
                    ..
                } => {
                    self.facts
                        .legacy_arithmetic
                        .push(LegacyArithmeticFragmentFact { span: part.span });
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression_ast.as_ref(),
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
                        });
                    } else {
                        self.collect_word(expression_word_ast, context);
                    }
                }
                WordPart::ArithmeticExpansion {
                    expression,
                    expression_word_ast,
                    expression_ast,
                    ..
                } => {
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression_ast.as_ref(),
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
                        });
                    } else {
                        self.collect_word(expression_word_ast, context);
                    }
                }
                WordPart::CommandSubstitution {
                    syntax: CommandSubstitutionSyntax::Backtick,
                    body,
                    ..
                } => {
                    if self.opening_backtick_is_escaped(part.span) {
                        continue;
                    }
                    self.facts.backticks.push(BacktickFragmentFact {
                        span: part.span,
                        empty: body.is_empty(),
                    });
                }
                WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
                WordPart::Parameter(parameter) => {
                    self.collect_parameter_expansion(parameter, part.span, context);
                }
                WordPart::Variable(name)
                    if name.as_str() == "$"
                        && contains_nested_parameter_marker(part.span.slice(self.source)) =>
                {
                    self.facts
                        .nested_parameter_expansions
                        .push(NestedParameterExpansionFragmentFact { span: part.span });
                }
                WordPart::ParameterExpansion {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    if reference_has_array_subscript(reference) {
                        self.record_array_reference(part.span, false);
                    }
                    if parameter_pattern_target_is_special(reference, operator) {
                        for pattern_span in parameter_operator_special_target_word_spans(operator) {
                            self.record_parameter_pattern_special_target(pattern_span);
                        }
                    }
                    if matches!(
                        operator,
                        ParameterOp::UpperFirst
                            | ParameterOp::UpperAll
                            | ParameterOp::LowerFirst
                            | ParameterOp::LowerAll
                    ) {
                        self.record_case_modification(part.span);
                    }
                    if matches!(
                        operator,
                        ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
                    ) {
                        self.record_replacement_expansion(part.span);
                    }
                    if matches!(operator, ParameterOp::RemoveSuffixLong { .. })
                        && reference.name.as_str() == "*"
                    {
                        self.record_star_glob_removal(part.span);
                    }
                    self.record_var_ref_subscript(reference);
                    self.collect_parameter_operator_patterns(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_ref(),
                        context,
                    );
                }
                WordPart::Length(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::Transformation { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArrayAccess(reference) => {
                    if reference_has_array_subscript(reference) {
                        self.record_array_reference(part.span, true);
                        let case_modification_span = parts
                            .get(index + 1)
                            .filter(|next_part| {
                                matches!(&next_part.kind, WordPart::Literal(text) if {
                                    let text = text.as_str(self.source, next_part.span);
                                    text.starts_with('^') || text.starts_with(',')
                                })
                            })
                            .map_or(part.span, |next_part| part.span.merge(next_part.span));
                        self.record_case_modification(case_modification_span);
                    }
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArrayIndices(reference) => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: true,
                        });
                }
                WordPart::Substring { reference, .. } => {
                    self.record_substring_expansion(part.span);
                    self.record_var_ref_subscript(reference);
                }
                WordPart::ArraySlice { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: Some(operator),
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                    self.collect_parameter_operator_patterns(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_ref(),
                        context,
                    );
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator: None,
                    ..
                } => {
                    self.record_var_ref_subscript(reference);
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                }
                WordPart::PrefixMatch { .. } => {
                    self.facts
                        .indirect_expansions
                        .push(IndirectExpansionFragmentFact {
                            span: part.span,
                            array_keys: false,
                        });
                }
                WordPart::Literal(_) | WordPart::Variable(_) => {}
            }
        }
    }

    fn collect_heredoc_body_parts(
        &mut self,
        parts: &[HeredocBodyPartNode],
        context: SurfaceScanContext<'_>,
    ) {
        for (index, part) in parts.iter().enumerate() {
            if let HeredocBodyPart::Variable(name) = &part.kind
                && matches!(
                    name.as_str(),
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                )
                && let Some(next_part) = parts.get(index + 1)
                && let HeredocBodyPart::Literal(text) = &next_part.kind
                && text
                    .as_str(self.source, next_part.span)
                    .starts_with(|char: char| char.is_ascii_digit())
                && self.looks_like_unbraced_positional_above_nine(part.span.merge(next_part.span))
            {
                self.facts
                    .positional_parameters
                    .push(PositionalParameterFragmentFact {
                        span: part.span.merge(next_part.span),
                        kind: PositionalParameterFragmentKind::AboveNine,
                        guarded: context.guarded_parameter_operand,
                    });
            }

            match &part.kind {
                HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
                HeredocBodyPart::CommandSubstitution {
                    syntax: CommandSubstitutionSyntax::Backtick,
                    body,
                    ..
                } => {
                    if self.opening_backtick_is_escaped(part.span) {
                        continue;
                    }
                    self.facts.backticks.push(BacktickFragmentFact {
                        span: part.span,
                        empty: body.is_empty(),
                    });
                }
                HeredocBodyPart::CommandSubstitution { .. } => {}
                HeredocBodyPart::ArithmeticExpansion {
                    expression,
                    syntax: ArithmeticExpansionSyntax::LegacyBracket,
                    expression_word_ast,
                    expression_ast,
                    ..
                } => {
                    self.facts
                        .legacy_arithmetic
                        .push(LegacyArithmeticFragmentFact { span: part.span });
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression_ast.as_ref(),
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
                        });
                    } else {
                        self.collect_word(expression_word_ast, context);
                    }
                }
                HeredocBodyPart::ArithmeticExpansion {
                    expression,
                    expression_word_ast,
                    expression_ast,
                    ..
                } => {
                    collect_positional_parameter_operator_spans_in_arithmetic(
                        part.span,
                        expression_ast.as_ref(),
                        expression,
                        self.source,
                        &mut self.facts.positional_parameter_operator_spans,
                    );
                    if let Some(expression_ast) = expression_ast.as_ref() {
                        query::visit_arithmetic_words(expression_ast, &mut |word| {
                            self.collect_word(word, context);
                        });
                    } else {
                        self.collect_word(expression_word_ast, context);
                    }
                }
                HeredocBodyPart::Parameter(parameter) => {
                    self.collect_parameter_expansion(parameter, part.span, context);
                }
            }
        }
    }

    pub(super) fn collect_pattern(&mut self, pattern: &Pattern, context: SurfaceScanContext<'_>) {
        self.collect_pattern_impl(pattern, context, true);
    }

    pub(super) fn collect_pattern_structure(
        &mut self,
        pattern: &Pattern,
        context: SurfaceScanContext<'_>,
    ) {
        self.collect_pattern_impl(pattern, context, false);
    }

    fn collect_pattern_impl(
        &mut self,
        pattern: &Pattern,
        context: SurfaceScanContext<'_>,
        collect_words: bool,
    ) {
        for (part, span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { kind, patterns } => {
                    if *kind == PatternGroupKind::ExactlyOne {
                        self.facts.pattern_exactly_one_extglob_spans.push(span);
                    }
                    for pattern in patterns {
                        self.collect_pattern_impl(pattern, context, collect_words);
                    }
                }
                PatternPart::Word(word) if collect_words => {
                    self.collect_word(word, context);
                }
                PatternPart::Word(_) => {}
                PatternPart::CharClass(_) if context.collect_pattern_charclasses => {
                    self.facts.pattern_charclass_spans.push(span);
                    if context.nested_word_command {
                        self.facts.nested_pattern_charclass_spans.push(span);
                    }
                }
                PatternPart::CharClass(_)
                | PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar => {}
            }
        }
    }

    fn collect_fragment_word(
        &mut self,
        word: Option<&Word>,
        text: Option<&SourceText>,
        context: SurfaceScanContext<'_>,
    ) {
        let Some(text) = text else {
            return;
        };
        let snippet = text.slice(self.source);
        if snippet.is_empty() {
            return;
        }

        debug_assert!(
            word.is_some(),
            "parser-backed fragment text should always carry a word AST"
        );
        let Some(word) = word else {
            return;
        };
        self.collect_word(word, context.without_open_double_quote_scan());
    }

    fn collect_zsh_qualified_glob(
        &mut self,
        glob: &ZshQualifiedGlob,
        context: SurfaceScanContext<'_>,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_pattern(pattern, context);
            }
        }
    }

    fn collect_parameter_expansion(
        &mut self,
        parameter: &shuck_ast::ParameterExpansion,
        span: Span,
        context: SurfaceScanContext<'_>,
    ) {
        let guarded_reference = context.guarded_parameter_operand
            || parameter_expansion_guards_unset_reference(parameter);

        self.record_special_positional_parameter(parameter, guarded_reference);
        if span.slice(self.source).starts_with("${##") {
            self.facts
                .positional_parameters
                .push(PositionalParameterFragmentFact {
                    span,
                    kind: PositionalParameterFragmentKind::General,
                    guarded: guarded_reference,
                });
        }
        if is_nested_parameter_expansion(parameter, self.source) {
            self.facts
                .nested_parameter_expansions
                .push(NestedParameterExpansionFragmentFact { span });
        }
        if parameter_has_array_reference(parameter) {
            self.record_array_reference(span, parameter_is_plain_array_reference(parameter));
        }
        if parameter_has_substring_expansion(parameter) {
            self.record_substring_expansion(span);
        }
        if parameter_has_case_modification(parameter) {
            self.record_case_modification(span);
        }
        if parameter_has_replacement_expansion(parameter) {
            self.record_replacement_expansion(span);
        }
        if parameter_has_star_glob_removal(parameter) {
            self.record_star_glob_removal(span);
        }
        self.record_parameter_subscripts(parameter);
        if let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax {
            if matches!(
                syntax,
                BourneParameterExpansion::Indirect { .. }
                    | BourneParameterExpansion::PrefixMatch { .. }
                    | BourneParameterExpansion::Indices { .. }
            ) {
                self.facts
                    .indirect_expansions
                    .push(IndirectExpansionFragmentFact {
                        span,
                        array_keys: matches!(syntax, BourneParameterExpansion::Indices { .. }),
                    });
            }
            match syntax {
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                }
                | BourneParameterExpansion::Indirect {
                    reference,
                    operator: Some(operator),
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    if parameter_pattern_target_is_special(reference, operator) {
                        for pattern_span in parameter_operator_special_target_word_spans(operator) {
                            self.record_parameter_pattern_special_target(pattern_span);
                        }
                    }
                    self.collect_parameter_operator_patterns(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_ref(),
                        context,
                    );
                }
                BourneParameterExpansion::Access { .. }
                | BourneParameterExpansion::Length { .. }
                | BourneParameterExpansion::Indices { .. }
                | BourneParameterExpansion::Indirect { operator: None, .. }
                | BourneParameterExpansion::PrefixMatch { .. }
                | BourneParameterExpansion::Slice { .. }
                | BourneParameterExpansion::Transformation { .. } => {}
            }
        }
    }

    fn record_special_positional_parameter(
        &mut self,
        parameter: &shuck_ast::ParameterExpansion,
        guarded: bool,
    ) {
        let reference = match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Indirect { reference, .. }
                | BourneParameterExpansion::Slice { reference, .. }
                | BourneParameterExpansion::Operation { reference, .. }
                | BourneParameterExpansion::Transformation { reference, .. } => Some(reference),
                BourneParameterExpansion::PrefixMatch { .. } => None,
            },
            ParameterExpansionSyntax::Zsh(_) => None,
        };

        if let Some(reference) = reference
            && reference.subscript.is_none()
            && matches!(reference.name.as_str(), "@" | "*" | "#")
        {
            self.facts
                .positional_parameters
                .push(PositionalParameterFragmentFact {
                    span: reference.span,
                    kind: PositionalParameterFragmentKind::General,
                    guarded,
                });
        }
    }

    fn collect_parameter_operator_patterns(
        &mut self,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        operand_word_ast: Option<&Word>,
        context: SurfaceScanContext<'_>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                self.record_parameter_pattern(pattern.span);
                self.collect_pattern(pattern, context.with_pattern_charclass_scan())
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => {
                self.record_parameter_pattern(pattern.span);
                self.collect_pattern(pattern, context.with_pattern_charclass_scan());
                self.collect_fragment_word(
                    operator.replacement_word_ast(),
                    Some(replacement),
                    context,
                );
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error => {
                self.collect_fragment_word(
                    operand_word_ast,
                    operand,
                    context.guarded_parameter_operand(),
                );
            }
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    fn record_parameter_subscripts(&mut self, parameter: &shuck_ast::ParameterExpansion) {
        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Indirect { reference, .. }
                | BourneParameterExpansion::Slice { reference, .. }
                | BourneParameterExpansion::Operation { reference, .. }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    self.record_var_ref_subscript(reference);
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
            },
            ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    self.record_var_ref_subscript(reference)
                }
                ZshExpansionTarget::Nested(parameter) => {
                    self.record_parameter_subscripts(parameter)
                }
                ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => {}
            },
        }
    }

    pub(super) fn record_var_ref_subscript(&mut self, reference: &VarRef) {
        self.record_subscript(reference.subscript.as_ref());
    }

    pub(super) fn record_subscript(&mut self, subscript: Option<&Subscript>) {
        let Some(subscript) = subscript else {
            return;
        };
        if subscript.selector().is_some() {
            return;
        }
        self.facts.subscript_spans.push(subscript.span());
    }

    fn single_quoted_fragment_diagnostic_span(&self, part_span: Span) -> Span {
        if !self
            .facts
            .backticks
            .iter()
            .any(|fragment| span_contains(fragment.span, part_span))
        {
            return part_span;
        }

        let escaped_dollar_count =
            backtick_display_escaped_dollar_count(part_span.slice(self.source));
        let Some(chain_start) = continued_line_chain_start(part_span.start, self.source) else {
            return adjust_end_column(part_span, escaped_dollar_count);
        };

        let start = shellcheck_collapsed_position(chain_start, part_span.start, self.source);
        let end = adjust_end_column(
            Span::from_positions(
                start,
                shellcheck_collapsed_position(chain_start, part_span.end, self.source),
            ),
            escaped_dollar_count,
        )
        .end;

        Span::from_positions(start, end)
    }
}

fn continued_line_chain_start(target: Position, source: &str) -> Option<Position> {
    let mut line_start_offset = source[..target.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let mut line = target.line;
    let original_start = line_start_offset;

    while line_start_offset > 0 {
        let previous_line_end = line_start_offset - 1;
        let previous_line_start = source[..previous_line_end]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let previous_line = &source[previous_line_start..previous_line_end];
        if !previous_line.trim_end_matches([' ', '\t']).ends_with('\\') {
            break;
        }
        line_start_offset = previous_line_start;
        line -= 1;
    }

    (line_start_offset != original_start).then_some(Position {
        line,
        column: 1,
        offset: line_start_offset,
    })
}

fn shellcheck_collapsed_position(
    chain_start: Position,
    target: Position,
    source: &str,
) -> Position {
    let mut line = chain_start.line;
    let mut column = chain_start.column;
    let prefix = &source[chain_start.offset..target.offset];
    let mut index = 0usize;

    while index < prefix.len() {
        if prefix[index..].starts_with("\\\r\n") {
            index += "\\\r\n".len();
            continue;
        }

        if prefix[index..].starts_with("\\\n") {
            index += "\\\n".len();
            continue;
        }

        let ch = prefix[index..]
            .chars()
            .next()
            .expect("prefix iteration should stay on UTF-8 boundaries");
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
        index += ch.len_utf8();
    }

    Position {
        line,
        column,
        offset: target.offset,
    }
}

fn adjust_end_column(span: Span, display_escape_count: usize) -> Span {
    if display_escape_count == 0 {
        return span;
    }

    let mut end = span.end;
    end.column = end.column.saturating_sub(display_escape_count);
    Span::from_positions(span.start, end)
}

fn backtick_display_escaped_dollar_count(text: &str) -> usize {
    let Some(inner) = text
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''))
    else {
        return 0;
    };

    inner
        .as_bytes()
        .windows(2)
        .filter(|pair| pair[0] == b'\\' && pair[1] == b'$')
        .count()
}

fn quoted_parameter_target_len(text: &str) -> Option<usize> {
    match text.as_bytes().first().copied() {
        Some(b'\'') => single_quoted_fragment_len(text),
        Some(b'"') => double_quoted_fragment_len(text),
        _ => None,
    }
}

fn zsh_parameter_index_flag_spans_in_word(text: &str, span: Span) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut search_from = 0usize;

    while let Some(start) = next_live_parameter_expansion_start(text, search_from) {
        let body = &text[start + 2..];
        let Some(target_len) = quoted_parameter_target_len(body) else {
            search_from = start + 2;
            continue;
        };
        if !body[target_len..].starts_with('[') {
            search_from = start + 2;
            continue;
        }

        let target_start = span.start.advanced_by(&text[..start]);
        let target_end = target_start.advanced_by(&text[start..start + 2 + target_len]);
        spans.push(Span::from_positions(target_start, target_end));
        search_from = start + 2 + target_len;
    }

    spans
}

fn next_live_parameter_expansion_start(text: &str, search_from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = search_from;
    let mut in_double_quotes = false;

    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }

        if !in_double_quotes && bytes[index..].starts_with(b"$'") {
            index += 1 + dollar_single_quoted_fragment_len(&text[index + 1..])?;
            continue;
        }

        if !in_double_quotes && bytes[index] == b'\'' {
            index += single_quoted_fragment_len(&text[index..])?;
            continue;
        }

        if bytes[index] == b'"' {
            in_double_quotes = !in_double_quotes;
            index += 1;
            continue;
        }

        if bytes[index..].starts_with(b"${") {
            return Some(index);
        }

        index += 1;
    }

    None
}

fn single_quoted_fragment_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with('\''));
    text[1..].find('\'').map(|offset| offset + 2)
}

fn dollar_single_quoted_fragment_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with('\''));
    let bytes = text.as_bytes();
    let mut index = 1usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }
        if bytes[index] == b'\'' {
            return Some(index + 1);
        }
        index += 1;
    }

    None
}

fn single_quoted_backslash_continuation_span(
    parts: &[WordPartNode],
    index: usize,
    source: &str,
) -> Option<Span> {
    let part = parts.get(index)?;
    if !single_quoted_part_contains_backslash_letter(part, source) {
        return None;
    }

    let next_part = parts.get(index + 1)?;
    let WordPart::Literal(text) = &next_part.kind else {
        return None;
    };
    if !text
        .as_str(source, next_part.span)
        .starts_with(|char: char| char.is_ascii_alphabetic())
    {
        return None;
    }

    let raw = part.span.slice(source);
    let closing_quote = part.span.start.advanced_by(&raw[..raw.len() - 1]);
    Some(Span::from_positions(closing_quote, closing_quote))
}

fn single_quoted_part_contains_backslash_letter(part: &WordPartNode, source: &str) -> bool {
    let WordPart::SingleQuoted { dollar: false, .. } = part.kind else {
        return false;
    };
    let Some(inner) = part
        .span
        .slice(source)
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''))
    else {
        return false;
    };

    inner
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_alphabetic())
}

fn double_quoted_fragment_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with('"'));
    let bytes = text.as_bytes();
    let mut index = 1usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }
        if bytes[index..].starts_with(b"${") {
            index += parameter_expansion_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"$(") {
            index += command_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'`' {
            index += backtick_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }

    None
}

fn parameter_expansion_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with("${"));
    let bytes = text.as_bytes();
    let mut index = 2usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }
        if bytes[index] == b'\'' {
            index += single_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'"' {
            index += double_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'`' {
            index += backtick_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"${") {
            index += parameter_expansion_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"$(") {
            index += command_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'}' {
            return Some(index + 1);
        }
        index += 1;
    }

    None
}

fn command_substitution_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with("$("));
    let bytes = text.as_bytes();
    let mut index = 2usize;
    let mut paren_depth = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }
        if bytes[index] == b'\'' {
            index += single_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'"' {
            index += double_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'`' {
            index += backtick_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"${") {
            index += parameter_expansion_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"$(") {
            index += command_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'(' {
            paren_depth += 1;
            index += 1;
            continue;
        }
        if bytes[index] == b')' {
            if paren_depth == 0 {
                return Some(index + 1);
            }
            paren_depth -= 1;
            index += 1;
            continue;
        }
        index += 1;
    }

    None
}

fn backtick_substitution_len(text: &str) -> Option<usize> {
    debug_assert!(text.starts_with('`'));
    let bytes = text.as_bytes();
    let mut index = 1usize;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }
        if bytes[index] == b'\'' {
            index += single_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'"' {
            index += double_quoted_fragment_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"${") {
            index += parameter_expansion_len(&text[index..])?;
            continue;
        }
        if bytes[index..].starts_with(b"$(") {
            index += command_substitution_len(&text[index..])?;
            continue;
        }
        if bytes[index] == b'`' {
            return Some(index + 1);
        }
        index += 1;
    }

    None
}

fn parameter_expansion_guards_unset_reference(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Operation { operator, .. }
            | BourneParameterExpansion::Indirect {
                operator: Some(operator),
                ..
            },
        ) => parameter_operator_guards_unset_reference(operator),
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { operator: None, .. }
            | BourneParameterExpansion::PrefixMatch { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. },
        )
        | ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn parameter_operator_guards_unset_reference(operator: &ParameterOp) -> bool {
    matches!(
        operator,
        ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
    )
}

fn parameter_has_array_reference(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Slice { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                reference_has_array_subscript(reference)
            }
            BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => reference_has_array_subscript(reference),
            ZshExpansionTarget::Nested(parameter) => parameter_has_array_reference(parameter),
            ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_is_plain_array_reference(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            reference_has_array_subscript(reference)
        }
        ParameterExpansionSyntax::Zsh(syntax)
            if syntax.length_prefix.is_none()
                && syntax.operation.is_none()
                && syntax.modifiers.is_empty() =>
        {
            match &syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    reference_has_array_subscript(reference)
                }
                ZshExpansionTarget::Nested(parameter) => {
                    parameter_is_plain_array_reference(parameter)
                }
                ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
            }
        }
        _ => false,
    }
}

fn parameter_operator_has_pattern(operator: &ParameterOp) -> bool {
    matches!(
        operator,
        ParameterOp::RemovePrefixShort { .. }
            | ParameterOp::RemovePrefixLong { .. }
            | ParameterOp::RemoveSuffixShort { .. }
            | ParameterOp::RemoveSuffixLong { .. }
            | ParameterOp::ReplaceFirst { .. }
            | ParameterOp::ReplaceAll { .. }
    )
}

fn parameter_operator_special_target_word_spans(operator: &ParameterOp) -> Vec<Span> {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern }
        | ParameterOp::ReplaceFirst { pattern, .. }
        | ParameterOp::ReplaceAll { pattern, .. } => pattern_special_target_word_spans(pattern),
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => Vec::new(),
    }
}

fn pattern_special_target_word_spans(pattern: &Pattern) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_pattern_special_target_word_spans(pattern, &mut spans);
    spans
}

fn collect_pattern_special_target_word_spans(pattern: &Pattern, spans: &mut Vec<Span>) {
    for (part, span) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_pattern_special_target_word_spans(pattern, spans);
                }
            }
            PatternPart::Word(_) => spans.push(span),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn parameter_pattern_target_is_special(reference: &VarRef, operator: &ParameterOp) -> bool {
    parameter_operator_has_pattern(operator)
        && (reference_has_array_subscript(reference) || reference.name.as_str() == "0")
}

fn parameter_has_substring_expansion(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice { reference, .. }) => {
            reference.subscript.is_none()
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_substring_expansion(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_case_modification(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            operator, ..
        }) => {
            matches!(
                operator,
                ParameterOp::UpperFirst
                    | ParameterOp::UpperAll
                    | ParameterOp::LowerFirst
                    | ParameterOp::LowerAll
            )
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_case_modification(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_replacement_expansion(parameter: &shuck_ast::ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            operator, ..
        }) => {
            matches!(
                operator,
                ParameterOp::ReplaceFirst { .. } | ParameterOp::ReplaceAll { .. }
            )
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Access { .. }
            | BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Transformation { .. }
            | BourneParameterExpansion::PrefixMatch { .. },
        ) => false,
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Nested(parameter) => parameter_has_replacement_expansion(parameter),
            ZshExpansionTarget::Reference(_)
            | ZshExpansionTarget::Word(_)
            | ZshExpansionTarget::Empty => false,
        },
    }
}

fn parameter_has_star_glob_removal(parameter: &shuck_ast::ParameterExpansion) -> bool {
    matches!(
        &parameter.syntax,
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
            reference,
            operator: ParameterOp::RemoveSuffixLong { .. },
            ..
        }) if reference.name.as_str() == "*"
    )
}

fn reference_has_array_subscript(reference: &VarRef) -> bool {
    reference.subscript.is_some()
}

fn collect_positional_parameter_operator_spans_in_arithmetic(
    expansion_span: Span,
    expression_ast: Option<&ArithmeticExprNode>,
    expression: &SourceText,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(expression_ast) = expression_ast {
        if arithmetic_expr_has_positional_parameter_operator(expression_ast, source) {
            spans.push(Span::from_positions(
                expansion_span.start,
                expansion_span.start,
            ));
        }
        return;
    }

    let text = expression.slice(source);
    let mut should_report = false;
    let mut state = ArithmeticScanState::default();
    let mut chars = text.char_indices();

    while let Some((index, char)) = chars.next() {
        match state {
            ArithmeticScanState::Normal => match char {
                '\'' => state = ArithmeticScanState::SingleQuoted,
                '"' => state = ArithmeticScanState::DoubleQuoted,
                '\\' => {
                    chars.next();
                }
                '$' => {
                    let Some(token_end) = positional_parameter_token_end(text, index) else {
                        continue;
                    };

                    let immediate_prev = text[..index].chars().next_back();
                    let immediate_next = text[token_end..].chars().next();
                    let same_word_prefix =
                        immediate_prev.is_some_and(|ch| !raw_arithmetic_word_boundary(ch));
                    let same_word_suffix =
                        immediate_next.is_some_and(|ch| !raw_arithmetic_word_boundary(ch));

                    if same_word_prefix || same_word_suffix {
                        if same_word_prefix {
                            let word_start = raw_arithmetic_word_start(text, index);
                            let prefix = &text[word_start..index];
                            if prefix_starts_with_identifier_like_text(prefix) {
                                should_report = true;
                                break;
                            }
                        }
                        continue;
                    }

                    let prev = text[..index].chars().rev().find(|ch| !ch.is_whitespace());
                    let next = text[token_end..].chars().find(|ch| !ch.is_whitespace());

                    if prev.is_some_and(is_left_operand_neighbor)
                        || next.is_some_and(is_right_operand_neighbor)
                    {
                        should_report = true;
                        break;
                    }
                }
                _ => {}
            },
            ArithmeticScanState::SingleQuoted => {
                if char == '\'' {
                    state = ArithmeticScanState::Normal;
                }
            }
            ArithmeticScanState::DoubleQuoted => match char {
                '"' => state = ArithmeticScanState::Normal,
                '\\' => {
                    chars.next();
                }
                _ => {}
            },
        }
    }

    if should_report {
        spans.push(Span::from_positions(
            expansion_span.start,
            expansion_span.start,
        ));
    }
}

fn raw_arithmetic_word_start(text: &str, end: usize) -> usize {
    let mut start = end;

    while let Some((index, ch)) = text[..start].char_indices().next_back() {
        if raw_arithmetic_word_boundary(ch) {
            break;
        }
        start = index;
    }

    start
}

fn raw_arithmetic_word_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '+' | '-'
                | '*'
                | '/'
                | '%'
                | '&'
                | '|'
                | '^'
                | '?'
                | ':'
                | '<'
                | '>'
                | '='
                | '!'
                | '~'
                | ','
                | '('
                | '['
        )
}

fn arithmetic_expr_has_positional_parameter_operator(
    expression: &ArithmeticExprNode,
    source: &str,
) -> bool {
    let mut should_report = false;
    query::visit_arithmetic_words(expression, &mut |word| {
        if word_has_unquoted_positional_parameter_operator_neighbors(word, source) {
            should_report = true;
        }
    });
    should_report
}

fn word_has_unquoted_positional_parameter_operator_neighbors(word: &Word, source: &str) -> bool {
    word.parts.iter().enumerate().any(|(index, part)| {
        part_is_unquoted_positional_parameter(&part.kind)
            && positional_parameter_part_has_identifier_like_prefix(word, index, source)
    })
}

fn part_is_unquoted_positional_parameter(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name_is_positional_parameter(name),
        WordPart::Parameter(parameter) => matches!(
            &parameter.syntax,
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none()
                    && name_is_positional_parameter(&reference.name)
        ),
        WordPart::ArrayAccess(reference) => {
            reference.subscript.is_none() && name_is_positional_parameter(&reference.name)
        }
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::DoubleQuoted { .. }
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. } => false,
    }
}

fn name_is_positional_parameter(name: &Name) -> bool {
    !name.as_str().is_empty() && name.as_str().bytes().all(|byte| byte.is_ascii_digit())
}

fn positional_parameter_part_has_identifier_like_prefix(
    word: &Word,
    index: usize,
    source: &str,
) -> bool {
    let Some(part) = word.parts.get(index) else {
        return false;
    };

    let prefix = &source[word.span.start.offset..part.span.start.offset];
    prefix_starts_with_identifier_like_text(prefix)
}

fn prefix_starts_with_identifier_like_text(prefix: &str) -> bool {
    let Some(first_non_whitespace) = prefix.chars().find(|ch| !ch.is_whitespace()) else {
        return false;
    };

    first_non_whitespace == '_' || first_non_whitespace.is_ascii_alphabetic()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ArithmeticScanState {
    #[default]
    Normal,
    SingleQuoted,
    DoubleQuoted,
}

fn positional_parameter_token_end(text: &str, start: usize) -> Option<usize> {
    let rest = text.get(start..)?;
    if !rest.starts_with('$') {
        return None;
    }

    let bytes = rest.as_bytes();
    if bytes.get(1).is_some_and(u8::is_ascii_digit) {
        let mut idx = 2usize;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
        return Some(start + idx);
    }

    if bytes.get(1) == Some(&b'{') {
        let mut idx = 2usize;
        let mut saw_digit = false;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            saw_digit = true;
            idx += 1;
        }
        if saw_digit && bytes.get(idx) == Some(&b'}') {
            return Some(start + idx + 1);
        }
    }

    None
}

fn is_left_operand_neighbor(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | ')' | ']' | '}' | '"' | '\'')
}

fn is_right_operand_neighbor(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '(' | '[' | '{' | '"' | '\'')
}

pub(super) fn build_subscript_index_reference_spans(
    semantic: &SemanticModel,
    subscript_spans: &[Span],
) -> FxHashSet<FactSpan> {
    if subscript_spans.is_empty() {
        return FxHashSet::default();
    }

    let references = semantic.references();
    if references.len().saturating_mul(subscript_spans.len()) <= 4_096 {
        return build_subscript_index_reference_spans_linear(references, subscript_spans);
    }

    let subscript_index = SubscriptSpanIndex::new(subscript_spans);
    references
        .iter()
        .filter(|reference| subscript_index.contains(reference.span))
        .map(|reference| FactSpan::new(reference.span))
        .collect()
}

fn build_subscript_index_reference_spans_linear(
    references: &[shuck_semantic::Reference],
    subscript_spans: &[Span],
) -> FxHashSet<FactSpan> {
    references
        .iter()
        .filter(|reference| {
            subscript_spans
                .iter()
                .any(|subscript| span_contains(*subscript, reference.span))
        })
        .map(|reference| FactSpan::new(reference.span))
        .collect()
}

#[derive(Debug, Default)]
struct SubscriptSpanIndex {
    starts: Vec<usize>,
    prefix_max_ends: Vec<usize>,
}

impl SubscriptSpanIndex {
    fn new(subscript_spans: &[Span]) -> Self {
        let mut bounds = subscript_spans
            .iter()
            .map(|span| (span.start.offset, span.end.offset))
            .collect::<Vec<_>>();
        bounds.sort_unstable();

        let mut starts = Vec::with_capacity(bounds.len());
        let mut prefix_max_ends = Vec::with_capacity(bounds.len());
        let mut max_end = 0usize;

        for (start, end) in bounds {
            starts.push(start);
            max_end = max_end.max(end);
            prefix_max_ends.push(max_end);
        }

        Self {
            starts,
            prefix_max_ends,
        }
    }

    fn contains(&self, span: Span) -> bool {
        let candidate_count = self
            .starts
            .partition_point(|start| *start <= span.start.offset);
        candidate_count > 0 && self.prefix_max_ends[candidate_count - 1] >= span.end.offset
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn word_looks_like_unset_array_target(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    let Some((name, _)) = text.split_once('[') else {
        return false;
    };
    text.ends_with(']') && is_shell_name(name)
}

fn is_shell_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|char| char == '_' || char.is_ascii_alphanumeric())
}

fn suspect_double_quote_spans(
    word: &Word,
    source: &str,
    _command_name: Option<&str>,
    assignment_target: Option<&str>,
) -> Vec<(Span, Span)> {
    word.parts
        .iter()
        .enumerate()
        .filter_map(|(index, current)| {
            if !suspicious_open_quote_fragment(
                word,
                source,
                index,
                current,
                assignment_target.is_some(),
            ) {
                return None;
            }

            Some((
                opening_quote_span(current, source)?,
                closing_quote_span(current, source)?,
            ))
        })
        .collect()
}

pub(super) fn rewrite_word_as_single_double_quoted_string(
    word: &Word,
    source: &str,
    assignment_target: Option<&str>,
) -> Box<str> {
    let mut rendered = String::from("\"");
    for part in &word.parts {
        render_word_part_inside_double_quotes(&mut rendered, part, source, false);
    }
    rendered.push('"');
    if let Some(assignment_target) = assignment_target {
        format!("{assignment_target}={rendered}").into_boxed_str()
    } else {
        rendered.into_boxed_str()
    }
}

pub(super) fn rewrite_pattern_as_single_double_quoted_string(
    pattern: &Pattern,
    source: &str,
) -> Box<str> {
    let mut rendered = String::from("\"");
    for part in &pattern.parts {
        render_pattern_part_inside_double_quotes(&mut rendered, part, source);
    }
    rendered.push('"');
    rendered.into_boxed_str()
}

fn render_pattern_part_inside_double_quotes(
    rendered: &mut String,
    part: &PatternPartNode,
    source: &str,
) {
    match &part.kind {
        PatternPart::Literal(text) => {
            push_double_quoted_literal(rendered, text.as_str(source, part.span));
        }
        PatternPart::Word(word) => {
            for word_part in &word.parts {
                render_word_part_inside_double_quotes(rendered, word_part, source, false);
            }
        }
        PatternPart::AnyString
        | PatternPart::AnyChar
        | PatternPart::CharClass(_)
        | PatternPart::Group { .. } => {
            let syntax = Pattern {
                parts: vec![part.clone()],
                span: part.span,
            }
            .render_syntax(source);
            push_double_quoted_literal(rendered, &syntax);
        }
    }
}

fn render_word_part_inside_double_quotes(
    rendered: &mut String,
    part: &WordPartNode,
    source: &str,
    source_is_double_quoted: bool,
) {
    match &part.kind {
        WordPart::Literal(text) => {
            if source_is_double_quoted {
                push_double_quoted_literal(rendered, text.as_str(source, part.span));
            } else {
                push_cooked_unquoted_literal_inside_double_quotes(
                    rendered,
                    text.as_str(source, part.span),
                );
            }
        }
        WordPart::SingleQuoted { value, .. } => {
            push_double_quoted_literal(rendered, value.slice(source));
        }
        WordPart::DoubleQuoted { parts, .. } => {
            for nested_part in parts {
                render_word_part_inside_double_quotes(rendered, nested_part, source, true);
            }
        }
        WordPart::Variable(name) => {
            rendered.push_str("${");
            rendered.push_str(name.as_ref());
            rendered.push('}');
        }
        _ => rendered.push_str(&word_part_syntax(part, source)),
    }
}

fn push_double_quoted_literal(rendered: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '"' | '\\' | '$' | '`' => {
                rendered.push('\\');
                rendered.push(ch);
            }
            _ => rendered.push(ch),
        }
    }
}

fn push_cooked_unquoted_literal_inside_double_quotes(rendered: &mut String, text: &str) {
    let mut cooked = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some(escaped) => cooked.push(escaped),
                None => cooked.push('\\'),
            }
            continue;
        }

        cooked.push(ch);
    }

    push_double_quoted_literal(rendered, &cooked);
}

fn word_part_syntax(part: &WordPartNode, source: &str) -> String {
    Word {
        parts: vec![part.clone()],
        span: part.span,
        brace_syntax: Vec::new(),
    }
    .render_syntax(source)
}

pub(super) fn word_has_reopened_double_quote_window(
    word: &Word,
    source: &str,
    command_name: Option<&str>,
) -> bool {
    !suspect_double_quote_spans(word, source, command_name, None).is_empty()
}

fn suspicious_open_quote_fragment(
    word: &Word,
    source: &str,
    index: usize,
    current: &WordPartNode,
    assignment_context: bool,
) -> bool {
    (!assignment_context && suspicious_multiline_double_quote_suffix(word, source, index, current))
        || suspicious_reopened_single_quote_window(word, source, index, current)
}

fn suspicious_multiline_double_quote_suffix(
    word: &Word,
    source: &str,
    index: usize,
    current: &WordPartNode,
) -> bool {
    if !matches!(current.kind, WordPart::DoubleQuoted { .. })
        || !current.span.slice(source).contains('\n')
        || !quote_starts_nonempty_multiline_fragment(current, source)
    {
        return false;
    }

    let Some(next) = word.parts.get(index + 1) else {
        return false;
    };
    if immediate_double_quote_continuation_is_suspicious(next, source) {
        return true;
    }

    word_part_is_empty_literal(next, source)
        && !word.parts[..index]
            .iter()
            .any(|part| matches!(part.kind, WordPart::DoubleQuoted { .. }) && part.span.slice(source).contains('\n'))
        && word
            .parts
            .get(index + 2)
            .is_some_and(|part| matches!(part.kind, WordPart::DoubleQuoted { .. }))
}

fn suspicious_reopened_single_quote_window(
    word: &Word,
    source: &str,
    index: usize,
    current: &WordPartNode,
) -> bool {
    let WordPart::SingleQuoted { dollar: false, .. } = current.kind else {
        return false;
    };
    current.span.slice(source).contains('\n')
        && single_quote_reopens_after_literal_run(&word.parts[index + 1..], source)
}

fn single_quote_reopens_after_literal_run(parts: &[WordPartNode], source: &str) -> bool {
    let mut saw_literal = false;
    let mut first_nonempty_literal: Option<&str> = None;

    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let text = text.as_str(source, part.span);
                if !text.is_empty() {
                    saw_literal = true;
                    if first_nonempty_literal.is_none() {
                        first_nonempty_literal = Some(text);
                    }
                }
            }
            WordPart::SingleQuoted { dollar: false, .. } => {
                return if let Some(first_literal) = first_nonempty_literal {
                    !first_literal.starts_with('\\')
                } else {
                    saw_literal || !parts.is_empty()
                };
            }
            WordPart::SingleQuoted { dollar: true, .. }
            | WordPart::DoubleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
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
            | WordPart::ZshQualifiedGlob(_) => return false,
        }
    }

    false
}

fn middle_part_is_word_like_literal_gap(part: &WordPartNode, source: &str) -> bool {
    let WordPart::Literal(text) = &part.kind else {
        return false;
    };
    let text = text.as_str(source, part.span);
    split_quote_tail_is_suspicious(text) || backslash_prefixed_word_like_literal_gap(text)
}

fn word_part_is_empty_literal(part: &WordPartNode, source: &str) -> bool {
    matches!(&part.kind, WordPart::Literal(text) if text.as_str(source, part.span).is_empty())
}

fn quote_starts_nonempty_multiline_fragment(part: &WordPartNode, source: &str) -> bool {
    let quote = match part.kind {
        WordPart::DoubleQuoted { .. } => '"',
        WordPart::SingleQuoted { .. } => '\'',
        _ => return false,
    };
    let text = part.span.slice(source);
    let Some(quote_offset) = text.find(quote) else {
        return false;
    };
    let after_quote = &text[quote_offset + quote.len_utf8()..];
    !after_quote.starts_with('\n') && !after_quote.starts_with("\r\n")
}

fn immediate_double_quote_continuation_is_suspicious(part: &WordPartNode, source: &str) -> bool {
    matches!(part.kind, WordPart::DoubleQuoted { .. })
        || immediate_double_quote_scalar_gap(part)
        || immediate_double_quote_substitution_gap(part)
        || middle_part_is_word_like_literal_gap(part, source)
}

fn immediate_double_quote_scalar_gap(part: &WordPartNode) -> bool {
    matches!(
        part.kind,
        WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
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
    )
}

fn immediate_double_quote_substitution_gap(part: &WordPartNode) -> bool {
    matches!(
        part.kind,
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. }
    )
}

fn double_quoted_part_is_empty(part: &WordPartNode, source: &str) -> bool {
    let WordPart::DoubleQuoted { parts, .. } = &part.kind else {
        return false;
    };
    parts.iter().all(|inner| match &inner.kind {
        WordPart::Literal(text) => text.as_str(source, inner.span).is_empty(),
        _ => false,
    })
}

fn split_suspect_closing_quote_spans(
    word: &Word,
    source: &str,
    has_later_words: bool,
) -> Vec<Span> {
    word.parts
        .windows(2)
        .enumerate()
        .filter_map(|window| {
            let (index, [current, next]) = window else {
                return None;
            };
            let WordPart::DoubleQuoted { .. } = &current.kind else {
                return None;
            };
            let WordPart::Literal(text) = &next.kind else {
                return None;
            };
            if !current.span.slice(source).contains('\n') {
                return None;
            }

            let tail = text.as_str(source, next.span);
            if !split_quote_tail_is_suspicious(tail) {
                return None;
            }

            let span = closing_quote_span(current, source)?;
            if span.start.column == 1
                || (index > 0
                    && double_quoted_part_is_empty(&word.parts[index - 1], source)
                    && has_later_words)
            {
                Some(span)
            } else {
                None
            }
        })
        .collect()
}

fn split_quote_tail_is_suspicious(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic()) && chars.all(|char| !char.is_whitespace())
}

fn opening_quote_span(part: &WordPartNode, source: &str) -> Option<Span> {
    let quote = match part.kind {
        WordPart::DoubleQuoted { .. } => '"',
        WordPart::SingleQuoted { .. } => '\'',
        _ => return None,
    };
    let span = part.span;
    let text = span.slice(source);
    let quote_offset = text.find(quote)?;
    let start = span.start.advanced_by(&text[..quote_offset]);
    Some(Span::from_positions(start, start))
}

fn closing_quote_span(part: &WordPartNode, source: &str) -> Option<Span> {
    let quote = match part.kind {
        WordPart::DoubleQuoted { .. } => '"',
        WordPart::SingleQuoted { .. } => '\'',
        _ => return None,
    };
    let span = part.span;
    let text = span.slice(source);
    let quote_offset = text.rfind(quote)?;
    let start = span.start.advanced_by(&text[..quote_offset]);
    Some(Span::from_positions(start, start))
}

fn backslash_prefixed_word_like_literal_gap(text: &str) -> bool {
    let text = text.trim();
    let Some(stripped) = text.strip_prefix('\\') else {
        return false;
    };
    !escaped_dollar_literal_gap(text) && split_quote_tail_is_suspicious(stripped)
}

fn escaped_dollar_literal_gap(text: &str) -> bool {
    let mut saw_escaped_dollar = false;
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }

        saw_escaped_dollar = true;
        if chars.next() != Some('$') {
            return false;
        }
    }

    saw_escaped_dollar
}

fn is_nested_parameter_expansion(parameter: &shuck_ast::ParameterExpansion, source: &str) -> bool {
    matches!(&parameter.syntax, ParameterExpansionSyntax::Bourne(_))
        && contains_nested_parameter_marker(parameter.raw_body.slice(source).trim_start())
}

fn contains_nested_parameter_marker(text: &str) -> bool {
    let inner = text
        .strip_prefix("${${")
        .or_else(|| text.strip_prefix("${#${"))
        .or_else(|| text.strip_prefix("${!${"));
    inner
        .and_then(|inner| inner.chars().next())
        .is_some_and(is_bourne_nested_parameter_start)
}

fn is_bourne_nested_parameter_start(char: char) -> bool {
    matches!(char, '_' | '@' | '*' | '#' | '?' | '$' | '!' | '-') || char.is_ascii_alphanumeric()
}
pub(super) fn simple_command_variable_set_operand<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    let operands = simple_test_operands(command, source)?;
    (operands.len() == 2 && static_word_text(&operands[0], source).as_deref() == Some("-v"))
        .then(|| &operands[1])
}

fn collect_unicode_smart_quote_spans_in_word_parts(
    parts: &[WordPartNode],
    source: &str,
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) if !quoted => {
                let literal = text.as_str(source, part.span);
                for (offset, char) in literal.char_indices() {
                    if !is_unicode_smart_quote(char) {
                        continue;
                    }
                    let start = part.span.start.advanced_by(&literal[..offset]);
                    let end = start.advanced_by(char.encode_utf8(&mut [0; 4]));
                    spans.push(Span::from_positions(start, end));
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unicode_smart_quote_spans_in_word_parts(parts, source, true, spans)
            }
            _ => {}
        }
    }
}

fn is_unicode_smart_quote(char: char) -> bool {
    matches!(char, '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}')
}

#[cfg(test)]
mod tests {
    use super::{
        SubscriptSpanIndex, arithmetic_expr_has_positional_parameter_operator,
        word_has_unquoted_positional_parameter_operator_neighbors,
    };
    use shuck_ast::{Command, Position, Span, WordPart};
    use shuck_parser::parser::Parser;

    fn span(start: usize, end: usize) -> Span {
        Span::from_positions(
            Position {
                line: 1,
                column: start + 1,
                offset: start,
            },
            Position {
                line: 1,
                column: end + 1,
                offset: end,
            },
        )
    }

    #[test]
    fn subscript_span_index_uses_prefix_max_for_containment() {
        let index = SubscriptSpanIndex::new(&[span(50, 60), span(0, 100), span(120, 130)]);

        assert!(index.contains(span(55, 56)));
        assert!(index.contains(span(80, 90)));
        assert!(index.contains(span(99, 100)));
        assert!(!index.contains(span(100, 101)));
        assert!(!index.contains(span(110, 115)));
    }

    #[test]
    fn detects_identifier_led_prefixes_before_positional_parameters_in_arithmetic_words() {
        for text in ["prefix$1", "a${1}", "foo${bar}$1"] {
            let word = Parser::parse_word_string(text);
            assert!(
                word_has_unquoted_positional_parameter_operator_neighbors(&word, text),
                "expected {text:?} to be flagged",
            );
        }
    }

    #[test]
    fn ignores_suffixes_and_non_identifier_prefixes_around_positional_parameters() {
        for text in [
            "$1",
            "${1}",
            "$1suffix",
            "${1}suffix",
            "\"$1\"",
            "'$1'",
            "16#$1",
            "0x$1",
            "0x${1}${2}",
            "1a$1",
            "${base}$1",
        ] {
            let word = Parser::parse_word_string(text);
            assert!(
                !word_has_unquoted_positional_parameter_operator_neighbors(&word, text),
                "expected {text:?} to be ignored",
            );
        }
    }

    #[test]
    fn detects_positional_parameter_operator_in_parsed_arithmetic_shell_word() {
        let source = "#!/bin/sh\necho \"$(( value + prefix$1 ))\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = output.file.body.stmts.first().expect("expected command");
        let Command::Simple(command) = &command.command else {
            panic!("expected simple command");
        };
        let expression_ast = command.args[0]
            .parts
            .iter()
            .find_map(|part| match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => parts.iter().find_map(|part| {
                    if let WordPart::ArithmeticExpansion {
                        expression_ast: Some(expression_ast),
                        ..
                    } = &part.kind
                    {
                        Some(expression_ast)
                    } else {
                        None
                    }
                }),
                _ => None,
            })
            .expect("expected parsed arithmetic expression");

        assert!(arithmetic_expr_has_positional_parameter_operator(
            expression_ast,
            source
        ));
    }
}
