use shuck_ast::Span;

use crate::{Checker, Rule, Violation};

pub struct LiteralBackslashInSingleQuotes;

impl Violation for LiteralBackslashInSingleQuotes {
    fn rule() -> Rule {
        Rule::LiteralBackslashInSingleQuotes
    }

    fn message(&self) -> String {
        "a backslash inside single quotes stays literal".to_owned()
    }
}

pub fn literal_backslash_in_single_quotes(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter(|fragment| !fragment.dollar_quoted())
        .filter_map(|fragment| literal_backslash_in_single_quotes_span(fragment.span(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LiteralBackslashInSingleQuotes);
}

fn literal_backslash_in_single_quotes_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    if !contains_backslash_letter(inner) {
        return None;
    }

    let next_byte = *source.as_bytes().get(span.end.offset)?;
    if !next_byte.is_ascii_alphabetic() {
        return None;
    }

    let closing_quote = span.start.advanced_by(&text[..text.len() - 1]);
    Some(Span::from_positions(closing_quote, closing_quote))
}

fn contains_backslash_letter(text: &str) -> bool {
    text.as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
