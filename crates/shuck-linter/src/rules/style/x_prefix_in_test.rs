use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Rule, SimpleTestSyntax, Violation,
    leading_literal_word_prefix,
};

pub struct XPrefixInTest;

impl Violation for XPrefixInTest {
    fn rule() -> Rule {
        Rule::XPrefixInTest
    }

    fn message(&self) -> String {
        "this comparison uses the legacy x-prefix idiom".to_owned()
    }
}

pub fn x_prefix_in_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                spans.extend(simple_test_spans(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || XPrefixInTest);
}

fn simple_test_spans(simple_test: &crate::SimpleTestFact<'_>, source: &str) -> Vec<Span> {
    if simple_test.syntax() != SimpleTestSyntax::Test
        && simple_test.syntax() != SimpleTestSyntax::Bracket
    {
        return Vec::new();
    }

    simple_test
        .string_binary_expression_words(source)
        .into_iter()
        .filter_map(|(left, _operator, right)| {
            (word_has_x_prefix(left, source) && word_has_x_prefix(right, source))
                .then_some(left.span)
        })
        .collect()
}

fn conditional_spans(conditional: &crate::ConditionalFact<'_>, source: &str) -> Vec<Span> {
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
                    binary.left().word().map(|word| word.span).or_else(|| {
                        let span = binary.left().expression().span();
                        Some(span)
                    })
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
