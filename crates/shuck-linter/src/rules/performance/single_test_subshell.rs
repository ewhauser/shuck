use crate::{Checker, Rule, Violation};

pub struct SingleTestSubshell;

impl Violation for SingleTestSubshell {
    fn rule() -> Rule {
        Rule::SingleTestSubshell
    }

    fn message(&self) -> String {
        "drop the subshell around this single test condition".to_owned()
    }
}

pub fn single_test_subshell(checker: &mut Checker) {
    let spans = checker.facts().single_test_subshell_spans().to_vec();
    checker.report_all_dedup(spans, || SingleTestSubshell);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_condition_subshell() {
        let source = "\
#!/bin/sh
if (test -f /etc/passwd); then :; fi
if (test -f /etc/passwd) >/dev/null 2>&1; then :; fi
if (test -f /etc/passwd || test -f /etc/hosts); then :; fi
while ([ -f /etc/passwd ]); do :; done
until (command test -f /etc/passwd); do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SingleTestSubshell));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "(test -f /etc/passwd)",
                "(test -f /etc/passwd)",
                "(test -f /etc/passwd || test -f /etc/hosts)",
                "([ -f /etc/passwd ])",
                "(command test -f /etc/passwd)",
            ]
        );
    }
}
