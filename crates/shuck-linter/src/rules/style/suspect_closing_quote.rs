use crate::{Checker, Rule, Violation};

pub struct SuspectClosingQuote;

impl Violation for SuspectClosingQuote {
    fn rule() -> Rule {
        Rule::SuspectClosingQuote
    }

    fn message(&self) -> String {
        "quote is closed but the following character looks ambiguous".to_owned()
    }
}

pub fn suspect_closing_quote(checker: &mut Checker) {
    let spans = checker
        .facts()
        .suspect_closing_quote_fragments()
        .iter()
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SuspectClosingQuote);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_suspicious_closing_quote() {
        let source = "#!/bin/bash\necho \"#!/bin/bash\nif [[ \"$@\" =~ x ]]; then :; fi\n\"\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 7);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_sc2140_style_multiline_quote_joins() {
        let source = "\
#!/bin/bash
echo \"[Unit]
Description=Heimdall
ExecStart=\"/usr/bin/php\" artisan serve
WantedBy=multi-user.target\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert!(diagnostics.is_empty());
    }
}
