use crate::{Checker, Rule, ShellDialect, Violation};

pub struct EchoToSedSubstitution;

impl Violation for EchoToSedSubstitution {
    fn rule() -> Rule {
        Rule::EchoToSedSubstitution
    }

    fn message(&self) -> String {
        "prefer a shell rewrite over piping echo into sed for one substitution".to_owned()
    }
}

pub fn echo_to_sed_substitution(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    checker.report_all_dedup(
        checker.facts().echo_to_sed_substitution_spans().to_vec(),
        || EchoToSedSubstitution,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LinterSettings;
    use crate::test::test_snippet;

    #[test]
    fn reports_shellcheck_columns_for_escaped_dollar_backtick_patterns() {
        let source = "\
#!/bin/bash
EC2_REGION=\"`echo \\\"$EC2_AVAIL_ZONE\\\" | sed -e 's:\\([0-9][0-9]*\\)[a-z]*\\$:\\\\1:'`\"
";

        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EchoToSedSubstitution),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 14);
        assert_eq!(diagnostics[0].span.end.line, 2);
        assert_eq!(diagnostics[0].span.end.column, 76);
    }
}
