use crate::rules::common::expansion::analyze_redirect_target;
use crate::rules::common::query::{self, CommandWalkOptions, visit_command_redirects};
use crate::{Checker, Rule, Violation};

pub struct ArithmeticRedirectionTarget;

impl Violation for ArithmeticRedirectionTarget {
    fn rule() -> Rule {
        Rule::ArithmeticRedirectionTarget
    }

    fn message(&self) -> String {
        "redirection targets should not use arithmetic expansion".to_owned()
    }
}

pub fn arithmetic_redirection_target(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let _command = visit.command;
            visit_command_redirects(visit, &mut |redirect| {
                let Some(target) = redirect.word_target() else {
                    return;
                };

                if analyze_redirect_target(redirect, checker.source())
                    .is_some_and(|analysis| analysis.expansion.hazards.arithmetic_expansion)
                {
                    spans.push(target.span);
                }
            });
        },
    );

    for span in spans {
        checker.report(ArithmeticRedirectionTarget, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_redirect_targets_with_arithmetic_expansion() {
        let source = "\
#!/bin/bash
echo hi > \"$((i++))\"
echo hi > \"$target\"
echo hi > out.txt
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArithmeticRedirectionTarget),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }
}
