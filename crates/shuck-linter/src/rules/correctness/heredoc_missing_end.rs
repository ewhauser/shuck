use crate::{Checker, Rule, Violation};

pub struct HeredocMissingEnd;

impl Violation for HeredocMissingEnd {
    fn rule() -> Rule {
        Rule::HeredocMissingEnd
    }

    fn message(&self) -> String {
        "this here-document is missing its closing marker".to_owned()
    }
}

pub fn heredoc_missing_end(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.heredoc_missing_end_spans(),
        || HeredocMissingEnd,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_heredoc_without_a_closing_marker() {
        let source = "\
#!/bin/sh
cat <<MARK
line
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "<<MARK");
    }

    #[test]
    fn reports_unclosed_empty_delimiter_heredoc() {
        let source = "\
#!/bin/sh
cat <<''
line
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "<<''");
    }

    #[test]
    fn ignores_closed_heredoc_even_when_delimiter_has_no_trailing_newline() {
        let source = "#!/bin/sh\ncat <<MARK\nline\nMARK";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert!(diagnostics.is_empty());
    }
}
