use crate::{Checker, Rule, Violation};

pub struct DuplicateShebangFlag;

impl Violation for DuplicateShebangFlag {
    fn rule() -> Rule {
        Rule::DuplicateShebangFlag
    }

    fn message(&self) -> String {
        "remove the repeated shebang flag".to_owned()
    }
}

pub fn duplicate_shebang_flag(checker: &mut Checker) {
    if let Some(span) = checker.facts().duplicate_shebang_flag_span() {
        checker.report(DuplicateShebangFlag, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_repeated_shebang_flags() {
        for source in [
            "#!/bin/sh -u -u\necho hello\n",
            "#!/usr/bin/env bash -u -u\necho hello\n",
            "#!/usr/bin/env bash -o pipefail -o pipefail\necho hello\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::DuplicateShebangFlag),
            );

            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].span.start.line, 1);
        }
    }

    #[test]
    fn ignores_distinct_or_missing_shebang_flags() {
        for source in [
            "#!/bin/sh -u\n",
            "#!/bin/sh -u -e\n",
            "#!/usr/bin/env bash -u -e\n",
            " #!/bin/sh -u -u\n",
            "# !/bin/sh -u -u\n",
            "# comment\n#!/bin/sh -u -u\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::DuplicateShebangFlag),
            );

            assert!(diagnostics.is_empty());
        }
    }
}
