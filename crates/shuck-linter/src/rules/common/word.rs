use shuck_ast::{
    ArithmeticExpr, BourneParameterExpansion, Command, CompoundCommand, ConditionalBinaryOp,
    ConditionalExpr, Pattern, PatternPart, Word, WordPart, WordPartNode,
};
use shuck_parser::parser::Parser;

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
    if let [part] = word.parts.as_slice() {
        match &part.kind {
            WordPart::Literal(text) => return Some(text.as_str(source, word.span).to_owned()),
            WordPart::SingleQuoted { value, .. } => return Some(value.slice(source).to_owned()),
            WordPart::DoubleQuoted { parts, .. } => {
                let mut result = String::new();
                return collect_static_word_text(parts, source, &mut result).then_some(result);
            }
            _ => {}
        }
    }

    let mut result = String::new();
    collect_static_word_text(&word.parts, source, &mut result).then_some(result)
}

pub fn is_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

pub fn text_looks_like_nontrivial_arithmetic_expression(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let source = format!("(( {text} ))");
    let file = Parser::new(&source).parse();
    if file.is_err() {
        return false;
    }

    let Some(statement) = file.file.body.first() else {
        return false;
    };

    let Command::Compound(CompoundCommand::Arithmetic(command)) = &statement.command else {
        return false;
    };

    command.expr_ast.as_ref().is_some_and(|expr| {
        !matches!(
            expr.kind,
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_)
        )
    })
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

pub fn word_is_standalone_status_capture(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [part] if is_standalone_status_capture_part(&part.kind))
}

fn is_standalone_status_capture_part(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "?",
        WordPart::DoubleQuoted { parts, .. } => {
            matches!(parts.as_slice(), [part] if is_standalone_status_capture_part(&part.kind))
        }
        WordPart::Parameter(parameter) => matches!(
            parameter.bourne(),
            Some(BourneParameterExpansion::Access { reference })
                if reference.name.as_str() == "?" && reference.subscript.is_none()
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
    use shuck_ast::{BuiltinCommand, Command, CompoundCommand};
    use shuck_parser::parser::Parser;

    use super::{
        ExpansionContext, TestOperandClass, WordExpansionKind, WordLiteralness, WordQuote,
        WordSubstitutionShape, classify_conditional_operand, classify_contextual_operand,
        classify_word, is_shell_variable_name, static_word_text,
        text_looks_like_nontrivial_arithmetic_expression, word_is_standalone_status_capture,
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
    fn static_word_text_keeps_nested_command_names_in_prefixed_quoted_substitutions() {
        let source = "\
echo \"\\\"$BUILDSCRIPT\\\" --library $(test \"${PKG_DIR%/*}\" = \"gpkg\" && echo \"glibc\" || echo \"bionic\")\"\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands[0].command else {
            panic!("expected simple command");
        };
        let Some(body) = command.args[0]
            .parts
            .iter()
            .find_map(|part| match &part.kind {
                shuck_ast::WordPart::DoubleQuoted { parts, .. } => {
                    parts.iter().find_map(|part| match &part.kind {
                        shuck_ast::WordPart::CommandSubstitution { body, .. } => Some(body),
                        _ => None,
                    })
                }
                _ => None,
            })
        else {
            panic!("expected command substitution inside quoted word");
        };

        let Command::Binary(or_chain) = &body[0].command else {
            panic!("expected short-circuit binary command");
        };
        let Command::Binary(and_chain) = &or_chain.left.command else {
            panic!("expected left-hand && chain");
        };
        let Command::Simple(test_command) = &and_chain.left.command else {
            panic!("expected test command");
        };
        let Command::Simple(then_echo) = &and_chain.right.command else {
            panic!("expected then echo");
        };
        let Command::Simple(else_echo) = &or_chain.right.command else {
            panic!("expected else echo");
        };

        assert_eq!(
            static_word_text(&test_command.name, source).as_deref(),
            Some("test")
        );
        assert_eq!(
            static_word_text(&then_echo.name, source).as_deref(),
            Some("echo")
        );
        assert_eq!(
            static_word_text(&else_echo.name, source).as_deref(),
            Some("echo")
        );
    }

    #[test]
    fn shell_variable_name_helper_matches_identifier_rules() {
        assert!(is_shell_variable_name("name"));
        assert!(is_shell_variable_name("_name123"));
        assert!(!is_shell_variable_name("1name"));
        assert!(!is_shell_variable_name("name-value"));
    }

    #[test]
    fn arithmetic_text_helper_requires_nontrivial_expressions() {
        assert!(text_looks_like_nontrivial_arithmetic_expression("1 + 2"));
        assert!(text_looks_like_nontrivial_arithmetic_expression("arr[1]"));
        assert!(text_looks_like_nontrivial_arithmetic_expression("++count"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression("123"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression("name"));
        assert!(!text_looks_like_nontrivial_arithmetic_expression(
            "latest value"
        ));
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
    fn word_is_standalone_status_capture_handles_plain_and_quoted_forms() {
        let source = "return $?; return \"$?\"; return ${?+0}; return ${?:-1}; return $foo\n";
        let commands = parse_commands(source);

        let Command::Builtin(BuiltinCommand::Return(plain)) = &commands[0].command else {
            panic!("expected return builtin");
        };
        assert!(word_is_standalone_status_capture(
            plain.code.as_ref().unwrap()
        ));

        let Command::Builtin(BuiltinCommand::Return(quoted)) = &commands[1].command else {
            panic!("expected return builtin");
        };
        assert!(word_is_standalone_status_capture(
            quoted.code.as_ref().unwrap()
        ));

        let Command::Builtin(BuiltinCommand::Return(operator_default)) = &commands[2].command
        else {
            panic!("expected return builtin");
        };
        assert!(!word_is_standalone_status_capture(
            operator_default.code.as_ref().unwrap()
        ));

        let Command::Builtin(BuiltinCommand::Return(operator_assign)) = &commands[3].command else {
            panic!("expected return builtin");
        };
        assert!(!word_is_standalone_status_capture(
            operator_assign.code.as_ref().unwrap()
        ));

        let Command::Builtin(BuiltinCommand::Return(other)) = &commands[4].command else {
            panic!("expected return builtin");
        };
        assert!(!word_is_standalone_status_capture(
            other.code.as_ref().unwrap()
        ));
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
