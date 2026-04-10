use crate::{Checker, Rule, Violation};

use super::unchecked_directory_change::unchecked_directory_change_in_function_spans;

pub struct UncheckedDirectoryChangeInFunction {
    pub command: &'static str,
}

impl Violation for UncheckedDirectoryChangeInFunction {
    fn rule() -> Rule {
        Rule::UncheckedDirectoryChangeInFunction
    }

    fn message(&self) -> String {
        format!(
            "`{}` inside a function should check whether the directory change succeeded",
            self.command
        )
    }
}

pub fn unchecked_directory_change_in_function(checker: &mut Checker) {
    for (command, span) in unchecked_directory_change_in_function_spans(checker) {
        checker.report(UncheckedDirectoryChangeInFunction { command }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn only_reports_the_function_specific_rule_when_both_are_enabled() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::UncheckedDirectoryChange,
                Rule::UncheckedDirectoryChangeInFunction,
            ]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].rule,
            Rule::UncheckedDirectoryChangeInFunction
        );
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "\
#!/bin/zsh
f() {
\tcd /tmp
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
