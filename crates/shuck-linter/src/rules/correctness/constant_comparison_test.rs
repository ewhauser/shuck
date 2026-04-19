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
        .flat_map(|fact| {
            let simple_test_spans = fact
                .simple_test()
                .into_iter()
                .filter(|simple_test| simple_test_is_constant(simple_test))
                .filter_map(simple_test_report_span);
            let conditional_spans = fact
                .conditional()
                .into_iter()
                .flat_map(conditional_report_spans);

            simple_test_spans.chain(conditional_spans)
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || ConstantComparisonTest);
}

fn simple_test_is_constant(fact: &crate::SimpleTestFact<'_>) -> bool {
    fact.shape() == SimpleTestShape::Binary
        && fact.operator_family() == SimpleTestOperatorFamily::StringBinary
        && fact
            .binary_operand_classes()
            .is_some_and(|(left, right)| left.is_fixed_literal() && right.is_fixed_literal())
}

fn simple_test_report_span(fact: &crate::SimpleTestFact<'_>) -> Option<shuck_ast::Span> {
    (fact.shape() == SimpleTestShape::Binary
        && fact.operator_family() == SimpleTestOperatorFamily::StringBinary)
        .then(|| fact.effective_operator_word().map(|word| word.span))
        .flatten()
}

fn conditional_node_is_constant(fact: &crate::ConditionalNodeFact<'_>) -> bool {
    match fact {
        ConditionalNodeFact::Binary(binary) => {
            binary.operator_family() == ConditionalOperatorFamily::StringBinary
                && binary.left().class().is_fixed_literal()
                && binary.right().class().is_fixed_literal()
        }
        ConditionalNodeFact::BareWord(_)
        | ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Other(_) => false,
    }
}

fn conditional_report_spans<'a>(
    fact: &'a crate::ConditionalFact<'a>,
) -> impl Iterator<Item = shuck_ast::Span> + 'a {
    fact.nodes().iter().filter_map(|node| match node {
        ConditionalNodeFact::Binary(binary) if conditional_node_is_constant(node) => {
            Some(binary.operator_span())
        }
        ConditionalNodeFact::BareWord(_)
        | ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Binary(_)
        | ConditionalNodeFact::Other(_) => None,
    })
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
    fn ignores_unary_string_tests_that_belong_to_c019() {
        let source = "\
#!/bin/bash
[ -n foo ]
[[ -z bar ]]
[ -z \"${rootfs_path}_path\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn anchors_binary_simple_tests_on_the_operator() {
        let source = "\
#!/bin/bash
[ left = right ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "=");
    }

    #[test]
    fn reports_nested_constant_conditionals_on_the_operator() {
        let source = "\
#!/bin/bash
if [[ \"$value\" = ok || \"@TERMUX_PACKAGE_FORMAT@\" = \"pacman\" ]]; then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "=");
    }
}
