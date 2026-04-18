use crate::{Checker, Rule, ShellDialect, Violation};

pub struct WaitOption;

impl Violation for WaitOption {
    fn rule() -> Rule {
        Rule::WaitOption
    }

    fn message(&self) -> String {
        "this builtin option is not portable in `sh` scripts".to_owned()
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
        .filter_map(|fact| fact.options().nonportable_sh_builtin_option_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || WaitOption);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nonportable_builtin_options_in_sh() {
        let source = "\
#!/bin/sh
read -r name
read -p prompt name
read -\"$mode\" name
printf -v out '%s' foo
export -fn greet
trap -p EXIT
wait -n
ulimit -n
type -P printf
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert_eq!(diagnostics.len(), 8);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-p", "-\"$mode\"", "-v", "-fn", "-p", "-n", "-n", "-P"]
        );
    }

    #[test]
    fn ignores_builtin_options_in_bash() {
        let source = "\
#!/bin/bash
read -p prompt name
wait -n
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_operands_after_double_dash() {
        let source = "\
#!/bin/sh
read -- -p name
printf -- -v out
export -- -f greet
trap -- -p EXIT
wait -- -n
ulimit -- -n
type -- -P printf
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_the_first_nonportable_option_word_per_command() {
        let source = "\
#!/bin/sh
read -r -p prompt -n 1 name
wait -p jobid -n
ulimit -H -n 1
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-p", "-p", "-H"]
        );
    }

    #[test]
    fn reports_combined_and_dynamic_option_words() {
        let source = "\
#!/bin/sh
read -rp prompt name
wait -fpn job
printf -\"$mode\" '%s' foo
trap -lp EXIT
export -pn greet
ulimit -HSn 1
type -ap printf
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::WaitOption));

        assert_eq!(diagnostics.len(), 7);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-rp", "-fpn", "-\"$mode\"", "-lp", "-pn", "-HSn", "-ap"]
        );
    }
}
