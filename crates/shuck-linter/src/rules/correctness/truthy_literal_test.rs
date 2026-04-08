use shuck_ast::Span;

use crate::{Checker, ConditionalNodeFact, Rule, SimpleTestShape, Violation};

pub struct TruthyLiteralTest;

impl Violation for TruthyLiteralTest {
    fn rule() -> Rule {
        Rule::TruthyLiteralTest
    }

    fn message(&self) -> String {
        "this test checks a fixed literal instead of runtime data".to_owned()
    }
}

pub fn truthy_literal_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            if let Some(simple_test) = fact.simple_test()
                && simple_test_matches(simple_test)
            {
                return simple_test_report_span(simple_test);
            }

            fact.conditional().and_then(conditional_report_span)
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || TruthyLiteralTest);
}

fn simple_test_matches(fact: &crate::SimpleTestFact<'_>) -> bool {
    fact.shape() == SimpleTestShape::Truthy
        && fact
            .truthy_operand_class()
            .is_some_and(|class| class.is_fixed_literal())
}

fn simple_test_report_span(fact: &crate::SimpleTestFact<'_>) -> Option<Span> {
    (fact.shape() == SimpleTestShape::Truthy)
        .then(|| fact.operands().first().map(|word| word.span))
        .flatten()
}

fn conditional_report_span(fact: &crate::ConditionalFact<'_>) -> Option<Span> {
    match fact.root() {
        ConditionalNodeFact::BareWord(word) if word.operand().class().is_fixed_literal() => {
            word.operand().word().map(|word| word.span)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_runtime_sensitive_literal_words() {
        let source = "\
#!/bin/bash
[ ~ ]
test ~user
test x=~
test *.sh
[ {a,b} ]
[[ ~ ]]
[[ *.sh ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8]
        );
    }

    #[test]
    fn still_reports_plain_fixed_literals() {
        let source = "\
#!/bin/bash
[ 1 ]
test foo
[[ bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn anchors_truthy_simple_tests_on_the_operand() {
        let source = "\
#!/bin/bash
[ \"\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"\"");
    }
}
