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
    let spans = checker
        .facts()
        .single_quoted_fragments()
        .iter()
        .filter_map(|fragment| fragment.literal_backslash_in_single_quotes_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LiteralBackslashInSingleQuotes);
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
