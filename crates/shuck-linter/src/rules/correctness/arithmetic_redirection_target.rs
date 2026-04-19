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
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.redirect_facts()
                .iter()
                .flat_map(|redirect| redirect.arithmetic_update_operator_spans().iter().copied())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArithmeticRedirectionTarget);
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
echo hi > \"$((i + 1))\"
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
        assert_eq!(diagnostics[0].span.slice(source), "++");
    }
}
