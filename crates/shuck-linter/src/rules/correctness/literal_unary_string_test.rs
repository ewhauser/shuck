use shuck_ast::{Span, Word};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, Violation,
    double_quoted_scalar_affix_span, quoted_word_content_span_in_source,
};

pub struct LiteralUnaryStringTest;

impl Violation for LiteralUnaryStringTest {
    fn rule() -> Rule {
        Rule::LiteralUnaryStringTest
    }

    fn message(&self) -> String {
        "this string test checks a fixed literal".to_owned()
    }
}

pub fn literal_unary_string_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                spans.extend(simple_test_report_spans(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_report_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LiteralUnaryStringTest);
}

fn simple_test_report_spans(fact: &crate::SimpleTestFact<'_>, source: &str) -> Vec<Span> {
    fact.string_unary_expression_words(source)
        .into_iter()
        .filter_map(|(_, operand)| simple_test_operand_span(fact, operand, source))
        .collect()
}

fn simple_test_operand_span(
    fact: &crate::SimpleTestFact<'_>,
    operand: &Word,
    source: &str,
) -> Option<Span> {
    let index = fact
        .effective_operands()
        .iter()
        .position(|candidate| candidate.span == operand.span)?;

    if fact
        .effective_operand_class(index)
        .is_some_and(|class| class.is_fixed_literal())
    {
        return quoted_word_content_span_in_source(operand, source).or(Some(operand.span));
    }

    double_quoted_scalar_affix_span(operand)
}

fn conditional_report_spans(fact: &crate::ConditionalFact<'_>, source: &str) -> Vec<Span> {
    fact.nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary)
                if unary.operator_family() == ConditionalOperatorFamily::StringUnary =>
            {
                conditional_operand_span(unary.operand(), source)
            }
            _ => None,
        })
        .collect()
}

fn conditional_operand_span(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<Span> {
    if operand.class().is_fixed_literal() {
        return operand
            .word()
            .map(|word| quoted_word_content_span_in_source(word, source).unwrap_or(word.span))
            .or_else(|| Some(operand.expression().span()));
    }

    operand.word().and_then(double_quoted_scalar_affix_span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nested_unary_string_tests_in_simple_and_conditional_logical_chains() {
        let source = "\
#!/bin/bash
[ -z foo -o -z \"$path\" ]
[[ -z \"name\" || -z \"$path\" ]]
[[ ! -n bar ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "name", "bar"]
        );
    }

    #[test]
    fn reports_quoted_scalar_affixes_that_make_unary_string_tests_constant() {
        let source = "\
#!/bin/bash
[ -z \"${rootfs_path}_path\" ]
[[ -n \"prefix${rootfs_path}\" ]]
[ -n \"$rootfs_path\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["_path", "prefix"]
        );
    }
}
