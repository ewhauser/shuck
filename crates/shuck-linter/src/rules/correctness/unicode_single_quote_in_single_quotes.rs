use crate::{Checker, Rule, Violation};

pub struct UnicodeSingleQuoteInSingleQuotes;

impl Violation for UnicodeSingleQuoteInSingleQuotes {
    fn rule() -> Rule {
        Rule::UnicodeSingleQuoteInSingleQuotes
    }

    fn message(&self) -> String {
        "a unicode curly single quote appears inside a single-quoted string".to_owned()
    }
}

pub fn unicode_single_quote_in_single_quotes(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .flat_map(|fragment| {
            let text = fragment.span().slice(source);
            text.char_indices().filter_map(|(offset, char)| {
                if !matches!(char, '\u{2018}' | '\u{2019}') {
                    return None;
                }

                let start = fragment.span().start.advanced_by(&text[..offset]);
                Some(shuck_ast::Span::from_positions(start, start))
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnicodeSingleQuoteInSingleQuotes);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
