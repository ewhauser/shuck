#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpansionContext {
    CommandName,
    CommandArgument,
    AssignmentValue,
    DeclarationAssignmentValue,
    RedirectTarget(RedirectKind),
    DescriptorDupTarget(RedirectKind),
    HereString,
    ForList,
    SelectList,
    CasePattern,
    ConditionalPattern,
    StringTestOperand,
    RegexOperand,
    ConditionalVarRefSubscript,
    ParameterPattern,
    TrapAction,
}

impl ExpansionContext {
    pub fn from_redirect_kind(kind: RedirectKind) -> Option<Self> {
        match kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => None,
            RedirectKind::HereString => Some(Self::HereString),
            RedirectKind::DupOutput | RedirectKind::DupInput => {
                Some(Self::DescriptorDupTarget(kind))
            }
            RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::OutputBoth => Some(Self::RedirectTarget(kind)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordQuote {
    FullyQuoted,
    Mixed,
    Unquoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordLiteralness {
    FixedLiteral,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordExpansionKind {
    None,
    Scalar,
    Array,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordSubstitutionShape {
    None,
    Plain,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionValueShape {
    None,
    Scalar,
    Array,
    MultiField,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExpansionHazards {
    pub field_splitting: bool,
    pub pathname_matching: bool,
    pub tilde_expansion: bool,
    pub brace_fanout: bool,
    pub runtime_pattern: bool,
    pub command_or_process_substitution: bool,
    pub arithmetic_expansion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpansionAnalysis {
    pub quote: WordQuote,
    pub literalness: WordLiteralness,
    pub value_shape: ExpansionValueShape,
    pub substitution_shape: WordSubstitutionShape,
    pub hazards: ExpansionHazards,
    pub array_valued: bool,
    pub can_expand_to_multiple_fields: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeLiteralAnalysis {
    pub runtime_sensitive: bool,
    pub hazards: ExpansionHazards,
}

impl RuntimeLiteralAnalysis {
    pub fn is_runtime_sensitive(self) -> bool {
        self.runtime_sensitive
    }
}

impl ExpansionAnalysis {
    pub fn is_fixed_literal(self) -> bool {
        self.literalness == WordLiteralness::FixedLiteral
    }

    pub fn has_scalar_expansion(self) -> bool {
        matches!(
            self.value_shape,
            ExpansionValueShape::Scalar | ExpansionValueShape::Unknown
        ) || (self.value_shape == ExpansionValueShape::MultiField && !self.array_valued)
    }

    pub fn has_array_expansion(self) -> bool {
        self.array_valued
    }

    pub fn has_command_substitution(self) -> bool {
        self.substitution_shape != WordSubstitutionShape::None
    }

    pub fn has_plain_command_substitution(self) -> bool {
        self.substitution_shape == WordSubstitutionShape::Plain
    }

    pub fn expansion_kind(self) -> WordExpansionKind {
        match (self.has_scalar_expansion(), self.array_valued) {
            (false, false) => WordExpansionKind::None,
            (true, false) => WordExpansionKind::Scalar,
            (false, true) => WordExpansionKind::Array,
            (true, true) => WordExpansionKind::Mixed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactContext {
    Expansion(ExpansionContext),
    CaseSubject,
    ArithmeticCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordFactHostKind {
    Direct,
    CommandWrapperTarget,
    AssignmentTargetSubscript,
    DeclarationNameSubscript,
    ArrayKeySubscript,
    ConditionalVarRefSubscript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordClassification {
    pub quote: WordQuote,
    pub literalness: WordLiteralness,
    pub expansion_kind: WordExpansionKind,
    pub substitution_shape: WordSubstitutionShape,
}

impl WordClassification {
    pub fn is_fixed_literal(self) -> bool {
        self.literalness == WordLiteralness::FixedLiteral
    }

    pub fn is_expanded(self) -> bool {
        self.literalness == WordLiteralness::Expanded
    }

    pub fn has_scalar_expansion(self) -> bool {
        matches!(
            self.expansion_kind,
            WordExpansionKind::Scalar | WordExpansionKind::Mixed
        )
    }

    pub fn has_array_expansion(self) -> bool {
        matches!(
            self.expansion_kind,
            WordExpansionKind::Array | WordExpansionKind::Mixed
        )
    }

    pub fn has_command_substitution(self) -> bool {
        self.substitution_shape != WordSubstitutionShape::None
    }

    pub fn has_plain_command_substitution(self) -> bool {
        self.substitution_shape == WordSubstitutionShape::Plain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOperandClass {
    FixedLiteral,
    RuntimeSensitive,
}

impl TestOperandClass {
    pub fn is_fixed_literal(self) -> bool {
        self == Self::FixedLiteral
    }
}

pub(crate) fn classify_word(word: &Word, source: &str) -> WordClassification {
    word_classification_from_analysis(analyze_word(word, source, None))
}

pub(crate) fn classify_contextual_operand(
    word: &Word,
    source: &str,
    context: ExpansionContext,
) -> TestOperandClass {
    let analysis = analyze_word(word, source, None);
    if analysis.literalness == WordLiteralness::Expanded {
        return TestOperandClass::RuntimeSensitive;
    }

    if analyze_literal_runtime(word, source, context, None).is_runtime_sensitive() {
        TestOperandClass::RuntimeSensitive
    } else {
        TestOperandClass::FixedLiteral
    }
}

pub(crate) fn classify_conditional_operand(
    expression: &ConditionalExpr,
    source: &str,
) -> TestOperandClass {
    match expression {
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            let context = match expression {
                ConditionalExpr::Word(_) => ExpansionContext::StringTestOperand,
                ConditionalExpr::Regex(_) => ExpansionContext::RegexOperand,
                _ => unreachable!(),
            };
            classify_contextual_operand(word, source, context)
        }
        ConditionalExpr::Pattern(pattern) => classify_pattern_operand(pattern, source),
        ConditionalExpr::VarRef(_) => TestOperandClass::RuntimeSensitive,
        ConditionalExpr::Parenthesized(expression) => {
            classify_conditional_operand(&expression.expr, source)
        }
        ConditionalExpr::Binary(_) | ConditionalExpr::Unary(_) => {
            TestOperandClass::RuntimeSensitive
        }
    }
}

fn classify_pattern_operand(pattern: &Pattern, source: &str) -> TestOperandClass {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                if patterns
                    .iter()
                    .any(|pattern| !classify_pattern_operand(pattern, source).is_fixed_literal())
                {
                    return TestOperandClass::RuntimeSensitive;
                }
                return TestOperandClass::RuntimeSensitive;
            }
            PatternPart::Word(word) => {
                if !classify_contextual_operand(word, source, ExpansionContext::CasePattern)
                    .is_fixed_literal()
                {
                    return TestOperandClass::RuntimeSensitive;
                }
            }
            PatternPart::AnyString | PatternPart::AnyChar | PatternPart::CharClass(_) => {
                return TestOperandClass::RuntimeSensitive;
            }
            PatternPart::Literal(_) => {}
        }
    }

    TestOperandClass::FixedLiteral
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartValueShape {
    Scalar,
    Array,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PartAnalysis {
    value_shape: PartValueShape,
    array_valued: bool,
    can_expand_to_multiple_fields: bool,
    hazards: ExpansionHazards,
    command_substitution: bool,
    process_substitution: bool,
}

#[derive(Default)]
struct AnalysisSummary {
    has_non_literal: bool,
    has_scalar_value: bool,
    has_array_value: bool,
    has_unknown_value: bool,
    can_expand_to_multiple_fields: bool,
    hazards: ExpansionHazards,
    command_substitution_count: usize,
    has_process_substitution: bool,
}

pub(crate) fn analyze_word(
    word: &Word,
    _source: &str,
    options: Option<&ZshOptionState>,
) -> ExpansionAnalysis {
    let mut summary = AnalysisSummary::default();
    analyze_parts(&word.parts, false, options, &mut summary);

    ExpansionAnalysis {
        quote: if is_fully_quoted(word) {
            WordQuote::FullyQuoted
        } else if word.parts.iter().any(|part| is_quoted_part(&part.kind)) {
            WordQuote::Mixed
        } else {
            WordQuote::Unquoted
        },
        literalness: if summary.has_non_literal {
            WordLiteralness::Expanded
        } else {
            WordLiteralness::FixedLiteral
        },
        value_shape: if summary.has_unknown_value {
            ExpansionValueShape::Unknown
        } else if summary.can_expand_to_multiple_fields {
            ExpansionValueShape::MultiField
        } else if summary.has_array_value {
            ExpansionValueShape::Array
        } else if summary.has_scalar_value {
            ExpansionValueShape::Scalar
        } else {
            ExpansionValueShape::None
        },
        substitution_shape: if summary.command_substitution_count == 0 {
            WordSubstitutionShape::None
        } else if is_plain_command_substitution(&word.parts) {
            WordSubstitutionShape::Plain
        } else {
            WordSubstitutionShape::Mixed
        },
        hazards: summary.hazards,
        array_valued: summary.has_array_value,
        can_expand_to_multiple_fields: summary.can_expand_to_multiple_fields,
    }
}

pub(crate) fn analyze_literal_runtime(
    word: &Word,
    source: &str,
    context: ExpansionContext,
    options: Option<&ZshOptionState>,
) -> RuntimeLiteralAnalysis {
    let mut analysis = RuntimeLiteralAnalysis::default();
    let mut state = RuntimeLiteralState::default();

    analyze_literal_runtime_parts(
        &word.parts,
        source,
        context,
        options,
        &mut state,
        &mut analysis,
    );

    analysis
}

#[derive(Debug, Default)]
struct RuntimeLiteralState {
    seen_text: bool,
    last_unquoted_char: Option<char>,
}

fn analyze_literal_runtime_parts(
    parts: &[WordPartNode],
    source: &str,
    context: ExpansionContext,
    options: Option<&ZshOptionState>,
    state: &mut RuntimeLiteralState,
    analysis: &mut RuntimeLiteralAnalysis,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {
                scan_literal_runtime_text(
                    part.span.slice(source),
                    context,
                    options,
                    state,
                    analysis,
                );
            }
            WordPart::SingleQuoted { value, .. } => {
                if !value.slice(source).is_empty() {
                    state.seen_text = true;
                    state.last_unquoted_char = None;
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !parts.is_empty() {
                    state.seen_text = true;
                    state.last_unquoted_char = None;
                }
            }
            _ => {}
        }
    }
}

fn scan_literal_runtime_text(
    text: &str,
    context: ExpansionContext,
    options: Option<&ZshOptionState>,
    state: &mut RuntimeLiteralState,
    analysis: &mut RuntimeLiteralAnalysis,
) {
    let mut escaped = false;
    let mut brace_candidate: Option<usize> = None;

    for (idx, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            state.seen_text = true;
            state.last_unquoted_char = Some(ch);
            continue;
        }

        if ch == '\\' {
            escaped = true;
            state.seen_text = true;
            continue;
        }

        if context_allows_tilde(context) && ch == '~' && tilde_is_runtime_sensitive(state) {
            analysis.runtime_sensitive = true;
            analysis.hazards.tilde_expansion = true;
        }

        if context_allows_pathname_matching(context)
            && glob_is_effectively_enabled(options)
            && matches!(ch, '*' | '?' | '[')
        {
            analysis.runtime_sensitive = true;
            analysis.hazards.pathname_matching = true;
        }

        if context_allows_brace_fanout(context) {
            if ch == '{' {
                brace_candidate = Some(idx);
            } else if ch == '}'
                && let Some(start) = brace_candidate.take()
                && brace_fanout_is_runtime_sensitive(&text[start + 1..idx])
            {
                analysis.runtime_sensitive = true;
                analysis.hazards.brace_fanout = true;
            }
        }

        state.seen_text = true;
        state.last_unquoted_char = Some(ch);
    }
}

fn tilde_is_runtime_sensitive(state: &RuntimeLiteralState) -> bool {
    !state.seen_text || matches!(state.last_unquoted_char, Some('=') | Some(':'))
}

fn brace_fanout_is_runtime_sensitive(content: &str) -> bool {
    content.contains(',') || content.contains("..")
}

fn context_allows_tilde(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::ForList
            | ExpansionContext::SelectList
            | ExpansionContext::AssignmentValue
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::StringTestOperand
            | ExpansionContext::RegexOperand
            | ExpansionContext::RedirectTarget(_)
    )
}

fn context_allows_pathname_matching(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::ForList
            | ExpansionContext::SelectList
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::RedirectTarget(_)
    )
}

fn context_allows_brace_fanout(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::ForList
            | ExpansionContext::SelectList
            | ExpansionContext::AssignmentValue
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::RedirectTarget(_)
    )
}

fn analyze_parts(
    parts: &[WordPartNode],
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
    summary: &mut AnalysisSummary,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                analyze_parts(parts, true, options, summary);
            }
            kind => {
                let analysis = analyze_part(kind, in_double_quotes, options);
                summary.has_non_literal = true;
                summary.has_scalar_value |= analysis.value_shape == PartValueShape::Scalar;
                summary.has_array_value |= analysis.array_valued;
                summary.has_unknown_value |= analysis.value_shape == PartValueShape::Unknown;
                summary.can_expand_to_multiple_fields |= analysis.can_expand_to_multiple_fields;
                summary.hazards.field_splitting |= analysis.hazards.field_splitting;
                summary.hazards.pathname_matching |= analysis.hazards.pathname_matching;
                summary.hazards.tilde_expansion |= analysis.hazards.tilde_expansion;
                summary.hazards.brace_fanout |= analysis.hazards.brace_fanout;
                summary.hazards.runtime_pattern |= analysis.hazards.runtime_pattern;
                summary.hazards.command_or_process_substitution |=
                    analysis.hazards.command_or_process_substitution;
                summary.hazards.arithmetic_expansion |= analysis.hazards.arithmetic_expansion;
                summary.command_substitution_count += usize::from(analysis.command_substitution);
                summary.has_process_substitution |= analysis.process_substitution;
            }
        }
    }
}

fn analyze_part(
    part: &WordPart,
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> PartAnalysis {
    match part {
        WordPart::ZshQualifiedGlob(_) => PartAnalysis {
            value_shape: PartValueShape::Unknown,
            array_valued: false,
            can_expand_to_multiple_fields: !in_double_quotes
                && glob_is_effectively_enabled(options),
            hazards: ExpansionHazards {
                pathname_matching: !in_double_quotes && glob_is_effectively_enabled(options),
                ..ExpansionHazards::default()
            },
            command_substitution: false,
            process_substitution: false,
        },
        WordPart::Parameter(parameter) => {
            analyze_parameter_part(parameter, in_double_quotes, options)
        }
        WordPart::CommandSubstitution { .. } => scalar_part(
            substitution_can_expand_to_multiple_fields(in_double_quotes, options),
            ExpansionHazards {
                field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                pathname_matching: substitution_pathname_matching_hazard(in_double_quotes, options),
                command_or_process_substitution: true,
                ..ExpansionHazards::default()
            },
            true,
            false,
        ),
        WordPart::ProcessSubstitution { .. } => scalar_part(
            false,
            ExpansionHazards {
                command_or_process_substitution: true,
                ..ExpansionHazards::default()
            },
            false,
            true,
        ),
        WordPart::Variable(name) if matches!(name.as_str(), "@") => {
            array_part(true, false, false, false)
        }
        WordPart::Variable(name) if matches!(name.as_str(), "*") => {
            array_part(!in_double_quotes, !in_double_quotes, false, false)
        }
        WordPart::Variable(_)
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::Substring { .. } => scalar_part(
            substitution_can_expand_to_multiple_fields(in_double_quotes, options),
            ExpansionHazards {
                field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                pathname_matching: substitution_pathname_matching_hazard(in_double_quotes, options),
                arithmetic_expansion: matches!(part, WordPart::ArithmeticExpansion { .. }),
                ..ExpansionHazards::default()
            },
            false,
            false,
        ),
        WordPart::Transformation { .. } => {
            scalar_part(false, ExpansionHazards::default(), false, false)
        }
        WordPart::ParameterExpansion { operator, .. } => scalar_part(
            substitution_can_expand_to_multiple_fields(in_double_quotes, options),
            ExpansionHazards {
                field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                pathname_matching: substitution_pathname_matching_hazard(in_double_quotes, options),
                runtime_pattern: parameter_operator_uses_pattern(operator),
                ..ExpansionHazards::default()
            },
            false,
            false,
        ),
        WordPart::ArrayAccess(reference) => match reference
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.selector())
        {
            Some(SubscriptSelector::At) => array_part(true, false, false, false),
            Some(SubscriptSelector::Star) => {
                array_part(!in_double_quotes, !in_double_quotes, false, false)
            }
            None => scalar_part(
                substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    ..ExpansionHazards::default()
                },
                false,
                false,
            ),
        },
        WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_) => {
            array_part(true, false, false, false)
        }
        WordPart::PrefixMatch { kind, .. } => {
            let multi_field = prefix_match_can_expand_to_multiple_fields(*kind, in_double_quotes);
            PartAnalysis {
                value_shape: if multi_field {
                    PartValueShape::Scalar
                } else {
                    PartValueShape::Unknown
                },
                array_valued: false,
                can_expand_to_multiple_fields: multi_field,
                hazards: ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    ..ExpansionHazards::default()
                },
                command_substitution: false,
                process_substitution: false,
            }
        }
        WordPart::IndirectExpansion { operator, .. } => PartAnalysis {
            value_shape: PartValueShape::Unknown,
            array_valued: false,
            can_expand_to_multiple_fields: substitution_can_expand_to_multiple_fields(
                in_double_quotes,
                options,
            ),
            hazards: ExpansionHazards {
                field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                pathname_matching: substitution_pathname_matching_hazard(in_double_quotes, options),
                runtime_pattern: operator
                    .as_ref()
                    .is_some_and(parameter_operator_uses_pattern),
                ..ExpansionHazards::default()
            },
            command_substitution: false,
            process_substitution: false,
        },
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. } => {
            unreachable!("literal parts should be handled by analyze_parts")
        }
    }
}

fn analyze_parameter_part(
    parameter: &ParameterExpansion,
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> PartAnalysis {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => match reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector())
            {
                Some(SubscriptSelector::At) => array_part(true, false, false, false),
                Some(SubscriptSelector::Star) => {
                    array_part(!in_double_quotes, !in_double_quotes, false, false)
                }
                None => scalar_part(
                    substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                    ExpansionHazards {
                        field_splitting: substitution_field_splitting_hazard(
                            in_double_quotes,
                            options,
                        ),
                        pathname_matching: substitution_pathname_matching_hazard(
                            in_double_quotes,
                            options,
                        ),
                        ..ExpansionHazards::default()
                    },
                    false,
                    false,
                ),
            },
            BourneParameterExpansion::Length { .. } => scalar_part(
                substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    ..ExpansionHazards::default()
                },
                false,
                false,
            ),
            BourneParameterExpansion::Indices { .. } => array_part(true, false, false, false),
            BourneParameterExpansion::Indirect { operator, .. } => PartAnalysis {
                value_shape: PartValueShape::Unknown,
                array_valued: false,
                can_expand_to_multiple_fields: substitution_can_expand_to_multiple_fields(
                    in_double_quotes,
                    options,
                ),
                hazards: ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    runtime_pattern: operator
                        .as_ref()
                        .is_some_and(parameter_operator_uses_pattern),
                    ..ExpansionHazards::default()
                },
                command_substitution: false,
                process_substitution: false,
            },
            BourneParameterExpansion::PrefixMatch { kind, .. } => {
                let multi_field =
                    prefix_match_can_expand_to_multiple_fields(*kind, in_double_quotes);
                PartAnalysis {
                    value_shape: if multi_field {
                        PartValueShape::Scalar
                    } else {
                        PartValueShape::Unknown
                    },
                    array_valued: false,
                    can_expand_to_multiple_fields: multi_field,
                    hazards: ExpansionHazards {
                        field_splitting: substitution_field_splitting_hazard(
                            in_double_quotes,
                            options,
                        ),
                        pathname_matching: substitution_pathname_matching_hazard(
                            in_double_quotes,
                            options,
                        ),
                        ..ExpansionHazards::default()
                    },
                    command_substitution: false,
                    process_substitution: false,
                }
            }
            BourneParameterExpansion::Slice { reference, .. } => {
                if reference.has_array_selector() {
                    array_part(true, false, false, false)
                } else {
                    scalar_part(
                        substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                        ExpansionHazards {
                            field_splitting: substitution_field_splitting_hazard(
                                in_double_quotes,
                                options,
                            ),
                            pathname_matching: substitution_pathname_matching_hazard(
                                in_double_quotes,
                                options,
                            ),
                            ..ExpansionHazards::default()
                        },
                        false,
                        false,
                    )
                }
            }
            BourneParameterExpansion::Operation { operator, .. } => scalar_part(
                substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    runtime_pattern: parameter_operator_uses_pattern(operator),
                    ..ExpansionHazards::default()
                },
                false,
                false,
            ),
            BourneParameterExpansion::Transformation { .. } => scalar_part(
                substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(in_double_quotes, options),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        options,
                    ),
                    ..ExpansionHazards::default()
                },
                false,
                false,
            ),
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            let effective_options = overlay_zsh_modifier_overrides(options, syntax);
            PartAnalysis {
                value_shape: PartValueShape::Unknown,
                array_valued: false,
                can_expand_to_multiple_fields: substitution_can_expand_to_multiple_fields(
                    in_double_quotes,
                    effective_options.as_ref(),
                ),
                hazards: ExpansionHazards {
                    field_splitting: substitution_field_splitting_hazard(
                        in_double_quotes,
                        effective_options.as_ref(),
                    ),
                    pathname_matching: substitution_pathname_matching_hazard(
                        in_double_quotes,
                        effective_options.as_ref(),
                    ),
                    runtime_pattern: syntax
                        .operation
                        .as_ref()
                        .is_some_and(zsh_operation_uses_pattern),
                    ..ExpansionHazards::default()
                },
                command_substitution: false,
                process_substitution: false,
            }
        }
    }
}

