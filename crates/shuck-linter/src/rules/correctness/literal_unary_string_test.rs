use shuck_ast::{Command, CompoundCommand, ConditionalExpr, ConditionalUnaryOp, Word};

use crate::{Checker, Rule, Violation};

use super::syntax::{simple_test_operands, static_word_text, walk_commands};

pub struct LiteralUnaryStringTest;

impl Violation for LiteralUnaryStringTest {
    fn rule() -> Rule {
        Rule::LiteralUnaryStringTest
    }

    fn message(&self) -> String {
        "this string test checks a fixed literal".to_owned()
    }
}

pub fn literal_unary_string_test(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| match command {
        Command::Simple(command) => {
            if simple_test_operands(command, source)
                .is_some_and(|operands| is_literal_unary_simple_test(operands, source))
            {
                spans.push(command.span);
            }
        }
        Command::Compound(CompoundCommand::Conditional(command), _)
            if is_literal_unary_conditional_test(&command.expression, source) =>
        {
            spans.push(command.span);
        }
        _ => {}
    });

    for span in spans {
        checker.report(LiteralUnaryStringTest, span);
    }
}

fn is_literal_unary_simple_test(operands: &[Word], source: &str) -> bool {
    if operands.len() != 2 {
        return false;
    }

    let Some(operator) = static_word_text(&operands[0], source) else {
        return false;
    };

    matches!(operator.as_str(), "-z" | "-n") && static_word_text(&operands[1], source).is_some()
}

fn is_literal_unary_conditional_test(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Unary(expression) => {
            matches!(
                expression.op,
                ConditionalUnaryOp::EmptyString | ConditionalUnaryOp::NonEmptyString
            ) && conditional_literal(expression.expr.as_ref(), source)
        }
        ConditionalExpr::Parenthesized(expression) => {
            is_literal_unary_conditional_test(&expression.expr, source)
        }
        _ => false,
    }
}

fn conditional_literal(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => static_word_text(word, source).is_some(),
        ConditionalExpr::Parenthesized(expression) => conditional_literal(&expression.expr, source),
        _ => false,
    }
}
