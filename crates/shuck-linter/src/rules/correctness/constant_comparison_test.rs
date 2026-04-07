use shuck_ast::{
    Command, CompoundCommand, ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, Word,
};

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
        checker.source(),
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
    match operands {
        [operator, operand] => static_word_text(operator, source).is_some_and(|operator| {
            is_unary_test_operator(&operator)
                && classify_test_operand(operand, source).is_fixed_literal()
        }),
        [left, operator, right] => static_word_text(operator, source).is_some_and(|operator| {
            is_string_binary_test_operator(&operator)
                && classify_test_operand(left, source).is_fixed_literal()
                && classify_test_operand(right, source).is_fixed_literal()
        }),
        _ => false,
    }
}

fn is_constant_conditional_test(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Binary(expression) => {
            is_string_comparison_binary_op(expression.op)
                && classify_conditional_operand(expression.left.as_ref(), source).is_fixed_literal()
                && classify_conditional_operand(expression.right.as_ref(), source)
                    .is_fixed_literal()
        }
        ConditionalExpr::Unary(expression) => {
            is_unary_string_test_operator(expression.op)
                && classify_conditional_operand(expression.expr.as_ref(), source).is_fixed_literal()
        }
        ConditionalExpr::Parenthesized(expression) => {
            is_constant_conditional_test(&expression.expr, source)
        }
        _ => false,
    }
}

fn is_unary_test_operator(operator: &str) -> bool {
    matches!(operator, "-n" | "-z")
}

fn is_string_binary_test_operator(operator: &str) -> bool {
    matches!(operator, "=" | "==" | "!=" | "<" | ">")
}

fn is_string_comparison_binary_op(operator: ConditionalBinaryOp) -> bool {
    !matches!(
        operator,
        ConditionalBinaryOp::And
            | ConditionalBinaryOp::Or
            | ConditionalBinaryOp::RegexMatch
            | ConditionalBinaryOp::NewerThan
            | ConditionalBinaryOp::OlderThan
            | ConditionalBinaryOp::SameFile
            | ConditionalBinaryOp::ArithmeticEq
            | ConditionalBinaryOp::ArithmeticNe
            | ConditionalBinaryOp::ArithmeticLe
            | ConditionalBinaryOp::ArithmeticGe
            | ConditionalBinaryOp::ArithmeticLt
            | ConditionalBinaryOp::ArithmeticGt
    )
}

fn is_unary_string_test_operator(operator: ConditionalUnaryOp) -> bool {
    matches!(
        operator,
        ConditionalUnaryOp::EmptyString | ConditionalUnaryOp::NonEmptyString
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_runtime_sensitive_and_non_string_comparisons() {
        let source = "\
#!/bin/bash
[ ~ = /tmp ]
[ *.sh = target ]
[ {a,b} = foo ]
[[ i -ge 10 ]]
[ \"/a\" -ot \"/b\" ]
[[ left == *.sh ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_constant_unary_string_tests() {
        let source = "\
#!/bin/bash
[ -n foo ]
[[ -z bar ]]
[ -n ~ ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
