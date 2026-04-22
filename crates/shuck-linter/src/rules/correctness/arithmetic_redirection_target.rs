use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct ArithmeticRedirectionTarget;

impl Violation for ArithmeticRedirectionTarget {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ArithmeticRedirectionTarget
    }

    fn message(&self) -> String {
        "redirection targets should not use arithmetic expansion".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the update operator from the redirect target".to_owned())
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

    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(ArithmeticRedirectionTarget, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_redirect_targets_with_arithmetic_expansion() {
        let source = "\
#!/bin/bash
echo hi > \"$((i++))\"
echo hi > \"$((i + 1))\"
echo hi > \"$((i--))\"
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
            vec![2, 4]
        );
        assert_eq!(diagnostics[0].span.slice(source), "++");
        assert_eq!(diagnostics[1].span.slice(source), "--");
    }

    #[test]
    fn applies_unsafe_fix_to_redirect_update_operators() {
        let source = "\
#!/bin/bash
i=1
echo hi > \"$((i++))\"
echo hi > \"$((i--))\"
echo hi > \"$((i + 1))\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ArithmeticRedirectionTarget),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
i=1
echo hi > \"$((i))\"
echo hi > \"$((i))\"
echo hi > \"$((i + 1))\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C050.sh").as_path(),
            &LinterSettings::for_rule(Rule::ArithmeticRedirectionTarget),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C050_fix_C050.sh", result);
        Ok(())
    }
}
