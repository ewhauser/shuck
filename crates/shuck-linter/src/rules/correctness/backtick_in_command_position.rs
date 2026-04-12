use crate::{Checker, Rule, Violation};

pub struct BacktickInCommandPosition;

impl Violation for BacktickInCommandPosition {
    fn rule() -> Rule {
        Rule::BacktickInCommandPosition
    }

    fn message(&self) -> String {
        "run the command directly instead of executing backtick output as a command name".to_owned()
    }
}

pub fn backtick_in_command_position(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().backtick_command_name_spans().to_vec(),
        || BacktickInCommandPosition,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_backticks_used_as_command_names() {
        let source = "\
#!/bin/sh
`echo hello` | cat
if `echo true`; then :; fi
FOO=1 `echo run`
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`echo hello`", "`echo true`", "`echo run`"]
        );
    }

    #[test]
    fn ignores_wrapped_quoted_and_affixed_backticks() {
        let source = "\
#!/bin/sh
command `echo hello`
\"`echo hello`\" | cat
x`echo hello`
echo `date`
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickInCommandPosition),
        );

        assert!(diagnostics.is_empty());
    }
}
