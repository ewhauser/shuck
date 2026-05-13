use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct UnicodeSingleQuoteInSingleQuotes;

impl Violation for UnicodeSingleQuoteInSingleQuotes {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnicodeSingleQuoteInSingleQuotes
    }

    fn message(&self) -> String {
        "a unicode curly single quote appears inside a single-quoted string".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace it with a shell-safe apostrophe".to_owned())
    }
}

pub fn unicode_single_quote_in_single_quotes(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .words()
        .single_quoted_fragments()
        .iter()
        .flat_map(|fragment| {
            let text = fragment.span().slice(source);
            text.char_indices().filter_map(|(offset, char)| {
                if !matches!(char, '\u{2018}' | '\u{2019}') {
                    return None;
                }

                let start = fragment.span().start.advanced_by(&text[..offset]);
                let span = shuck_ast::Span::from_positions(start, start);
                let fix = Fix::unsafe_edit(Edit::replacement_at(
                    start.offset,
                    start.offset + char.len_utf8(),
                    "'\\''",
                ));
                Some(Diagnostic::new(UnicodeSingleQuoteInSingleQuotes, span).with_fix(fix))
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_unicode_single_quotes_inside_single_quoted_strings() {
        let source = "\
#!/bin/sh
echo 'hello ‘world’'
echo \"hello ‘world’\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnicodeSingleQuoteInSingleQuotes),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                        source[diagnostic.span.start.offset..]
                            .chars()
                            .next()
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![(2, 13, 2, 13, '‘'), (2, 19, 2, 19, '’')]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_unicode_single_quotes() {
        let source = "\
#!/bin/sh
echo 'hello ‘world’'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnicodeSingleQuoteInSingleQuotes),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
echo 'hello '\\''world'\\'''
"
        );
        assert_eq!(result.fixes_applied, 2);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_unicode_single_quotes_unchanged() {
        let source = "\
#!/bin/sh
echo 'hello ‘world’'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnicodeSingleQuoteInSingleQuotes),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C137.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnicodeSingleQuoteInSingleQuotes),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C137_fix_C137.sh", result);
        Ok(())
    }
}
