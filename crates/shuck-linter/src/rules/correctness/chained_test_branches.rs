use shuck_ast::{Command, ListOperator};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::span;
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
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::List(list) = command else {
                return;
            };

            let mut current = None;
            for item in &list.rest {
                if !matches!(item.operator, ListOperator::And | ListOperator::Or) {
                    current = None;
                    continue;
                }

                match current {
                    None => current = Some(item.operator),
                    Some(previous) if previous == item.operator => {}
                    Some(_) => {
                        checker
                            .report_dedup(ChainedTestBranches, span::list_item_operator_span(item));
                        break;
                    }
                }
            }
        },
    );
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
