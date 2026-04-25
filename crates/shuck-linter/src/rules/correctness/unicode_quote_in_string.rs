use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct UnicodeQuoteInString;

impl Violation for UnicodeQuoteInString {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnicodeQuoteInString
    }

    fn message(&self) -> String {
        "a unicode smart quote appears inside a shell string".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the smart quote with a matching ASCII quote".to_owned())
    }
}

pub fn unicode_quote_in_string(checker: &mut Checker) {
    let source = checker.source();
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for span in facts.unicode_smart_quote_spans().iter().copied() {
            report(
                Diagnostic::new(UnicodeQuoteInString, span).with_fix(Fix::unsafe_edit(
                    Edit::replacement(ascii_quote_replacement(span.slice(source)), span),
                )),
            );
        }
    });
}

fn ascii_quote_replacement(quote: &str) -> &'static str {
    match quote {
        "\u{2018}" | "\u{2019}" => "'",
        "\u{201C}" | "\u{201D}" => "\"",
        _ => unreachable!("unicode smart quote spans should contain only smart quote characters"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn attaches_unsafe_fix_metadata_for_unicode_smart_quotes() {
        let source = "#!/bin/sh\necho “hello”\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
        );

        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic.fix.as_ref().map(|fix| fix.applicability()) == Some(Applicability::Unsafe)
        }));
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix_title.as_deref()
                    == Some("replace the smart quote with a matching ASCII quote"))
        );
    }

    #[test]
    fn applies_unsafe_fix_to_unicode_smart_quotes() {
        let source = "\
#!/bin/sh
echo “hello”
echo ‘world’
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo \"hello\"
echo 'world'
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_ascii_quoted_strings_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
echo \"hello “world”\"
echo 'hello ‘world’'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C072.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnicodeQuoteInString),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C072_fix_C072.sh", result);
        Ok(())
    }
}
