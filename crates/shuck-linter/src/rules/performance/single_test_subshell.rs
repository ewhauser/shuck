use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SingleTestSubshell;

impl Violation for SingleTestSubshell {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SingleTestSubshell
    }

    fn message(&self) -> String {
        "drop the subshell around this single test condition".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the subshell parentheses".to_owned())
    }
}

pub fn single_test_subshell(checker: &mut Checker) {
    let spans = checker.facts().single_test_subshell_spans().to_vec();
    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SingleTestSubshell, span).with_fix(remove_outer_parens_fix(span)),
        );
    }
}

fn remove_outer_parens_fix(span: Span) -> Fix {
    Fix::unsafe_edits([
        Edit::deletion_at(span.start.offset, span.start.offset + 1),
        Edit::deletion_at(span.end.offset.saturating_sub(1), span.end.offset),
    ])
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_the_condition_subshell() {
        let source = "\
#!/bin/sh
if (test -f /etc/passwd); then :; fi
if (test -f /etc/passwd) >/dev/null 2>&1; then :; fi
if ! (test -f /etc/passwd); then :; fi
if ( ! test -f /etc/passwd ); then :; fi
if (test -f /etc/passwd || test -f /etc/hosts); then :; fi
if ! (test -f /etc/passwd || test -f /etc/hosts); then :; fi
while ([ -f /etc/passwd ]); do :; done
while ! ([ -f /etc/passwd ]); do :; done
until (command test -f /etc/passwd); do :; done
until ! (command test -f /etc/passwd); do :; done
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
                "(test -f /etc/passwd)",
                "( ! test -f /etc/passwd )",
                "(test -f /etc/passwd || test -f /etc/hosts)",
                "([ -f /etc/passwd ])",
                "([ -f /etc/passwd ])",
                "(command test -f /etc/passwd)",
                "(command test -f /etc/passwd)",
            ]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_remove_subshell_parentheses() {
        let source = "#!/bin/sh\nif (test -f /etc/passwd); then :; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleTestSubshell),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nif test -f /etc/passwd; then :; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
