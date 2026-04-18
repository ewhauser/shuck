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

    #[test]
    fn reports_each_split_suspicious_closing_quote_in_echo_arguments() {
        let source = "\
#!/bin/bash
echo \"alpha
\"_beta \"gamma
\"_delta
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
        assert_eq!(diagnostics[1].span.start.line, 4);
        assert_eq!(diagnostics[1].span.start.column, 1);
        assert_eq!(diagnostics[1].span.start, diagnostics[1].span.end);
    }

    #[test]
    fn ignores_literal_double_quotes_inside_multiline_single_quoted_words() {
        let source = "\
#!/bin/bash
echo 'alpha
\"_beta'
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_each_reopened_quote_window_in_a_single_word() {
        let source = "\
#!/bin/bash
echo \"alpha
\"$foo\"beta
\"$bar\"gamma\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
        assert_eq!(diagnostics[1].span.start.line, 4);
        assert_eq!(diagnostics[1].span.start.column, 1);
        assert_eq!(diagnostics[1].span.start, diagnostics[1].span.end);
    }

    #[test]
    fn reports_only_the_leading_triple_quote_script_builder_closing_quote() {
        let source = "\
#!/bin/bash
echo \"\"\"#!/usr/bin/env bash
echo \"GEM_HOME FIRST: \\$GEM_HOME\"
echo \"GEM_PATH: \\$GEM_PATH\"
\"\"\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SuspectClosingQuote));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 6);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }
}
