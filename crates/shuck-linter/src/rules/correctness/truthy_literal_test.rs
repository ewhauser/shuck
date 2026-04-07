use shuck_ast::{Command, CompoundCommand, ConditionalExpr};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::{
    ExpansionContext, classify_conditional_operand, classify_contextual_operand,
};
use crate::{Checker, Rule, Violation};

use super::syntax::simple_test_operands;

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
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| match command {
            Command::Simple(command) => {
                if simple_test_operands(command, source).is_some_and(|operands| {
                    operands.len() == 1
                        && classify_contextual_operand(
                            &operands[0],
                            source,
                            ExpansionContext::CommandArgument,
                        )
                        .is_fixed_literal()
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
        ConditionalExpr::Word(_) => {
            classify_conditional_operand(expression, source).is_fixed_literal()
        }
        ConditionalExpr::Parenthesized(expression) => {
            is_truthy_literal_conditional(&expression.expr, source)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_runtime_sensitive_literal_words() {
        let source = "\
#!/bin/bash
[ ~ ]
test ~user
test x=~
test *.sh
[ {a,b} ]
[[ ~ ]]
[[ *.sh ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8]
        );
    }

    #[test]
    fn still_reports_plain_fixed_literals() {
        let source = "\
#!/bin/bash
[ 1 ]
test foo
[[ bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }
}