fn substitution_can_expand_to_multiple_fields(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    !in_double_quotes
        && (substitution_field_splitting_hazard(false, options)
            || substitution_pathname_matching_hazard(false, options)
            || options.is_none())
}

fn substitution_field_splitting_hazard(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    if in_double_quotes {
        return false;
    }

    match options {
        Some(options) => !matches!(options.sh_word_split, OptionValue::Off),
        None => true,
    }
}

fn substitution_pathname_matching_hazard(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    if in_double_quotes || !glob_is_effectively_enabled(options) {
        return false;
    }

    match options {
        Some(options) => !matches!(options.glob_subst, OptionValue::Off),
        None => true,
    }
}

fn glob_is_effectively_enabled(options: Option<&ZshOptionState>) -> bool {
    !matches!(options.map(|options| options.glob), Some(OptionValue::Off))
}

fn overlay_zsh_modifier_overrides(
    options: Option<&ZshOptionState>,
    syntax: &shuck_ast::ZshParameterExpansion,
) -> Option<ZshOptionState> {
    let mut effective = options.cloned()?;
    apply_toggle_override(
        &mut effective.sh_word_split,
        syntax
            .modifiers
            .iter()
            .filter(|modifier| modifier.name == '=')
            .count(),
    );
    apply_toggle_override(
        &mut effective.glob_subst,
        syntax
            .modifiers
            .iter()
            .filter(|modifier| modifier.name == '~')
            .count(),
    );
    apply_toggle_override(
        &mut effective.rc_expand_param,
        syntax
            .modifiers
            .iter()
            .filter(|modifier| modifier.name == '^')
            .count(),
    );
    Some(effective)
}

fn apply_toggle_override(value: &mut OptionValue, count: usize) {
    if count == 0 {
        return;
    }

    *value = if count % 2 == 1 {
        OptionValue::On
    } else {
        OptionValue::Off
    };
}

fn scalar_part(
    can_expand_to_multiple_fields: bool,
    hazards: ExpansionHazards,
    command_substitution: bool,
    process_substitution: bool,
) -> PartAnalysis {
    PartAnalysis {
        value_shape: PartValueShape::Scalar,
        array_valued: false,
        can_expand_to_multiple_fields,
        hazards,
        command_substitution,
        process_substitution,
    }
}

fn array_part(
    can_expand_to_multiple_fields: bool,
    field_splitting: bool,
    pathname_matching: bool,
    runtime_pattern: bool,
) -> PartAnalysis {
    PartAnalysis {
        value_shape: PartValueShape::Array,
        array_valued: true,
        can_expand_to_multiple_fields,
        hazards: ExpansionHazards {
            field_splitting,
            pathname_matching,
            runtime_pattern,
            ..ExpansionHazards::default()
        },
        command_substitution: false,
        process_substitution: false,
    }
}

fn parameter_operator_uses_pattern(operator: &shuck_ast::ParameterOp) -> bool {
    matches!(
        operator,
        shuck_ast::ParameterOp::RemovePrefixShort { .. }
            | shuck_ast::ParameterOp::RemovePrefixLong { .. }
            | shuck_ast::ParameterOp::RemoveSuffixShort { .. }
            | shuck_ast::ParameterOp::RemoveSuffixLong { .. }
            | shuck_ast::ParameterOp::ReplaceFirst { .. }
            | shuck_ast::ParameterOp::ReplaceAll { .. }
    )
}

fn zsh_operation_uses_pattern(operation: &ZshExpansionOperation) -> bool {
    matches!(
        operation,
        ZshExpansionOperation::PatternOperation { .. }
            | ZshExpansionOperation::TrimOperation { .. }
            | ZshExpansionOperation::ReplacementOperation { .. }
    )
}

fn prefix_match_can_expand_to_multiple_fields(
    kind: PrefixMatchKind,
    in_double_quotes: bool,
) -> bool {
    matches!(kind, PrefixMatchKind::At) || !in_double_quotes
}

