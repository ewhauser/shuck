use crate::{Checker, Rule, ShellDialect, Violation};

pub struct WaitOption;

impl Violation for WaitOption {
    fn rule() -> Rule {
        Rule::WaitOption
    }

    fn message(&self) -> String {
        "wait options are not portable in `sh` scripts".to_owned()
    }
}

pub fn wait_option(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("wait"))
        .flat_map(|fact| {
            fact.options()
                .wait()
                .into_iter()
                .flat_map(|wait| wait.option_spans().iter().copied())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || WaitOption);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_wait_options_in_sh() {
        let source = "\
#!/bin/sh
wait -n
wait -pn x
wait -f -n %1
wait -x
wait -1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert_eq!(diagnostics.len(), 6);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-n", "-pn", "-f", "-n", "-x", "-1"]
        );
    }

    #[test]
    fn ignores_wait_options_in_bash() {
        let source = "\
#!/bin/bash
wait -n
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_wait_after_double_dash() {
        let source = "\
#!/bin/sh
wait -- -n
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert!(diagnostics.is_empty());
    }
}
