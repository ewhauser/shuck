use crate::{Checker, Rule, ShellDialect, Violation};

pub struct EchoFlags;

impl Violation for EchoFlags {
    fn rule() -> Rule {
        Rule::EchoFlags
    }

    fn message(&self) -> String {
        "echo flags are not portable in `sh` scripts".to_owned()
    }
}

pub fn echo_flags(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.wrappers().is_empty())
        .filter_map(|fact| {
            fact.options()
                .echo()
                .and_then(|echo| echo.portability_flag_word())
                .map(|word| word.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EchoFlags);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_portability_sensitive_echo_flags_in_sh() {
        let source = "\
#!/bin/sh
echo -n hi
echo -e hi
echo -E hi
echo -nn hi
echo -neE hi
value=$(echo -en hi)
value=$(/usr/ucb/echo -n hi)
echo \"-s\"
echo '-e'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoFlags));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "-n", "-e", "-E", "-nn", "-neE", "-en", "-n", "\"-s\"", "'-e'",
            ]
        );
    }

    #[test]
    fn ignores_non_flag_operands_and_wrapped_echoes() {
        let source = "\
#!/bin/sh
echo -- hi
echo - hello
echo -x hi
echo -nfoo hi
echo \"-I\" hi
echo '-F' hi
command echo -n hi
builtin echo -n hi
echo \"$flag\" hi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoFlags));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_echo_flags_in_bash() {
        let source = "\
#!/bin/bash
echo -n hi
echo \"-s\" hi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoFlags));

        assert!(diagnostics.is_empty());
    }
}