fn is_plain_command_substitution(parts: &[WordPartNode]) -> bool {
    matches!(
        parts,
        [part] if match &part.kind {
            WordPart::CommandSubstitution { .. } => true,
            WordPart::DoubleQuoted { parts, .. } => is_plain_command_substitution(parts),
            _ => false,
        }
    )
}

fn is_quoted_part(part: &WordPart) -> bool {
    matches!(
        part,
        WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
    )
}

fn is_fully_quoted(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if is_quoted_part(&part.kind))
}

#[derive(Debug)]
pub struct WordNode<'a> {
    key: FactSpan,
    word: &'a Word,
    analysis: ExpansionAnalysis,
    derived: WordNodeDerived<'a>,
}

#[derive(Debug)]
pub(crate) struct WordNodeDerived<'a> {
    static_text: Option<&'a str>,
    trailing_literal_char: Option<char>,
    starts_with_extglob: bool,
    has_literal_affixes: bool,
    contains_shell_quoting_literals: bool,
    active_expansion_spans: IdRange<Span>,
    scalar_expansion_spans: IdRange<Span>,
    unquoted_scalar_expansion_spans: IdRange<Span>,
    array_expansion_spans: IdRange<Span>,
    all_elements_array_expansion_spans: IdRange<Span>,
    direct_all_elements_array_expansion_spans: IdRange<Span>,
    unquoted_all_elements_array_expansion_spans: IdRange<Span>,
    unquoted_array_expansion_spans: IdRange<Span>,
    command_substitution_spans: IdRange<Span>,
    unquoted_command_substitution_spans: IdRange<Span>,
    unquoted_dollar_paren_command_substitution_spans: IdRange<Span>,
    double_quoted_expansion_spans: IdRange<Span>,
    unquoted_literal_between_double_quoted_segments_spans: IdRange<Span>,
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
    enclosing_expansion_context: Option<ExpansionContext>,
    array_assignment_split_scalar_expansion_spans: IdRange<Span>,
}

#[derive(Clone, Copy)]
pub struct WordOccurrenceRef<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
    id: WordOccurrenceId,
}

pub struct WordOccurrenceIter<'facts, 'a> {
    facts: &'facts LinterFacts<'a>,
    source: WordOccurrenceIterSource<'facts>,
    filter: WordOccurrenceFilter,
}

enum WordOccurrenceIterSource<'facts> {
    All { next: usize },
    Ids(std::slice::Iter<'facts, WordOccurrenceId>),
}

#[derive(Clone, Copy)]
pub(crate) enum WordOccurrenceFilter {
    Any,
    NonArithmetic,
    ArithmeticCommand,
    Expansion(ExpansionContext),
    CaseSubject,
}

impl<'facts, 'a> WordOccurrenceIter<'facts, 'a> {
    pub(crate) fn all(facts: &'facts LinterFacts<'a>, filter: WordOccurrenceFilter) -> Self {
        Self {
            facts,
            source: WordOccurrenceIterSource::All { next: 0 },
            filter,
        }
    }

    pub(crate) fn ids(
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

    fn word(self) -> &'a Word {
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

    pub fn runtime_literal(self) -> RuntimeLiteralAnalysis {
        self.occurrence().runtime_literal
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

    pub fn diagnostic_part_span(self, part: &WordPart, part_span: Span, source: &str) -> Span {
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
            source,
            self.facts.backtick_substitution_spans(),
        )
    }

    pub fn has_direct_all_elements_array_expansion_in_source(self, source: &str) -> bool {
        word_spans::word_has_direct_all_elements_array_expansion_in_source(self.word(), source)
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

    pub fn unquoted_glob_pattern_spans_outside_brace_expansion(self, source: &str) -> Vec<Span> {
        word_spans::word_unquoted_glob_pattern_spans_outside_brace_expansion(self.word(), source)
    }

    pub fn suspicious_bracket_glob_spans(self, source: &str) -> Vec<Span> {
        word_spans::word_suspicious_bracket_glob_spans(self.word(), source)
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

    pub fn folded_all_elements_array_span_in_source(self, source: &str) -> Option<Span> {
        word_spans::word_folded_all_elements_array_span_in_source(self.word(), source)
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
            .filter(|brace| brace.expands())
            .map(|brace| brace.span)
            .collect()
    }
}

fn shellcheck_parameter_span_inside_escaped_quotes(span: Span, source: &str) -> Option<Span> {
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

fn offset_for_line_column(source: &str, line: usize, column: usize) -> Option<usize> {
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

fn offset_for_column_in_line(source: &str, line_start: usize, column: usize) -> Option<usize> {
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

fn line_end_offset(source: &str, line: usize) -> Option<usize> {
    let line_start = offset_for_line_column(source, line, 1)?;
    Some(
        line_start
            + source
                .get(line_start..)?
                .find('\n')
                .unwrap_or(source.len() - line_start),
    )
}

fn parameter_expansion_end_offset(source: &str, dollar_offset: usize) -> Option<usize> {
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

fn contains_template_placeholder_text_in_word(text: &str) -> bool {
    let Some(start) = text.find("{{") else {
        return false;
    };
    text[start + 2..].contains("}}")
}

pub(crate) fn occurrence_word<'a>(nodes: &[WordNode<'a>], occurrence: &WordOccurrence) -> &'a Word {
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

pub(crate) fn word_node_derived<'node, 'word>(
    node: &'node WordNode<'word>,
) -> &'node WordNodeDerived<'word> {
    &node.derived
}

fn word_is_plain_scalar_reference(word: &Word) -> bool {
    word_is_plain_reference(word, false)
}

fn word_is_plain_parameter_reference(word: &Word) -> bool {
    word_is_plain_reference(word, true)
}

fn word_is_plain_reference(word: &Word, allow_all_elements_parameters: bool) -> bool {
    let [part] = word.parts.as_slice() else {
        return false;
    };
    word_part_is_plain_reference(&part.kind, allow_all_elements_parameters)
}

fn word_part_is_plain_reference(part: &WordPart, allow_all_elements_parameters: bool) -> bool {
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

fn parameter_is_plain_reference(
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
    fact_store: &FactStore<'_>,
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
                    let derived = word_node_derived(&nodes[fact.node_id.index()]);
                    fact_store
                        .word_spans(derived.active_expansion_spans)
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
    let (_, value) = definition.split_once('=').unwrap_or(("", &definition));
    let end = words.last()?.span.end;
    contains_positional_parameter_reference(value)
        .then(|| Span::from_positions(words[0].span.start, end))
}

fn static_alias_definition_text(words: &[&Word], source: &str) -> Option<String> {
    let mut text = String::new();
    for word in words {
        text.push_str(&static_word_text(word, source)?);
    }
    Some(text)
}

fn contains_positional_parameter_reference(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    while let Some(byte) = bytes.get(index).copied() {
        match byte {
            b'\''
                if !in_double_quotes && (in_single_quotes || !is_escaped_dollar(value, index)) =>
            {
                in_single_quotes = !in_single_quotes;
                index += 1;
                continue;
            }
            b'"' if !in_single_quotes && !is_escaped_dollar(value, index) => {
                in_double_quotes = !in_double_quotes;
                index += 1;
                continue;
            }
            b'#' if !in_single_quotes
                && !in_double_quotes
                && !is_escaped_dollar(value, index)
                && starts_comment(value, index) =>
            {
                return false;
            }
            b'$' if !in_single_quotes && !is_escaped_dollar(value, index) => {}
            _ => {
                index += 1;
                continue;
            }
        }

        index += 1;
        let Some(next) = bytes.get(index).copied() else {
            return false;
        };

        if next == b'$' {
            index += 1;
            continue;
        }

        if is_positional_parameter_start(next) {
            return true;
        }

        if next == b'{' && braced_parameter_starts_with_positional(value, index + 1) {
            return true;
        }

        if next == b'{' {
            index += 1;
        }
    }
    false
}

fn starts_comment(value: &str, hash: usize) -> bool {
    hash == 0
        || value.as_bytes()[hash - 1].is_ascii_whitespace()
        || matches!(
            value.as_bytes()[hash - 1],
            b';' | b'&' | b'|' | b'(' | b')' | b'{' | b'}'
        )
}

fn is_escaped_dollar(value: &str, dollar: usize) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = dollar;
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

fn braced_parameter_starts_with_positional(value: &str, index: usize) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.get(index).copied() else {
        return false;
    };

    if is_positional_parameter_start(first) {
        return true;
    }

    matches!(first, b'#' | b'!')
        && bytes
            .get(index + 1)
            .copied()
            .is_some_and(is_positional_parameter_start)
}

fn is_positional_parameter_start(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'@' | b'*')
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

#[derive(Clone, Copy)]
struct WordFactLookup<'facts, 'a> {
    nodes: &'facts [WordNode<'a>],
    occurrences: &'facts [WordOccurrence],
    word_index: &'facts FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    fact_store: &'facts FactStore<'a>,
    source: &'a str,
}

fn build_echo_to_sed_substitution_spans<'a>(
    commands: CommandFacts<'_, 'a>,
    pipelines: &[PipelineFact<'a>],
    backticks: &[BacktickFragmentFact],
    lookup: WordFactLookup<'_, 'a>,
) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut pipeline_sed_command_ids = FxHashSet::default();

    for pipeline in pipelines {
        if let Some(span) = sc2001_like_pipeline_span(commands, pipeline, backticks, lookup) {
            spans.push(span);
            if let Some(last_segment) = pipeline.last_segment() {
                pipeline_sed_command_ids.insert(last_segment.command_id());
            }
        }
    }

    spans.extend(commands.iter().filter_map(|command| {
        (!pipeline_sed_command_ids.contains(&command.id()))
            .then(|| sc2001_like_here_string_span(command, backticks, lookup.source))
            .flatten()
    }));

    sort_and_dedup_spans(&mut spans);
    spans
}

fn sc2001_like_pipeline_span<'a>(
    commands: CommandFacts<'_, 'a>,
    pipeline: &PipelineFact<'a>,
    backticks: &[BacktickFragmentFact],
    lookup: WordFactLookup<'_, 'a>,
) -> Option<Span> {
    let [left_segment, right_segment] = pipeline.segments() else {
        return None;
    };

    let left = command_fact_ref(commands, left_segment.command_id());
    let right = command_fact_ref(commands, right_segment.command_id());

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

    if !command_has_sc2001_like_sed_script(right, backticks, lookup.source) {
        return None;
    }

    let [argument] = left.body_args() else {
        return None;
    };

    let word_fact = word_occurrence_with_context(
        lookup.nodes,
        lookup.occurrences,
        lookup.word_index,
        argument.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    )?;

    if occurrence_static_text(lookup.nodes, word_fact, lookup.source).is_some() {
        return None;
    }

    let derived = word_node_derived(&lookup.nodes[word_fact.node_id.index()]);
    if derived.scalar_expansion_spans.is_empty()
        && derived.array_expansion_spans.is_empty()
        && derived.command_substitution_spans.is_empty()
    {
        return None;
    }

    if derived.has_literal_affixes
        && !word_occurrence_is_pure_quoted_dynamic(
            lookup.nodes,
            word_fact,
            lookup.fact_store,
            lookup.source,
        )
    {
        return None;
    }

    if command_is_inside_backtick_fragment(right, backticks)
        && word_occurrence_is_escaped_double_quoted_dynamic(
            lookup.nodes,
            word_fact,
            lookup.fact_store,
            lookup.source,
        )
    {
        return sc2001_like_backtick_pipeline_span(commands, pipeline, right, lookup.source);
    }

    if word_occurrence_is_escaped_double_quoted_dynamic(
        lookup.nodes,
        word_fact,
        lookup.fact_store,
        lookup.source,
    ) {
        return None;
    }

    Some(pipeline_span_with_shellcheck_tail(
        commands,
        pipeline,
        lookup.source,
    ))
}

fn sc2001_like_here_string_span(
    command: CommandFactRef<'_, '_>,
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

    if command_is_inside_backtick_fragment(command, backticks) {
        return sc2001_like_backtick_command_span(command, source);
    }

    command_span_with_redirects_and_shellcheck_tail(command, source)
}

fn command_is_plain_named(command: CommandFactRef<'_, '_>, name: &str) -> bool {
    command.effective_name_is(name) && command.wrappers().is_empty()
}

fn sc2001_like_backtick_pipeline_span(
    commands: CommandFacts<'_, '_>,
    pipeline: &PipelineFact<'_>,
    sed_command: CommandFactRef<'_, '_>,
    source: &str,
) -> Option<Span> {
    let first_segment = pipeline.first_segment()?;
    let first = command_fact_ref(commands, first_segment.command_id());
    let start = first.body_name_word()?.span.start;
    let end = sc2001_like_backtick_sed_script_end(sed_command.body_args(), source)?;
    Some(Span::from_positions(start, end))
}

fn sc2001_like_backtick_command_span(
    command: CommandFactRef<'_, '_>,
    source: &str,
) -> Option<Span> {
    let start = command.body_name_word()?.span.start;
    let end = sc2001_like_backtick_sed_script_end(command.body_args(), source)?;
    Some(Span::from_positions(start, end))
}

fn sc2001_like_backtick_sed_script_end(args: &[&Word], source: &str) -> Option<Position> {
    let script_words = match args {
        [flag, words @ ..] if static_word_text(flag, source).as_deref() == Some("-e") => words,
        _ => args,
    };

    let raw_script_end = match script_words {
        [script] => backtick_sed_script_content_end_offset(
            script.span.slice(source),
            script.span.end.offset,
        )?,
        [first, .., last]
            if first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"") =>
        {
            last.span.end.offset.checked_sub(2)?
        }
        _ => return None,
    };

    let trim_chars = sc2001_like_backtick_sed_script_trim_chars(script_words, source)?;
    let end_offset = rewind_offset_by_chars(source, raw_script_end, trim_chars)?;
    position_at_offset(source, end_offset)
}

fn backtick_sed_script_content_end_offset(text: &str, end_offset: usize) -> Option<usize> {
    if text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\"") {
        end_offset.checked_sub(2)
    } else if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        end_offset.checked_sub(1)
    } else {
        Some(end_offset)
    }
}

