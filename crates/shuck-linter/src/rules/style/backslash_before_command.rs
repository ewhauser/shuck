use crate::{Checker, Rule, Violation};

pub struct BackslashBeforeCommand;

impl Violation for BackslashBeforeCommand {
    fn rule() -> Rule {
        Rule::BackslashBeforeCommand
    }

    fn message(&self) -> String {
        "a leading backslash before a command name is only used to bypass aliases".to_owned()
    }
}

pub fn backslash_before_command(_checker: &mut Checker) {
    // Current oracle runs do not expose a distinct backslash-before-command code,
    // and modern SC2268 is used for a different warning. Keep the legacy rule
    // wired without active reports until a stable compatibility boundary exists.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn currently_reports_no_distinct_diagnostics() {
        let source = "\
#!/bin/bash
\\command echo hi
\\command \\rm tmp.txt || echo fail
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::BackslashBeforeCommand));

        assert!(diagnostics.is_empty());
    }
}
