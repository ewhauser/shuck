use crate::{Checker, Rule, Violation};

use super::broken_test_common::malformed_bracket_test_spans;

pub struct BrokenTestParse;

impl Violation for BrokenTestParse {
    fn rule() -> Rule {
        Rule::BrokenTestParse
    }

    fn message(&self) -> String {
        "`[` test expression is malformed".to_owned()
    }
}

pub fn broken_test_parse(checker: &mut Checker) {
    checker.report_all(malformed_bracket_test_spans(checker), || BrokenTestParse);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_malformed_bracket_tests() {
        let source = "#!/bin/sh\nif [ x = y; then :; fi\n[ foo\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestParse));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "[");
        assert_eq!(diagnostics[1].span.slice(source), "[");
    }

    #[test]
    fn ignores_well_formed_test_commands() {
        let source = "#!/bin/sh\nif [ x = y ]; then :; fi\ntest x = y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BrokenTestParse));

        assert!(diagnostics.is_empty());
    }
}