fn sc2001_like_backtick_sed_script_trim_chars(
    script_words: &[&Word],
    source: &str,
) -> Option<usize> {
    let uses_backtick_escaped_double_quotes =
        backtick_sed_script_uses_escaped_double_quotes(script_words, source);
    let text = sed_script_text(
        script_words,
        source,
        SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes,
    )?;
    let text = text.as_ref();

    let remainder = text.strip_prefix('s')?;
    let delimiter = remainder.chars().next()?;
    let delimiter_len = delimiter.len_utf8();

    let pattern_start = 1 + delimiter_len;
    let (pattern_end, _) = find_sed_substitution_section(text, pattern_start, delimiter)?;
    let replacement_start = pattern_end + delimiter_len;
    let (replacement_end, _) = find_sed_substitution_section(text, replacement_start, delimiter)?;
    let pattern = &text[pattern_start..pattern_end];
    let replacement = &text[replacement_start..replacement_end];
    let flags = &text[replacement_end + delimiter_len..];

    let mut trim_chars = if flags.is_empty() {
        if uses_backtick_escaped_double_quotes && replacement_start == replacement_end {
            2
        } else {
            1
        }
    } else {
        flags.chars().count()
    };

    // ShellCheck trims one additional character for these legacy backtick sed sites
    // when the match pattern itself ends with an escaped dollar.
    if pattern.ends_with(r"\$") {
        trim_chars += 1;
        if replacement.starts_with(r"\\") {
            trim_chars += 1;
        }
    }

    Some(trim_chars)
}

fn backtick_sed_script_uses_escaped_double_quotes(script_words: &[&Word], source: &str) -> bool {
    match script_words {
        [script] => {
            let text = script.span.slice(source);
            text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\"")
        }
        [first, .., last] => {
            first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"")
        }
        _ => false,
    }
}

fn rewind_offset_by_chars(source: &str, mut offset: usize, count: usize) -> Option<usize> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }

    for _ in 0..count {
        let prefix = source.get(..offset)?;
        let (_, ch) = prefix.char_indices().next_back()?;
        offset = offset.checked_sub(ch.len_utf8())?;
    }

    Some(offset)
}

fn command_has_sc2001_like_sed_script(
    command: CommandFactRef<'_, '_>,
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
    command: CommandFactRef<'_, '_>,
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
    source: &'a str,
) -> Option<Cow<'a, str>> {
    let node = &nodes[occurrence.node_id.index()];
    word_node_derived(node)
        .static_text
        .map(Cow::Borrowed)
        .or_else(|| static_word_text(node.word, source))
}

fn word_occurrence_is_pure_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let word = occurrence_word(nodes, fact);
    !word_spans::word_double_quoted_scalar_only_expansion_spans(word).is_empty()
        || !word_spans::word_quoted_all_elements_array_slice_spans(word).is_empty()
        || word_occurrence_is_double_quoted_command_substitution_only(
            nodes, fact, fact_store, source,
        )
        || word_occurrence_is_escaped_double_quoted_dynamic(
            nodes, fact, fact_store, source,
        )
}

fn collect_unquoted_literal_between_double_quoted_segments_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;

    for index in 0..word.parts.len() {
        let middle_is_nested = command_depth > 0 || parameter_depth > 0;

        if index > 0 && index + 1 < word.parts.len() {
            let left = &word.parts[index - 1];
            let middle = &word.parts[index];
            let right = &word.parts[index + 1];

            if let (
                WordPart::DoubleQuoted {
                    parts: left_inner, ..
                },
                WordPart::Literal(text),
                WordPart::DoubleQuoted {
                    parts: right_inner, ..
                },
            ) = (&left.kind, &middle.kind, &right.kind)
            {
                let neighbor_has_literal =
                    mixed_quote_double_quoted_parts_contain_literal_content(left_inner)
                        || mixed_quote_double_quoted_parts_contain_literal_content(right_inner);
                if neighbor_has_literal
                    && !middle_is_nested
                    && mixed_quote_literal_is_warnable_between_double_quotes(
                        text.as_str(source, middle.span),
                    )
                {
                    spans.push(middle.span);
                }
            }
        }

        let (command_delta, parameter_delta) =
            mixed_quote_shell_fragment_balance_delta_for_part(&word.parts[index], source);
        command_depth += command_delta;
        parameter_depth += parameter_delta;
        command_depth = command_depth.max(0);
        parameter_depth = parameter_depth.max(0);
    }

    for span in mixed_quote_line_join_between_double_quotes_spans(word, source) {
        if !spans.contains(&span) {
            spans.push(span);
        }
    }

    if let Some(span) = mixed_quote_following_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }

    for span in mixed_quote_chained_line_join_between_double_quotes_spans(word, source) {
        if !spans.contains(&span) {
            spans.push(span);
        }
    }

    if let Some(span) = mixed_quote_trailing_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }
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
        if text
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | '{' | '}'))
        {
            return false;
        }

        if mixed_quote_literal_has_shellcheck_skipped_word_operator(text) {
            return false;
        }

        return !text.chars().any(char::is_whitespace);
    }

    if text.chars().all(|ch| ch == ':') {
        return text.len() > 1;
    }

    text.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '+' | '-' | '%' | ':')
    })
}

fn mixed_quote_literal_has_shellcheck_skipped_word_operator(text: &str) -> bool {
    text.contains('+') || text.contains('@')
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
        WordPart::DoubleQuoted { .. } => {
            let text = part.span.slice(source);
            let body = text
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .unwrap_or(text);
            mixed_quote_shell_fragment_balance_delta(body, false)
        }
        _ => mixed_quote_shell_fragment_balance_delta(part.span.slice(source), false),
    }
}

#[derive(Clone, Copy)]
enum MixedQuoteShellParenFrame {
    Command { opened_in_double_quotes: bool },
    Group,
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
    let mut command_frames = SmallVec::<[MixedQuoteShellParenFrame; 4]>::new();
    let mut parameter_frames = SmallVec::<[bool; 4]>::new();
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

        let allow_top_level_command_comment =
            allow_top_level_command_comments && parameter_delta == 0;
        if ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_delta,
                allow_top_level_command_comment,
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
                    command_frames.push(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes: in_double_quotes,
                    });
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some('{') => {
                    parameter_delta += 1;
                    parameter_frames.push(in_double_quotes);
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        match ch {
            '(' if !in_double_quotes && command_delta > 0 => {
                command_frames.push(MixedQuoteShellParenFrame::Group);
            }
            ')' => match command_frames.last().copied() {
                Some(MixedQuoteShellParenFrame::Group) if !in_double_quotes => {
                    command_frames.pop();
                }
                Some(MixedQuoteShellParenFrame::Command {
                    opened_in_double_quotes,
                }) if !in_double_quotes || opened_in_double_quotes => {
                    command_frames.pop();
                    command_delta -= 1;
                }
                None if !in_double_quotes => command_delta -= 1,
                _ => {}
            },
            '}' => match parameter_frames.last().copied() {
                Some(opened_in_double_quotes) if !in_double_quotes || opened_in_double_quotes => {
                    parameter_frames.pop();
                    parameter_delta -= 1;
                }
                None if !in_double_quotes => parameter_delta -= 1,
                _ => {}
            },
            _ => {}
        }

        if command_delta <= 0 {
            command_frames.clear();
        }
        if parameter_delta <= 0 {
            parameter_frames.clear();
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

    if !mixed_quote_text_ends_with_unescaped_double_quote(prefix)
        || !source[word.span.end.offset..].starts_with('"')
    {
        return None;
    }

    let start = word.span.start.advanced_by(prefix);
    Some(Span::from_positions(start, start.advanced_by(suffix)))
}

fn mixed_quote_line_join_between_double_quotes_spans(word: &Word, source: &str) -> Vec<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return Vec::new();
    }

    let text = word.span.slice(source);
    let mut spans = Vec::new();
    let mut byte_offset = 0;

    while byte_offset < text.len() {
        let Some(relative_offset) = text[byte_offset..].find('\\') else {
            break;
        };
        let start_offset = byte_offset + relative_offset;
        let Some(suffix) = text[start_offset..]
            .strip_prefix("\\\r\n\"")
            .map(|_| "\\\r\n")
            .or_else(|| text[start_offset..].strip_prefix("\\\n\"").map(|_| "\\\n"))
        else {
            byte_offset = start_offset + 1;
            continue;
        };

        if mixed_quote_text_ends_with_unescaped_double_quote(&text[..start_offset]) {
            let start = word.span.start.advanced_by(&text[..start_offset]);
            spans.push(Span::from_positions(start, start.advanced_by(suffix)));
        }

        byte_offset = start_offset + suffix.len();
    }

    spans
}

fn mixed_quote_following_line_join_between_double_quotes_span(
    word: &Word,
    source: &str,
) -> Option<Span> {
    let suffix = mixed_quote_following_line_join_suffix_after_word(word, source)?;
    Some(Span::from_positions(
        word.span.end,
        word.span.end.advanced_by(suffix),
    ))
}

fn mixed_quote_following_line_join_suffix_after_word(
    word: &Word,
    source: &str,
) -> Option<&'static str> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return None;
    }

    let tail = &source[word.span.end.offset..];
    let suffix = tail
        .strip_prefix("\\\r\n\"")
        .map(|_| "\\\r\n")
        .or_else(|| tail.strip_prefix("\\\n\"").map(|_| "\\\n"))?;

    if !mixed_quote_text_ends_with_unescaped_double_quote(word.span.slice(source)) {
        return None;
    }

    Some(suffix)
}

fn mixed_quote_chained_line_join_between_double_quotes_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return Vec::new();
    }

    let text = word.span.slice(source);
    if !(text.ends_with("\\\n") || text.ends_with("\\\r\n")) {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut cursor = word.span.end.offset;
    while source[cursor..].starts_with('"') {
        let Some(closing_quote_relative) =
            mixed_quote_closing_double_quote_offset(&source[cursor..])
        else {
            break;
        };
        let after_closing_quote = cursor + closing_quote_relative + 1;
        let Some(suffix) = source[after_closing_quote..]
            .strip_prefix("\\\r\n\"")
            .map(|_| "\\\r\n")
            .or_else(|| {
                source[after_closing_quote..]
                    .strip_prefix("\\\n\"")
                    .map(|_| "\\\n")
            })
        else {
            break;
        };

        let start = Position::new().advanced_by(&source[..after_closing_quote]);
        spans.push(Span::from_positions(start, start.advanced_by(suffix)));
        cursor = after_closing_quote + suffix.len();
    }

    spans
}

