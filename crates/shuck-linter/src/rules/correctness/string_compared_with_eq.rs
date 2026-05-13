use shuck_ast::{ConditionalBinaryOp, Name, Span, is_shell_variable_name, static_word_text};
use shuck_parser::text_is_self_contained_arithmetic_expression;

use crate::{
    Checker, ConditionalBinaryFact, ConditionalNodeFact, ConditionalOperandFact, Diagnostic, Edit,
    Fix, FixAvailability, Rule, Violation, WordQuote,
};

use super::variable_reference_common::binding_defines_variable_name_at;

pub struct StringComparedWithEq;

impl Violation for StringComparedWithEq {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::StringComparedWithEq
    }

    fn message(&self) -> String {
        "this numeric comparison uses text instead of a number".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("use a string comparison".to_owned())
    }
}

pub fn string_compared_with_eq(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.conditional()
                .map(|conditional| conditional_string_eq_spans(checker, conditional, source))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        checker.report_diagnostic_dedup(Diagnostic::new(StringComparedWithEq, span).with_fix(fix));
    }
}

fn conditional_string_eq_spans(
    checker: &Checker<'_>,
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<(Span, Fix)> {
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
                string_eq_diagnostics(checker, *binary, source)
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => Vec::new(),
        })
        .collect()
}

fn string_eq_diagnostics(
    checker: &Checker<'_>,
    binary: ConditionalBinaryFact<'_>,
    source: &str,
) -> Vec<(Span, Fix)> {
    let operands = [binary.left(), binary.right()]
        .into_iter()
        .filter_map(|operand| conditional_operand_string_value(checker, operand, source))
        .collect::<Vec<_>>();
    if operands.is_empty() {
        return Vec::new();
    }

    let Some(fix) = string_eq_fix(binary, &operands) else {
        return Vec::new();
    };

    operands
        .into_iter()
        .map(|operand| (operand.span, fix.clone()))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct StringOperand {
    span: Span,
    quote: Option<WordQuote>,
}

fn conditional_operand_string_value(
    checker: &Checker<'_>,
    operand: ConditionalOperandFact<'_>,
    source: &str,
) -> Option<StringOperand> {
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
                .map(|word| StringOperand {
                    span: word.span,
                    quote: operand.quote(),
                })
        })
}

fn string_eq_fix(binary: ConditionalBinaryFact<'_>, operands: &[StringOperand]) -> Option<Fix> {
    let operator_replacement = match binary.op() {
        ConditionalBinaryOp::ArithmeticEq => "=",
        ConditionalBinaryOp::ArithmeticNe => "!=",
        ConditionalBinaryOp::RegexMatch
        | ConditionalBinaryOp::NewerThan
        | ConditionalBinaryOp::OlderThan
        | ConditionalBinaryOp::SameFile
        | ConditionalBinaryOp::ArithmeticLe
        | ConditionalBinaryOp::ArithmeticGe
        | ConditionalBinaryOp::ArithmeticLt
        | ConditionalBinaryOp::ArithmeticGt
        | ConditionalBinaryOp::And
        | ConditionalBinaryOp::Or
        | ConditionalBinaryOp::PatternEqShort
        | ConditionalBinaryOp::PatternEq
        | ConditionalBinaryOp::PatternNe
        | ConditionalBinaryOp::LexicalBefore
        | ConditionalBinaryOp::LexicalAfter => return None,
    };

    let mut edits = vec![Edit::replacement(
        operator_replacement,
        binary.operator_span(),
    )];
    for operand in operands
        .iter()
        .filter(|operand| operand.quote == Some(WordQuote::Unquoted))
    {
        edits.push(Edit::insertion(operand.span.start.offset, "\""));
        edits.push(Edit::insertion(operand.span.end.offset, "\""));
    }

    Some(Fix::unsafe_edits(edits))
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
            binding.span.start.offset != span.start.offset
                && binding_defines_variable_name_at(checker, binding, span)
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

    use std::path::Path;

    use crate::test::{
        test_path_with_fix, test_snippet, test_snippet_at_path, test_snippet_with_fix,
    };
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};
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
[[ foo+1 -eq 3 ]]
[[ \"foo+1\" -eq 3 ]]
[[ arr[1] -eq 3 ]]
[[ \"arr[1]\" -eq 3 ]]
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
                "\"(1+2)\"",
                "foo+1",
                "\"foo+1\"",
                "arr[1]",
                "\"arr[1]\"",
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
    fn same_file_bindings_suppress_literal_variable_name_heuristics() {
        let source = "\
#!/bin/bash
[[ foo -eq 1 ]]
foo=1
helper() {
  local bar=1
}
[[ bar -eq 1 ]]
show_launch() {
  [[ \"$LEGACY_JOY2KEY\" -eq 0 && \"DISABLE_MENU\" -ne 1 ]]
}
get_config() {
  DISABLE_MENU=1
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn function_definitions_do_not_suppress_diagnostics() {
        let source = "\
#!/bin/bash
DISABLE_MENU() {
  :
}
[[ \"DISABLE_MENU\" -ne 1 ]]
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
            vec!["\"DISABLE_MENU\""]
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

    #[test]
    fn applies_unsafe_fix_to_text_numeric_comparisons() {
        let source = "\
#!/bin/bash
[[ $VER -eq \"latest\" ]]
[[ foo -ne 3 ]]
[[ foo -eq bar ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[[ $VER = \"latest\" ]]
[[ \"foo\" != 3 ]]
[[ \"foo\" = \"bar\" ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_text_numeric_comparisons_unchanged() {
        let source = "#!/bin/bash\n[[ foo -eq 3 ]]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C121.sh").as_path(),
            &LinterSettings::for_rule(Rule::StringComparedWithEq),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C121_fix_C121.sh", result);
        Ok(())
    }
}
