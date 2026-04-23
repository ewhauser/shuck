use shuck_ast::{ConditionalBinaryOp, static_word_text};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Edit, Fix, FixAvailability, Rule,
    SimpleTestOperatorFamily, SimpleTestShape, Violation,
};

pub struct ConstantComparisonTest;

impl Violation for ConstantComparisonTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::ConstantComparisonTest
    }

    fn message(&self) -> String {
        "this comparison only checks fixed literals".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the constant comparison with its known result".to_owned())
    }
}

pub fn constant_comparison_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let simple_test_diagnostics = fact
                .simple_test()
                .into_iter()
                .filter_map(|simple_test| simple_test_diagnostic(simple_test, source));
            let conditional_diagnostics = fact
                .conditional()
                .into_iter()
                .flat_map(|conditional| conditional_diagnostics(conditional, source));

            simple_test_diagnostics.chain(conditional_diagnostics)
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
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

fn simple_test_diagnostic(
    fact: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Option<crate::Diagnostic> {
    if !simple_test_is_constant(fact) {
        return None;
    }

    let span = simple_test_report_span(fact)?;
    let diagnostic = crate::Diagnostic::new(ConstantComparisonTest, span);
    match simple_test_fix(fact, source) {
        Some(fix) => Some(diagnostic.with_fix(fix)),
        None => Some(diagnostic),
    }
}

fn simple_test_fix(fact: &crate::SimpleTestFact<'_>, source: &str) -> Option<Fix> {
    let (left, operator, right) = fact
        .string_binary_expression_words(source)
        .into_iter()
        .next()?;
    let left_text = static_word_text(left, source)?;
    let operator_text = static_word_text(operator, source)?;
    let right_text = static_word_text(right, source)?;
    let result = evaluate_simple_test_comparison(&left_text, &operator_text, &right_text)?;
    let replacement_span = shuck_ast::Span::from_positions(left.span.start, right.span.end);

    Some(Fix::safe_edit(Edit::replacement(
        constant_boolean_replacement(result),
        replacement_span,
    )))
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

fn conditional_diagnostics<'a>(
    fact: &'a crate::ConditionalFact<'a>,
    source: &'a str,
) -> impl Iterator<Item = crate::Diagnostic> + 'a {
    fact.nodes().iter().filter_map(|node| match node {
        ConditionalNodeFact::Binary(binary) if conditional_node_is_constant(node) => {
            let diagnostic = crate::Diagnostic::new(ConstantComparisonTest, binary.operator_span());
            match conditional_fix(binary, source) {
                Some(fix) => Some(diagnostic.with_fix(fix)),
                None => Some(diagnostic),
            }
        }
        ConditionalNodeFact::BareWord(_)
        | ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Binary(_)
        | ConditionalNodeFact::Other(_) => None,
    })
}

fn conditional_fix(binary: &crate::ConditionalBinaryFact<'_>, source: &str) -> Option<Fix> {
    let left_text = conditional_operand_static_text(binary.left(), source)?;
    let right_text = conditional_operand_static_text(binary.right(), source)?;
    let result = evaluate_conditional_comparison(binary.op(), &left_text, &right_text)?;

    Some(Fix::safe_edit(Edit::replacement(
        constant_boolean_replacement(result),
        binary.expression().span(),
    )))
}

fn conditional_operand_static_text(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<String> {
    if let Some(word) = operand.word() {
        return static_word_text(word, source).map(|text| text.into_owned());
    }

    operand
        .class()
        .is_fixed_literal()
        .then(|| operand.expression().span().slice(source).to_owned())
}

fn evaluate_simple_test_comparison(left: &str, operator: &str, right: &str) -> Option<bool> {
    match operator {
        "=" | "==" => Some(left == right),
        "!=" => Some(left != right),
        _ => None,
    }
}

fn evaluate_conditional_comparison(
    op: ConditionalBinaryOp,
    left: &str,
    right: &str,
) -> Option<bool> {
    match op {
        ConditionalBinaryOp::PatternEqShort | ConditionalBinaryOp::PatternEq => {
            (!looks_like_conditional_pattern(right)).then_some(left == right)
        }
        ConditionalBinaryOp::PatternNe => {
            (!looks_like_conditional_pattern(right)).then_some(left != right)
        }
        _ => None,
    }
}

fn looks_like_conditional_pattern(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '*' | '?' | '[' | ']') {
            return true;
        }

        if matches!(ch, '@' | '!' | '+' | '?') && matches!(chars.peek(), Some('(')) {
            return true;
        }
    }

    false
}

fn constant_boolean_replacement(result: bool) -> &'static str {
    if result { "x" } else { "\"\"" }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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
    fn attaches_safe_fix_metadata_to_simple_constant_comparisons() {
        let source = "#!/bin/bash\n[ 1 = 1 ]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Safe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("replace the constant comparison with its known result")
        );
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

    #[test]
    fn applies_safe_fix_to_simple_and_nested_constant_comparisons() {
        let source = "\
#!/bin/bash
[ 1 = 1 ]
test foo != bar
[[ left == right ]]
[[ \"$value\" = ok || left == right ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ x ]
test x
[[ \"\" ]]
[[ \"$value\" = ok || \"\" ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_ordering_conditionals_unfixed() {
        let source = "\
#!/bin/bash
[[ a < b ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
            Applicability::Safe,
        );

        assert_eq!(result.diagnostics.len(), 1);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C017.sh").as_path(),
            &LinterSettings::for_rule(Rule::ConstantComparisonTest),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C017_fix_C017.sh", result);
        Ok(())
    }
}