fn mixed_quote_closing_double_quote_offset(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    let (_, first) = chars.next()?;
    if first != '"' {
        return None;
    }

    let mut escaped = false;
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;
    let mut command_frames = SmallVec::<[MixedQuoteShellParenFrame; 4]>::new();
    let mut parameter_frames = SmallVec::<[bool; 4]>::new();
    let mut in_backtick_command = false;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut previous_char = Some('"');

    while let Some((offset, ch)) = chars.next() {
        let nested_depth = command_depth > 0 || parameter_depth > 0 || in_backtick_command;

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

        if ch == '\\' {
            escaped = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '`' && !in_single_quotes {
            in_backtick_command = !in_backtick_command;
            previous_char = Some(ch);
            continue;
        }

        if nested_depth && ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '"' {
            if !nested_depth {
                return Some(offset);
            }
            in_double_quotes = !in_double_quotes;
            previous_char = Some(ch);
            continue;
        }

        let allow_top_level_command_comment = in_backtick_command && parameter_depth == 0;
        if nested_depth
            && ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_depth,
                allow_top_level_command_comment,
                previous_char,
            )
        {
            in_comment = true;
            continue;
        }

        if ch == '$' {
            match chars.peek().copied() {
                Some((_, '(')) => {
                    command_depth += 1;
                    command_frames.push(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes: in_double_quotes,
                    });
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some((_, '{')) => {
                    parameter_depth += 1;
                    parameter_frames.push(in_double_quotes);
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        if nested_depth {
            match ch {
                '(' if !in_double_quotes && command_depth > 0 => {
                    command_frames.push(MixedQuoteShellParenFrame::Group);
                }
                ')' => match command_frames.last().copied() {
                    Some(MixedQuoteShellParenFrame::Group) if !in_double_quotes => {
                        command_frames.pop();
                    }
                    Some(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes,
                    }) if !in_double_quotes || opened_in_double_quotes => {
                        command_frames.pop();
                        command_depth -= 1;
                    }
                    None if !in_double_quotes => command_depth -= 1,
                    _ => {}
                },
                '}' => match parameter_frames.last().copied() {
                    Some(opened_in_double_quotes)
                        if !in_double_quotes || opened_in_double_quotes =>
                    {
                        parameter_frames.pop();
                        parameter_depth -= 1;
                    }
                    None if !in_double_quotes => parameter_depth -= 1,
                    _ => {}
                },
                _ => {}
            }
            command_depth = command_depth.max(0);
            parameter_depth = parameter_depth.max(0);
            if command_depth == 0 {
                command_frames.clear();
            }
            if parameter_depth == 0 {
                parameter_frames.clear();
            }
        }

        previous_char = Some(ch);
    }

    None
}

fn mixed_quote_text_ends_with_unescaped_double_quote(text: &str) -> bool {
    let Some(prefix) = text.strip_suffix('"') else {
        return false;
    };

    let backslash_count = prefix.chars().rev().take_while(|ch| *ch == '\\').count();
    backslash_count % 2 == 0
}

fn word_occurrence_is_double_quoted_command_substitution_only(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()]);
    let command_substitution_spans = fact_store.word_spans(derived.command_substitution_spans);
    let [command_substitution] = command_substitution_spans else {
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

fn word_occurrence_is_escaped_double_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()]);
    let word_text = occurrence_span(nodes, fact).slice(source);
    if !word_text.starts_with("\\\"") || !word_text.ends_with("\\\"") {
        return false;
    }

    let inner = &word_text[2..word_text.len() - 2];
    match (
        fact_store.word_spans(derived.scalar_expansion_spans),
        fact_store.word_spans(derived.array_expansion_spans),
        fact_store.word_spans(derived.command_substitution_spans),
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
        .filter(|fact| {
            fact.context == WordFactContext::Expansion(ExpansionContext::CommandArgument)
        })
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
    let mut pending_arithmetic_word_occurrences = Vec::new();
    let mut compound_assignment_value_word_spans = FxHashSet::default();
    let mut array_assignment_split_word_ids = Vec::new();
    let mut seen_word_occurrences = FxHashSet::default();
    let mut seen_pending_arithmetic_word_occurrences = FxHashSet::default();
    let mut word_spans = ListArena::new();
    let mut word_span_scratch = Vec::new();
    let mut assoc_binding_visibility_memo = FxHashMap::default();
    let mut case_pattern_expansions = Vec::new();
    let mut pattern_literal_spans = Vec::new();
    let mut arithmetic_summary = ArithmeticFactSummary::default();
    let mut surface_fragments = SurfaceFragmentSink::new(source);

    let mut next_command_id = 0;
    walk_commands(
        &file.body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit, context| {
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
                    nested_word_command: context.nested_word_command,
                },
                &normalized,
                command_zsh_options,
                WordFactOutputs {
                    word_nodes: &mut word_nodes,
                    word_node_ids_by_span: &mut word_node_ids_by_span,
                    word_occurrences: &mut word_occurrences,
                    pending_arithmetic_word_occurrences: &mut pending_arithmetic_word_occurrences,
                    compound_assignment_value_word_spans: &mut compound_assignment_value_word_spans,
                    array_assignment_split_word_ids: &mut array_assignment_split_word_ids,
                    seen_word_occurrences: &mut seen_word_occurrences,
                    seen_pending_arithmetic_word_occurrences:
                        &mut seen_pending_arithmetic_word_occurrences,
                    word_spans: &mut word_spans,
                    word_span_scratch: &mut word_span_scratch,
                    assoc_binding_visibility_memo: &mut assoc_binding_visibility_memo,
                    case_pattern_expansions: &mut case_pattern_expansions,
                    pattern_literal_spans: &mut pattern_literal_spans,
                    arithmetic: &mut arithmetic_summary,
                    surface: &mut surface_fragments,
                },
            );
            next_command_id += 1;
        },
    );

    let surface_fragments = surface_fragments.finish();

    word_occurrences.len()
        + pending_arithmetic_word_occurrences.len()
        + word_nodes.len()
        + compound_assignment_value_word_spans.len()
        + array_assignment_split_word_ids.len()
        + case_pattern_expansions.len()
        + pattern_literal_spans.len()
        + arithmetic_summary.array_index_arithmetic_spans.len()
        + arithmetic_summary.arithmetic_score_line_spans.len()
        + arithmetic_summary.dollar_in_arithmetic_spans.len()
        + arithmetic_summary
            .arithmetic_command_substitution_spans
            .len()
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
    word_spans: &'out mut ListArena<Span>,
    word_span_scratch: &'out mut Vec<Span>,
    word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    word_occurrences: &'out mut Vec<WordOccurrence>,
    pending_arithmetic_word_occurrences: &'out mut Vec<PendingArithmeticWordOccurrence>,
    compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    seen_word_occurrences: &'out mut FxHashSet<WordOccurrenceSeenKey>,
    seen_pending_arithmetic_word_occurrences: &'out mut FxHashSet<PendingArithmeticSeenKey>,
    assoc_binding_visibility_memo: &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    case_pattern_expansions: &'out mut Vec<CasePatternExpansionFact>,
    pattern_literal_spans: &'out mut Vec<Span>,
    arithmetic: &'out mut ArithmeticFactSummary,
    surface: &'out mut SurfaceFragmentSink<'a>,
}

struct PendingArithmeticWordOccurrence {
    node_id: WordNodeId,
    command_id: CommandId,
    nested_word_command: bool,
    host_kind: WordFactHostKind,
    enclosing_expansion_context: ExpansionContext,
}

type WordOccurrenceSeenKey = (FactSpan, WordFactContext, WordFactHostKind);
type PendingArithmeticSeenKey = (FactSpan, ExpansionContext, WordFactHostKind);

fn derive_word_fact_data<'a>(
    word: &'a Word,
    source: &'a str,
    span_store: &mut ListArena<Span>,
    scratch: &mut Vec<Span>,
) -> WordNodeDerived<'a> {
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
                word_spans::collect_active_expansion_spans_in_source(word, source, spans);
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
                word_spans::collect_all_elements_array_expansion_part_spans(word, source, spans);
            },
        ),
        direct_all_elements_array_expansion_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_runtime_expansion_spans,
            |spans| {
                word_spans::collect_direct_all_elements_array_expansion_part_spans(
                    word, source, spans,
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
                word_spans::collect_command_substitution_part_spans_in_source(word, source, spans);
            },
        ),
        unquoted_command_substitution_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_command_substitution_spans,
            |spans| {
                word_spans::collect_unquoted_command_substitution_part_spans_in_source(
                    word, source, spans,
                );
            },
        ),
        unquoted_dollar_paren_command_substitution_spans: push_needed_word_span_list(
            span_store,
            scratch,
            may_have_command_substitution_spans,
            |spans| {
                word_spans::collect_unquoted_dollar_paren_command_substitution_part_spans_in_source(
                    word, source, spans,
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

fn push_word_span_list(
    span_store: &mut ListArena<Span>,
    scratch: &mut Vec<Span>,
    collect: impl FnOnce(&mut Vec<Span>),
) -> IdRange<Span> {
    scratch.clear();
    collect(scratch);
    span_store.push_many(scratch.drain(..))
}

fn push_needed_word_span_list(
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

fn word_may_have_runtime_expansion_spans(word: &Word) -> bool {
    word_parts_may_have_runtime_expansion_spans(&word.parts)
}

fn word_parts_may_have_runtime_expansion_spans(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => word_parts_may_have_runtime_expansion_spans(parts),
        _ => true,
    })
}

fn word_may_have_command_substitution_spans(word: &Word) -> bool {
    word_parts_may_have_command_substitution_spans(&word.parts)
}

fn word_parts_may_have_command_substitution_spans(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => word_parts_may_have_command_substitution_spans(parts),
        WordPart::CommandSubstitution { .. } => true,
        _ => false,
    })
}

fn word_may_have_unquoted_literal_between_double_quoted_segments_spans(
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

fn borrowed_static_word_text<'a>(word: &'a Word, source: &'a str) -> Option<&'a str> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    borrowed_static_word_part_text(part, source)
}

fn borrowed_static_word_part_text<'a>(
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

fn word_trailing_literal_char(word: &Word, source: &str) -> Option<char> {
    trailing_literal_char_in_parts(&word.parts, source)
}

fn trailing_literal_char_in_parts(parts: &[WordPartNode], source: &str) -> Option<char> {
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

struct WordFactCollector<'out, 'a, 'norm> {
    source: &'a str,
    semantic: &'a SemanticModel,
    command_id: CommandId,
    nested_word_command: bool,
    surface_command_name: Option<&'norm str>,
    surface_body_arg_start_offset: Option<usize>,
    command_zsh_options: Option<ZshOptionState>,
    word_nodes: &'out mut Vec<WordNode<'a>>,
    word_spans: &'out mut ListArena<Span>,
    word_span_scratch: &'out mut Vec<Span>,
    word_node_ids_by_span: &'out mut FxHashMap<FactSpan, WordNodeId>,
    word_occurrences: &'out mut Vec<WordOccurrence>,
    pending_arithmetic_word_occurrences: &'out mut Vec<PendingArithmeticWordOccurrence>,
    array_assignment_split_word_ids: &'out mut Vec<WordOccurrenceId>,
    assoc_binding_visibility_memo: &'out mut FxHashMap<(Name, ScopeId, Option<FactSpan>), bool>,
    seen: &'out mut FxHashSet<WordOccurrenceSeenKey>,
    seen_pending_arithmetic: &'out mut FxHashSet<PendingArithmeticSeenKey>,
    compound_assignment_value_word_spans: &'out mut FxHashSet<FactSpan>,
    case_pattern_expansions: &'out mut Vec<CasePatternExpansionFact>,
    pattern_literal_spans: &'out mut Vec<Span>,
    arithmetic: &'out mut ArithmeticFactSummary,
    surface: &'out mut SurfaceFragmentSink<'a>,
}

fn simple_command_wrapper_target_index(command: &SimpleCommand, source: &str) -> Option<usize> {
    let command_name = static_command_name_text(&command.name, source)?;
    let word_count = 1 + command.args.len();
    match static_command_wrapper_target_index(word_count, 0, command_name.as_ref(), |index| {
        static_word_text(simple_command_word_at(command, index), source)
    }) {
        StaticCommandWrapperTarget::Wrapper { target_index } => target_index,
        StaticCommandWrapperTarget::NotWrapper => None,
    }
}

