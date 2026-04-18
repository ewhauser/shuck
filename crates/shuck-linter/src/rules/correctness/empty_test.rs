use crate::{Checker, Rule, SimpleTestShape, SimpleTestSyntax, Violation};

pub struct EmptyTest;

impl Violation for EmptyTest {
    fn rule() -> Rule {
        Rule::EmptyTest
    }

    fn message(&self) -> String {
        "test expression is empty".to_owned()
    }
}

pub fn empty_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.simple_test()
                .map(|simple_test| (fact.span(), simple_test))
        })
        .filter(|(_, fact)| {
            fact.syntax() == SimpleTestSyntax::Bracket
                && fact.shape() == SimpleTestShape::Empty
                && !fact.empty_test_suppressed()
        })
        .map(|(span, _)| span)
        .collect::<Vec<_>>();

    checker.report_all(spans, || EmptyTest);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_empty_test_builtin_calls() {
        let source = "\
#!/bin/sh
test
test || __() { :; }
test \"\" && exit
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EmptyTest));

        assert!(diagnostics.is_empty());
    }
}
