use shuck_ast::{Command, CompoundCommand, ConditionalBinaryOp, ConditionalExpr, Word};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::{
    classify_conditional_operand, classify_test_operand, static_word_text,
};
use crate::{Checker, Rule, Violation};

use super::syntax::simple_test_operands;

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

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| match command {
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
        },
    );

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
        && classify_test_operand(&operands[0], source).is_fixed_literal()
        && classify_test_operand(&operands[2], source).is_fixed_literal()
}

fn is_constant_conditional_test(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Binary(expression) => {
            is_comparison_binary_op(expression.op)
                && classify_conditional_operand(expression.left.as_ref(), source).is_fixed_literal()
                && classify_conditional_operand(expression.right.as_ref(), source)
                    .is_fixed_literal()
        }
        ConditionalExpr::Parenthesized(expression) => {
            is_constant_conditional_test(&expression.expr, source)
        }
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