fn simple_command_word_at(command: &SimpleCommand, index: usize) -> &Word {
    if index == 0 {
        &command.name
    } else {
        &command.args[index - 1]
    }
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
            surface_body_arg_start_offset: normalized
                .body_args()
                .first()
                .map(|word| word.span.start.offset),
            command_zsh_options,
            word_nodes: outputs.word_nodes,
            word_spans: outputs.word_spans,
            word_span_scratch: outputs.word_span_scratch,
            word_node_ids_by_span: outputs.word_node_ids_by_span,
            word_occurrences: outputs.word_occurrences,
            pending_arithmetic_word_occurrences: outputs.pending_arithmetic_word_occurrences,
            array_assignment_split_word_ids: outputs.array_assignment_split_word_ids,
            assoc_binding_visibility_memo: outputs.assoc_binding_visibility_memo,
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
                    } else if single_quoted_literal_exempt_argument(
                        surface_command_name,
                        &command.args,
                        arg_index,
                        body_arg_start,
                        word,
                        trap_action,
                        self.source,
                    ) {
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
                        analyze_word(word, self.source, self.command_zsh_options.as_ref());
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
        let analysis = analyze_word(word, self.source, self.command_zsh_options.as_ref());
        let derived =
            derive_word_fact_data(word, self.source, self.word_spans, self.word_span_scratch);
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
                self.command_zsh_options.as_ref(),
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
                    operand_word_ast.as_ref(),
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
                operand_word_ast.as_ref(),
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

        !owner_name.is_some_and(|name| {
            self.assoc_binding_visible_for_subscript(name, owner_name_span, subscript)
        })
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

        let visible = if let Some(visible) =
            self.assoc_binding_visible_in_nearest_scope(owner_name, owner_name_span, subscript)
        {
            visible
        } else {
            self.assoc_binding_visible_from_named_callers(owner_name, subscript.span())
        };
        self.assoc_binding_visibility_memo.insert(key, visible);
        visible
    }

    fn assoc_binding_visible_in_nearest_scope(
        &self,
        owner_name: &Name,
        owner_name_span: Option<Span>,
        subscript: &Subscript,
    ) -> Option<bool> {
        let lookup_span = owner_name_span.unwrap_or(subscript.span());
        let current_scope = self.semantic.scope_at(subscript.span().start.offset);
        self.semantic
            .visible_binding_for_assoc_lookup(owner_name, current_scope, lookup_span)
            .map(|binding| binding.attributes.contains(BindingAttributes::ASSOC))
    }

    fn assoc_binding_visible_from_named_callers(&self, owner_name: &Name, span: Span) -> bool {
        let Some(function_names) = self.named_function_scope_names(span.start.offset) else {
            return false;
        };

        let mut seen = AssocCallerSeenNames::new();
        let mut worklist = SmallVec::<[Name; 4]>::new();
        worklist.extend(function_names.iter().cloned());

        while let Some(function_name) = worklist.pop() {
            if !seen.insert(&function_name) {
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

                if let Some(caller_names) =
                    self.named_function_scope_names(call_site.name_span.start.offset)
                {
                    worklist.extend(caller_names.iter().cloned());
                }
            }
        }

        false
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
        self.semantic.ancestor_scopes(scope).find_map(|scope_id| {
            match &self.semantic.scope(scope_id).kind {
                shuck_semantic::ScopeKind::Function(shuck_semantic::FunctionScopeKind::Named(
                    names,
                )) => Some(names.as_slice()),
                _ => None,
            }
        })
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

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }

        while chars.peek().is_some_and(|next| *next == '\\') {
            chars.next();
        }

        if chars.peek().is_some_and(|next| {
            matches!(next, '"' | '\'')
                || (next.is_whitespace()
                    && (matches!(
                        context,
                        ShellQuotingLiteralTextContext::LiteralBackslashNewlines
                    ) || !matches!(next, '\n' | '\r')))
        }) {
            return true;
        }
    }

    false
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
    visit_arithmetic_words(expression, &mut |word| {
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
    visit_arithmetic_words(expression, &mut |word| {
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
        if !is_unescaped_dollar(bytes, index)
            || bytes[index + 1] != b'('
            || bytes[index + 2] != b'('
        {
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
                    visit_arithmetic_words(expression, &mut |word| {
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
                    visit_arithmetic_words(expression, &mut |word| {
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
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_from_parts_impl(parts, semantic, source, spans, false);
}

fn collect_arithmetic_update_operator_spans_from_parts_impl(
    parts: &[WordPartNode],
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_arithmetic_update_operator_spans_from_parts_impl(
                    parts,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                )
            }
            WordPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &expression_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            WordPart::Parameter(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
                    parameter,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                )
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                    operator,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                )
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
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &offset_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &length_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                if include_nested_commands {
                    collect_arithmetic_update_operator_spans_in_nested_command_body(
                        body, semantic, source, spans,
                    );
                }
            }
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_var_ref(
    reference: &VarRef,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_var_ref_impl(
        reference, semantic, source, spans, false,
    );
}

fn collect_arithmetic_update_operator_spans_in_var_ref_impl(
    reference: &VarRef,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    if !var_ref_subscript_has_assoc_semantics(reference, semantic) {
        collect_arithmetic_update_operator_spans_in_subscript(
            reference.subscript.as_deref(),
            source,
            spans,
        );
    }
    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_arithmetic_update_operator_spans_from_parts_impl(
            &word.parts,
            semantic,
            source,
            spans,
            include_nested_commands,
        );
    });
}

fn collect_arithmetic_update_operator_spans_in_parameter_expansion_with_nested_commands(
    parameter: &ParameterExpansion,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
        parameter, semantic, source, spans, true,
    );
}

fn collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
    parameter: &ParameterExpansion,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(operator) = operator.as_ref() {
                    collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                        operator,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &operand_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
                    operator,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(operand_word_ast) = operand_word_ast.as_ref() {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &operand_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
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
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
                if let Some(expression) = offset_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &offset_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
                if let Some(expression) = length_ast {
                    collect_arithmetic_update_operator_spans(Some(expression), source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_arithmetic_update_operator_spans_from_parts_impl(
                        &length_word_ast.parts,
                        semantic,
                        source,
                        spans,
                        include_nested_commands,
                    );
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => match &syntax.target {
            ZshExpansionTarget::Reference(reference) => {
                collect_arithmetic_update_operator_spans_in_var_ref_impl(
                    reference,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Nested(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion_impl(
                    parameter,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Word(word) => {
                collect_arithmetic_update_operator_spans_from_parts_impl(
                    &word.parts,
                    semantic,
                    source,
                    spans,
                    include_nested_commands,
                );
            }
            ZshExpansionTarget::Empty => {}
        },
    }
}

fn collect_arithmetic_update_operator_spans_in_parameter_operator_impl(
    operator: &ParameterOp,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
    include_nested_commands: bool,
) {
    match operator {
        ParameterOp::ReplaceFirst {
            replacement_word_ast,
            ..
        }
        | ParameterOp::ReplaceAll {
            replacement_word_ast,
            ..
        } => collect_arithmetic_update_operator_spans_from_parts_impl(
            &replacement_word_ast.parts,
            semantic,
            source,
            spans,
            include_nested_commands,
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
    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
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
        expansion_kind: analysis.expansion_kind(),
        substitution_shape: analysis.substitution_shape,
    }
}

fn double_quoted_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_expansion_part_spans(word, &mut spans);
    spans
}

fn collect_double_quoted_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_double_quoted_expansion_spans(&word.parts, false, spans);
}

fn single_quoted_equivalent_if_plain_double_quoted_word(
    word: &Word,
    source: &str,
) -> Option<String> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    let WordPart::DoubleQuoted { dollar: false, .. } = &part.kind else {
        return None;
    };

    let text = word.span.slice(source);
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut cooked = String::with_capacity(body.len());
    push_cooked_double_quoted_word_text(body, &mut cooked);

    Some(shell_single_quoted_literal(&cooked))
}

fn push_cooked_double_quoted_word_text(text: &str, out: &mut String) {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some(escaped @ ('$' | '"' | '\\' | '`')) => out.push(escaped),
            Some('\n') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
}

fn shell_single_quoted_literal(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
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

    if args
        .get(index)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("--")
    {
        index += 1;
    }

    let target_name_uses = args
        .get(index)
        .filter(|word| !word_starts_with_literal_dash(word, source))
        .map(|word| comparable_read_target_name_uses(word, source))
        .unwrap_or_default();

    MapfileCommandFacts {
        input_fd,
        target_name_uses,
    }
}

fn mapfile_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's')
}

fn parse_xargs_command<'a>(args: &[&'a Word], source: &str) -> XargsCommandFacts<'a> {
    let mut uses_null_input = false;
    let mut max_procs = None;
    let mut zero_digit_option_word = false;
    let mut inline_replace_options = Vec::new();
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

        zero_digit_option_word |= text.contains('0');

        if let Some(long) = text.strip_prefix("--") {
            if long_name(long) == "null" {
                uses_null_input = true;
            }
            if long_name(long) == "max-procs"
                && let Some(argument) =
                    xargs_long_option_argument(long, args.get(index + 1), source)
            {
                max_procs = argument.parse::<u64>().ok();
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
                inline_replace_options.push(XargsInlineReplaceOptionFact {
                    span: word.span,
                    uses_default_replacement: chars.peek().is_none(),
                });
            }
            if flag == 'P' {
                let remainder = chars.collect::<String>();
                let has_inline_argument = !remainder.is_empty();
                let argument = if has_inline_argument {
                    Some(remainder)
                } else {
                    args.get(index + 1)
                        .and_then(|next| static_word_text(next, source))
                        .map(|value| value.into_owned())
                };
                max_procs = argument.and_then(|value| value.parse::<u64>().ok());
                consume_next_argument = !has_inline_argument;
                break;
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
        max_procs,
        zero_digit_option_word,
        inline_replace_options: inline_replace_options.into_boxed_slice(),
        command_operand_words: args[index..].to_vec().into_boxed_slice(),
        sc2267_default_replace_silent_shape: xargs_sc2267_default_replace_silent_shape(
            &args[index..],
            source,
        ),
    }
}

fn xargs_sc2267_default_replace_silent_shape(args: &[&Word], source: &str) -> bool {
    xargs_command_is_shell_c_wrapper(args, source)
        || xargs_command_is_echo_leading_dash_replacement(args, source)
}

fn xargs_command_is_shell_c_wrapper(args: &[&Word], source: &str) -> bool {
    let args = if args
        .first()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("command")
    {
        &args[1..]
    } else {
        args
    };

    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    matches!(
        command_basename(command_name.as_ref()),
        "sh" | "bash" | "dash" | "ksh" | "zsh"
    ) && args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("-c")
}

fn xargs_command_is_echo_leading_dash_replacement(args: &[&Word], source: &str) -> bool {
    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    if command_basename(command_name.as_ref()) != "echo" {
        return false;
    }

    let Some(first_operand) = args.get(1) else {
        return false;
    };
    let literal_prefix = leading_literal_word_prefix(first_operand, source);
    literal_prefix.starts_with('-') && literal_prefix != "-" && literal_prefix.contains("{}")
}

fn command_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn xargs_long_option_argument(
    option: &str,
    next_word: Option<&&Word>,
    source: &str,
) -> Option<String> {
    if let Some((_, value)) = option.split_once('=') {
        return Some(value.to_owned());
    }

    next_word
        .and_then(|word| static_word_text(word, source))
        .map(|value| value.into_owned())
}

fn long_name(option: &str) -> &str {
    option.split_once('=').map_or(option, |(name, _)| name)
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

fn single_quoted_literal_exempt_argument(
    command_name: Option<&str>,
    args: &[Word],
    arg_index: usize,
    body_arg_start: usize,
    word: &Word,
    trap_action: Option<&Word>,
    source: &str,
) -> bool {
    let Some(command_name) = command_name else {
        return false;
    };

    if trap_action.is_some_and(|action| std::ptr::eq(action, word)) {
        return true;
    }

    let Some(body_args) = args.get(body_arg_start..) else {
        return false;
    };
    let Some(relative_arg_index) = arg_index.checked_sub(body_arg_start) else {
        return false;
    };

    match command_name {
        "alias" => static_word_text(word, source).is_some_and(|text| text.contains('=')),
        "eval" => true,
        "git filter-branch" | "mumps -run %XCMD" | "mumps -run LOOP%XCMD" => true,
        "docker" | "podman" | "oc" => {
            container_shell_command_argument_index(body_args, source) == Some(relative_arg_index)
                || format_option_argument_index(body_args, source) == Some(relative_arg_index)
                || format_option_value_word(body_args, relative_arg_index, source)
        }
        "dpkg-query" => {
            dpkg_query_format_argument_index(body_args, source) == Some(relative_arg_index)
                || dpkg_query_format_option_value_word(body_args, relative_arg_index, source)
        }
        "jq" => jq_literal_argument_index(body_args, source).contains(&relative_arg_index),
        "rename" => rename_program_argument_index(body_args, source) == Some(relative_arg_index),
        "rg" => rg_pattern_argument_index(body_args, source) == Some(relative_arg_index),
        "sh" | "bash" | "dash" | "ksh" | "zsh" => {
            shell_command_argument_index(body_args, source) == Some(relative_arg_index)
        }
        "ssh" => ssh_remote_command_argument_index(body_args, source).is_some_and(|index| {
            relative_arg_index >= index
                && static_word_text(word, source).is_some_and(|text| text.as_ref() != "-t")
        }),
        "unset" => true,
        "xprop" => xprop_value_argument_index(body_args, source) == Some(relative_arg_index),
        _ if command_name.ends_with("awk") => {
            awk_literal_argument_index(body_args, source).contains(&relative_arg_index)
        }
        _ if command_name.starts_with("perl") => {
            perl_program_argument_index(body_args, source).contains(&relative_arg_index)
        }
        _ => false,
    }
}

fn single_quoted_literal_exempt_here_string(command_name: Option<&str>) -> bool {
    matches!(command_name, Some("sh" | "bash" | "dash" | "ksh" | "zsh"))
}

fn shell_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        shell_flag_contains_command_string(flag.as_ref()).then_some(index + 1)
    })
}

fn awk_literal_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            let raw = args[index].span.slice(source);
            if raw.starts_with("-F") {
                index += 1;
                continue;
            }
            result.push(index);
            index += 1;
            continue;
        };
        match text.as_ref() {
            "--" => {
                result.extend(index + 1..args.len());
                break;
            }
            "-F" | "-f" | "--field-separator" | "--file" => index += 2,
            "-v" | "--assign" => {
                if args.get(index + 1).is_some() {
                    result.push(index + 1);
                }
                index += 2;
            }
            _ if text.starts_with("--assign=") => {
                result.push(index);
                index += 1;
            }
            _ if text.starts_with("-F") && text.len() > 2 => index += 1,
            _ if text.starts_with("--field-separator=") || text.starts_with("--file=") => {
                index += 1;
            }
            _ if text.starts_with('-') && text != "-" => {
                if short_option_cluster_contains_flag(text.as_ref(), 'F')
                    || short_option_cluster_contains_flag(text.as_ref(), 'f')
                {
                    index += 2;
                } else {
                    if short_option_cluster_contains_flag(text.as_ref(), 'v')
                        && args.get(index + 1).is_some()
                    {
                        result.push(index + 1);
                    }
                    index += 1;
                }
            }
            _ => {
                result.push(index);
                index += 1;
            }
        }
    }
    result
}

