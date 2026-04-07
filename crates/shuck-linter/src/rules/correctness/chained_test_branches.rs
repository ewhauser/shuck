use shuck_ast::{BinaryOp, Command};

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
    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            if let Some(span) = mixed_short_circuit_operator_span(command) {
                checker.report_dedup(ChainedTestBranches, span);
            }
        },
    );
}

fn mixed_short_circuit_operator_span(command: &Command) -> Option<shuck_ast::Span> {
    let mut operators = Vec::new();
    collect_short_circuit_operators(command, &mut operators);
    let mut current = None;
    for (operator, span) in operators {
        match current {
            None => current = Some(operator),
            Some(previous) if previous == operator => {}
            Some(_) => return Some(span),
        }
    }
    None
}

fn collect_short_circuit_operators(command: &Command, operators: &mut Vec<(BinaryOp, shuck_ast::Span)>) {
    let Command::Binary(command) = command else {
        return;
    };
    if !matches!(command.op, BinaryOp::And | BinaryOp::Or) {
        return;
    }

    collect_short_circuit_operators(&command.left.command, operators);
    operators.push((command.op, command.op_span));
    collect_short_circuit_operators(&command.right.command, operators);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_operator_that_introduces_mixed_short_circuiting() {
        let source = "\
true && false || printf '%s\\n' fallback
false || true && printf '%s\\n' fallback
true && false; false || printf '%s\\n' ok
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["||", "&&"]
        );
    }
}
