use shuck_ast::{Command, CompoundCommand, ConditionalExpr};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

use super::syntax::{simple_test_operands, static_word_text};

pub struct TruthyLiteralTest;

impl Violation for TruthyLiteralTest {
    fn rule() -> Rule {
        Rule::TruthyLiteralTest
    }

    fn message(&self) -> String {
        "this test checks a fixed literal instead of runtime data".to_owned()
    }
}

pub fn truthy_literal_test(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| match command {
            Command::Simple(command) => {
                if simple_test_operands(command, source).is_some_and(|operands| {
                    operands.len() == 1 && static_word_text(&operands[0], source).is_some()
                }) {
                    spans.push(command.span);
                }
            }
            Command::Compound(CompoundCommand::Conditional(command), _)
                if is_truthy_literal_conditional(&command.expression, source) =>
            {
                spans.push(command.span);
            }
            _ => {}
        },
    );

    for span in spans {
        checker.report(TruthyLiteralTest, span);
    }
}

fn is_truthy_literal_conditional(expression: &ConditionalExpr, source: &str) -> bool {
    match expression {
        ConditionalExpr::Word(word) => static_word_text(word, source).is_some(),
        ConditionalExpr::Parenthesized(expression) => {
            is_truthy_literal_conditional(&expression.expr, source)
        }
        _ => false,
    }
}
