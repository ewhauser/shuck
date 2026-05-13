use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct LiteralBackslashInSingleQuotes;

impl Violation for LiteralBackslashInSingleQuotes {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LiteralBackslashInSingleQuotes
    }

    fn message(&self) -> String {
        "a backslash inside single quotes stays literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite as a double-quoted string".to_owned())
    }
}

pub fn literal_backslash_in_single_quotes(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .words()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| {
            let span = fragment.literal_backslash_in_single_quotes_span()?;
            Some(
                Diagnostic::new(LiteralBackslashInSingleQuotes, span)
                    .with_fix(rewrite_single_quoted_fragment_fix(fragment.span(), source)?),
            )
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn rewrite_single_quoted_fragment_fix(fragment_span: Span, source: &str) -> Option<Fix> {
    let fragment = fragment_span.slice(source);
    let content = fragment.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut replacement = String::from("\"");
    for ch in content.chars() {
        match ch {
            '$' | '`' | '"' | '\\' => {
                replacement.push('\\');
                replacement.push(ch);
            }
            _ => replacement.push(ch),
        }
    }
    replacement.push('"');

    Some(Fix::safe_edit(Edit::replacement(
        replacement,
        fragment_span,
    )))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_single_quoted_escape_sequences_that_run_into_more_literal_text() {
        let source = "\
#!/bin/sh
grep ^start'\\s'end file.txt
printf '%s\\n' '\\n'foo
printf '%s\\n' 'ab\\n'c
printf '%s\\n' '\\\\n'foo
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralBackslashInSingleQuotes),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 15), (3, 18), (4, 20), (5, 19)]
        );
    }

    #[test]
    fn ignores_standalone_digit_continued_and_dollar_quoted_fragments() {
        let source = "\
#!/bin/sh
printf '%s\\n' 'foo\\nbar'
printf '%s\\n' '\\x'41
printf '%s\\n' '\\0'foo
printf '%s\\n' '\\n'_
printf '%s\\n' $'\\n'foo
printf '%s\\n' a'\\'bc
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralBackslashInSingleQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_by_rewriting_single_quotes_as_double_quotes() {
        let source = "\
#!/bin/sh
grep ^start'\\s'end file.txt
printf '%s\\n' 'a$`\"\\\\b\\n'c
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LiteralBackslashInSingleQuotes),
            Applicability::Safe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep ^start\"\\\\s\"end file.txt
printf '%s\\n' \"a\\$\\`\\\"\\\\\\\\b\\\\n\"c
"
        );
        assert_eq!(result.fixes_applied, 2);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S039.sh").as_path(),
            &LinterSettings::for_rule(Rule::LiteralBackslashInSingleQuotes),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S039_fix_S039.sh", result);
        Ok(())
    }
}
