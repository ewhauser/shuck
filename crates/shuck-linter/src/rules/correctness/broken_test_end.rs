use crate::{Checker, Rule, Violation};

use super::broken_test_common::malformed_bracket_test_spans;

pub struct BrokenTestEnd;

impl Violation for BrokenTestEnd {
    fn rule() -> Rule {
        Rule::BrokenTestEnd
    }

    fn message(&self) -> String {
        "`[` test is missing its closing `]`".to_owned()
    }
}

pub fn broken_test_end(checker: &mut Checker) {
    checker.report_all(malformed_bracket_test_spans(checker), || BrokenTestEnd);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_missing_closing_brackets_in_test_commands() {
        let source = "#!/bin/sh\nif [ x = y; then :; fi\n[ foo\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestEnd));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "[");
        assert_eq!(diagnostics[1].span.slice(source), "[");
    }

    #[test]
    fn ignores_well_formed_bracket_tests_and_test_builtin() {
        let source = "#!/bin/sh\nif [ x = y ]; then :; fi\ntest x = y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestEnd));

        assert!(diagnostics.is_empty());
    }
}
