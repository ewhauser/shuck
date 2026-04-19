use super::*;
use shuck_ast::{HeredocBody, HeredocBodyPart, HeredocBodyPartNode, PatternGroupKind};

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

    fn record_array_reference(&mut self, span: Span) {
        if self
            .facts
            .indexed_array_references
            .iter()
            .any(|fragment| fragment.span() == span)
        {
            return;
        }
        self.facts
            .indexed_array_references
            .push(IndexedArrayReferenceFragmentFact { span });
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

    pub(super) fn collect_words(&mut self, words: &[Word], context: SurfaceScanContext<'_>) {
        if context.assignment_target.is_none()
            && matches!(context.command_name, Some("echo" | "printf"))
        {
            self.collect_split_suspect_closing_quote_fragment_in_words(words);
        }
        for word in words {
            self.collect_word(word, context);
        }
    }

    pub(super) fn collect_patterns(
        &mut self,
        patterns: &[Pattern],
        context: SurfaceScanContext<'_>,
    ) {
        for pattern in patterns {
            self.collect_pattern(pattern, context);
        }
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
        if context.collect_open_double_quotes && context.assignment_target.is_none() {
            self.collect_open_double_quote_fragments(word, context.command_name);
        }
        self.collect_word_parts(&word.parts, context);
        self.facts.open_double_quotes.len() > open_double_quote_count
    }

    pub(super) fn collect_heredoc_body(
        &mut self,
        body: &HeredocBody,
        context: SurfaceScanContext<'_>,
    ) {
        self.collect_single_quoted_fragments_in_heredoc_body_parts(&body.parts, context);
        self.collect_heredoc_body_parts(&body.parts, context);
    }

    pub(super) fn record_unset_array_target_word(&mut self, word: &Word) {
        if word_looks_like_unset_array_target(word, self.source) {
            self.facts.subscript_spans.push(word.span);
        }
    }

    fn collect_open_double_quote_fragments(&mut self, word: &Word, command_name: Option<&str>) {
        for (opening_span, closing_span) in
            suspect_double_quote_spans(word, self.source, command_name)
        {
            self.facts
                .open_double_quotes
                .push(OpenDoubleQuoteFragmentFact { span: opening_span });
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

    fn collect_single_quoted_fragments_in_heredoc_body_parts(
        &mut self,
        parts: &[HeredocBodyPartNode],
        context: SurfaceScanContext<'_>,
    ) {
        let mut open_quote = None;

        for part in parts {
            match &part.kind {
                HeredocBodyPart::Literal(text) => {
                    let text = text.as_str(self.source, part.span);

                    for (quote_offset, ch) in text.char_indices() {
                        match ch {
                            '\n' => open_quote = None,
                            '\'' => {
                                let quote_start =
                                    part.span.start.advanced_by(&text[..quote_offset]);
                                if heredoc_single_quote_is_escaped(text, quote_offset) {
                                    continue;
                                }
                                let quote_end = quote_start.advanced_by("'");

                                if let Some(open_start) = open_quote.take() {
                                    self.facts.single_quoted.push(SingleQuotedFragmentFact {
                                        span: Span::from_positions(open_start, quote_end),
                                        dollar_quoted: false,
                                        command_name: context
                                            .command_name
                                            .map(str::to_owned)
                                            .map(String::into_boxed_str),
                                        assignment_target: context
                                            .assignment_target
                                            .map(str::to_owned)
                                            .map(String::into_boxed_str),
                                        variable_set_operand: context.variable_set_operand,
                                        literal_backslash_in_single_quotes_span: None,
                                    });
                                } else {
                                    open_quote = Some(quote_start);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {
                    if part.span.start.line != part.span.end.line {
                        open_quote = None;
                    }
                }
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
                        self.record_array_reference(part.span);
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
        for (part, span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { kind, patterns } => {
                    if *kind == PatternGroupKind::ExactlyOne {
                        self.facts.pattern_exactly_one_extglob_spans.push(span);
                    }
                    self.collect_patterns(patterns, context);
                }
                PatternPart::Word(word) => {
                    self.collect_word(word, context);
                }
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

    pub(super) fn collect_redirects(
        &mut self,
        redirects: &[Redirect],
        context: SurfaceScanContext<'_>,
    ) {
        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => {
                    self.collect_word(word, context);
                }
                None => {
                    let heredoc = redirect.heredoc().expect("expected heredoc redirect");
                    if heredoc.delimiter.expands_body {
                        self.collect_heredoc_body(
                            &heredoc.body,
                            context.without_open_double_quote_scan(),
                        );
                    }
                }
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
            self.record_array_reference(span);
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
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                }
                | BourneParameterExpansion::Indirect {
                    operator: Some(operator),
                    operand,
                    operand_word_ast,
                    ..
                } => {
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

fn heredoc_single_quote_is_escaped(text: &str, quote_offset: usize) -> bool {
    let mut backslash_count = 0usize;
    let mut cursor = quote_offset;
    while cursor > 0 {
        match text.as_bytes()[cursor - 1] {
            b'\\' => {
                backslash_count += 1;
                cursor -= 1;
            }
            b'\n' => break,
            _ => break,
        }
    }

    !backslash_count.is_multiple_of(2)
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
            BourneParameterExpansion::Access { reference } => {
                reference_has_array_subscript(reference)
            }
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. } => false,
            BourneParameterExpansion::PrefixMatch { .. } => false,
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => reference_has_array_subscript(reference),
            ZshExpansionTarget::Nested(parameter) => parameter_has_array_reference(parameter),
            ZshExpansionTarget::Word(_) | ZshExpansionTarget::Empty => false,
        },
    }
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
    command_name: Option<&str>,
) -> Vec<(Span, Span)> {
    word.parts
        .windows(3)
        .enumerate()
        .filter_map(|(index, window)| {
            let [current, middle, next] = window else {
                return None;
            };
            if !suspicious_reopened_double_quote_window(
                word,
                source,
                command_name,
                index,
                current,
                middle,
                next,
            ) {
                return None;
            }

            Some((
                opening_double_quote_span(current.span, source)?,
                closing_double_quote_span(current.span, source)?,
            ))
        })
        .collect()
}

pub(super) fn word_has_reopened_double_quote_window(
    word: &Word,
    source: &str,
    command_name: Option<&str>,
) -> bool {
    word.parts.windows(3).enumerate().any(|(index, window)| {
        let [current, middle, next] = window else {
            return false;
        };
        suspicious_reopened_double_quote_window(
            word,
            source,
            command_name,
            index,
            current,
            middle,
            next,
        )
    })
}

fn suspicious_reopened_double_quote_window(
    word: &Word,
    source: &str,
    command_name: Option<&str>,
    index: usize,
    current: &WordPartNode,
    middle: &WordPartNode,
    next: &WordPartNode,
) -> bool {
    let WordPart::DoubleQuoted { parts, .. } = &current.kind else {
        return false;
    };
    if !matches!(next.kind, WordPart::DoubleQuoted { .. })
        || !current.span.slice(source).contains('\n')
        || double_quoted_parts_contain_live_scalar(parts)
    {
        return false;
    }

    let has_scalar_gap = middle_part_is_live_scalar_gap(middle);
    let has_word_literal_gap = index == 0
        && matches!(command_name, Some("echo" | "printf"))
        && !double_quoted_parts_contain_command_like_substitution(parts)
        && middle_part_is_word_like_literal_gap(middle, source);
    let has_triple_quote_literal_gap = index > 0
        && double_quoted_part_is_empty(&word.parts[index - 1], source)
        && middle_part_is_word_like_literal_gap(middle, source);

    has_scalar_gap || has_word_literal_gap || has_triple_quote_literal_gap
}

fn double_quoted_parts_contain_live_scalar(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::CommandSubstitution { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => double_quoted_parts_contain_live_scalar(parts),
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
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => true,
    })
}

fn double_quoted_parts_contain_command_like_substitution(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
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
        | WordPart::ZshQualifiedGlob(_) => false,
        WordPart::DoubleQuoted { parts, .. } => {
            double_quoted_parts_contain_command_like_substitution(parts)
        }
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => true,
    })
}

fn middle_part_is_live_scalar_gap(part: &WordPartNode) -> bool {
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

fn middle_part_is_word_like_literal_gap(part: &WordPartNode, source: &str) -> bool {
    let WordPart::Literal(text) = &part.kind else {
        return false;
    };
    let text = text.as_str(source, part.span);
    split_quote_tail_is_suspicious(text) || backslash_prefixed_word_like_literal_gap(text)
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

            let span = closing_double_quote_span(current.span, source)?;
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

fn opening_double_quote_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let quote_offset = text.find('"')?;
    let start = span.start.advanced_by(&text[..quote_offset]);
    Some(Span::from_positions(start, start))
}

fn closing_double_quote_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let quote_offset = text.rfind('"')?;
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
