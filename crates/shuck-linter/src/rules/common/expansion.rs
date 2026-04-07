use std::collections::HashMap;

use shuck_ast::{
    Redirect, RedirectKind, SourceText, Span, SubscriptSelector, Word, WordPart, WordPartNode,
};
use shuck_parser::parser::Parser;

use super::query::{self, CommandSubstitutionKind, CommandWalkOptions, NestedCommandSubstitution};
use super::span;
use super::word::static_word_text;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExpansionContext {
    CommandArgument,
    AssignmentValue,
    RedirectTarget(RedirectKind),
    DescriptorDupTarget(RedirectKind),
    HereString,
    ForList,
    SelectList,
    CasePattern,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTextExpansionAnalysis {
    pub expansion_spans: Vec<Span>,
}

impl SourceTextExpansionAnalysis {
    pub fn is_expanded(&self) -> bool {
        !self.expansion_spans.is_empty()
    }
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
            || matches!(
                self.dev_null_status,
                Some(RedirectDevNullStatus::MaybeDevNull)
            )
    }

    pub fn can_expand_to_multiple_fields(self) -> bool {
        self.expansion.can_expand_to_multiple_fields
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionOutputIntent {
    Captured,
    Discarded,
    Rerouted,
    Mixed,
}

impl SubstitutionOutputIntent {
    fn merge(self, other: Self) -> Self {
        if self == other { self } else { Self::Mixed }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstitutionClassification {
    pub kind: CommandSubstitutionKind,
    pub span: Span,
    pub stdout_intent: SubstitutionOutputIntent,
    pub has_stdout_redirect: bool,
}

impl SubstitutionClassification {
    pub fn stdout_is_captured(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Captured
    }

    pub fn stdout_is_discarded(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Discarded
    }

    pub fn stdout_is_rerouted(self) -> bool {
        self.stdout_intent == SubstitutionOutputIntent::Rerouted
    }
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

pub fn analyze_word(word: &Word, source: &str) -> ExpansionAnalysis {
    let mut summary = AnalysisSummary::default();
    analyze_parts(&word.parts, source, false, &mut summary);

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

pub fn analyze_source_text_operand(text: &SourceText, source: &str) -> SourceTextExpansionAnalysis {
    debug_assert!(text.is_source_backed());
    let source_text = text.slice(source);
    let parsed = Parser::parse_word_string(source_text);
    let base = text.span().start;
    let expansion_spans = span::expansion_part_spans(&parsed)
        .into_iter()
        .filter(|span| source_text_span_is_active(source_text, *span))
        .map(|span| span.rebased(base))
        .collect();

    SourceTextExpansionAnalysis { expansion_spans }
}

pub fn analyze_literal_runtime(
    word: &Word,
    source: &str,
    context: ExpansionContext,
) -> RuntimeLiteralAnalysis {
    if static_word_text(word, source).is_none() {
        return RuntimeLiteralAnalysis::default();
    }

    let mut analysis = RuntimeLiteralAnalysis::default();
    let mut state = RuntimeLiteralState::default();

    analyze_literal_runtime_parts(&word.parts, source, context, &mut state, &mut analysis);

    analysis
}

pub fn analyze_redirect_target(
    redirect: &Redirect,
    source: &str,
) -> Option<RedirectTargetAnalysis> {
    let target = redirect.word_target()?;
    let expansion = analyze_word(target, source);

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
                } else if expansion.is_fixed_literal() {
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
    })
}

pub fn classify_substitution(
    substitution: NestedCommandSubstitution<'_>,
    source: &str,
) -> SubstitutionClassification {
    let mut stdout_intent: Option<SubstitutionOutputIntent> = None;
    let mut has_stdout_redirect = false;

    query::walk_commands(
        substitution.commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            let state = classify_command_redirects(query::command_redirects(command), source);
            has_stdout_redirect |= state.has_stdout_redirect;
            stdout_intent = Some(match stdout_intent {
                Some(current) => current.merge(state.stdout_intent),
                None => state.stdout_intent,
            });
        },
    );

    SubstitutionClassification {
        kind: substitution.kind,
        span: substitution.span,
        stdout_intent: stdout_intent.unwrap_or(SubstitutionOutputIntent::Captured),
        has_stdout_redirect,
    }
}

fn source_text_span_is_active(source: &str, span: Span) -> bool {
    if !span.slice(source).starts_with('$') {
        return true;
    }

    let mut backslashes = 0usize;
    let bytes = source.as_bytes();
    let mut offset = span.start.offset;

    while offset > 0 && bytes[offset - 1] == b'\\' {
        backslashes += 1;
        offset -= 1;
    }

    backslashes.is_multiple_of(2)
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
    state: &mut RuntimeLiteralState,
    analysis: &mut RuntimeLiteralAnalysis,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {
                scan_literal_runtime_text(part.span.slice(source), context, state, analysis);
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

        if context_allows_pathname_matching(context) && matches!(ch, '*' | '?' | '[') {
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
        ExpansionContext::CommandArgument
            | ExpansionContext::AssignmentValue
            | ExpansionContext::StringTestOperand
            | ExpansionContext::RegexOperand
            | ExpansionContext::RedirectTarget(_)
    )
}

fn context_allows_pathname_matching(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandArgument | ExpansionContext::RedirectTarget(_)
    )
}

fn context_allows_brace_fanout(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandArgument
            | ExpansionContext::AssignmentValue
            | ExpansionContext::RedirectTarget(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputSink {
    Captured,
    DevNull,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedirectState {
    stdout_intent: SubstitutionOutputIntent,
    has_stdout_redirect: bool,
}

fn classify_command_redirects(redirects: &[Redirect], source: &str) -> RedirectState {
    let mut fds = HashMap::from([(1, OutputSink::Captured), (2, OutputSink::Other)]);
    let mut has_stdout_redirect = false;

    for redirect in redirects {
        match redirect.kind {
            RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append => {
                let sink = redirect_file_sink(redirect, source);
                let fd = redirect.fd.unwrap_or(1);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::OutputBoth => {
                let sink = redirect_file_sink(redirect, source);
                has_stdout_redirect = true;
                fds.insert(1, sink);
                fds.insert(2, sink);
            }
            RedirectKind::DupOutput => {
                let fd = redirect.fd.unwrap_or(1);
                let sink = redirect_dup_output_sink(redirect, &fds, source);
                has_stdout_redirect |= fd == 1;
                fds.insert(fd, sink);
            }
            RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => {}
        }
    }

    let stdout_sink = *fds.get(&1).unwrap_or(&OutputSink::Other);
    let stderr_sink = *fds.get(&2).unwrap_or(&OutputSink::Other);
    let stdout_intent = if matches!(stdout_sink, OutputSink::Captured)
        || matches!(stderr_sink, OutputSink::Captured)
    {
        SubstitutionOutputIntent::Captured
    } else if matches!(stdout_sink, OutputSink::DevNull) {
        SubstitutionOutputIntent::Discarded
    } else {
        SubstitutionOutputIntent::Rerouted
    };

    RedirectState {
        stdout_intent,
        has_stdout_redirect,
    }
}

fn redirect_file_sink(redirect: &Redirect, source: &str) -> OutputSink {
    match analyze_redirect_target(redirect, source) {
        Some(analysis) if analysis.is_definitely_dev_null() => OutputSink::DevNull,
        Some(_) => OutputSink::Other,
        None => OutputSink::Other,
    }
}

fn redirect_dup_output_sink(
    redirect: &Redirect,
    fds: &HashMap<i32, OutputSink>,
    source: &str,
) -> OutputSink {
    let Some(analysis) = analyze_redirect_target(redirect, source) else {
        return OutputSink::Other;
    };

    let Some(fd) = analysis.numeric_descriptor_target else {
        return OutputSink::Other;
    };

    *fds.get(&fd).unwrap_or(&OutputSink::Other)
}

fn analyze_parts(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
    summary: &mut AnalysisSummary,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                analyze_parts(parts, source, true, summary);
            }
            kind => {
                let analysis = analyze_part(kind, part.span, source, in_double_quotes);
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

fn analyze_part(part: &WordPart, span: Span, source: &str, in_double_quotes: bool) -> PartAnalysis {
    match part {
        WordPart::CommandSubstitution { .. } => scalar_part(
            !in_double_quotes,
            ExpansionHazards {
                field_splitting: !in_double_quotes,
                pathname_matching: !in_double_quotes,
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
        WordPart::Variable(_)
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayLength(_)
        | WordPart::Substring { .. } => scalar_part(
            !in_double_quotes,
            ExpansionHazards {
                field_splitting: !in_double_quotes,
                pathname_matching: !in_double_quotes,
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
            !in_double_quotes,
            ExpansionHazards {
                field_splitting: !in_double_quotes,
                pathname_matching: !in_double_quotes,
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
                !in_double_quotes,
                ExpansionHazards {
                    field_splitting: !in_double_quotes,
                    pathname_matching: !in_double_quotes,
                    ..ExpansionHazards::default()
                },
                false,
                false,
            ),
        },
        WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_) => {
            array_part(true, false, false, false)
        }
        WordPart::PrefixMatch(_) => {
            let multi_field =
                prefix_match_can_expand_to_multiple_fields(span, source, in_double_quotes);
            PartAnalysis {
                value_shape: if multi_field {
                    PartValueShape::Scalar
                } else {
                    PartValueShape::Unknown
                },
                array_valued: false,
                can_expand_to_multiple_fields: multi_field,
                hazards: ExpansionHazards {
                    field_splitting: !in_double_quotes,
                    pathname_matching: !in_double_quotes,
                    ..ExpansionHazards::default()
                },
                command_substitution: false,
                process_substitution: false,
            }
        }
        WordPart::IndirectExpansion { operator, .. } => PartAnalysis {
            value_shape: PartValueShape::Unknown,
            array_valued: false,
            can_expand_to_multiple_fields: !in_double_quotes,
            hazards: ExpansionHazards {
                field_splitting: !in_double_quotes,
                pathname_matching: !in_double_quotes,
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

fn prefix_match_can_expand_to_multiple_fields(
    span: Span,
    source: &str,
    in_double_quotes: bool,
) -> bool {
    let text = span.slice(source);
    text.ends_with("@}") || !in_double_quotes
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
    use shuck_ast::{Command, ParameterOp, SourceText, WordPart};
    use shuck_parser::parser::Parser;

    use super::{
        ExpansionAnalysis, ExpansionContext, ExpansionValueShape, RedirectDevNullStatus,
        SubstitutionOutputIntent, WordLiteralness, WordQuote, analyze_literal_runtime,
        analyze_redirect_target, analyze_source_text_operand, analyze_word, classify_substitution,
    };
    use crate::rules::common::query::iter_word_command_substitutions;

    fn parse_argument_words(source: &str) -> Vec<shuck_ast::Word> {
        let commands = Parser::new(source).parse().unwrap().script.commands;
        let Command::Simple(command) = &commands[0] else {
            panic!("expected simple command");
        };
        command.args.clone()
    }

    fn analyze_argument_words(source: &str) -> Vec<ExpansionAnalysis> {
        parse_argument_words(source)
            .iter()
            .map(|word| analyze_word(word, source))
            .collect()
    }

    fn parse_commands(source: &str) -> Vec<Command> {
        Parser::new(source).parse().unwrap().script.commands
    }

    fn assignment_scalar_word(command: &Command) -> &shuck_ast::Word {
        let Command::Simple(command) = command else {
            panic!("expected simple command");
        };
        let shuck_ast::AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        word
    }

    fn first_parameter_operand(word: &shuck_ast::Word) -> SourceText {
        let [part] = word.parts.as_slice() else {
            panic!("expected single-part word");
        };
        let WordPart::ParameterExpansion {
            operand: Some(operand),
            ..
        } = &part.kind
        else {
            panic!("expected parameter expansion operand");
        };
        operand.clone()
    }

    fn first_replacement_text(word: &shuck_ast::Word) -> SourceText {
        let [part] = word.parts.as_slice() else {
            panic!("expected single-part word");
        };
        let WordPart::ParameterExpansion { operator, .. } = &part.kind else {
            panic!("expected parameter expansion");
        };
        match operator {
            ParameterOp::ReplaceFirst { replacement, .. }
            | ParameterOp::ReplaceAll { replacement, .. } => replacement.clone(),
            _ => panic!("expected replacement operator"),
        }
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
    fn analyze_source_text_operand_respects_escaping_and_nested_expansions() {
        let source = "\
escaped=${value:-\\$keep}
single=${value:-'$single'}
quoted=${value:-\"$quoted\"}
nested=${value:-$(date)}
mixed=${value:-prefix${name}suffix}
replaced=${value/pat/prefix$replacement}
";
        let commands = parse_commands(source);

        let escaped_span = first_parameter_operand(assignment_scalar_word(&commands[0])).span();
        assert_eq!(escaped_span.slice(source), "\\$keep");
        let escaped = analyze_source_text_operand(&SourceText::source(escaped_span), source);
        assert!(!escaped.is_expanded());

        let single_operand = first_parameter_operand(assignment_scalar_word(&commands[1]));
        assert!(single_operand.is_source_backed());
        assert_eq!(single_operand.slice(source), "'$single'");
        let single = analyze_source_text_operand(&single_operand, source);
        assert!(!single.is_expanded());

        let quoted_operand = first_parameter_operand(assignment_scalar_word(&commands[2]));
        assert!(quoted_operand.is_source_backed());
        assert_eq!(quoted_operand.slice(source), "\"$quoted\"");
        let quoted = analyze_source_text_operand(&quoted_operand, source);
        assert_eq!(
            quoted
                .expansion_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$quoted"]
        );

        let nested_operand = first_parameter_operand(assignment_scalar_word(&commands[3]));
        assert!(nested_operand.is_source_backed());
        assert_eq!(nested_operand.slice(source), "$(date)");
        let nested = analyze_source_text_operand(&nested_operand, source);
        assert_eq!(
            nested
                .expansion_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(date)"]
        );

        let mixed_operand = first_parameter_operand(assignment_scalar_word(&commands[4]));
        assert!(mixed_operand.is_source_backed());
        assert_eq!(mixed_operand.slice(source), "prefix${name}suffix");
        let mixed = analyze_source_text_operand(&mixed_operand, source);
        assert_eq!(
            mixed
                .expansion_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}"]
        );

        let replacement_text = first_replacement_text(assignment_scalar_word(&commands[5]));
        assert!(replacement_text.is_source_backed());
        assert_eq!(replacement_text.slice(source), "prefix$replacement");
        let replacement = analyze_source_text_operand(&replacement_text, source);
        assert_eq!(
            replacement
                .expansion_spans
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$replacement"]
        );
    }

    #[test]
    fn analyze_redirect_target_distinguishes_descriptor_dups_and_dev_null() {
        let static_dup_source = "echo hi 2>&3\n";
        let static_dup_command = Parser::new(static_dup_source)
            .parse()
            .unwrap()
            .script
            .commands;
        let Command::Simple(static_dup_simple) = &static_dup_command[0] else {
            panic!("expected simple command");
        };
        let static_dup =
            analyze_redirect_target(&static_dup_simple.redirects[0], static_dup_source)
                .expect("expected redirect analysis");
        assert!(static_dup.is_descriptor_dup());
        assert_eq!(static_dup.numeric_descriptor_target, Some(3));
        assert!(!static_dup.is_runtime_sensitive());

        let file_source = "echo hi > /dev/null\n";
        let file_command = Parser::new(file_source).parse().unwrap().script.commands;
        let Command::Simple(file_simple) = &file_command[0] else {
            panic!("expected simple command");
        };
        let file = analyze_redirect_target(&file_simple.redirects[0], file_source)
            .expect("expected redirect analysis");
        assert!(file.is_file_target());
        assert!(file.is_definitely_dev_null());
        assert!(!file.is_runtime_sensitive());

        let maybe_source = "echo hi > \"$target\"\n";
        let maybe_command = Parser::new(maybe_source).parse().unwrap().script.commands;
        let Command::Simple(maybe_simple) = &maybe_command[0] else {
            panic!("expected simple command");
        };
        let maybe = analyze_redirect_target(&maybe_simple.redirects[0], maybe_source)
            .expect("expected redirect analysis");
        assert!(maybe.is_file_target());
        assert_eq!(
            maybe.dev_null_status,
            Some(RedirectDevNullStatus::MaybeDevNull)
        );
        assert!(maybe.is_runtime_sensitive());

        let fanout_source = "echo hi > ${targets[@]}\n";
        let fanout_command = Parser::new(fanout_source).parse().unwrap().script.commands;
        let Command::Simple(fanout_simple) = &fanout_command[0] else {
            panic!("expected simple command");
        };
        let fanout = analyze_redirect_target(&fanout_simple.redirects[0], fanout_source)
            .expect("expected redirect analysis");
        assert!(fanout.can_expand_to_multiple_fields());
        assert!(fanout.is_runtime_sensitive());
    }

    #[test]
    fn analyze_literal_runtime_tracks_context_sensitive_literals() {
        let source = "printf ~ ~user x=~ *.sh {a,b} \"~\" '*.sh' \"{a,b}\"\n";
        let words = parse_argument_words(source);

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::CommandArgument)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[1], source, ExpansionContext::CommandArgument)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[2], source, ExpansionContext::CommandArgument)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[3], source, ExpansionContext::CommandArgument)
                .hazards
                .pathname_matching
        );
        assert!(
            analyze_literal_runtime(&words[4], source, ExpansionContext::CommandArgument)
                .hazards
                .brace_fanout
        );

        assert!(
            !analyze_literal_runtime(&words[5], source, ExpansionContext::CommandArgument)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[6], source, ExpansionContext::CommandArgument)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[7], source, ExpansionContext::CommandArgument)
                .is_runtime_sensitive()
        );

        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::StringTestOperand)
                .hazards
                .tilde_expansion
        );
        assert!(
            analyze_literal_runtime(&words[0], source, ExpansionContext::RegexOperand)
                .hazards
                .tilde_expansion
        );
        assert!(
            !analyze_literal_runtime(&words[3], source, ExpansionContext::StringTestOperand)
                .is_runtime_sensitive()
        );
        assert!(
            !analyze_literal_runtime(&words[4], source, ExpansionContext::CasePattern)
                .is_runtime_sensitive()
        );
    }

    #[test]
    fn classify_substitution_reports_stdout_intent_and_redirects() {
        let cases = [
            (
                "out=$(printf hi)\n",
                SubstitutionOutputIntent::Captured,
                false,
            ),
            (
                "out=$(printf hi > out.txt)\n",
                SubstitutionOutputIntent::Rerouted,
                true,
            ),
            (
                "out=$(printf hi >/dev/null 2>&1)\n",
                SubstitutionOutputIntent::Discarded,
                true,
            ),
            (
                "out=$(whiptail 3>&1 1>&2 2>&3)\n",
                SubstitutionOutputIntent::Captured,
                true,
            ),
            (
                "out=$(jq -r . <<< \"$status\" || die >&2)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(awk 'BEGIN { print \"ok\" }' || warn >&2)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(getopt -o a -- \"$@\" || { usage >&2 && false; })\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)\n",
                SubstitutionOutputIntent::Captured,
                true,
            ),
            (
                "out=$(cat <<'EOF'\nhello\nEOF\n)\n",
                SubstitutionOutputIntent::Captured,
                false,
            ),
            (
                "out=$(printf quiet >/dev/null; printf loud)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(printf quiet >/dev/null; printf loud > out.txt)\n",
                SubstitutionOutputIntent::Mixed,
                true,
            ),
            (
                "out=$(printf hi > \"$target\")\n",
                SubstitutionOutputIntent::Rerouted,
                true,
            ),
            (
                "out=$(printf hi > ${targets[@]})\n",
                SubstitutionOutputIntent::Rerouted,
                true,
            ),
            (
                "out=$(printf hi 2>&\"$fd\")\n",
                SubstitutionOutputIntent::Captured,
                false,
            ),
        ];

        for (source, expected_intent, expected_redirect) in cases {
            let commands = Parser::new(source).parse().unwrap().script.commands;
            let Command::Simple(command) = &commands[0] else {
                panic!("expected simple command");
            };
            let substitution =
                iter_word_command_substitutions(match &command.assignments[0].value {
                    shuck_ast::AssignmentValue::Scalar(word) => word,
                    shuck_ast::AssignmentValue::Compound(_) => panic!("expected scalar assignment"),
                })
                .next()
                .expect("expected command substitution");

            let classification = classify_substitution(substitution, source);
            assert_eq!(classification.stdout_intent, expected_intent);
            assert_eq!(classification.has_stdout_redirect, expected_redirect);
        }
    }
}
