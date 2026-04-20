use shuck_ast::Span;

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, ExpansionContext, Rule,
    SimpleTestOperatorFamily, SimpleTestShape, Violation, WordFactContext, WordQuote,
    double_quoted_scalar_affix_span, is_shell_variable_name,
};

pub struct ConstantInTestAssignment;

impl Violation for ConstantInTestAssignment {
    fn rule() -> Rule {
        Rule::ConstantInTestAssignment
    }

    fn message(&self) -> String {
        "this comparison hides an assignment-looking value".to_owned()
    }
}

pub fn constant_in_test_assignment(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut spans = Vec::new();
            if let Some(simple_test) = fact.simple_test()
                && let Some(span) = simple_test_assignment_span(checker, simple_test, source)
            {
                spans.push(span);
            }
            if let Some(conditional) = fact.conditional() {
                spans.extend(conditional_assignment_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ConstantInTestAssignment);
}

fn simple_test_assignment_span(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Option<Span> {
    if simple_test.effective_shape() != SimpleTestShape::Binary
        || simple_test.effective_operator_family() != SimpleTestOperatorFamily::StringBinary
    {
        return None;
    }

    let operands = simple_test.effective_operands();
    if operands.len() != 3 {
        return None;
    }

    if simple_test_operand_looks_like_assignment_like(checker, operands[0], source)
        || simple_test_operand_looks_like_assignment_like(checker, operands[2], source)
    {
        return Some(operands[1].span);
    }

    None
}

fn simple_test_operand_looks_like_assignment_like(
    checker: &Checker<'_>,
    word: &shuck_ast::Word,
    source: &str,
) -> bool {
    checker
        .facts()
        .word_fact(
            word.span,
            WordFactContext::Expansion(ExpansionContext::CommandArgument),
        )
        .is_some_and(|fact| {
            let classification = fact.classification();
            word_fact_looks_like_assignment_like(
                fact,
                classification.quote,
                classification.is_fixed_literal(),
                source,
            )
        })
}

fn conditional_assignment_spans(
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
            {
                let left = binary.left();
                let right = binary.right();
                if conditional_operand_looks_like_assignment_like(left, source)
                    || conditional_operand_looks_like_assignment_like(right, source)
                {
                    Some(binary.operator_span())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn conditional_operand_looks_like_assignment_like(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    operand
        .word()
        .zip(operand.word_classification())
        .is_some_and(|(word, classification)| {
            word_looks_like_assignment_like(
                word,
                classification.quote,
                classification.is_fixed_literal(),
                source,
            )
        })
}

fn word_looks_like_assignment_like(
    word: &shuck_ast::Word,
    quote: WordQuote,
    is_fixed_literal: bool,
    source: &str,
) -> bool {
    if quote != WordQuote::FullyQuoted || is_fixed_literal {
        return false;
    }

    let Some(prefix_span) = double_quoted_scalar_affix_span(word) else {
        return false;
    };
    let prefix = prefix_span.slice(source);
    if !prefix.ends_with('=') {
        return false;
    }

    let name = prefix.trim_end_matches('=');
    is_shell_variable_name(name)
}

fn word_fact_looks_like_assignment_like(
    fact: crate::WordOccurrenceRef<'_, '_>,
    quote: WordQuote,
    is_fixed_literal: bool,
    source: &str,
) -> bool {
    if quote != WordQuote::FullyQuoted || is_fixed_literal {
        return false;
    }

    let Some(prefix_span) = fact.double_quoted_scalar_affix_span() else {
        return false;
    };
    let prefix = prefix_span.slice(source);
    if !prefix.ends_with('=') {
        return false;
    }

    let name = prefix.trim_end_matches('=');
    is_shell_variable_name(name)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_expanded_assignment_looking_operands() {
        let source = "\
#!/bin/bash
[ \"QT6=${QT6:-no}\" = yes ]
[ yes = \"QT6=${QT6:-no}\" ]
[[ \"QT6=${QT6:-no}\" != yes ]]
[[ yes != \"QT6=${QT6:-no}\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantInTestAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["=", "=", "!=", "!="]
        );
    }

    #[test]
    fn ignores_plain_literals_and_unexpanded_words() {
        let source = "\
#!/bin/bash
[ \"A=B\" = yes ]
[ A=B = yes ]
[ \"QT6=no\" = yes ]
[ \"$QT6\" = yes ]
[[ \"A=B\" != yes ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConstantInTestAssignment),
        );

        assert!(diagnostics.is_empty());
    }
}
