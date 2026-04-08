use shuck_ast::{Span, Word, WordPart, WordPartNode};

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
        .filter_map(|fact| {
            if let Some(simple_test) = fact.simple_test()
                && simple_test_is_constant(simple_test)
            {
                return Some(simple_test_report_span(simple_test, fact.span()));
            }

            fact.conditional()
                .is_some_and(conditional_is_constant)
                .then_some(fact.span())
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || ConstantComparisonTest);
}

fn simple_test_is_constant(fact: &crate::SimpleTestFact<'_>) -> bool {
    match fact.shape() {
        SimpleTestShape::Unary => {
            fact.operator_family() == SimpleTestOperatorFamily::StringUnary
                && (fact
                    .unary_operand_class()
                    .is_some_and(|class| class.is_fixed_literal())
                    || simple_test_unary_affix_span(fact).is_some())
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

fn simple_test_unary_affix_span(fact: &crate::SimpleTestFact<'_>) -> Option<Span> {
    let operand = fact.operands().get(1)?;

    word_scalar_affix_span(operand)
}

fn word_scalar_affix_span(word: &Word) -> Option<Span> {
    if !word.is_fully_double_quoted() {
        return None;
    }

    let mut saw_literal = false;
    let mut saw_scalar_expansion = false;
    let mut literal_span = None;
    if !word_scalar_affix_span_parts(
        &word.parts,
        &mut saw_literal,
        &mut saw_scalar_expansion,
        &mut literal_span,
    ) {
        return None;
    }

    (saw_literal && saw_scalar_expansion)
        .then_some(literal_span)
        .flatten()
}

fn word_scalar_affix_span_parts(
    parts: &[WordPartNode],
    saw_literal: &mut bool,
    saw_scalar_expansion: &mut bool,
    literal_span: &mut Option<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                *saw_literal = true;
                if literal_span.is_none() {
                    *literal_span = Some(part.span);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !word_scalar_affix_span_parts(
                    parts,
                    saw_literal,
                    saw_scalar_expansion,
                    literal_span,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                *saw_scalar_expansion = true;
            }
            WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

fn simple_test_report_span(fact: &crate::SimpleTestFact<'_>, fallback: Span) -> Span {
    match fact.shape() {
        SimpleTestShape::Unary
            if fact.operator_family() == SimpleTestOperatorFamily::StringUnary =>
        {
            simple_test_unary_affix_span(fact)
                .or_else(|| fact.operands().get(1).map(|word| word.span))
                .unwrap_or(fallback)
        }
        _ => fallback,
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

    #[test]
    fn anchors_unary_simple_tests_on_the_operand() {
        let source = "\
#!/bin/bash
[ -n TEMP_NVM_COLORS ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "TEMP_NVM_COLORS");
    }

    #[test]
    fn reports_affixed_quoted_unary_tests_but_not_plain_variables() {
        let source = "\
#!/bin/bash
[ -z \"${rootfs_path}_path\" ]
[ -n \"prefix${rootfs_path}\" ]
[ -n \"$rootfs_path\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["_path", "prefix"]
        );
    }
}
