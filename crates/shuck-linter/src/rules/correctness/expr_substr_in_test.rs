use shuck_ast::Span;

use crate::rules::common::word::WordClassification;
use crate::{
    Checker, CommandSubstitutionKind, ConditionalNodeFact, ConditionalOperatorFamily,
    ExpansionContext, Rule, SimpleTestOperatorFamily, SimpleTestShape, SubstitutionFact, Violation,
    WordFactContext,
};

pub struct ExprSubstrInTest;

impl Violation for ExprSubstrInTest {
    fn rule() -> Rule {
        Rule::ExprSubstrInTest
    }

    fn message(&self) -> String {
        "this test uses `expr substr` instead of shell substring expansion".to_owned()
    }
}

pub fn expr_substr_in_test(checker: &mut Checker) {
    let expr_substr_spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.effective_name_is("expr")
                && fact
                    .options()
                    .expr()
                    .is_some_and(|expr| expr.uses_substr_string_form())
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    let substitutions = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_expr_substr_span(
                    checker,
                    simple_test,
                    &substitutions,
                    &expr_substr_spans,
                )
            {
                spans.push(span);
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_expr_substr_spans(
                    conditional,
                    &substitutions,
                    &expr_substr_spans,
                ));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExprSubstrInTest);
}

fn simple_test_expr_substr_span(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
    substitutions: &[&SubstitutionFact],
    expr_substr_spans: &[Span],
) -> Option<Span> {
    if simple_test.effective_shape() != SimpleTestShape::Binary
        || simple_test.effective_operator_family() != SimpleTestOperatorFamily::StringBinary
    {
        return None;
    }

    simple_test.effective_operands().iter().find_map(|word| {
        test_operand_contains_expr_substr(checker, word, substitutions, expr_substr_spans)
    })
}

fn conditional_expr_substr_spans(
    conditional: &crate::ConditionalFact<'_>,
    substitutions: &[&SubstitutionFact],
    expr_substr_spans: &[Span],
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
            {
                conditional_operand_contains_expr_substr(
                    binary.left().word(),
                    binary.left().word_classification(),
                    substitutions,
                    expr_substr_spans,
                )
                .or_else(|| {
                    conditional_operand_contains_expr_substr(
                        binary.right().word(),
                        binary.right().word_classification(),
                        substitutions,
                        expr_substr_spans,
                    )
                })
            }
            _ => None,
        })
        .collect()
}

fn test_operand_contains_expr_substr(
    checker: &Checker<'_>,
    word: &shuck_ast::Word,
    substitutions: &[&SubstitutionFact],
    expr_substr_spans: &[Span],
) -> Option<Span> {
    checker
        .facts()
        .word_fact(
            word.span,
            WordFactContext::Expansion(ExpansionContext::CommandArgument),
        )
        .filter(|fact| fact.classification().has_plain_command_substitution())
        .and_then(|_| {
            command_substitution_contains_expr_substr(word.span, substitutions, expr_substr_spans)
        })
}

fn conditional_operand_contains_expr_substr(
    word: Option<&shuck_ast::Word>,
    classification: Option<WordClassification>,
    substitutions: &[&SubstitutionFact],
    expr_substr_spans: &[Span],
) -> Option<Span> {
    let word = word?;
    let classification = classification?;
    if !classification.has_plain_command_substitution() {
        return None;
    }

    command_substitution_contains_expr_substr(word.span, substitutions, expr_substr_spans)
}

fn command_substitution_contains_expr_substr(
    word_span: Span,
    substitutions: &[&SubstitutionFact],
    expr_substr_spans: &[Span],
) -> Option<Span> {
    substitutions
        .iter()
        .copied()
        .filter(|substitution| {
            substitution.kind() == CommandSubstitutionKind::Command
                && substitution.host_word_span() == word_span
        })
        .find_map(|substitution| {
            expr_substr_spans
                .iter()
                .copied()
                .find(|expr_span| span_contains(substitution.span(), *expr_span))
                .map(|_| substitution.span())
        })
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_expr_substr_inside_test_comparisons() {
        let source = "\
#!/bin/sh
# shellcheck disable=2046
if test \"$(expr substr $(uname -s) 1 5)\" = \"Linux\"; then echo linux; fi
if [[ \"$(expr substr $(uname -s) 1 5)\" == \"Linux\" ]]; then echo linux; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprSubstrInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(expr substr $(uname -s) 1 5)",
                "$(expr substr $(uname -s) 1 5)"
            ]
        );
    }

    #[test]
    fn ignores_non_substr_expr_and_non_test_contexts() {
        let source = "\
#!/bin/sh
x=$(expr 1 + 2)
test \"$x\" = \"Linux\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExprSubstrInTest));

        assert!(diagnostics.is_empty());
    }
}
