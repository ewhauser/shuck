use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, SimpleTestOperatorFamily,
    SimpleTestShape, Violation,
};

pub struct ConstantComparisonTest;

impl Violation for ConstantComparisonTest {
    fn rule() -> Rule {
        Rule::ConstantComparisonTest
    }

    fn message(&self) -> String {
        "this comparison only checks fixed literals".to_owned()
    }
}

pub fn constant_comparison_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.simple_test().is_some_and(simple_test_is_constant)
                || fact.conditional().is_some_and(conditional_is_constant)
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || ConstantComparisonTest);
}

fn simple_test_is_constant(fact: &crate::SimpleTestFact<'_>) -> bool {
    match fact.shape() {
        SimpleTestShape::Unary => {
            fact.operator_family() == SimpleTestOperatorFamily::StringUnary
                && fact
                    .unary_operand_class()
                    .is_some_and(|class| class.is_fixed_literal())
        }
        SimpleTestShape::Binary => {
            fact.operator_family() == SimpleTestOperatorFamily::StringBinary
                && fact.binary_operand_classes().is_some_and(|(left, right)| {
                    left.is_fixed_literal() && right.is_fixed_literal()
                })
        }
        SimpleTestShape::Empty | SimpleTestShape::Truthy | SimpleTestShape::Other => false,
    }
}

fn conditional_is_constant(fact: &crate::ConditionalFact<'_>) -> bool {
    match fact.root() {
        ConditionalNodeFact::Binary(binary) => {
            binary.operator_family() == ConditionalOperatorFamily::StringBinary
                && binary.left().class().is_fixed_literal()
                && binary.right().class().is_fixed_literal()
        }
        ConditionalNodeFact::Unary(unary) => {
            unary.operator_family() == ConditionalOperatorFamily::StringUnary
                && unary.operand().class().is_fixed_literal()
        }
        ConditionalNodeFact::BareWord(_) | ConditionalNodeFact::Other(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_runtime_sensitive_and_non_string_comparisons() {
        let source = "\
#!/bin/bash
[ ~ = /tmp ]
[ *.sh = target ]
[ {a,b} = foo ]
[[ i -ge 10 ]]
[ \"/a\" -ot \"/b\" ]
[[ left == *.sh ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_constant_unary_string_tests() {
        let source = "\
#!/bin/bash
[ -n foo ]
[[ -z bar ]]
[ -n ~ ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
