use shuck_ast::{ConditionalBinaryOp, Span};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperandFact, Rule, SimpleTestShape, Violation,
    WordQuote, static_word_text,
};

pub struct StringComparedWithEq;

impl Violation for StringComparedWithEq {
    fn rule() -> Rule {
        Rule::StringComparedWithEq
    }

    fn message(&self) -> String {
        "this comparison uses `-eq` with a string value".to_owned()
    }
}

pub fn string_compared_with_eq(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_string_eq_span(simple_test, source)
            {
                spans.push(span);
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_string_eq_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StringComparedWithEq);
}

fn simple_test_string_eq_span(
    simple_test: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Option<Span> {
    if simple_test.effective_shape() != SimpleTestShape::Binary {
        return None;
    }

    let operands = simple_test.effective_operands();
    if static_word_text(operands.get(1)?, source).as_deref() != Some("-eq") {
        return None;
    }

    if simple_test_operand_looks_like_string_value(simple_test, 0, source)
        || simple_test_operand_looks_like_string_value(simple_test, 2, source)
    {
        Some(operands[1].span)
    } else {
        None
    }
}

fn conditional_string_eq_spans(
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.op() == ConditionalBinaryOp::ArithmeticEq =>
            {
                if conditional_operand_looks_like_string_value(binary.left(), source)
                    || conditional_operand_looks_like_string_value(binary.right(), source)
                {
                    Some(binary.operator_span())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn simple_test_operand_looks_like_string_value(
    simple_test: &crate::SimpleTestFact<'_>,
    index: usize,
    source: &str,
) -> bool {
    simple_test
        .effective_operand_class(index)
        .is_some_and(|class| class.is_fixed_literal())
        && simple_test
            .effective_operands()
            .get(index)
            .and_then(|word| static_word_text(word, source))
            .is_some_and(|text| !looks_like_decimal_integer(&text))
}

fn conditional_operand_looks_like_string_value(
    operand: ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    operand
        .word()
        .zip(operand.word_classification())
        .is_some_and(|(word, classification)| {
            classification.is_fixed_literal()
                && static_word_text(word, source).is_some_and(|text| {
                    !(looks_like_decimal_integer(&text)
                        || (operand.quote() == Some(WordQuote::Unquoted)
                            && looks_like_shell_variable_name(&text)))
                })
        })
}

fn looks_like_decimal_integer(text: &str) -> bool {
    let text = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_string_values_compared_with_numeric_eq() {
        let source = "\
#!/bin/bash
[[ $VER -eq \"latest\" ]]
[ $VER -eq \"latest\" ]
[[ \"latest\" -eq $VER ]]
[ \"latest\" -eq $VER ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-eq", "-eq", "-eq", "-eq"]
        );
    }

    #[test]
    fn ignores_numeric_and_non_eq_comparisons() {
        let source = "\
#!/bin/bash
[[ 1 -eq 2 ]]
[ 1 -eq 2 ]
[[ $VER = latest ]]
[[ $VER -ne latest ]]
[[ $VER -eq 123 ]]
[ $VER -eq +123 ]
[[ __iterator -eq 0 || -n \"${__next}\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert!(diagnostics.is_empty());
    }
}
