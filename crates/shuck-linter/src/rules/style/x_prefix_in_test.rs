use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Diagnostic, Edit, Fix,
    FixAvailability, Rule, SimpleTestSyntax, Violation, leading_literal_word_prefix,
};

pub struct XPrefixInTest;

impl Violation for XPrefixInTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::XPrefixInTest
    }

    fn message(&self) -> String {
        "this comparison uses the legacy x-prefix idiom".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the x-prefix comparison padding".to_owned())
    }
}

pub fn x_prefix_in_test(checker: &mut Checker) {
    let source = checker.source();
    let reports = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut reports = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                reports.extend(simple_test_reports(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                reports.extend(conditional_reports(conditional, source));
            }
            reports
        })
        .collect::<Vec<_>>();

    for report in reports {
        checker.report_diagnostic_dedup(
            Diagnostic::new(XPrefixInTest, report.diagnostic_span)
                .with_fix(x_prefix_fix(source, &report)),
        );
    }
}

struct XPrefixComparisonReport {
    diagnostic_span: Span,
    left_span: Span,
    right_span: Span,
}

fn simple_test_reports(
    simple_test: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Vec<XPrefixComparisonReport> {
    if simple_test.syntax() != SimpleTestSyntax::Test
        && simple_test.syntax() != SimpleTestSyntax::Bracket
    {
        return Vec::new();
    }

    simple_test
        .string_binary_expression_words(source)
        .into_iter()
        .filter_map(|(left, _operator, right)| {
            (word_has_x_prefix(left, source) && word_has_x_prefix(right, source)).then_some(
                XPrefixComparisonReport {
                    diagnostic_span: left.span,
                    left_span: left.span,
                    right_span: right.span,
                },
            )
        })
        .collect()
}

fn conditional_reports(
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<XPrefixComparisonReport> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
            {
                if conditional_operand_has_x_prefix(binary.left(), source)
                    && conditional_operand_has_x_prefix(binary.right(), source)
                {
                    let left_span = conditional_operand_span(binary.left());
                    let right_span = conditional_operand_span(binary.right());
                    Some(XPrefixComparisonReport {
                        diagnostic_span: left_span,
                        left_span,
                        right_span,
                    })
                } else {
                    None
                }
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .collect()
}

fn conditional_operand_span(operand: crate::ConditionalOperandFact<'_>) -> Span {
    operand
        .word()
        .map(|word| word.span)
        .unwrap_or_else(|| operand.expression().span())
}

fn conditional_operand_has_x_prefix(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    operand
        .word()
        .map(|word| word_has_x_prefix(word, source))
        .unwrap_or_else(|| has_legacy_x_prefix(operand.expression().span().slice(source)))
}

fn word_has_x_prefix(word: &shuck_ast::Word, source: &str) -> bool {
    has_legacy_x_prefix(&leading_literal_word_prefix(word, source))
}

fn has_legacy_x_prefix(text: &str) -> bool {
    matches!(text.as_bytes().first(), Some(b'x' | b'X'))
}

fn x_prefix_fix(source: &str, report: &XPrefixComparisonReport) -> Fix {
    Fix::unsafe_edits([
        Edit::replacement(
            x_prefix_operand_replacement(source, report.left_span),
            report.left_span,
        ),
        Edit::replacement(
            x_prefix_operand_replacement(source, report.right_span),
            report.right_span,
        ),
    ])
}

fn x_prefix_operand_replacement(source: &str, span: Span) -> String {
    let text = span.slice(source);
    let body = strip_simple_quotes(text).unwrap_or(text);
    let without_prefix = body
        .char_indices()
        .nth(1)
        .map_or("", |(offset, _)| &body[offset..]);
    format!("\"{}\"", without_prefix.replace('"', "\\\""))
}

fn strip_simple_quotes(text: &str) -> Option<&str> {
    text.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_x_prefix_comparisons_in_simple_tests_and_conditionals() {
        let source = "\
#!/bin/bash
[ x = x ]
[ X = Xbar ]
[ \"Xfoo\" = \"X$browser\" ]
[ X = \"X$browser\" ]
test \"x$browser\" != \"x\"
[ \"X`id -u`\" = \"X0\" -a -z \"$RUN_AS_USER\" ]
[ \"pkg-config --exists libffmpegthumbnailer\" -a \"x${VIDEO_THUMBNAILS}\" != \"xno\" ]
[ X = X ]
[[ X = Xbar ]]
[[ \"X$browser\" != \"X\" ]]
[ \"x$browser\" = \"x$other\" ]
[ x = \"x$browser\" ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::XPrefixInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "x",
                "X",
                "\"Xfoo\"",
                "X",
                "\"x$browser\"",
                "\"X`id -u`\"",
                "\"x${VIDEO_THUMBNAILS}\"",
                "X",
                "X",
                "\"X$browser\"",
                "\"x$browser\"",
                "x"
            ]
        );
    }

    #[test]
    fn ignores_non_x_prefix_or_single_sided_comparisons() {
        let source = "\
#!/bin/bash
[ \"x$browser\" = \"$other\" ]
[ x = \"$browser\" ]
[ \"X$browser\" = \"$other\" ]
[ X = \"$browser\" ]
[ Xfoo = y ]
[ \"X$browser\" = \"Y\" ]
[[ prefix$browser = prefix ]]
[[ Prefix$browser = Prefix ]]
[[ x = y ]]
[[ X = Y ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::XPrefixInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_remove_x_prefixes_from_both_operands() {
        let source = "#!/bin/bash\n[ \"x$browser\" = \"x\" ]\n[[ X = Xbar ]]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::XPrefixInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\n[ \"$browser\" = \"\" ]\n[[ \"\" = \"bar\" ]]\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
