use crate::{Checker, Rule, ShellDialect, Violation, static_word_text};

pub struct PipefailOption;

impl Violation for PipefailOption {
    fn rule() -> Rule {
        Rule::PipefailOption
    }

    fn message(&self) -> String {
        "the `pipefail` option is not portable in `sh` scripts".to_owned()
    }
}

pub fn pipefail_option(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
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
                .is_some_and(|set| set.pipefail_change.is_some())
        })
        .flat_map(|fact| {
            fact.body_args().iter().filter_map(|word| {
                static_word_text(word, checker.source())
                    .is_some_and(|text| text == "pipefail")
                    .then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PipefailOption);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_pipefail_option_in_sh() {
        let source = "\
#!/bin/sh
set -o pipefail
set -eo pipefail
set +o pipefail
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipefailOption));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["pipefail", "pipefail", "pipefail"]
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
}
