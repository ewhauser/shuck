use crate::{Checker, Rule, ShellDialect, Violation};

pub struct DollarStringInSh;

impl Violation for DollarStringInSh {
    fn rule() -> Rule {
        Rule::DollarStringInSh
    }

    fn message(&self) -> String {
        "`$\"...\"` strings are not portable in `sh`".to_owned()
    }
}

pub fn dollar_string_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .dollar_double_quoted_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || DollarStringInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_each_dollar_double_quoted_fragment() {
        let source = "\
#!/bin/sh
echo $\"Usage: $0 {start|stop}\"
printf '%s\\n' \"$\"'not-a-dollar-double-quote'\" plain
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarStringInSh));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$\"Usage: $0 {start|stop}\""]
        );
    }

    #[test]
    fn ignores_dollar_double_quoted_fragments_in_bash() {
        let source = "echo $\"hi\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarStringInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
