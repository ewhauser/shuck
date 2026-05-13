use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SingleQuoteBackslash;

impl Violation for SingleQuoteBackslash {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SingleQuoteBackslash
    }

    fn message(&self) -> String {
        "a backslash at the end of a single-quoted string is literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move the trailing backslash outside the quotes".to_owned())
    }
}

pub fn single_quote_backslash(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| single_quoted_fragment_backslash_span(fragment.span(), source))
        .map(|span| {
            Diagnostic::new(SingleQuoteBackslash, span).with_fix(Fix::safe_edit(
                Edit::replacement_at(span.start.offset, span.start.offset + 2, "'\\\\"),
            ))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn single_quoted_fragment_backslash_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !text.ends_with("\\'") {
        return None;
    }

    let prefix = &text[..text.len() - 2];
    let start = span.start.advanced_by(prefix);
    Some(Span::from_positions(start, start))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_backslash_at_end_of_single_quoted_string() {
        let source = "\
#!/bin/sh
printf '%s\\n' 'foo\\'
printf '%s\\n' 'foo\\bar'
printf '%s\\n' \"foo\\\\bar\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleQuoteBackslash),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![""]
        );
    }

    #[test]
    fn ignores_escaped_single_quotes_inside_replacement_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${dest_dir//\\'/\\'\\\\\\'\\'}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SingleQuoteBackslash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_trailing_single_quote_backslash() {
        let source = "#!/bin/sh\nprintf '%s\\n' 'foo\\'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SingleQuoteBackslash),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\nprintf '%s\\n' 'foo'\\\\\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
