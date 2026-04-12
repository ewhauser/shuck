use shuck_ast::{ConditionalUnaryOp, Span, Word};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, ExpansionContext, Rule,
    SimpleTestOperatorFamily, SimpleTestShape, Violation, WordFactContext, WordQuote,
};

pub struct QuotedCommandInTest;

impl Violation for QuotedCommandInTest {
    fn rule() -> Rule {
        Rule::QuotedCommandInTest
    }

    fn message(&self) -> String {
        "this test uses a quoted pipeline literal instead of command output".to_owned()
    }
}

pub fn quoted_command_in_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| {
            let mut spans = Vec::new();
            if let Some(simple_test) = command.simple_test() {
                spans.extend(collect_simple_test_spans(checker, simple_test));
            }
            if let Some(conditional) = command.conditional() {
                spans.extend(collect_conditional_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedCommandInTest);
}

fn collect_simple_test_spans(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
) -> Vec<Span> {
    simple_test_condition_operand(simple_test)
        .and_then(|word| {
            simple_test_word_looks_like_quoted_pipeline(checker, word.span).then_some(word.span)
        })
        .into_iter()
        .collect()
}

fn simple_test_condition_operand<'a>(
    simple_test: &'a crate::SimpleTestFact<'a>,
) -> Option<&'a Word> {
    match simple_test.effective_shape() {
        SimpleTestShape::Truthy => simple_test.effective_operands().first().copied(),
        SimpleTestShape::Unary
            if simple_test.effective_operator_family() == SimpleTestOperatorFamily::StringUnary =>
        {
            simple_test.effective_operands().get(1).copied()
        }
        SimpleTestShape::Unary
        | SimpleTestShape::Empty
        | SimpleTestShape::Binary
        | SimpleTestShape::Other => None,
    }
}

fn collect_conditional_spans(conditional: &crate::ConditionalFact<'_>, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let unary_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary) => unary.operand().word().map(|word| word.span),
            _ => None,
        })
        .collect::<Vec<_>>();
    let comparison_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() != ConditionalOperatorFamily::Logical =>
            {
                Some([
                    binary.left().word().map(|word| word.span),
                    binary.right().word().map(|word| word.span),
                ])
            }
            _ => None,
        })
        .flatten()
        .flatten()
        .collect::<Vec<_>>();

    match conditional.root() {
        ConditionalNodeFact::BareWord(bare_word)
            if conditional_operand_looks_like_quoted_pipeline(bare_word.operand(), source) =>
        {
            if let Some(word) = bare_word.operand().word() {
                spans.push(word.span);
            }
        }
        ConditionalNodeFact::Unary(unary)
            if conditional_unary_checks_literal_string(unary)
                && conditional_operand_looks_like_quoted_pipeline(unary.operand(), source) =>
        {
            if let Some(word) = unary.operand().word() {
                spans.push(word.span);
            }
        }
        _ => {}
    }

    for node in conditional.nodes().iter().skip(1) {
        match node {
            ConditionalNodeFact::BareWord(bare_word)
                if bare_word.operand().word().is_some_and(|word| {
                    !unary_operand_spans.contains(&word.span)
                        && !comparison_operand_spans.contains(&word.span)
                        && conditional_operand_looks_like_quoted_pipeline(
                            bare_word.operand(),
                            source,
                        )
                }) =>
            {
                if let Some(word) = bare_word.operand().word() {
                    spans.push(word.span);
                }
            }
            ConditionalNodeFact::Unary(unary)
                if conditional_unary_checks_literal_string(unary)
                    && conditional_operand_looks_like_quoted_pipeline(unary.operand(), source) =>
            {
                if let Some(word) = unary.operand().word() {
                    spans.push(word.span);
                }
            }
            _ => {}
        }
    }

    spans
}

fn conditional_unary_checks_literal_string(unary: &crate::ConditionalUnaryFact<'_>) -> bool {
    unary.operator_family() == ConditionalOperatorFamily::StringUnary
        || unary.op() == ConditionalUnaryOp::Not
}

fn simple_test_word_looks_like_quoted_pipeline(checker: &Checker<'_>, span: Span) -> bool {
    checker
        .facts()
        .word_fact(
            span,
            WordFactContext::Expansion(ExpansionContext::CommandArgument),
        )
        .is_some_and(word_fact_looks_like_quoted_pipeline)
}

fn word_fact_looks_like_quoted_pipeline(fact: &crate::WordFact<'_>) -> bool {
    fact.classification().quote == WordQuote::FullyQuoted
        && fact.classification().is_fixed_literal()
        && fact
            .static_text()
            .is_some_and(looks_like_quoted_pipeline_literal)
}

fn conditional_operand_looks_like_quoted_pipeline(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    operand
        .word()
        .zip(operand.word_classification())
        .and_then(|(word, classification)| {
            (classification.quote == WordQuote::FullyQuoted && classification.is_fixed_literal())
                .then(|| crate::static_word_text(word, source))
        })
        .flatten()
        .as_deref()
        .is_some_and(looks_like_quoted_pipeline_literal)
}

fn looks_like_quoted_pipeline_literal(text: &str) -> bool {
    if text.contains("||") {
        return false;
    }

    let segments = text.split('|').map(str::trim).collect::<Vec<_>>();

    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .split_ascii_whitespace()
                    .any(looks_like_command_word)
        })
}

fn looks_like_command_word(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_pipeline_literals_in_simple_tests() {
        let source = "\
#!/bin/sh
[ \"lsmod | grep v4l2loopback\" ]
[ -n \"modprobe | grep snd\" ]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"lsmod | grep v4l2loopback\"", "\"modprobe | grep snd\""]
        );
    }

    #[test]
    fn reports_negated_quoted_pipeline_literals_in_simple_tests() {
        let source = "\
#!/bin/sh
[ ! \"lsmod | grep v4l2loopback\" ]
test ! \"modprobe | grep snd\"
[ ! -n \"lsmod | grep usb\" ]
test ! -z \"modprobe | grep snd_hda\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"lsmod | grep v4l2loopback\"",
                "\"modprobe | grep snd\"",
                "\"lsmod | grep usb\"",
                "\"modprobe | grep snd_hda\""
            ]
        );
    }

    #[test]
    fn reports_nested_quoted_pipeline_literals_in_conditionals() {
        let source = "\
#!/bin/bash
[[ \"$ok\" && \"lsmod | grep v4l2loopback\" ]]
[[ -n \"lsmod | grep v4l2loopback\" && -n \"$ok\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"lsmod | grep v4l2loopback\"",
                "\"lsmod | grep v4l2loopback\""
            ]
        );
    }

    #[test]
    fn reports_negated_quoted_pipeline_literals_in_conditionals() {
        let source = "\
#!/bin/bash
[[ ! \"lsmod | grep v4l2loopback\" ]]
[[ \"$ok\" && ! \"modprobe | grep snd\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"lsmod | grep v4l2loopback\"", "\"modprobe | grep snd\""]
        );
    }

    #[test]
    fn ignores_non_pipeline_literals_and_binary_comparisons() {
        let source = "\
#!/bin/sh
[ \"echo hi\" ]
[ ! \"echo hi\" ]
[ \"foo | bar\" = x ]
[[ \"grep foo file\" = x ]]
[[ -n \"echo hi\" ]]
[[ ! \"echo hi\" ]]
[[ -f \"foo | bar\" ]]
[[ -x \"foo | bar\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert!(diagnostics.is_empty());
    }
}
