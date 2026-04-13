use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ExpansionContext, Rule, SimpleTestShape, Violation,
    WordFactContext, WordQuote, static_word_text,
};
use shuck_ast::ConditionalBinaryOp;

pub struct MalformedArithmeticInCondition;

impl Violation for MalformedArithmeticInCondition {
    fn rule() -> Rule {
        Rule::MalformedArithmeticInCondition
    }

    fn message(&self) -> String {
        "this test mixes arithmetic syntax into a comparison".to_owned()
    }
}

pub fn malformed_arithmetic_in_condition(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_span(checker, simple_test)
            {
                spans.push(span);
            }
            if let Some(conditional) = fact.conditional()
                && let Some(span) = conditional_span(conditional, source)
            {
                spans.push(span);
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MalformedArithmeticInCondition);
}

fn simple_test_span(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
) -> Option<Span> {
    match simple_test.effective_shape() {
        SimpleTestShape::Binary | SimpleTestShape::Other => {}
        SimpleTestShape::Empty | SimpleTestShape::Truthy | SimpleTestShape::Unary => return None,
    }

    let operands = simple_test.effective_operands();
    if !operands
        .iter()
        .any(|word| simple_test_word_is_bare_operator(checker, word, is_numeric_test_operator))
    {
        return None;
    }

    operands.iter().find_map(|word| {
        simple_test_word_is_bare_operator(checker, word, is_arithmetic_operator)
            .then_some(word.span)
    })
}

fn conditional_span(conditional: &crate::ConditionalFact<'_>, source: &str) -> Option<Span> {
    if !conditional_contains_numeric_comparison(conditional) {
        return None;
    }

    conditional.nodes().iter().find_map(|node| {
        let ConditionalNodeFact::BareWord(word) = node else {
            return None;
        };

        let operand = word.operand();
        let word = operand.word()?;
        let classification = operand.word_classification()?;
        if classification.quote != WordQuote::Unquoted {
            return None;
        }

        static_word_text(word, source)
            .filter(|text| is_arithmetic_operator(text))
            .map(|_| word.span)
    })
}

fn conditional_contains_numeric_comparison(conditional: &crate::ConditionalFact<'_>) -> bool {
    conditional.nodes().iter().any(|node| match node {
        ConditionalNodeFact::Binary(binary) => matches!(
            binary.op(),
            ConditionalBinaryOp::ArithmeticEq
                | ConditionalBinaryOp::ArithmeticNe
                | ConditionalBinaryOp::ArithmeticLe
                | ConditionalBinaryOp::ArithmeticGe
                | ConditionalBinaryOp::ArithmeticLt
                | ConditionalBinaryOp::ArithmeticGt
        ),
        _ => false,
    })
}

fn is_numeric_test_operator(text: &str) -> bool {
    matches!(text, "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge")
}

fn is_arithmetic_operator(text: &str) -> bool {
    matches!(
        text,
        "+" | "-" | "*" | "/" | "%" | "**" | "<<" | ">>" | "&" | "|" | "^"
    )
}

fn simple_test_word_is_bare_operator(
    checker: &Checker<'_>,
    word: &shuck_ast::Word,
    predicate: fn(&str) -> bool,
) -> bool {
    let Some(fact) = checker.facts().word_fact(
        word.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    ) else {
        return false;
    };

    fact.classification().quote == WordQuote::Unquoted
        && fact.classification().is_fixed_literal()
        && fact.static_text().is_some_and(predicate)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bare_arithmetic_tokens_used_with_numeric_comparisons() {
        let source = "\
#!/bin/bash
if [ 1 + 2 -eq 3 ]; then :; fi
if test 1 + 2 -eq 3; then :; fi
if [[ 1 + 2 -eq 3 ]]; then :; fi
if [ \"$x\" + 1 -eq 2 ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MalformedArithmeticInCondition),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["+", "+", "+"]
        );
    }

    #[test]
    fn ignores_valid_arithmetic_expansions_and_plain_comparisons() {
        let source = "\
#!/bin/bash
if [ $((1 + 2)) -eq 3 ]; then :; fi
if [[ $((1 + 2)) -eq 3 ]]; then :; fi
if [ 1 + 2 = 3 ]; then :; fi
if [[ foo -eq bar ]]; then :; fi
if [ $# -ge 2 -a \"$2\" = \"-\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MalformedArithmeticInCondition),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
