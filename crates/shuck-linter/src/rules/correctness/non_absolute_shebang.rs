use crate::{Checker, Rule, Violation};

pub struct NonAbsoluteShebang;

impl Violation for NonAbsoluteShebang {
    fn rule() -> Rule {
        Rule::NonAbsoluteShebang
    }

    fn message(&self) -> String {
        "shebang should use an absolute path or `/usr/bin/env`".to_owned()
    }
}

pub fn non_absolute_shebang(checker: &mut Checker) {
    if let Some(span) = checker.facts().non_absolute_shebang_span() {
        checker.report(NonAbsoluteShebang, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_non_absolute_shebangs() {
        let source = "#!bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.slice(source), "#!bin/sh");
    }

    #[test]
    fn ignores_absolute_and_env_shebangs() {
        for source in [
            "#!/bin/sh\n:\n",
            "#!/usr/bin/env sh\n:\n",
            "#! /bin/sh\n:\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn ignores_non_absolute_shebang_when_shellcheck_shell_directive_is_present() {
        let source = "#!@TERMUX_PREFIX@/bin/sh\n# shellcheck shell=sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::NonAbsoluteShebang));

        assert!(diagnostics.is_empty());
    }
}
