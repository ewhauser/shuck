use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct EscapedNegationInTest;

impl Violation for EscapedNegationInTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::EscapedNegationInTest
    }

    fn message(&self) -> String {
        "write ! directly when negating a test".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the backslash before the leading `!`".to_owned())
    }
}

pub fn escaped_negation_in_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let simple_test = fact.simple_test()?;
            let (diagnostic_span, fix_span) = simple_test.escaped_negation_spans(source)?;
            Some(
                Diagnostic::new(EscapedNegationInTest, diagnostic_span)
                    .with_fix(Fix::safe_edit(Edit::deletion(fix_span))),
            )
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_escaped_negation_in_simple_tests() {
        let source = "\
#!/bin/bash
[ \\! -f \"$file\" ]
test \\! -n \"$value\"
[ \\! \"$value\" = ok ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-f", "\\!", "\\!"]
        );
    }

    #[test]
    fn attaches_safe_fix_metadata_for_escaped_negation() {
        let source = "#!/bin/bash\n[ \\! -f \"$file\" ]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Safe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the backslash before the leading `!`")
        );
    }

    #[test]
    fn applies_safe_fix_to_escaped_negation_in_test() {
        let source = "\
#!/bin/bash
[ \\! -f \"$file\" ]
test \\! -n \"$value\"
[ \\! \"$value\" = ok ]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ ! -f \"$file\" ]
test ! -n \"$value\"
[ ! \"$value\" = ok ]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_literal_bang_cases_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ ! -f \"$file\" ]
test !
[ \"$value\" = \\! ]
[[ \\! -f \"$file\" ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C082.sh").as_path(),
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C082_fix_C082.sh", result);
        Ok(())
    }
}
