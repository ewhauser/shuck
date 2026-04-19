use shuck_ast::Span;

use crate::{Checker, Rule, SimpleTestShape, SimpleTestSyntax, Violation};

pub struct CompoundTestOperator;

impl Violation for CompoundTestOperator {
    fn rule() -> Rule {
        Rule::CompoundTestOperator
    }

    fn message(&self) -> String {
        "split `-a` and `-o` into explicit condition branches".to_owned()
    }
}

pub fn compound_test_operator(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.simple_test())
        .flat_map(|simple_test| simple_test_spans(simple_test, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CompoundTestOperator);
}

fn simple_test_spans(simple_test: &crate::SimpleTestFact<'_>, source: &str) -> Vec<Span> {
    if simple_test.syntax() != SimpleTestSyntax::Bracket {
        return Vec::new();
    }

    match simple_test.effective_shape() {
        SimpleTestShape::Binary | SimpleTestShape::Other => {}
        SimpleTestShape::Empty | SimpleTestShape::Truthy | SimpleTestShape::Unary => {
            return Vec::new();
        }
    }

    simple_test.compound_operator_spans(source)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_compound_test_operators_in_brackets() {
        let source = "\
#!/bin/sh
[ \"$cross\" -a \"$nocross\" ]
[ \"$cross\" -o \"$nocross\" ]
[ \"$a\" = 1 -a \"$b\" = 2 ]
[ \"$a\" = 1 -o \"$b\" = 2 ]
[ ! \"$a\" = 1 -a \"$b\" = 2 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CompoundTestOperator),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-a", "-o", "-a", "-o", "-a"]
        );
    }

    #[test]
    fn ignores_simple_tests_without_compound_operators() {
        let source = "\
#!/bin/sh
[ \"$a\" = 1 ]
[ \"$1\" = \"-o\" ]
[ \"$a\" = 1 && \"$b\" = 2 ]
test \"$a\" = 1 -a \"$b\" = 2
[[ \"$a\" = 1 -a \"$b\" = 2 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CompoundTestOperator),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_grouped_compound_test_operators_in_brackets() {
        let source = "\
#!/bin/sh
[ ! '(' -f \"$left\" -o -f \"$right\" ')' ]
[ '(' '!' -f \"$quoted_left\" -o -f \"$quoted_right\" ')' ]
[ \"$a\" = 1 -a \\( \"$b\" = 2 -o \"$c\" = 3 \\) ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CompoundTestOperator),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-o", "-o", "-a", "-o"]
        );
    }

    #[test]
    fn ignores_malformed_grouped_tests_without_hiding_valid_ones() {
        let source = "\
#!/bin/sh
[ \"$cross\" -a \"$nocross\" ]
[ -n \"${TMPDIR-}\" -a '(' '(' -d \"${TMPDIR-}\" -a -w \"${TMPDIR-}\" ')' -o '!' '(' -d /tmp -a -w /tmp ')' ')' ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CompoundTestOperator),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-a"]
        );
    }
}
