use shuck_ast::{Command, CompoundCommand, ConditionalBinaryOp, ConditionalExpr, Word};

use crate::{Checker, Rule, Violation};

use super::syntax::{simple_test_operands, static_word_text, walk_commands};

pub struct ConstantComparisonTest;

impl Violation for ConstantComparisonTest {
    fn rule() -> Rule {
        Rule::ConstantComparisonTest
    }

    fn message(&self) -> String {
        "this comparison only checks fixed literals".to_owned()
    }
}

pub fn constant_comparison_test(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| match command {
        Command::Simple(command) => {
            if simple_test_operands(command, source)
                .is_some_and(|operands| is_constant_simple_test(operands, source))
            {
                spans.push(command.span);
            }
        }
        Command::Compound(CompoundCommand::Conditional(command), _)
            if is_constant_conditional_test(&command.expression, source) =>
        {
            spans.push(command.span);
        }
        _ => {}
    });

    for span in spans {
        checker.report(ConstantComparisonTest, span);
    }
}

fn is_constant_simple_test(operands: &[Word], source: &str) -> bool {
    if operands.len() != 3 {
        return false;
    }

    let Some(operator) = static_word_text(&operands[1], source) else {
        return false;
    };

    is_binary_test_operator(&operator)
        && static_word_text(&operands[0], source).is_some()
        && static_word_text(&operands[2], source).is_some()
}

fn is_constant_conditional_test(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Binary(expression) => {
            is_comparison_binary_op(expression.op)
                && conditional_literal(expression.left.as_ref(), source)
                && conditional_literal(expression.right.as_ref(), source)
        }
        ConditionalExpr::Parenthesized(expression) => {
            is_constant_conditional_test(&expression.expr, source)
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

fn is_binary_test_operator(operator: &str) -> bool {
    matches!(
        operator,
        "=" | "=="
            | "!="
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
            | "<"
            | ">"
    )
}

fn is_comparison_binary_op(operator: ConditionalBinaryOp) -> bool {
    !matches!(
        operator,
        ConditionalBinaryOp::And | ConditionalBinaryOp::Or | ConditionalBinaryOp::RegexMatch
    )
}
