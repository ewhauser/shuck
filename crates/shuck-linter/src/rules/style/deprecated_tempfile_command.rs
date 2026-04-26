use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct DeprecatedTempfileCommand;

impl Violation for DeprecatedTempfileCommand {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::DeprecatedTempfileCommand
    }

    fn message(&self) -> String {
        "use `mktemp` instead of `tempfile`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `tempfile` as `mktemp`".to_owned())
    }
}

pub fn deprecated_tempfile_command(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tempfile") && fact.wrappers().is_empty())
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(DeprecatedTempfileCommand, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement("mktemp", span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_plain_tempfile_invocations() {
        let source = "\
#!/bin/sh
tempfile -n \"$TMPDIR/Xauthority\"
tempfile
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["tempfile", "tempfile"]
        );
    }

    #[test]
    fn ignores_wrapped_tempfile_invocations() {
        let source = "\
#!/bin/sh
command tempfile -n \"$TMPDIR/Xauthority\"
sudo tempfile -n \"$TMPDIR/Xauthority\"
alias tempfile=mktemp
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_plain_tempfile_invocations() {
        let source = "\
#!/bin/sh
tempfile -n \"$TMPDIR/Xauthority\"
tempfile
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nmktemp -n \"$TMPDIR/Xauthority\"\nmktemp\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_wrapped_tempfile_invocations_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
command tempfile -n \"$TMPDIR/Xauthority\"
sudo tempfile -n \"$TMPDIR/Xauthority\"
alias tempfile=mktemp
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S059.sh").as_path(),
            &LinterSettings::for_rule(Rule::DeprecatedTempfileCommand),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S059_fix_S059.sh", result);
        Ok(())
    }
}
