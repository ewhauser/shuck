use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SetFlagsWithoutDashes;

impl Violation for SetFlagsWithoutDashes {
    fn rule() -> Rule {
        Rule::SetFlagsWithoutDashes
    }

    fn message(&self) -> String {
        "flags passed to `set` should start with `-` or `+`".to_owned()
    }
}

pub fn set_flags_without_dashes(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
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

    checker.report_all_dedup(spans, || SetFlagsWithoutDashes);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

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
    fn ignores_quoted_literals_and_unsupported_shells() {
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

        let unknown_shell_diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SetFlagsWithoutDashes)
                .with_shell(ShellDialect::Unknown),
        );
        assert!(unknown_shell_diagnostics.is_empty());
    }
}
