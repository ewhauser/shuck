use shuck_ast::{ConditionalBinaryOp, Span, static_word_text};

use crate::{Checker, ConditionalBinaryFact, ConditionalNodeFact, Rule, Violation};

pub struct StringComparisonForVersion;

impl Violation for StringComparisonForVersion {
    fn rule() -> Rule {
        Rule::StringComparisonForVersion
    }

    fn message(&self) -> String {
        "this `[[ ... ]]` comparison orders version-like values lexicographically".to_owned()
    }
}

pub fn string_comparison_for_version(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.conditional())
        .flat_map(|conditional| {
            conditional
                .nodes()
                .iter()
                .filter_map(|node| report_span(node, checker.source()))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StringComparisonForVersion);
}

fn report_span(node: &ConditionalNodeFact<'_>, source: &str) -> Option<Span> {
    let ConditionalNodeFact::Binary(binary) = node else {
        return None;
    };
    if !matches!(
        binary.op(),
        ConditionalBinaryOp::LexicalBefore | ConditionalBinaryOp::LexicalAfter
    ) {
        return None;
    }

    version_operand_span(binary, source)
}

fn version_operand_span(binary: &ConditionalBinaryFact<'_>, source: &str) -> Option<Span> {
    let left = binary
        .left()
        .word()
        .and_then(|word| static_word_text(word, source).map(|text| (word.span, text)));
    let right = binary
        .right()
        .word()
        .and_then(|word| static_word_text(word, source).map(|text| (word.span, text)));

    match (
        left.as_ref()
            .is_some_and(|(_, text)| is_dotted_numeric_version_like(text)),
        right
            .as_ref()
            .is_some_and(|(_, text)| is_dotted_numeric_version_like(text)),
    ) {
        (false, false) => None,
        (true, false) => left.map(|(span, _)| span),
        (false, true) | (true, true) => right.map(|(span, _)| span),
    }
}

// Match the SC2072 oracle's decimal-like shape: one dot with digits on both
// sides. Multi-segment versions are intentionally outside C087 unless the
// other operand has this decimal form.
fn is_dotted_numeric_version_like(text: &str) -> bool {
    let Some((left, right)) = text.split_once('.') else {
        return false;
    };

    !left.is_empty()
        && !right.is_empty()
        && !right.contains('.')
        && left.bytes().all(|byte| byte.is_ascii_digit())
        && right.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_decimal_version_comparisons_in_double_brackets() {
        let source = "\
#!/bin/bash
[[ $ver < 1.27 ]]
[[ 1.2 < $ver ]]
[[ 1.2.3 < 2.0 ]]
[[ $ver > 1.27 ]]
[[ 1.2 > $ver ]]
[[ $ver < 1.27 && -n $x ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparisonForVersion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["1.27", "1.2", "2.0", "1.27", "1.2", "1.27"]
        );
    }

    #[test]
    fn reports_quoted_version_literals_and_dotted_version_sources() {
        let source = "\
#!/bin/bash
[[ \"$actual\" > \"0.8\" ]]
[[ \"1.2\" < $ver ]]
[[ $(cat version.txt) < 2.5 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparisonForVersion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"0.8\"", "\"1.2\"", "2.5"]
        );
    }

    #[test]
    fn reports_nested_version_comparisons_inside_logical_expressions() {
        let source = "\
#!/bin/bash
[[ -n $x && $ver < 1.27 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparisonForVersion),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "1.27");
    }

    #[test]
    fn ignores_integer_and_plain_string_lexical_comparisons() {
        let source = "\
#!/bin/bash
[[ $count < 10 ]]
[[ foo < bar ]]
[[ $tag < v1.2 ]]
[[ 1.2.3 < $ver ]]
[[ $ver < 1.2.3 ]]
[[ 1.2.3 < 2.3.4 ]]
[ \"$ver\" \\< 1.27 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparisonForVersion),
        );

        assert!(diagnostics.is_empty());
    }
}
