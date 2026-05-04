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
    pub field_splitting_behavior: FieldSplittingBehavior,
    pub pathname_expansion_behavior: PathnameExpansionBehavior,
    pub hazards: ExpansionHazards,
    pub array_valued: bool,
    pub can_expand_to_multiple_fields: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLiteralAnalysis {
    pub runtime_sensitive: bool,
    pub pathname_expansion_behavior: PathnameExpansionBehavior,
    pub glob_failure_behavior: GlobFailureBehavior,
    pub glob_dot_behavior: GlobDotBehavior,
    pub glob_pattern_behavior: GlobPatternBehavior,
    pub hazards: ExpansionHazards,
}

impl Default for RuntimeLiteralAnalysis {
    fn default() -> Self {
        Self {
            runtime_sensitive: false,
            pathname_expansion_behavior: PathnameExpansionBehavior::Disabled,
            glob_failure_behavior: GlobFailureBehavior::KeepLiteralOnNoMatch,
            glob_dot_behavior: GlobDotBehavior::ExplicitDotRequired,
            glob_pattern_behavior: default_glob_pattern_behavior(),
            hazards: ExpansionHazards::default(),
        }
    }
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

pub(super) fn classify_word(word: &Word, source: &str) -> WordClassification {
    word_classification_from_analysis(analyze_word(word, source, None))
}

pub(super) fn classify_contextual_operand(
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

pub(super) fn classify_conditional_operand(
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

pub(super) fn classify_pattern_operand(pattern: &Pattern, source: &str) -> TestOperandClass {
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

pub(super) fn field_splitting_behavior_for_options(
    options: Option<&ZshOptionState>,
) -> FieldSplittingBehavior {
    match options {
        Some(options) => match options.sh_word_split {
            OptionValue::Off => FieldSplittingBehavior::Never,
            OptionValue::On => FieldSplittingBehavior::UnquotedOnly,
            OptionValue::Unknown => FieldSplittingBehavior::Ambiguous,
        },
        None => FieldSplittingBehavior::UnquotedOnly,
    }
}

pub(super) fn pathname_expansion_behavior_for_options(
    options: Option<&ZshOptionState>,
) -> PathnameExpansionBehavior {
    match options {
        Some(options) => match options.glob {
            OptionValue::Off => PathnameExpansionBehavior::Disabled,
            OptionValue::On => match options.glob_subst {
                OptionValue::Off => PathnameExpansionBehavior::LiteralGlobsOnly,
                OptionValue::On => PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted,
                OptionValue::Unknown => PathnameExpansionBehavior::Ambiguous,
            },
            OptionValue::Unknown => match options.glob_subst {
                OptionValue::Off => PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled,
                OptionValue::On | OptionValue::Unknown => PathnameExpansionBehavior::Ambiguous,
            },
        },
        None => PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted,
    }
}

pub(super) fn glob_failure_behavior_for_options(
    options: Option<&ZshOptionState>,
) -> GlobFailureBehavior {
    match options {
        Some(options) => match options.glob {
            OptionValue::Off => GlobFailureBehavior::KeepLiteralOnNoMatch,
            OptionValue::On => match options.csh_null_glob {
                OptionValue::On => GlobFailureBehavior::CshNullGlob,
                OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                OptionValue::Off => match options.null_glob {
                    OptionValue::On => GlobFailureBehavior::DropUnmatchedPattern,
                    OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                    OptionValue::Off => match options.nomatch {
                        OptionValue::On => GlobFailureBehavior::ErrorOnNoMatch,
                        OptionValue::Off => GlobFailureBehavior::KeepLiteralOnNoMatch,
                        OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                    },
                },
            },
            OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
        },
        None => GlobFailureBehavior::KeepLiteralOnNoMatch,
    }
}

pub(super) fn glob_dot_behavior_for_options(options: Option<&ZshOptionState>) -> GlobDotBehavior {
    match options {
        Some(options) => match options.glob_dots {
            OptionValue::Off => GlobDotBehavior::ExplicitDotRequired,
            OptionValue::On => GlobDotBehavior::DotfilesIncluded,
            OptionValue::Unknown => GlobDotBehavior::Ambiguous,
        },
        None => GlobDotBehavior::ExplicitDotRequired,
    }
}

pub(super) fn glob_pattern_behavior_for_options(
    options: Option<&ZshOptionState>,
) -> GlobPatternBehavior {
    options.map_or_else(default_glob_pattern_behavior, |options| {
        semantic_glob_pattern_behavior(
            pattern_operator_behavior_for_options(options.extended_glob),
            pattern_operator_behavior_for_options(options.ksh_glob),
            pattern_operator_behavior_for_options(options.sh_glob),
        )
    })
}

fn default_glob_pattern_behavior() -> GlobPatternBehavior {
    semantic_glob_pattern_behavior(
        PatternOperatorBehavior::Disabled,
        PatternOperatorBehavior::Disabled,
        PatternOperatorBehavior::Disabled,
    )
}

fn semantic_glob_pattern_behavior(
    extended_glob: PatternOperatorBehavior,
    ksh_glob: PatternOperatorBehavior,
    sh_glob: PatternOperatorBehavior,
) -> GlobPatternBehavior {
    GlobPatternBehavior::from_parts(
        extended_glob,
        ksh_glob,
        sh_glob,
    )
}

fn pattern_operator_behavior_for_options(value: OptionValue) -> PatternOperatorBehavior {
    match value {
        OptionValue::Off => PatternOperatorBehavior::Disabled,
        OptionValue::On => PatternOperatorBehavior::Enabled,
        OptionValue::Unknown => PatternOperatorBehavior::Ambiguous,
    }
}

fn merge_field_splitting_behavior(
    current: Option<FieldSplittingBehavior>,
    next: FieldSplittingBehavior,
) -> Option<FieldSplittingBehavior> {
    Some(match current {
        None => next,
        Some(existing) if existing == next => existing,
        Some(_) => FieldSplittingBehavior::Ambiguous,
    })
}

fn merge_pathname_expansion_behavior(
    current: Option<PathnameExpansionBehavior>,
    next: PathnameExpansionBehavior,
) -> Option<PathnameExpansionBehavior> {
    use PathnameExpansionBehavior::{
        Ambiguous, Disabled, LiteralGlobsOnly, LiteralGlobsOnlyOrDisabled,
    };

    Some(match current {
        None => next,
        Some(existing) if existing == next => existing,
        Some(Ambiguous) | Some(_) if next == Ambiguous => Ambiguous,
        Some(Disabled) if next == LiteralGlobsOnly => LiteralGlobsOnlyOrDisabled,
        Some(LiteralGlobsOnly) if next == Disabled => LiteralGlobsOnlyOrDisabled,
        Some(LiteralGlobsOnlyOrDisabled)
            if matches!(next, Disabled | LiteralGlobsOnly | LiteralGlobsOnlyOrDisabled) =>
        {
            LiteralGlobsOnlyOrDisabled
        }
        Some(Disabled) if next == LiteralGlobsOnlyOrDisabled => LiteralGlobsOnlyOrDisabled,
        Some(LiteralGlobsOnly) if next == LiteralGlobsOnlyOrDisabled => {
            LiteralGlobsOnlyOrDisabled
        }
        Some(_) => Ambiguous,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PartValueShape {
    Scalar,
    Array,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PartAnalysis {
    value_shape: PartValueShape,
    array_valued: bool,
    can_expand_to_multiple_fields: bool,
    field_splitting_behavior: FieldSplittingBehavior,
    pathname_expansion_behavior: PathnameExpansionBehavior,
    hazards: ExpansionHazards,
    command_substitution: bool,
    process_substitution: bool,
}

#[derive(Default)]
pub(super) struct AnalysisSummary {
    has_non_literal: bool,
    has_scalar_value: bool,
    has_array_value: bool,
    has_unknown_value: bool,
    can_expand_to_multiple_fields: bool,
    field_splitting_behavior: Option<FieldSplittingBehavior>,
    pathname_expansion_behavior: Option<PathnameExpansionBehavior>,
    hazards: ExpansionHazards,
    command_substitution_count: usize,
    has_process_substitution: bool,
}

pub(super) fn analyze_word(
    word: &Word,
    _source: &str,
    options: Option<&ZshOptionState>,
) -> ExpansionAnalysis {
    let mut summary = AnalysisSummary::default();
    analyze_parts(&word.parts, false, options, &mut summary);
    let default_field_splitting_behavior = field_splitting_behavior_for_options(options);
    let default_pathname_expansion_behavior = pathname_expansion_behavior_for_options(options);

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
        field_splitting_behavior: summary
            .field_splitting_behavior
            .unwrap_or(default_field_splitting_behavior),
        pathname_expansion_behavior: summary
            .pathname_expansion_behavior
            .unwrap_or(default_pathname_expansion_behavior),
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
    let mut analysis = RuntimeLiteralAnalysis {
        pathname_expansion_behavior: pathname_expansion_behavior_for_options(options),
        glob_failure_behavior: glob_failure_behavior_for_options(options),
        glob_dot_behavior: glob_dot_behavior_for_options(options),
        glob_pattern_behavior: glob_pattern_behavior_for_options(options),
        ..RuntimeLiteralAnalysis::default()
    };
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
pub(super) struct RuntimeLiteralState {
    seen_text: bool,
    last_unquoted_char: Option<char>,
}

pub(super) fn analyze_literal_runtime_parts(
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

pub(super) fn scan_literal_runtime_text(
    text: &str,
    context: ExpansionContext,
    _options: Option<&ZshOptionState>,
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
            && analysis
                .pathname_expansion_behavior
                .literal_globs_can_expand()
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

pub(super) fn tilde_is_runtime_sensitive(state: &RuntimeLiteralState) -> bool {
    !state.seen_text || matches!(state.last_unquoted_char, Some('=') | Some(':'))
}

pub(super) fn brace_fanout_is_runtime_sensitive(content: &str) -> bool {
    content.contains(',') || content.contains("..")
}

pub(super) fn context_allows_tilde(context: ExpansionContext) -> bool {
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

pub(super) fn context_allows_pathname_matching(context: ExpansionContext) -> bool {
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

pub(super) fn context_allows_brace_fanout(context: ExpansionContext) -> bool {
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

pub(super) fn analyze_parts(
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
                summary.field_splitting_behavior = merge_field_splitting_behavior(
                    summary.field_splitting_behavior,
                    analysis.field_splitting_behavior,
                );
                summary.pathname_expansion_behavior = merge_pathname_expansion_behavior(
                    summary.pathname_expansion_behavior,
                    analysis.pathname_expansion_behavior,
                );
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

pub(super) fn analyze_part(
    part: &WordPart,
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> PartAnalysis {
    let field_splitting_behavior = field_splitting_behavior_for_options(options);
    let pathname_expansion_behavior = pathname_expansion_behavior_for_options(options);

    match part {
        WordPart::ZshQualifiedGlob(_) => PartAnalysis {
            value_shape: PartValueShape::Unknown,
            array_valued: false,
            can_expand_to_multiple_fields: !in_double_quotes
                && pathname_expansion_behavior.literal_globs_can_expand(),
            field_splitting_behavior,
            pathname_expansion_behavior,
            hazards: ExpansionHazards {
                pathname_matching: !in_double_quotes
                    && pathname_expansion_behavior.literal_globs_can_expand(),
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
            field_splitting_behavior,
            pathname_expansion_behavior,
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
            field_splitting_behavior,
            pathname_expansion_behavior,
            ExpansionHazards {
                command_or_process_substitution: true,
                ..ExpansionHazards::default()
            },
            false,
            true,
        ),
        WordPart::Variable(name) if matches!(name.as_str(), "@") => {
            array_part(
                true,
                field_splitting_behavior,
                pathname_expansion_behavior,
                false,
                false,
                false,
            )
        }
        WordPart::Variable(name) if matches!(name.as_str(), "*") => {
            array_part(
                !in_double_quotes,
                field_splitting_behavior,
                pathname_expansion_behavior,
                !in_double_quotes,
                false,
                false,
            )
        }
        WordPart::Variable(_)
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::Substring { .. } => scalar_part(
            substitution_can_expand_to_multiple_fields(in_double_quotes, options),
            field_splitting_behavior,
            pathname_expansion_behavior,
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
            scalar_part(
                false,
                field_splitting_behavior,
                pathname_expansion_behavior,
                ExpansionHazards::default(),
                false,
                false,
            )
        }
        WordPart::ParameterExpansion { operator, .. } => scalar_part(
            substitution_can_expand_to_multiple_fields(in_double_quotes, options),
            field_splitting_behavior,
            pathname_expansion_behavior,
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
            Some(SubscriptSelector::At) => array_part(
                true,
                field_splitting_behavior,
                pathname_expansion_behavior,
                false,
                false,
                false,
            ),
            Some(SubscriptSelector::Star) => {
                array_part(
                    !in_double_quotes,
                    field_splitting_behavior,
                    pathname_expansion_behavior,
                    !in_double_quotes,
                    false,
                    false,
                )
            }
            None => scalar_part(
                substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                field_splitting_behavior,
                pathname_expansion_behavior,
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
            array_part(
                true,
                field_splitting_behavior,
                pathname_expansion_behavior,
                false,
                false,
                false,
            )
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
                field_splitting_behavior,
                pathname_expansion_behavior,
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
            field_splitting_behavior,
            pathname_expansion_behavior,
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

pub(super) fn analyze_parameter_part(
    parameter: &ParameterExpansion,
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> PartAnalysis {
    let field_splitting_behavior = field_splitting_behavior_for_options(options);
    let pathname_expansion_behavior = pathname_expansion_behavior_for_options(options);

    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => match reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector())
            {
                Some(SubscriptSelector::At) => array_part(
                    true,
                    field_splitting_behavior,
                    pathname_expansion_behavior,
                    false,
                    false,
                    false,
                ),
                Some(SubscriptSelector::Star) => {
                    array_part(
                        !in_double_quotes,
                        field_splitting_behavior,
                        pathname_expansion_behavior,
                        !in_double_quotes,
                        false,
                        false,
                    )
                }
                None => scalar_part(
                    substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                    field_splitting_behavior,
                    pathname_expansion_behavior,
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
                field_splitting_behavior,
                pathname_expansion_behavior,
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
            BourneParameterExpansion::Indices { .. } => array_part(
                true,
                field_splitting_behavior,
                pathname_expansion_behavior,
                false,
                false,
                false,
            ),
            BourneParameterExpansion::Indirect { operator, .. } => PartAnalysis {
                value_shape: PartValueShape::Unknown,
                array_valued: false,
                can_expand_to_multiple_fields: substitution_can_expand_to_multiple_fields(
                    in_double_quotes,
                    options,
                ),
                field_splitting_behavior,
                pathname_expansion_behavior,
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
                field_splitting_behavior,
                pathname_expansion_behavior,
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
                    array_part(
                        true,
                        field_splitting_behavior,
                        pathname_expansion_behavior,
                        false,
                        false,
                        false,
                    )
                } else {
                    scalar_part(
                        substitution_can_expand_to_multiple_fields(in_double_quotes, options),
                        field_splitting_behavior,
                        pathname_expansion_behavior,
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
                field_splitting_behavior,
                pathname_expansion_behavior,
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
                field_splitting_behavior,
                pathname_expansion_behavior,
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
            let field_splitting_behavior =
                field_splitting_behavior_for_options(effective_options.as_ref());
            let pathname_expansion_behavior =
                pathname_expansion_behavior_for_options(effective_options.as_ref());
            PartAnalysis {
                value_shape: PartValueShape::Unknown,
                array_valued: false,
                can_expand_to_multiple_fields: substitution_can_expand_to_multiple_fields(
                    in_double_quotes,
                    effective_options.as_ref(),
                ),
                field_splitting_behavior,
                pathname_expansion_behavior,
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

pub(super) fn substitution_can_expand_to_multiple_fields(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    !in_double_quotes
        && (field_splitting_behavior_for_options(options).unquoted_results_can_split()
            || pathname_expansion_behavior_for_options(options)
                .unquoted_substitution_results_can_glob())
}

pub(super) fn substitution_field_splitting_hazard(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    !in_double_quotes && field_splitting_behavior_for_options(options).unquoted_results_can_split()
}

pub(super) fn substitution_pathname_matching_hazard(
    in_double_quotes: bool,
    options: Option<&ZshOptionState>,
) -> bool {
    !in_double_quotes
        && pathname_expansion_behavior_for_options(options)
            .unquoted_substitution_results_can_glob()
}

pub(super) fn overlay_zsh_modifier_overrides(
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

pub(super) fn apply_toggle_override(value: &mut OptionValue, count: usize) {
    if count == 0 {
        return;
    }

    *value = if count % 2 == 1 {
        OptionValue::On
    } else {
        OptionValue::Off
    };
}

pub(super) fn scalar_part(
    can_expand_to_multiple_fields: bool,
    field_splitting_behavior: FieldSplittingBehavior,
    pathname_expansion_behavior: PathnameExpansionBehavior,
    hazards: ExpansionHazards,
    command_substitution: bool,
    process_substitution: bool,
) -> PartAnalysis {
    PartAnalysis {
        value_shape: PartValueShape::Scalar,
        array_valued: false,
        can_expand_to_multiple_fields,
        field_splitting_behavior,
        pathname_expansion_behavior,
        hazards,
        command_substitution,
        process_substitution,
    }
}

pub(super) fn array_part(
    can_expand_to_multiple_fields: bool,
    field_splitting_behavior: FieldSplittingBehavior,
    pathname_expansion_behavior: PathnameExpansionBehavior,
    field_splitting: bool,
    pathname_matching: bool,
    runtime_pattern: bool,
) -> PartAnalysis {
    PartAnalysis {
        value_shape: PartValueShape::Array,
        array_valued: true,
        can_expand_to_multiple_fields,
        field_splitting_behavior,
        pathname_expansion_behavior,
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

pub(super) fn parameter_operator_uses_pattern(operator: &shuck_ast::ParameterOp) -> bool {
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

pub(super) fn zsh_operation_uses_pattern(operation: &ZshExpansionOperation) -> bool {
    matches!(
        operation,
        ZshExpansionOperation::PatternOperation { .. }
            | ZshExpansionOperation::TrimOperation { .. }
            | ZshExpansionOperation::ReplacementOperation { .. }
    )
}

pub(super) fn prefix_match_can_expand_to_multiple_fields(
    kind: PrefixMatchKind,
    in_double_quotes: bool,
) -> bool {
    matches!(kind, PrefixMatchKind::At) || !in_double_quotes
}

pub(super) fn is_plain_command_substitution(parts: &[WordPartNode]) -> bool {
    matches!(
        parts,
        [part] if match &part.kind {
            WordPart::CommandSubstitution { .. } => true,
            WordPart::DoubleQuoted { parts, .. } => is_plain_command_substitution(parts),
            _ => false,
        }
    )
}

pub(super) fn is_quoted_part(part: &WordPart) -> bool {
    matches!(
        part,
        WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
    )
}

pub(super) fn is_fully_quoted(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if is_quoted_part(&part.kind))
}
