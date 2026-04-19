use crate::{Checker, Rule, Violation};

pub struct UnicodeQuoteInString;

impl Violation for UnicodeQuoteInString {
    fn rule() -> Rule {
        Rule::UnicodeQuoteInString
    }

    fn message(&self) -> String {
        "a unicode smart quote appears inside a shell string".to_owned()
    }
}

pub fn unicode_quote_in_string(checker: &mut Checker) {
    let spans = checker.facts().unicode_smart_quote_spans().to_vec();
    checker.report_all_dedup(spans, || UnicodeQuoteInString);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unicode_smart_quotes_in_unquoted_shell_words() {
        let source = "\
#!/bin/sh
echo “hello”
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["“", "”"]
        );
    }

    #[test]
    fn ignores_unicode_smart_quotes_inside_ascii_quoted_strings() {
        let source = "\
#!/bin/sh
echo \"hello “world”\"
echo 'hello ‘world’'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_unicode_smart_quotes_inside_heredoc_payloads() {
        let source = "\
#!/bin/sh
cat <<EOF
q { quotes: \"“\" \"”\" \"‘\" \"’\"; }
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
        );

        assert!(diagnostics.is_empty());
    }
}
