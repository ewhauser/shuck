use shuck_ast::{
    BourneParameterExpansion, ParameterExpansion, ParameterExpansionSyntax, PrefixMatchKind,
    Redirect, RedirectKind, Span, SubscriptSelector, Word, WordPart, WordPartNode,
    ZshExpansionOperation,
};
use shuck_semantic::{OptionValue, ZshOptionState};

use super::word::static_word_text;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectTargetKind {
    File,
    DescriptorDup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectDevNullStatus {
    DefinitelyDevNull,
    DefinitelyNotDevNull,
    MaybeDevNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedirectTargetAnalysis {
    pub kind: RedirectTargetKind,
    pub dev_null_status: Option<RedirectDevNullStatus>,
    pub numeric_descriptor_target: Option<i32>,
    pub expansion: ExpansionAnalysis,
    pub runtime_literal: RuntimeLiteralAnalysis,
}

impl RedirectTargetAnalysis {
    pub fn is_descriptor_dup(self) -> bool {
        matches!(self.kind, RedirectTargetKind::DescriptorDup)
    }

    pub fn is_file_target(self) -> bool {
        matches!(self.kind, RedirectTargetKind::File)
    }

    pub fn is_definitely_dev_null(self) -> bool {
        matches!(
            self.dev_null_status,
            Some(RedirectDevNullStatus::DefinitelyDevNull)
        )
    }

    pub fn is_definitely_not_dev_null(self) -> bool {
        matches!(
            self.dev_null_status,
            Some(RedirectDevNullStatus::DefinitelyNotDevNull)
        )
    }

    pub fn is_runtime_sensitive(self) -> bool {
        self.expansion.literalness == WordLiteralness::Expanded
            || self.runtime_literal.is_runtime_sensitive()
            || matches!(
                self.dev_null_status,
                Some(RedirectDevNullStatus::MaybeDevNull)
            )
    }

    pub fn can_expand_to_multiple_fields(self) -> bool {
        self.expansion.can_expand_to_multiple_fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ComparablePathKey {
    Literal(Box<str>),
    Parameter(Box<str>),
    Template(Box<[ComparablePathPart]>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ComparablePathPart {
    Literal(Box<str>),
    Parameter(Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComparablePath {
    span: Span,
    key: ComparablePathKey,
}

impl ComparablePath {
    pub(crate) fn span(&self) -> Span {
        self.span
    }

    pub(crate) fn key(&self) -> &ComparablePathKey {
        &self.key
    }
}

pub(crate) fn comparable_path(
    word: &Word,
    source: &str,
    context: ExpansionContext,
    options: Option<&ZshOptionState>,
) -> Option<ComparablePath> {
    let analysis = analyze_word(word, source, options);
    if analysis.has_command_substitution()
        || analysis.hazards.command_or_process_substitution
        || analysis.has_array_expansion()
    {
        return None;
    }

    let runtime_literal = analyze_literal_runtime(word, source, context, options);
    if runtime_literal.hazards.pathname_matching
        || runtime_literal.hazards.brace_fanout
        || runtime_literal.hazards.command_or_process_substitution
        || runtime_literal.hazards.arithmetic_expansion
        || runtime_literal.hazards.runtime_pattern
    {
        return None;
    }

    let key = comparable_path_key_from_parts(&word.parts, source)?;
    if key == ComparablePathKey::Literal("/dev/null".into()) {
        return None;
    }

    Some(ComparablePath {
        span: word.span,
        key,
    })
}

fn comparable_path_key_from_parts(
    parts: &[WordPartNode],
    source: &str,
) -> Option<ComparablePathKey> {
    let mut components = Vec::new();
    for part in parts {
        push_comparable_path_parts(part, source, &mut components)?;
    }

    match components.as_slice() {
        [ComparablePathPart::Literal(text)] => Some(ComparablePathKey::Literal(text.clone())),
        [ComparablePathPart::Parameter(name)] => Some(ComparablePathKey::Parameter(name.clone())),
        [] => None,
        _ => Some(ComparablePathKey::Template(components.into_boxed_slice())),
    }
}

fn push_comparable_path_parts(
    part: &WordPartNode,
    source: &str,
    components: &mut Vec<ComparablePathPart>,
) -> Option<()> {
    match &part.kind {
        WordPart::Literal(text) => {
            push_comparable_literal(text.as_str(source, part.span), components);
            Some(())
        }
        WordPart::SingleQuoted { value, .. } => {
            push_comparable_literal(value.slice(source), components);
            Some(())
        }
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                push_comparable_path_parts(part, source, components)?;
            }
            Some(())
        }
        WordPart::Variable(name) if is_comparable_parameter_name(name.as_str()) => {
            components.push(ComparablePathPart::Parameter(name.as_str().into()));
            Some(())
        }
        WordPart::Variable(_) => None,
        WordPart::Parameter(parameter) => match parameter.bourne()? {
            BourneParameterExpansion::Access { reference }
                if reference.subscript.is_none()
                    && is_comparable_parameter_name(reference.name.as_str()) =>
            {
                components.push(ComparablePathPart::Parameter(
                    reference.name.as_str().into(),
                ));
                Some(())
            }
            _ => None,
        },
        WordPart::ZshQualifiedGlob(_)
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
        | WordPart::Transformation { .. } => None,
    }
}

fn push_comparable_literal(text: &str, components: &mut Vec<ComparablePathPart>) {
    if text.is_empty() {
        return;
    }

    match components.last_mut() {
        Some(ComparablePathPart::Literal(existing)) => {
            let mut merged = existing.to_string();
            merged.push_str(text);
            *existing = merged.into_boxed_str();
        }
        _ => components.push(ComparablePathPart::Literal(text.into())),
    }
}

fn is_comparable_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionOutputIntent {
    Captured,
    Discarded,
    Rerouted,
    Mixed,
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
    if static_word_text(word, source).is_none() {
        return RuntimeLiteralAnalysis::default();
    }

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

pub(crate) fn analyze_redirect_target(
    redirect: &Redirect,
    source: &str,
    options: Option<&ZshOptionState>,
) -> Option<RedirectTargetAnalysis> {
    let target = redirect.word_target()?;
    let expansion = analyze_word(target, source, options);
    let runtime_literal = analyze_literal_runtime(
        target,
        source,
        ExpansionContext::RedirectTarget(redirect.kind),
        options,
    );

    let (kind, dev_null_status, numeric_descriptor_target) = match redirect.kind {
        RedirectKind::DupOutput | RedirectKind::DupInput => (
            RedirectTargetKind::DescriptorDup,
            None,
            static_word_text(target, source).and_then(|text| text.parse::<i32>().ok()),
        ),
        RedirectKind::HereDoc | RedirectKind::HereDocStrip => return None,
        RedirectKind::HereString
        | RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::OutputBoth => (
            RedirectTargetKind::File,
            Some(
                if static_word_text(target, source).as_deref() == Some("/dev/null") {
                    RedirectDevNullStatus::DefinitelyDevNull
                } else if expansion.is_fixed_literal() && !runtime_literal.is_runtime_sensitive() {
                    RedirectDevNullStatus::DefinitelyNotDevNull
                } else {
                    RedirectDevNullStatus::MaybeDevNull
                },
            ),
            None,
        ),
    };

    Some(RedirectTargetAnalysis {
        kind,
        dev_null_status,
        numeric_descriptor_target,
        expansion,
        runtime_literal,
    })
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
            BourneParameterExpansion::Transformation { .. } => {
                scalar_part(false, ExpansionHazards::default(), false, false)
            }
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

#[cfg(test)]
mod tests {
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
        command.args.clone()
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

        assert_eq!(analyses[5].value_shape, ExpansionValueShape::Scalar);
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
        let source = "cmd foo \"$src\" \"${dst}\" ~/.zshrc \"$dir/Cargo.toml\" $tmpf \"$@\" \"$(printf hi)\" <(cat) *.log /dev/null\n";
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