fn jq_literal_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            result.push(index);
            break;
        };
        match text.as_ref() {
            "--" => {
                if args.get(index + 1).is_some() {
                    result.push(index + 1);
                }
                break;
            }
            "-f" | "--from-file" => return result,
            "-L" | "--slurpfile" | "--rawfile" => index += 2,
            "--arg" | "--argjson" => {
                if args.get(index + 2).is_some() {
                    result.push(index + 2);
                }
                index += 3;
            }
            _ if text.starts_with("--from-file=") => return result,
            _ if text.starts_with("-L") && text.len() > 2 => index += 1,
            _ if text.starts_with('-') && text != "-" => index += 1,
            _ => {
                result.push(index);
                break;
            }
        }
    }
    result
}

fn perl_program_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    args.windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            let flag = static_word_text(&pair[0], source)?;
            perl_option_takes_program_argument(flag.as_ref()).then_some(index + 1)
        })
        .collect()
}

fn perl_option_takes_program_argument(option: &str) -> bool {
    matches!(option, "-e" | "-E")
        || (option.starts_with('-')
            && !option.starts_with("--")
            && option.chars().any(|character| matches!(character, 'e' | 'E')))
}

fn rename_program_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.iter()
        .position(|word| static_word_text(word, source).is_none_or(|text| !text.starts_with('-')))
}

fn ssh_remote_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let mut index = 0usize;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            return args.get(index + 1).map(|_| index + 1);
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        index += 1;
        if ssh_option_consumes_next_argument(text.as_ref())? {
            args.get(index)?;
            index += 1;
        }
    }

    args.get(index)?;
    args.get(index + 1).map(|_| index + 1)
}

fn rg_pattern_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let mut index = 0usize;
    while index < args.len() {
        let text = static_word_text(&args[index], source)?;
        match text.as_ref() {
            "--" => return args.get(index + 1).map(|_| index + 1),
            "-e" | "--regexp" => return args.get(index + 1).map(|_| index + 1),
            "-f" | "--file" => return None,
            _ if text.starts_with("--regexp=") => return Some(index),
            _ if text.starts_with("--file=") => return None,
            _ if text.starts_with('-') && text != "-" => {
                index += if rg_option_consumes_next_argument(text.as_ref()) {
                    2
                } else {
                    1
                };
            }
            _ => return Some(index),
        }
    }
    None
}

fn rg_option_consumes_next_argument(option: &str) -> bool {
    matches!(
        option,
        "-A" | "--after-context"
            | "-B"
            | "--before-context"
            | "-C"
            | "--context"
            | "-g"
            | "--glob"
            | "--iglob"
            | "-m"
            | "--max-count"
            | "-t"
            | "--type"
            | "-T"
            | "--type-not"
            | "--sort"
            | "--sort-files"
            | "--threads"
    )
}

fn container_shell_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let run_index = args
        .iter()
        .position(|word| static_word_text(word, source).as_deref() == Some("run"))?;
    let mut index = run_index + 1;
    let mut entrypoint_shell = None;

    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            break;
        };

        match text.as_ref() {
            "--" => {
                index += 1;
                break;
            }
            "--entrypoint" => {
                entrypoint_shell = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    .filter(|value| shell_command_name(value.as_ref()))
                    .map(|_| ());
                index += 2;
            }
            _ if text.starts_with("--entrypoint=") => {
                entrypoint_shell = shell_command_name(&text["--entrypoint=".len()..]).then_some(());
                index += 1;
            }
            _ if text.starts_with('-') && text != "-" => {
                index += if container_run_option_consumes_next_argument(text.as_ref()) {
                    2
                } else {
                    1
                };
            }
            _ => break,
        }
    }

    args.get(index)?;

    if entrypoint_shell.is_some() {
        return shell_command_argument_index(args.get(index + 1..).unwrap_or_default(), source)
            .map(|relative| index + 1 + relative);
    }

    let shell_index = (index + 1..args.len()).find(|candidate| {
        static_word_text(&args[*candidate], source).is_some_and(|text| shell_command_name(&text))
    })?;
    shell_command_argument_index(args.get(shell_index + 1..).unwrap_or_default(), source)
        .map(|relative| shell_index + 1 + relative)
}

fn shell_command_name(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "dash" | "ksh" | "zsh")
}

fn container_run_option_consumes_next_argument(option: &str) -> bool {
    matches!(
        option,
        "-a" | "--attach"
            | "--add-host"
            | "--annotation"
            | "--blkio-weight"
            | "--blkio-weight-device"
            | "-c"
            | "--cpu-shares"
            | "--cpus"
            | "--cpuset-cpus"
            | "--cpuset-mems"
            | "--device"
            | "--dns"
            | "--dns-option"
            | "--dns-search"
            | "-e"
            | "--env"
            | "--env-file"
            | "--expose"
            | "--gpus"
            | "-h"
            | "--hostname"
            | "--ip"
            | "--ip6"
            | "-l"
            | "--label"
            | "--label-file"
            | "--log-driver"
            | "--log-opt"
            | "--mount"
            | "--name"
            | "--network"
            | "--network-alias"
            | "-p"
            | "--publish"
            | "--pull"
            | "--restart"
            | "--stop-signal"
            | "--stop-timeout"
            | "--ulimit"
            | "-u"
            | "--user"
            | "--userns"
            | "-v"
            | "--volume"
            | "--volumes-from"
            | "-w"
            | "--workdir"
    )
}

fn format_option_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        matches!(flag.as_ref(), "-f" | "--format" | "--template").then_some(index + 1)
    })
}

fn format_option_value_word(args: &[Word], arg_index: usize, source: &str) -> bool {
    static_word_text(&args[arg_index], source).is_some_and(|text| {
        matches!(
            text.as_ref(),
            _ if text.starts_with("--format=") || text.starts_with("--template=")
        )
    })
}

fn dpkg_query_format_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        matches!(flag.as_ref(), "-f" | "--showformat").then_some(index + 1)
    })
}

fn dpkg_query_format_option_value_word(args: &[Word], arg_index: usize, source: &str) -> bool {
    static_word_text(&args[arg_index], source).is_some_and(|text| {
        text.starts_with("-f=")
            || text.starts_with("--showformat=")
            || (text.starts_with("-f") && text.len() > 2)
    })
}

fn xprop_value_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(3).enumerate().find_map(|(index, triple)| {
        let flag = static_word_text(&triple[0], source)?;
        (flag.as_ref() == "-set").then_some(index + 2)
    })
}

fn trap_action_word<'a>(command: &'a Command, source: &str) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    trap_action_word_from_simple_command(command, source)
}

fn trap_action_word_from_simple_command<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
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

#[cfg(test)]
mod word_classification_tests {
    use super::*;

    fn parse_commands(source: &str) -> StmtSeq {
        Parser::new(source).parse().unwrap().file.body
    }

    #[test]
    fn detects_alias_positional_parameters_with_runtime_quote_state() {
        assert!(contains_positional_parameter_reference("echo $1"));
        assert!(contains_positional_parameter_reference("echo \"${1}\""));
        assert!(contains_positional_parameter_reference("echo ${#1}"));
        assert!(contains_positional_parameter_reference("echo ${!1}"));
        assert!(contains_positional_parameter_reference(r"echo \$$1"));
        assert!(contains_positional_parameter_reference(r"echo \'$1"));
        assert!(contains_positional_parameter_reference("echo hi# $1"));
        assert!(!contains_positional_parameter_reference(r"echo \$1"));
        assert!(!contains_positional_parameter_reference(r"echo \${1}"));
        assert!(!contains_positional_parameter_reference("echo '$1'"));
        assert!(!contains_positional_parameter_reference("echo hi # $1"));
        assert!(!contains_positional_parameter_reference("echo hi; # $1"));
        assert!(!contains_positional_parameter_reference("echo hi;# $1"));
        assert!(!contains_positional_parameter_reference("echo hi &&# $1"));
        assert!(!contains_positional_parameter_reference("echo $$1"));
    }

    #[test]
    fn classify_word_distinguishes_fixed_literals_and_quoted_expansions() {
        let source = "printf \"literal\" \"prefix$foo\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let literal = classify_word(&command.args[0], source);
        assert_eq!(literal.quote, WordQuote::FullyQuoted);
        assert_eq!(literal.literalness, WordLiteralness::FixedLiteral);
        assert_eq!(literal.expansion_kind, WordExpansionKind::None);
        assert_eq!(literal.substitution_shape, WordSubstitutionShape::None);

        let expanded = classify_word(&command.args[1], source);
        assert_eq!(expanded.quote, WordQuote::FullyQuoted);
        assert_eq!(expanded.literalness, WordLiteralness::Expanded);
        assert_eq!(expanded.expansion_kind, WordExpansionKind::Scalar);
        assert_eq!(expanded.substitution_shape, WordSubstitutionShape::None);
    }

    #[test]
    fn classify_word_reports_plain_and_mixed_command_substitutions() {
        let source = "printf \"$(date)\" \"prefix$(date)\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).substitution_shape,
            WordSubstitutionShape::Plain
        );
        assert_eq!(
            classify_word(&command.args[1], source).substitution_shape,
            WordSubstitutionShape::Mixed
        );
    }

    #[test]
    fn classify_word_treats_escaped_backslash_before_command_substitution_as_mixed() {
        let source = "printf \"\\\\$(printf '%03o' \"$i\")\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let classification = classify_word(&command.args[0], source);
        assert_eq!(classification.quote, WordQuote::FullyQuoted);
        assert_eq!(classification.literalness, WordLiteralness::Expanded);
        assert_eq!(
            classification.substitution_shape,
            WordSubstitutionShape::Mixed
        );
    }

    #[test]
    fn classify_word_reports_scalar_and_array_expansions() {
        let source = "printf $foo ${arr[@]} ${arr[0]} ${arr[@]:1}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_word(&command.args[0], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[1], source).expansion_kind,
            WordExpansionKind::Array
        );
        assert_eq!(
            classify_word(&command.args[2], source).expansion_kind,
            WordExpansionKind::Scalar
        );
        assert_eq!(
            classify_word(&command.args[3], source).expansion_kind,
            WordExpansionKind::Array
        );
    }

    #[test]
    fn plain_parameter_reference_accepts_single_direct_expansions_only() {
        let source = "\
printf '%s\\n' \
$name \"$name\" ${name} \"${name}\" $1 \"$#\" \"$@\" ${*} \
${@:2} ${arr[0]} ${arr[@]} ${!name} ${name:-fallback} \"$@$@\" \"prefix$name\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        let plain = command
            .args
            .iter()
            .skip(1)
            .map(word_is_plain_parameter_reference)
            .collect::<Vec<_>>();

        assert_eq!(
            plain,
            vec![
                true, true, true, true, true, true, true, true, false, false, false, false, false,
                false, false
            ]
        );
    }

    #[test]
    fn classify_test_and_conditional_operands_share_literal_runtime_decisions() {
        let source = "test foo\ntest ~\n[[ \"$re\" ]]\n[[ literal ]]\n[[ ~ ]]\n";
        let commands = parse_commands(source);

        let Command::Simple(simple_test) = &commands[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(
            classify_contextual_operand(
                &simple_test.args[0],
                source,
                ExpansionContext::CommandArgument
            ),
            TestOperandClass::FixedLiteral
        );

        let Command::Simple(runtime_test) = &commands[1].command else {
            panic!("expected simple command");
        };
        assert_eq!(
            classify_contextual_operand(
                &runtime_test.args[0],
                source,
                ExpansionContext::CommandArgument
            ),
            TestOperandClass::RuntimeSensitive
        );

        let Command::Compound(CompoundCommand::Conditional(runtime)) = &commands[2].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&runtime.expression, source),
            TestOperandClass::RuntimeSensitive
        );

        let Command::Compound(CompoundCommand::Conditional(literal)) = &commands[3].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&literal.expression, source),
            TestOperandClass::FixedLiteral
        );

        let Command::Compound(CompoundCommand::Conditional(runtime)) = &commands[4].command else {
            panic!("expected conditional");
        };
        assert_eq!(
            classify_conditional_operand(&runtime.expression, source),
            TestOperandClass::RuntimeSensitive
        );
    }

    #[test]
    fn contextual_operand_classification_respects_regex_and_case_contexts() {
        let source = "printf ~ *.sh {a,b}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(
            classify_contextual_operand(&command.args[0], source, ExpansionContext::RegexOperand),
            TestOperandClass::RuntimeSensitive
        );
        assert_eq!(
            classify_contextual_operand(&command.args[1], source, ExpansionContext::CasePattern),
            TestOperandClass::FixedLiteral
        );
        assert_eq!(
            classify_contextual_operand(&command.args[2], source, ExpansionContext::CasePattern),
            TestOperandClass::FixedLiteral
        );
    }
}

