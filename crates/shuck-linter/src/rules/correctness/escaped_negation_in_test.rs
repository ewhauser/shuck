use shuck_ast::Span;

use crate::{Checker, Rule, SimpleTestFact, SimpleTestShape, Violation, static_word_text};

pub struct EscapedNegationInTest;

impl Violation for EscapedNegationInTest {
    fn rule() -> Rule {
        Rule::EscapedNegationInTest
    }

    fn message(&self) -> String {
        "write ! directly when negating a test".to_owned()
    }
}

pub fn escaped_negation_in_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.simple_test()
                .and_then(|simple_test| report_span(simple_test, checker.source()))
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || EscapedNegationInTest);
}

fn report_span(fact: &SimpleTestFact<'_>, source: &str) -> Option<Span> {
    let leading = fact.operands().first().copied()?;
    if leading.span.slice(source) != "\\!" {
        return None;
    }

    escaped_negation_is_operator(fact, source).then_some(leading.span)
}

fn escaped_negation_is_operator(fact: &SimpleTestFact<'_>, source: &str) -> bool {
    match fact.shape() {
        SimpleTestShape::Unary => true,
        SimpleTestShape::Binary => fact
            .operands()
            .get(1)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .is_some_and(|operator| !is_simple_test_binary_operator(operator)),
        SimpleTestShape::Other => fact
            .operands()
            .get(2)
            .and_then(|word| static_word_text(word, source))
            .as_deref()
            .is_some_and(is_simple_test_binary_operator),
        SimpleTestShape::Empty | SimpleTestShape::Truthy => false,
    }
}

fn is_simple_test_binary_operator(operator: &str) -> bool {
    matches!(
        operator,
        "=" | "=="
            | "!="
            | "-a"
            | "-o"
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-ef"
            | "-nt"
            | "-ot"
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_escaped_negation_in_simple_tests() {
        let source = "\
#!/bin/bash
[ \\! -f \"$file\" ]
test \\! -n \"$value\"
[ \\! \"$value\" = ok ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\\!", "\\!", "\\!"]
        );
    }

    #[test]
    fn ignores_plain_negation_truthy_literals_and_non_leading_bangs() {
        let source = "\
#!/bin/bash
[ ! -f \"$file\" ]
test !
[ \\! = \"$value\" ]
[ \\! -a foo ]
[ \\! -o foo ]
[ \\! -eq 1 ]
[ \"$value\" = \\! ]
[[ \\! -f \"$file\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EscapedNegationInTest),
        );

        assert!(diagnostics.is_empty());
    }
}
