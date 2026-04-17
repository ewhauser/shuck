use crate::{Checker, Rule, Violation};

pub struct IndentedShebang;

impl Violation for IndentedShebang {
    fn rule() -> Rule {
        Rule::IndentedShebang
    }

    fn message(&self) -> String {
        "shebang must start in column 1".to_owned()
    }
}

pub fn indented_shebang(checker: &mut Checker) {
    if let Some(span) = checker.facts().indented_shebang_span() {
        checker.report(IndentedShebang, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_indented_shebang_on_first_line() {
        let source = " #!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn reports_indented_shebang_after_header_prelude() {
        let source = "\n #!/bin/sh\n:\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn ignores_non_indented_or_non_header_shebangs() {
        for source in [
            "#!/bin/sh\n:\n",
            "#! /bin/sh\n:\n",
            "\n#!/bin/sh\n:\n",
            "\t# not a shebang\n:\n",
        ] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::IndentedShebang));
            assert!(diagnostics.is_empty());
        }
    }
}
