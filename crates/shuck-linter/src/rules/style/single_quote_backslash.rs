use shuck_ast::Span;

use crate::{Checker, Rule, Violation};

pub struct SingleQuoteBackslash;

impl Violation for SingleQuoteBackslash {
    fn rule() -> Rule {
        Rule::SingleQuoteBackslash
    }

    fn message(&self) -> String {
        "a backslash at the end of a single-quoted string is literal".to_owned()
    }
}

pub fn single_quote_backslash(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| single_quoted_fragment_backslash_span(fragment.span(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SingleQuoteBackslash);
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
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
