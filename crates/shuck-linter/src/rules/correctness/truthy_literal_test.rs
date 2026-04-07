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
        .filter(|fact| {
            fact.simple_test().is_some_and(simple_test_matches)
                || fact.conditional().is_some_and(conditional_matches)
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(TruthyLiteralTest, span);
    }
}

fn simple_test_matches(fact: &crate::SimpleTestFact<'_>) -> bool {
    fact.shape() == SimpleTestShape::Truthy
        && fact
            .truthy_operand_class()
            .is_some_and(|class| class.is_fixed_literal())
}

fn conditional_matches(fact: &crate::ConditionalFact<'_>) -> bool {
    matches!(
        fact.root(),
        ConditionalNodeFact::BareWord(word) if word.operand().class().is_fixed_literal()
    )
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
}
