use shuck_ast::{Command, ListOperator};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

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

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
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
        },
    );

    for span in spans {
        checker.report(ChainedTestBranches, span);
    }
}
