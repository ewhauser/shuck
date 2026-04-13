use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, SimpleTestOperatorFamily,
    SimpleTestShape, SimpleTestSyntax, Violation, leading_literal_word_prefix,
};

pub struct XPrefixInTest;

impl Violation for XPrefixInTest {
    fn rule() -> Rule {
        Rule::XPrefixInTest
    }

    fn message(&self) -> String {
        "this comparison uses the legacy x-prefix idiom".to_owned()
    }
}

pub fn x_prefix_in_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_span(simple_test, source)
            {
                spans.push(span);
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || XPrefixInTest);
}

fn simple_test_span(simple_test: &crate::SimpleTestFact<'_>, source: &str) -> Option<Span> {
    if simple_test.syntax() != SimpleTestSyntax::Test
        && simple_test.syntax() != SimpleTestSyntax::Bracket
    {
        return None;
    }
    if simple_test.effective_shape() != SimpleTestShape::Binary
        || simple_test.effective_operator_family() != SimpleTestOperatorFamily::StringBinary
    {
        return None;
    }

    let operands = simple_test.effective_operands();
    if operands.len() != 3 {
        return None;
    }

    if word_has_x_prefix(operands[0], source) && word_has_x_prefix(operands[2], source) {
        Some(operands[0].span)
    } else {
        None
    }
}

fn conditional_spans(conditional: &crate::ConditionalFact<'_>, source: &str) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
            {
                if conditional_operand_has_x_prefix(binary.left(), source)
                    && conditional_operand_has_x_prefix(binary.right(), source)
                {
                    binary.left().word().map(|word| word.span).or_else(|| {
                        let span = binary.left().expression().span();
                        Some(span)
                    })
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn conditional_operand_has_x_prefix(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    operand
        .word()
        .map(|word| word_has_x_prefix(word, source))
        .unwrap_or_else(|| operand.expression().span().slice(source).starts_with('x'))
}

fn word_has_x_prefix(word: &shuck_ast::Word, source: &str) -> bool {
    leading_literal_word_prefix(word, source).starts_with('x')
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_x_prefix_comparisons_in_simple_tests_and_conditionals() {
        let source = "\
#!/bin/bash
[ x = x ]
[ x = xbar ]
[ xfoo = xbar ]
[ \"xfoo\" = \"x$browser\" ]
test \"x$browser\" != \"x\"
[[ x = xbar ]]
[[ \"x$browser\" != \"x\" ]]
[ \"x$browser\" = \"x$other\" ]
[ x = \"x$browser\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::XPrefixInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "x",
                "x",
                "xfoo",
                "\"xfoo\"",
                "\"x$browser\"",
                "x",
                "\"x$browser\"",
                "\"x$browser\"",
                "x"
            ]
        );
    }

    #[test]
    fn ignores_non_x_prefix_or_single_sided_comparisons() {
        let source = "\
#!/bin/bash
[ \"x$browser\" = \"$other\" ]
[ x = \"$browser\" ]
[ xfoo = y ]
[ \"x$browser\" = \"y\" ]
[[ prefix$browser = prefix ]]
[[ x = y ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::XPrefixInTest));

        assert!(diagnostics.is_empty());
    }
}
