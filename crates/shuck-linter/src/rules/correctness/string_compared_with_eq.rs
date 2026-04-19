use shuck_ast::{ConditionalBinaryOp, Name, Span};
use shuck_semantic::{BindingAttributes, BindingKind};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperandFact, Rule, Violation, WordQuote,
    is_shell_variable_name, static_word_text, text_is_self_contained_arithmetic_expression,
};

pub struct StringComparedWithEq;

impl Violation for StringComparedWithEq {
    fn rule() -> Rule {
        Rule::StringComparedWithEq
    }

    fn message(&self) -> String {
        "this numeric comparison uses text instead of a number".to_owned()
    }
}

pub fn string_compared_with_eq(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.conditional()
                .map(|conditional| conditional_string_eq_spans(checker, conditional, source))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StringComparedWithEq);
}

fn conditional_string_eq_spans(
    checker: &Checker<'_>,
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .flat_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if matches!(
                    binary.op(),
                    ConditionalBinaryOp::ArithmeticEq | ConditionalBinaryOp::ArithmeticNe
                ) =>
            {
                let mut spans = Vec::new();
                if let Some(span) =
                    conditional_operand_string_value_span(checker, binary.left(), source)
                {
                    spans.push(span);
                }
                if let Some(span) =
                    conditional_operand_string_value_span(checker, binary.right(), source)
                {
                    spans.push(span);
                }
                spans
            }
            _ => Vec::new(),
        })
        .collect()
}

fn conditional_operand_string_value_span(
    checker: &Checker<'_>,
    operand: ConditionalOperandFact<'_>,
    source: &str,
) -> Option<Span> {
    operand
        .word()
        .zip(operand.word_classification())
        .and_then(|(word, classification)| {
            classification
                .is_fixed_literal()
                .then_some(word)
                .filter(|word| {
                    static_word_text(word, source).is_some_and(|text| {
                        operand_text_looks_like_string_value(
                            checker,
                            &text,
                            operand.quote(),
                            word.span,
                        )
                    })
                })
                .map(|word| word.span)
        })
}

fn operand_text_looks_like_string_value(
    checker: &Checker<'_>,
    text: &str,
    quote: Option<WordQuote>,
    span: Span,
) -> bool {
    if looks_like_decimal_integer(text) {
        return false;
    }

    if looks_like_defined_variable_name(checker, text, span) {
        return false;
    }

    if !text_is_self_contained_arithmetic_expression(text) {
        return true;
    }

    quote != Some(WordQuote::Unquoted) && text.trim().starts_with('(') && text.trim().ends_with(')')
}

fn looks_like_defined_variable_name(checker: &Checker<'_>, text: &str, span: Span) -> bool {
    if !is_shell_variable_name(text) {
        return false;
    }

    let name = Name::from(text);
    checker
        .semantic()
        .bindings_for(&name)
        .iter()
        .copied()
        .any(|binding_id| {
            let binding = checker.semantic().binding(binding_id);
            if binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FUNCTION)
                || matches!(binding.kind, BindingKind::FunctionDefinition)
            {
                return false;
            }

            checker.semantic().binding_visible_at(binding_id, span)
                && binding.span.start.offset != span.start.offset
        })
}

fn looks_like_decimal_integer(text: &str) -> bool {
    let text = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::test::test_snippet;
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};
    use tempfile::tempdir;

    #[test]
    fn reports_string_like_operands_in_double_bracket_numeric_comparisons() {
        let source = "\
#!/bin/bash
[[ $VER -eq \"latest\" ]]
[[ \"latest\" -eq $VER ]]
[[ foo -ne 3 ]]
[[ 3 -eq bar ]]
[[ foo -eq bar ]]
[[ \"(1+2)\" -eq 3 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"latest\"",
                "\"latest\"",
                "foo",
                "bar",
                "foo",
                "bar",
                "\"(1+2)\""
            ]
        );
    }

    #[test]
    fn ignores_single_bracket_and_numeric_arithmetic_comparisons() {
        let source = "\
#!/bin/bash
[ 1 -eq 2 ]
[[ 1 -eq 2 ]]
[ $VER -eq \"latest\" ]
[[ $VER = latest ]]
[[ 1+2 -eq 3 ]]
[[ (1+2) -eq 3 ]]
[[ \"1+2\" -eq 3 ]]
[[ \"1 + 2\" -eq 3 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_operands_that_match_defined_variable_names() {
        let source = "\
#!/bin/bash
retval=$?
[[ retval -ne 0 ]]
DISABLE_MENU=1
[[ \"$LEGACY_JOY2KEY\" -eq 0 && \"DISABLE_MENU\" -ne 1 ]]
__iterator=0
while [[ __iterator -eq 0 || -n \"${__next}\" ]]; do
  break
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn later_or_out_of_scope_bindings_do_not_suppress_diagnostics() {
        let source = "\
#!/bin/bash
[[ foo -eq 1 ]]
foo=1
helper() {
  local bar=1
}
[[ bar -eq 1 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn imported_bindings_only_suppress_after_the_source_site() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/bash
[[ flag -eq 1 ]]
source ./helper.sh
[[ flag -eq 1 ]]
";

        fs::write(&main, source).unwrap();
        fs::write(&helper, "flag=1\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["flag"]
        );
    }
}
