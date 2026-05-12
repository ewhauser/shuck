use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct SetFlagsWithoutDashes;

impl Violation for SetFlagsWithoutDashes {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SetFlagsWithoutDashes
    }

    fn message(&self) -> String {
        "flags passed to `set` should start with `-` or `+`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert `--` before positional arguments".to_owned())
    }
}

pub fn set_flags_without_dashes(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Unknown
            | ShellDialect::Sh
            | ShellDialect::Bash
            | ShellDialect::Dash
            | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("set"))
        .filter_map(|fact| fact.options().set())
        .flat_map(|set| set.flags_without_prefix_spans().iter().copied())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SetFlagsWithoutDashes, span)
                .with_fix(Fix::safe_edit(Edit::insertion(span.start.offset, "-- "))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_set_flags_without_prefix() {
        let source = "\
set euox pipefail
set foo bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["euox", "foo"]
        );
    }

    #[test]
    fn ignores_explicit_prefixes_or_single_positional_values() {
        let source = "\
set -euo pipefail
set +e
set -- foo bar
set foo
set n-aliases.conf n-env.conf
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_quoted_literals_but_keeps_unknown_shells_enabled() {
        let source = "\
set \"required\" \"$1\"
set f\"oo\" bar
set OFFLINE_PATH \"$PWD\"
";
        let bash_diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
        );
        assert_eq!(
            bash_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["OFFLINE_PATH"]
        );

        let unknown_source = "set foo bar\n";
        let unknown_shell_diagnostics = test_snippet(
            unknown_source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes)
                .with_shell(ShellDialect::Unknown),
        );
        assert_eq!(
            unknown_shell_diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(unknown_source))
                .collect::<Vec<_>>(),
            vec!["foo"]
        );
    }

    #[test]
    fn ignores_unsupported_known_shells() {
        let source = "set foo bar\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_bare_set_operands() {
        let source = "\
set euox pipefail
set foo bar
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
set -- euox pipefail
set -- foo bar
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_explicit_and_unambiguous_set_operands_unchanged_when_fixing() {
        let source = "\
set -euo pipefail
set -- foo bar
set foo
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C098.sh").as_path(),
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes).with_shell(ShellDialect::Bash),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C098_fix_C098.sh", result);
        Ok(())
    }
}
