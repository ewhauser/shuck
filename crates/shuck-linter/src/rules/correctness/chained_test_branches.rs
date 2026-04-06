use shuck_ast::{Command, ListOperator};

use crate::{Checker, Rule, Violation};

use super::syntax::walk_commands;

pub struct ChainedTestBranches;

impl Violation for ChainedTestBranches {
    fn rule() -> Rule {
        Rule::ChainedTestBranches
    }

    fn message(&self) -> String {
        "chaining `&&` and `||` makes the fallback depend on the middle command status".to_owned()
    }
}

pub fn chained_test_branches(checker: &mut Checker) {
    let mut spans = Vec::new();

    walk_commands(&checker.ast().commands, &mut |command, _| {
        let Command::List(list) = command else {
            return;
        };

        let has_and = list
            .rest
            .iter()
            .any(|(operator, _)| *operator == ListOperator::And);
        let has_or = list
            .rest
            .iter()
            .any(|(operator, _)| *operator == ListOperator::Or);

        if has_and && has_or {
            spans.push(list.span);
        }
    });

    for span in spans {
        checker.report(ChainedTestBranches, span);
    }
}
