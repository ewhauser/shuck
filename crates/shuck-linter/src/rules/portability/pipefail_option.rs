use crate::{Checker, Rule, ShellDialect, Violation};

pub struct PipefailOption;

impl Violation for PipefailOption {
    fn rule() -> Rule {
        Rule::PipefailOption
    }

    fn message(&self) -> String {
        "`set -o` uses an option that is not portable in `sh` scripts".to_owned()
    }
}

pub fn pipefail_option(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Sh {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("set"))
        .filter(|fact| {
            fact.options()
                .set()
                .is_some_and(|set| !set.non_posix_option_spans().is_empty())
        })
        .flat_map(|fact| {
            fact.options()
                .set()
                .into_iter()
                .flat_map(|set| set.non_posix_option_spans().iter().copied())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PipefailOption);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_non_posix_set_o_options_in_sh() {
        let source = "\
#!/bin/sh
set -o pipefail
set -eo emacs
set +o posix
set -o bogus
set -o \"privileged\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert_eq!(diagnostics.len(), 5);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["pipefail", "emacs", "posix", "bogus", "\"privileged\""]
        );
    }

    #[test]
    fn ignores_pipefail_option_in_bash() {
        let source = "\
#!/bin/bash
set -o pipefail
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dash_shells() {
        let source = "\
#!/bin/dash
set -o pipefail
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_posix_set_o_names_in_sh() {
        let source = "\
#!/bin/sh
set -o allexport
set -o errexit
set -o ignoreeof
set -o monitor
set -o noclobber
set -o noexec
set -o noglob
set -o nolog
set -o notify
set -o nounset
set -o verbose
set -o vi
set -o xtrace
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_positional_pipefail_after_double_dash() {
        let source = "\
#!/bin/sh
set -o pipefail -- pipefail
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "pipefail");
    }
}
