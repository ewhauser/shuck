use shuck_ast::{RedirectKind, Span, SubscriptSelector, Word, WordPart, WordPartNode};

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
    use shuck_ast::Command;
    use shuck_parser::parser::Parser;

    use super::{ExpansionAnalysis, ExpansionValueShape, WordLiteralness, WordQuote, analyze_word};

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
}
