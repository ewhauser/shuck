use shuck_ast::{
    ConditionalBinaryOp, ConditionalExpr, Pattern, PatternPart, Word, WordPart, WordPartNode,
};

pub use super::expansion::{
    ExpansionContext, WordExpansionKind, WordLiteralness, WordQuote, WordSubstitutionShape,
};

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

pub fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_word_text(&word.parts, source, &mut result).then_some(result)
}

pub fn conditional_binary_op_is_string_match(op: ConditionalBinaryOp) -> bool {
    matches!(
        op,
        ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe
    )
}

pub fn word_is_standalone_variable_like(word: &Word) -> bool {
    match word.parts.as_slice() {
        [part] => matches!(
            part.kind,
            WordPart::Variable(_)
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
        ),
        _ => false,
    }
}

pub(crate) fn classify_word(word: &Word, source: &str) -> WordClassification {
    let analysis = super::expansion::analyze_word(word, source, None);

    WordClassification {
        quote: analysis.quote,
        literalness: analysis.literalness,
        expansion_kind: analysis.expansion_kind(),
        substitution_shape: analysis.substitution_shape,
    }
}

pub(crate) fn classify_contextual_operand(
    word: &Word,
    source: &str,
    context: ExpansionContext,
) -> TestOperandClass {
    let analysis = super::expansion::analyze_word(word, source, None);
    if analysis.literalness == WordLiteralness::Expanded {
        return TestOperandClass::RuntimeSensitive;
    }

    if super::expansion::analyze_literal_runtime(word, source, context, None).is_runtime_sensitive()
    {
        TestOperandClass::RuntimeSensitive
    } else {
        TestOperandClass::FixedLiteral
    }
}

fn collect_static_word_text(parts: &[WordPartNode], source: &str, out: &mut String) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => out.push_str(text.as_str(source, part.span)),
            WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(source)),
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_static_word_text(parts, source, out) {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
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

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, CompoundCommand};
    use shuck_parser::parser::Parser;

    use super::{
        ExpansionContext, TestOperandClass, WordExpansionKind, WordLiteralness, WordQuote,
        WordSubstitutionShape, classify_conditional_operand, classify_contextual_operand,
        classify_word,
    };

    fn parse_commands(source: &str) -> shuck_ast::StmtSeq {
        Parser::new(source).parse().unwrap().file.body
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
