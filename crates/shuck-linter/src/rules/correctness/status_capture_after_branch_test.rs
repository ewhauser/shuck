use crate::{Checker, Rule, Violation};

pub struct StatusCaptureAfterBranchTest;

impl Violation for StatusCaptureAfterBranchTest {
    fn rule() -> Rule {
        Rule::StatusCaptureAfterBranchTest
    }

    fn message(&self) -> String {
        "`$?` here refers to a condition result, not an earlier command".to_owned()
    }
}

pub fn status_capture_after_branch_test(checker: &mut Checker) {
    checker.report_all(
        checker.facts().condition_status_capture_spans().to_vec(),
        || StatusCaptureAfterBranchTest,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_status_reads_in_first_branch_commands_after_test_conditions() {
        let source = "\
#!/bin/sh
if [ \"$x\" = y ]; then first=$?; fi
while [ \"$x\" = y ]; do again=$?; break; done
[[ \"$x\" = y ]] || return $?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?"]
        );
    }

    #[test]
    fn ignores_non_test_conditions_and_late_status_reads() {
        let source = "\
#!/bin/sh
if false; then ok=$?; fi
if [ \"$x\" = y ]; then :; later=$?; fi
if [ \"$x\" = y ] || true; then mixed=$?; fi
foo || return $?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert!(diagnostics.is_empty());
    }
}
