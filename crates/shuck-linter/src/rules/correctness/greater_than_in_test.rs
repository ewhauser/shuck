use shuck_ast::{ConditionalBinaryOp, RedirectKind, Span};
use shuck_semantic::{BindingAttributes, BindingId};

use crate::{
    Checker, CommandFact, ConditionalBinaryFact, ConditionalNodeFact, ConditionalOperandFact, Edit,
    Fix, FixAvailability, RedirectFact, Rule, SimpleTestSyntax, Violation, WordOccurrenceRef,
    static_word_text,
};

pub struct GreaterThanInTest;

const FIX_TITLE: &str = "replace `<` or `>` with `-lt` or `-gt`";

impl Violation for GreaterThanInTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GreaterThanInTest
    }

    fn message(&self) -> String {
        "use `-lt`/`-gt` instead of `<`/`>` for numeric comparisons".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some(FIX_TITLE.to_owned())
    }
}

pub fn greater_than_in_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| comparison_operator_diagnostics(command, checker, source))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn comparison_operator_diagnostics(
    command: &CommandFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    let mut diagnostics = bracket_comparison_redirect_diagnostics(command, source);
    diagnostics.extend(double_bracket_numeric_comparison_diagnostics(
        command, checker, source,
    ));
    diagnostics
}

fn bracket_comparison_redirect_diagnostics(
    command: &CommandFact<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
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
            numeric_comparison_redirect_diagnostic(
                redirect,
                opening_bracket,
                closing_bracket.span,
                source,
            )
        })
        .collect()
}

fn numeric_comparison_redirect_diagnostic(
    redirect: &RedirectFact<'_>,
    opening_bracket_span: Span,
    closing_bracket_span: Span,
    source: &str,
) -> Option<crate::Diagnostic> {
    let (redirect_data, replacement) = match redirect.redirect().kind {
        RedirectKind::Input => (redirect.redirect(), "-lt"),
        RedirectKind::Output => (redirect.redirect(), "-gt"),
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

    let operator_span = redirect.operator_span();
    if operator_span.start.offset != redirect_data.span.start.offset {
        return None;
    }

    let gap = source.get(operator_span.end.offset..target.span.start.offset)?;
    if !is_shell_token_gap(gap) {
        return None;
    }

    let fix_span = Span::from_positions(operator_span.start, target.span.start);
    let leading_separator = if has_trailing_token_boundary(&source[..operator_span.start.offset]) {
        ""
    } else {
        " "
    };
    let separator = Some(gap)
        .filter(|text| has_trailing_token_boundary(text))
        .unwrap_or(" ");

    Some(
        crate::Diagnostic::new(GreaterThanInTest, operator_span).with_fix(Fix::unsafe_edit(
            Edit::replacement(
                format!("{leading_separator}{replacement}{separator}"),
                fix_span,
            ),
        )),
    )
}

fn is_shell_token_gap(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'\\' if bytes.get(index + 1) == Some(&b'\n') => index += 2,
            _ => return false,
        }
    }

    true
}

fn has_trailing_token_boundary(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut end = bytes.len();

    while end > 0 {
        match bytes[end - 1] {
            b' ' | b'\t' | b'\r' => return true,
            b'\n' => {
                let mut cursor = end - 1;
                let mut backslashes = 0usize;
                while cursor > 0 && bytes[cursor - 1] == b'\\' {
                    backslashes += 1;
                    cursor -= 1;
                }

                if backslashes % 2 == 1 {
                    end = cursor;
                    continue;
                }

                return true;
            }
            _ => return false,
        }
    }

    false
}

fn double_bracket_numeric_comparison_diagnostics(
    command: &CommandFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    let Some(conditional) = command.conditional() else {
        return Vec::new();
    };

    conditional
        .nodes()
        .iter()
        .filter_map(|node| numeric_double_bracket_operator_diagnostic(node, checker, source))
        .collect()
}

fn numeric_double_bracket_operator_diagnostic(
    node: &ConditionalNodeFact<'_>,
    checker: &Checker<'_>,
    source: &str,
) -> Option<crate::Diagnostic> {
    let ConditionalNodeFact::Binary(binary) = node else {
        return None;
    };
    let replacement = match binary.op() {
        ConditionalBinaryOp::LexicalBefore => "-lt",
        ConditionalBinaryOp::LexicalAfter => "-gt",
        _ => return None,
    };

    if has_decimal_version_like_operand(binary, source)
        || !has_numeric_operand(binary, checker, source)
    {
        return None;
    }

    let operator_span = binary.operator_span();
    Some(
        crate::Diagnostic::new(GreaterThanInTest, operator_span).with_fix(Fix::unsafe_edit(
            Edit::replacement(replacement, operator_span),
        )),
    )
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

fn word_is_numeric_binding_reference(
    checker: &Checker<'_>,
    word_fact: WordOccurrenceRef<'_, '_>,
) -> bool {
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
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    use super::FIX_TITLE;

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

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/bash\n[ \"$version\" > \"10\" ]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(diagnostics[0].fix_title.as_deref(), Some(FIX_TITLE));
    }

    #[test]
    fn applies_unsafe_fix_to_numeric_test_operators() {
        let source = "\
#!/bin/bash
[ \"$version\" > \"10\" ]
[ \"$version\" < 10 ]
count=11
limit=3
[[ $count > 10 ]]
[[ \"$count\" < 1 ]]
[[ $count > $limit ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GreaterThanInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ \"$version\" -gt \"10\" ]
[ \"$version\" -lt 10 ]
count=11
limit=3
[[ $count -gt 10 ]]
[[ \"$count\" -lt 1 ]]
[[ $count -gt $limit ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn inserts_a_separator_for_compact_bracket_numeric_comparisons() {
        let source = "\
#!/bin/bash
[ \"$version\">\"10\" ]
[ \"$version\"<10 ]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GreaterThanInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ \"$version\" -gt \"10\" ]
[ \"$version\" -lt 10 ]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn normalizes_escaped_newline_separators_in_bracket_numeric_comparisons() {
        let source = "\
#!/bin/bash
[ \"$version\">\\
\"10\" ]
[ \"$version\"\\
<\\
10 ]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GreaterThanInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ \"$version\" -gt \"10\" ]
[ \"$version\"\\
 -lt 10 ]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_string_and_version_ordering_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ \"$value\" > \"$other\" ]
[ \"$value\" < \"$other\" ]
[[ \"$value\" > \"$other\" ]]
[[ \"$value\" < 1.2 ]]
[[ 1.2 > \"$value\" ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GreaterThanInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C086.sh").as_path(),
            &LinterSettings::for_rule(Rule::GreaterThanInTest),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C086_fix_C086.sh", result);
        Ok(())
    }
}
