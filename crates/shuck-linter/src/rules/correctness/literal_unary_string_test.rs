use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, SimpleTestOperatorFamily,
    SimpleTestShape, Violation,
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
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.simple_test().is_some_and(simple_test_matches)
                || fact.conditional().is_some_and(conditional_matches)
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(LiteralUnaryStringTest, span);
    }
}

fn simple_test_matches(fact: &crate::SimpleTestFact<'_>) -> bool {
    fact.shape() == SimpleTestShape::Unary
        && fact.operator_family() == SimpleTestOperatorFamily::StringUnary
        && fact
            .unary_operand_class()
            .is_some_and(|class| class.is_fixed_literal())
}

fn conditional_matches(fact: &crate::ConditionalFact<'_>) -> bool {
    matches!(
        fact.root(),
        ConditionalNodeFact::Unary(unary)
            if unary.operator_family() == ConditionalOperatorFamily::StringUnary
                && unary.operand().class().is_fixed_literal()
    )
}
