use shuck_ast::{ConditionalBinaryOp, RedirectKind, Span};
use shuck_semantic::{BindingAttributes, BindingId};

use crate::{
    Checker, CommandFact, ConditionalBinaryFact, ConditionalNodeFact, ConditionalOperandFact,
    RedirectFact, Rule, SimpleTestSyntax, Violation, WordFact, static_word_text,
};

pub struct GreaterThanInTest;

impl Violation for GreaterThanInTest {
    fn rule() -> Rule {
        Rule::GreaterThanInTest
    }

    fn message(&self) -> String {
        "use `-lt`/`-gt` instead of `<`/`>` for numeric comparisons".to_owned()
    }
}

pub fn greater_than_in_test(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| comparison_operator_spans(command, checker, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GreaterThanInTest);
}

fn comparison_operator_spans(
    command: &CommandFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Vec<Span> {
    let mut spans = bracket_comparison_redirect_spans(command, source);
    spans.extend(double_bracket_numeric_comparison_spans(
        command, checker, source,
    ));
    spans
}

fn bracket_comparison_redirect_spans(command: &CommandFact<'_>, source: &str) -> Vec<Span> {
    let Some(simple_test) = command.simple_test() else {
        return Vec::new();
    };
    if simple_test.syntax() != SimpleTestSyntax::Bracket {
        return Vec::new();
    }

    let Some(opening_bracket) = command.body_word_span() else {
        return Vec::new();
    };
    let Some(closing_bracket) = command.body_args().last() else {
        return Vec::new();
    };

    command
        .redirect_facts()
        .iter()
        .filter_map(|redirect| {
            numeric_comparison_redirect_span(
                redirect,
                opening_bracket,
                closing_bracket.span,
                source,
            )
        })
        .collect()
}

fn numeric_comparison_redirect_span(
    redirect: &RedirectFact<'_>,
    opening_bracket_span: Span,
    closing_bracket_span: Span,
    source: &str,
) -> Option<Span> {
    let redirect_data = redirect.redirect();
    let operator = match redirect_data.kind {
        RedirectKind::Input => "<",
        RedirectKind::Output => ">",
        _ => return None,
    };

    let target = redirect_data.word_target()?;
    if !static_word_text(target, source).is_some_and(|text| looks_like_decimal_integer(&text)) {
        return None;
    }

    if redirect_data.span.start.offset < opening_bracket_span.end.offset
        || redirect_data.span.start.offset >= closing_bracket_span.start.offset
    {
        return None;
    }

    let operator_text = source
        .get(redirect_data.span.start.offset..target.span.start.offset)?
        .trim_end();
    if operator_text != operator {
        return None;
    }

    Some(Span::from_positions(
        redirect_data.span.start,
        redirect_data.span.start.advanced_by(operator),
    ))
}

fn double_bracket_numeric_comparison_spans(
    command: &CommandFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Vec<Span> {
    let Some(conditional) = command.conditional() else {
        return Vec::new();
    };

    conditional
        .nodes()
        .iter()
        .filter_map(|node| numeric_double_bracket_operator_span(node, checker, source))
        .collect()
}

fn numeric_double_bracket_operator_span(
    node: &ConditionalNodeFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Option<Span> {
    let ConditionalNodeFact::Binary(binary) = node else {
        return None;
    };
    if !matches!(
        binary.op(),
        ConditionalBinaryOp::LexicalBefore | ConditionalBinaryOp::LexicalAfter
    ) {
        return None;
    }

    if has_decimal_version_like_operand(binary, source)
        || !has_numeric_operand(binary, checker, source)
    {
        return None;
    }

    Some(binary.operator_span())
}

fn has_numeric_operand(
    binary: &ConditionalBinaryFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> bool {
    [binary.left(), binary.right()]
        .into_iter()
        .any(|operand| operand_is_numeric_literal(checker, operand, source))
}

fn has_decimal_version_like_operand(binary: &ConditionalBinaryFact<'_>, source: &str) -> bool {
    [binary.left(), binary.right()].into_iter().any(|operand| {
        operand
            .word()
            .and_then(|word| static_word_text(word, source))
            .is_some_and(|text| is_decimal_version_like(&text))
    })
}

fn operand_is_numeric_literal(
    checker: &Checker<'_>,
    operand: ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    let Some(word) = operand.word() else {
        return false;
    };

    static_word_text(word, source).is_some_and(|text| looks_like_decimal_integer(&text))
        || checker
            .facts()
            .any_word_fact(word.span)
            .is_some_and(|word_fact| {
                word_fact.is_direct_numeric_expansion()
                    || word_is_numeric_binding_reference(checker, word_fact)
            })
}

fn word_is_numeric_binding_reference(checker: &Checker<'_>, word_fact: &WordFact<'_>) -> bool {
    if !word_fact.is_plain_scalar_reference() {
        return false;
    }

    let span = word_fact.span();
    let mut references = checker.semantic().references().iter().filter(|reference| {
        reference.span.start.offset >= span.start.offset
            && reference.span.end.offset <= span.end.offset
    });
    let Some(reference) = references.next() else {
        return false;
    };
    if references.next().is_some() {
        return false;
    }

    let reaching = checker
        .semantic_analysis()
        .reaching_bindings_for_name(&reference.name, reference.span);
    !reaching.is_empty()
        && reaching
            .iter()
            .all(|binding_id| binding_is_numeric_literal(checker, *binding_id))
}

fn binding_is_numeric_literal(checker: &Checker<'_>, binding_id: BindingId) -> bool {
    let binding = checker.semantic().binding(binding_id);
    if binding.attributes.contains(BindingAttributes::INTEGER) {
        return true;
    }

    checker
        .facts()
        .binding_value(binding_id)
        .and_then(|value| value.scalar_word())
        .and_then(|word| static_word_text(word, checker.source()))
        .is_some_and(|text| looks_like_decimal_integer(&text))
}

fn looks_like_decimal_integer(text: &str) -> bool {
    let text = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);

    !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit())
}

fn is_decimal_version_like(text: &str) -> bool {
    let mut saw_dot = false;
    let mut saw_digit = false;
    let mut segment_has_digit = false;

    for ch in text.chars() {
        match ch {
            '0'..='9' => {
                saw_digit = true;
                segment_has_digit = true;
            }
            '.' => {
                if !segment_has_digit {
                    return false;
                }
                saw_dot = true;
                segment_has_digit = false;
            }
            _ => return false,
        }
    }

    saw_digit && saw_dot && segment_has_digit
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_numeric_less_and_greater_operators_in_test_expressions() {
        let source = "\
#!/bin/bash
[ \"$version\" > \"10\" ]
[ \"$version\" < 10 ]
[ 1 > 2 ]
[[ $count > 10 ]]
[[ \"$count\" < 1 ]]
left=1
right=2
[[ $left < $right ]]
[[ \"$left\" < \"$right\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", "<", ">", ">", "<", "<", "<"]
        );
    }

    #[test]
    fn ignores_string_ordering_and_non_numeric_targets() {
        let source = "\
#!/bin/bash
>\"$log\" [ \"$value\" ]
[ \"$value\" ] > \"$log\"
[ \"$value\" > \"$other\" ]
[ \"$value\" < \"$other\" ]
[ \"$value\" \\> \"$other\" ]
[ \"$value\" \\< \"$other\" ]
[ \"$value\" \">\" \"$other\" ]
[ \"$value\" \"<\" \"$other\" ]
test \"$value\" > 10
[[ \"$value\" > \"$other\" ]]
[[ \"$value\" < 1.2 ]]
[[ 1.2 > \"$value\" ]]
left=alpha
right=beta
[[ $left < $right ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn handles_plain_references_and_length_expansions_without_operator_inference() {
        let source = "\
#!/bin/bash
name=alpha
label=alpha
num=7
[[ ${#name} > \"$label\" ]]
[[ \"${#name}\" < \"$label\" ]]
[[ ${num:-fallback} > \"$label\" ]]
[[ \"${num%7}\" < \"$label\" ]]
[[ ${num} > \"$label\" ]]
[[ \"${num}\" < \"$label\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", "<", ">", "<"]
        );
    }
}
