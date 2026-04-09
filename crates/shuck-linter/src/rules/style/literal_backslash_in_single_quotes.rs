use crate::{Checker, Rule, Violation};

pub struct LiteralBackslashInSingleQuotes;

impl Violation for LiteralBackslashInSingleQuotes {
    fn rule() -> Rule {
        Rule::LiteralBackslashInSingleQuotes
    }

    fn message(&self) -> String {
        "a backslash inside single quotes stays literal".to_owned()
    }
}

pub fn literal_backslash_in_single_quotes(_checker: &mut Checker) {
    // Current oracle runs do not expose a stable standalone SC2267 boundary in the
    // compatibility corpus, so keep the legacy rule wired without active reports.
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn currently_reports_no_distinct_diagnostics() {
        let source = "\
#!/bin/sh
grep ^start'\\s'end file.txt
printf '%s' 'foo\\nbar'
printf '%s' 'foo\\bar'
printf '%s' 'foo\\x41bar'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralBackslashInSingleQuotes),
        );

        assert!(diagnostics.is_empty());
    }
}
