use crate::{Checker, Rule, Violation};

pub struct SpaceAfterHashBang;

impl Violation for SpaceAfterHashBang {
    fn rule() -> Rule {
        Rule::SpaceAfterHashBang
    }

    fn message(&self) -> String {
        "remove the space so the shebang starts with `#!`".to_owned()
    }
}

pub fn space_after_hash_bang(checker: &mut Checker) {
    if let Some(span) = checker.facts().space_after_hash_bang_span() {
        checker.report(SpaceAfterHashBang, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_space_after_hash_bang_on_first_line() {
        let source = "# !/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }

    #[test]
    fn ignores_valid_or_non_header_comment_lines() {
        for source in [
            "#!/bin/sh\n:\n",
            " #!/bin/sh\n:\n",
            "# comment\n echo ok\n# !/bin/sh\n",
            "echo ok\n# !/bin/sh\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn reports_other_whitespace_between_hash_and_bang() {
        let source = "#\t!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }

    #[test]
    fn reports_space_after_hash_bang_after_header_prelude() {
        let source = "\n# !/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceAfterHashBang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 2);
        assert_eq!(diagnostics[0].span.end.column, 2);
    }
}
