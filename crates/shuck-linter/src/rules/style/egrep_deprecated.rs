use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct EgrepDeprecated;

impl Violation for EgrepDeprecated {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::EgrepDeprecated
    }

    fn message(&self) -> String {
        "use `grep -E` instead of `egrep`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `egrep` as `grep -E`".to_owned())
    }
}

pub fn egrep_deprecated(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("egrep") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(EgrepDeprecated, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("grep -E", span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_plain_egrep_invocations() {
        let source = "\
#!/bin/sh
egrep foo file
egrep
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EgrepDeprecated));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["egrep", "egrep"]
        );
    }

    #[test]
    fn ignores_wrapped_egrep_invocations() {
        let source = "\
#!/bin/sh
command egrep foo file
sudo egrep foo file
grep -E foo file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EgrepDeprecated));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_plain_egrep_invocations() {
        let source = "\
#!/bin/sh
egrep foo file
egrep
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EgrepDeprecated),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ngrep -E foo file\ngrep -E\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_wrapped_egrep_invocations_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
command egrep foo file
sudo egrep foo file
grep -E foo file
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EgrepDeprecated),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S060.sh").as_path(),
            &LinterSettings::for_rule(Rule::EgrepDeprecated),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S060_fix_S060.sh", result);
        Ok(())
    }
}
