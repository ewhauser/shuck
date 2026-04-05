use shuck_ast::Command;

use crate::{Checker, Rule, Violation};

use super::syntax::{simple_test_operands, walk_commands};

pub struct EmptyTest;

impl Violation for EmptyTest {
    fn rule() -> Rule {
        Rule::EmptyTest
    }

    fn message(&self) -> String {
        "test expression is empty".to_owned()
    }
}

pub fn empty_test(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| {
        let Command::Simple(command) = command else {
            return;
        };

        if simple_test_operands(command, source).is_some_and(|operands| operands.is_empty()) {
            spans.push(command.span);
        }
    });

    for span in spans {
        checker.report(EmptyTest, span);
    }
}
