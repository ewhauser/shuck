use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct FgrepDeprecated;

impl Violation for FgrepDeprecated {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::FgrepDeprecated
    }

    fn message(&self) -> String {
        "use `grep -F` instead of `fgrep`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `fgrep` as `grep -F`".to_owned())
    }
}

pub fn fgrep_deprecated(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("fgrep") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(FgrepDeprecated, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("grep -F", span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_plain_fgrep_invocations() {
        let source = "\
#!/bin/sh
fgrep foo file
fgrep
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FgrepDeprecated));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["fgrep", "fgrep"]
        );
    }

    #[test]
    fn ignores_wrapped_fgrep_invocations() {
        let source = "\
#!/bin/sh
command fgrep foo file
sudo fgrep foo file
grep -F foo file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FgrepDeprecated));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_plain_fgrep_invocations() {
        let source = "\
#!/bin/sh
fgrep foo file
fgrep
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FgrepDeprecated),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ngrep -F foo file\ngrep -F\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_wrapped_fgrep_invocations_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
command fgrep foo file
sudo fgrep foo file
grep -F foo file
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::FgrepDeprecated),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S061.sh").as_path(),
            &LinterSettings::for_rule(Rule::FgrepDeprecated),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S061_fix_S061.sh", result);
        Ok(())
    }
}