#[cfg(test)]
mod expansion_analysis_tests {
    use shuck_ast::Command;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::{
        ComparablePathKey, ComparablePathPart, ExpansionAnalysis, ExpansionContext,
        ExpansionValueShape, RedirectDevNullStatus, WordLiteralness, WordQuote,
        analyze_literal_runtime, analyze_redirect_target, analyze_word, comparable_path,
    };

    fn parse_argument_words(source: &str) -> Vec<shuck_ast::Word> {
        let file = Parser::new(source).parse().unwrap().file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        command.args.to_vec()
    }

    fn analyze_argument_words(source: &str) -> Vec<ExpansionAnalysis> {
        parse_argument_words(source)
            .iter()
            .map(|word| analyze_word(word, source, None))
            .collect()
    }

    fn analyze_argument_words_with_dialect(
        source: &str,
        dialect: ShellDialect,
    ) -> Vec<ExpansionAnalysis> {
        let file = Parser::with_dialect(source, dialect).parse().unwrap().file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        command
            .args
            .iter()
            .map(|word| analyze_word(word, source, None))
            .collect()
    }

    #[test]
    fn analyze_word_tracks_array_values_and_multi_field_expansions_separately() {
        let analyses = analyze_argument_words(
            "printf %s ${arr[@]} \"${arr[*]}\" ${!prefix@} ${!name} ${value@Q}\n",
        );

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].array_valued);
        assert!(analyses[1].can_expand_to_multiple_fields);

        assert_eq!(analyses[2].quote, WordQuote::FullyQuoted);
        assert_eq!(analyses[2].value_shape, ExpansionValueShape::Array);
        assert!(analyses[2].array_valued);
        assert!(!analyses[2].can_expand_to_multiple_fields);

        assert_eq!(analyses[3].value_shape, ExpansionValueShape::MultiField);
        assert!(!analyses[3].array_valued);
        assert!(analyses[3].can_expand_to_multiple_fields);

        assert_eq!(analyses[4].value_shape, ExpansionValueShape::Unknown);
        assert_eq!(analyses[4].literalness, WordLiteralness::Expanded);

        assert_eq!(analyses[5].value_shape, ExpansionValueShape::MultiField);
        assert_eq!(analyses[5].literalness, WordLiteralness::Expanded);
        assert!(!analyses[5].array_valued);
    }

    #[test]
    fn analyze_word_marks_prefix_match_at_as_multi_field_even_when_quoted() {
        let analyses = analyze_argument_words("printf %s \"${!prefix@}\" \"${!prefix*}\"\n");

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].can_expand_to_multiple_fields);

        assert_eq!(analyses[2].value_shape, ExpansionValueShape::Unknown);
        assert!(!analyses[2].can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_treats_bourne_transformations_as_split_and_glob_hazards() {
        let analyses = analyze_argument_words("printf %s ${name@U}\n");

        assert_eq!(analyses[1].value_shape, ExpansionValueShape::MultiField);
        assert!(analyses[1].hazards.field_splitting);
        assert!(analyses[1].hazards.pathname_matching);
        assert!(analyses[1].can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_word_distinguishes_typed_zsh_pattern_families() {
        let analyses = analyze_argument_words_with_dialect(
            "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:-$fallback}\n",
            ShellDialect::Zsh,
        );

        assert!(analyses[0].hazards.runtime_pattern);
        assert!(analyses[1].hazards.runtime_pattern);
        assert!(!analyses[2].hazards.runtime_pattern);
        assert!(!analyses[3].hazards.runtime_pattern);
        assert!(
            analyses
                .iter()
                .all(|analysis| analysis.value_shape == ExpansionValueShape::Unknown)
        );
    }

    #[test]
    fn analyze_word_treats_zsh_trailing_glob_qualifiers_as_non_literal_pathname_hazards() {
        let analyses =
            analyze_argument_words_with_dialect("print **/*(.om[1,3])\n", ShellDialect::Zsh);

        assert_eq!(analyses[0].literalness, WordLiteralness::Expanded);
        assert!(!analyses[0].is_fixed_literal());
        assert_eq!(analyses[0].value_shape, ExpansionValueShape::Unknown);
        assert!(analyses[0].hazards.pathname_matching);
        assert!(analyses[0].can_expand_to_multiple_fields);
        assert!(!analyses[0].array_valued);
    }

    #[test]
    fn analyze_word_treats_zsh_inline_glob_controls_as_non_literal_pathname_hazards() {
        let analyses = analyze_argument_words_with_dialect("print (#i)*.jpg\n", ShellDialect::Zsh);

        assert_eq!(analyses[0].literalness, WordLiteralness::Expanded);
        assert!(!analyses[0].is_fixed_literal());
        assert_eq!(analyses[0].value_shape, ExpansionValueShape::Unknown);
        assert!(analyses[0].hazards.pathname_matching);
        assert!(analyses[0].can_expand_to_multiple_fields);
        assert!(!analyses[0].array_valued);
    }

    #[test]
    fn analyze_word_suppresses_zsh_glob_fanout_when_glob_is_disabled() {
        let source = "print *.jpg\n";
        let file = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap()
            .file;
        let Command::Simple(command) = &file.body[0].command else {
            panic!("expected simple command");
        };
        let options = shuck_semantic::ZshOptionState {
            glob: shuck_semantic::OptionValue::Off,
            ..shuck_semantic::ZshOptionState::zsh_default()
        };
        let analysis = analyze_word(&command.args[0], source, Some(&options));

        assert!(!analysis.hazards.pathname_matching);
        assert!(!analysis.can_expand_to_multiple_fields);
    }

    #[test]
    fn analyze_literal_runtime_tracks_globs_in_mixed_words() {
        let source =
            "printf '%s\\n' \"$basedir/\"* \"$(dirname \"$0\")\"/../docs/usage/distrobox*\n";
        let words = parse_argument_words(source);
        let first = analyze_literal_runtime(&words[1], source, ExpansionContext::ForList, None);
        let second = analyze_literal_runtime(&words[2], source, ExpansionContext::ForList, None);

        assert!(first.hazards.pathname_matching);
        assert!(first.is_runtime_sensitive());
        assert!(second.hazards.pathname_matching);
        assert!(second.is_runtime_sensitive());
    }

    #[test]
    fn analyze_redirect_target_distinguishes_descriptor_dups_and_dev_null() {
        let static_dup_source = "echo hi 2>&3\n";
        let static_dup_file = Parser::new(static_dup_source).parse().unwrap().file;
        let Command::Simple(_) = &static_dup_file.body[0].command else {
            panic!("expected simple command");
        };
        let static_dup = analyze_redirect_target(
            &static_dup_file.body[0].redirects[0],
            static_dup_source,
            None,
        )
        .expect("expected redirect analysis");
        assert!(static_dup.is_descriptor_dup());
        assert_eq!(static_dup.numeric_descriptor_target, Some(3));
        assert!(!static_dup.is_runtime_sensitive());

        let file_source = "echo hi > /dev/null\n";
        let file_commands = Parser::new(file_source).parse().unwrap().file;
        let Command::Simple(_) = &file_commands.body[0].command else {
            panic!("expected simple command");
        };
        let file = analyze_redirect_target(&file_commands.body[0].redirects[0], file_source, None)
            .expect("expected redirect analysis");
        assert!(file.is_file_target());
        assert!(file.is_definitely_dev_null());
        assert!(!file.is_runtime_sensitive());

        let maybe_source = "echo hi > \"$target\"\n";
        let maybe_commands = Parser::new(maybe_source).parse().unwrap().file;
        let Command::Simple(_) = &maybe_commands.body[0].command else {
            panic!("expected simple command");
        };
        let maybe =
            analyze_redirect_target(&maybe_commands.body[0].redirects[0], maybe_source, None)
                .expect("expected redirect analysis");
        assert!(maybe.is_file_target());
        assert_eq!(
            maybe.dev_null_status,
            Some(RedirectDevNullStatus::MaybeDevNull)
        );
        assert!(maybe.is_runtime_sensitive());

        let fanout_source = "echo hi > ${targets[@]}\n";
        let fanout_commands = Parser::new(fanout_source).parse().unwrap().file;
        let Command::Simple(_) = &fanout_commands.body[0].command else {
            panic!("expected simple command");
        };
        let fanout =
            analyze_redirect_target(&fanout_commands.body[0].redirects[0], fanout_source, None)
                .expect("expected redirect analysis");
        assert!(fanout.can_expand_to_multiple_fields());
        assert!(fanout.is_runtime_sensitive());

        let tilde_source = "echo hi > ~/*.log\n";
        let tilde_commands = Parser::new(tilde_source).parse().unwrap().file;
        let Command::Simple(_) = &tilde_commands.body[0].command else {
            panic!("expected simple command");
        };
        let tilde =
            analyze_redirect_target(&tilde_commands.body[0].redirects[0], tilde_source, None)
                .expect("expected redirect analysis");
        assert!(tilde.is_file_target());
        assert_eq!(
            tilde.dev_null_status,
            Some(RedirectDevNullStatus::MaybeDevNull)
        );
        assert!(tilde.runtime_literal.is_runtime_sensitive());
        assert!(tilde.is_runtime_sensitive());
    }

    #[test]
    fn comparable_path_accepts_simple_literals_and_single_parameter_expansions() {
        let source = "cmd foo \"$src\" \"${dst}\" ~/.zshrc \"$dir/Cargo.toml\" $tmpf \"$@\" \"$(printf hi)\" <(cat) *.log /dev/null /dev/tty /dev/stdin /dev/fd/0 /proc/self/fd/1\n";
        let words = parse_argument_words(source);

        assert_eq!(
            comparable_path(&words[0], source, ExpansionContext::CommandArgument, None)
                .expect("expected literal path")
                .key(),
            &ComparablePathKey::Literal("foo".into())
        );
        assert_eq!(
            comparable_path(&words[1], source, ExpansionContext::CommandArgument, None)
                .expect("expected parameter path")
                .key(),
            &ComparablePathKey::Parameter("src".into())
        );
        assert_eq!(
            comparable_path(&words[2], source, ExpansionContext::CommandArgument, None)
                .expect("expected parameter path")
                .key(),
            &ComparablePathKey::Parameter("dst".into())
        );
        assert_eq!(
            comparable_path(&words[3], source, ExpansionContext::CommandArgument, None)
                .expect("expected tilde literal")
                .key(),
            &ComparablePathKey::Literal("~/.zshrc".into())
        );
        assert_eq!(
            comparable_path(&words[4], source, ExpansionContext::CommandArgument, None)
                .expect("expected path template")
                .key(),
            &ComparablePathKey::Template(
                [
                    ComparablePathPart::Parameter("dir".into()),
                    ComparablePathPart::Literal("/Cargo.toml".into()),
                ]
                .into()
            )
        );
        assert_eq!(
            comparable_path(&words[5], source, ExpansionContext::CommandArgument, None)
                .expect("expected bare parameter path")
                .key(),
            &ComparablePathKey::Parameter("tmpf".into())
        );
        assert!(
            comparable_path(&words[6], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[7], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[8], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[9], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[10], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[11], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[12], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[13], source, ExpansionContext::CommandArgument, None).is_none()
        );
        assert!(
            comparable_path(&words[14], source, ExpansionContext::CommandArgument, None).is_none()
        );
    }

    #[test]
    fn analyze_literal_runtime_tracks_context_sensitive_literals() {
        let source = "printf ~ ~user x=~ *.sh {a,b} \"~\" '*.sh' \"{a,b}\"\n";
        let words = parse_argument_words(source);

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[1], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[2], source, ExpansionContext::CommandArgument, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[3], source, ExpansionContext::CommandArgument, None)
                .hazards
                .pathname_matching
        );
        assert!(
            analyze_literal_runtime(&words[4], source, ExpansionContext::CommandArgument, None)
                .hazards
                .brace_fanout
        );

        assert!(
            !analyze_literal_runtime(&words[5], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[6], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[7], source, ExpansionContext::CommandArgument, None)
                .is_runtime_sensitive()
        );

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::StringTestOperand, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::RegexOperand, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            !analyze_literal_runtime(&words[3], source, ExpansionContext::StringTestOperand, None)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[4], source, ExpansionContext::CasePattern, None)
                .is_runtime_sensitive()
        );
    }

    #[test]
    fn analyze_literal_runtime_treats_loop_lists_like_argument_lists() {
        let source = "printf ~ *.sh {a,b}\n";
        let words = parse_argument_words(source);

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::ForList, None)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[1], source, ExpansionContext::ForList, None)
                .hazards
                .pathname_matching
        );
        assert!(
            analyze_literal_runtime(&words[2], source, ExpansionContext::ForList, None)
                .hazards
                .brace_fanout
        );
    }
}
