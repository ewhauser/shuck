use shuck_ast::{ConditionalUnaryOp, Span, Word};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Diagnostic, Edit, ExpansionContext,
    Fix, FixAvailability, Rule, SimpleTestOperatorFamily, SimpleTestShape, Violation,
    WordFactContext, WordQuote,
};

pub struct QuotedCommandInTest;

impl Violation for QuotedCommandInTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::QuotedCommandInTest
    }

    fn message(&self) -> String {
        "this test uses a quoted pipeline literal instead of command output".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the pipeline in command substitution".to_owned())
    }
}

pub fn quoted_command_in_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| {
            let mut diagnostics = Vec::new();
            if let Some(simple_test) = command.simple_test() {
                diagnostics.extend(collect_simple_test_diagnostics(checker, simple_test));
            }
            if let Some(conditional) = command.conditional() {
                diagnostics.extend(collect_conditional_diagnostics(conditional, source));
            }
            diagnostics
        })
        .collect::<Vec<_>>();

    for (span, replacement) in diagnostics {
        checker.report_diagnostic_dedup(
            Diagnostic::new(QuotedCommandInTest, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span))),
        );
    }
}

fn collect_simple_test_diagnostics(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
) -> Vec<(Span, String)> {
    simple_test_condition_operand(simple_test)
        .and_then(|word| {
            simple_test_quoted_pipeline_replacement(checker, word.span, checker.source())
                .map(|replacement| (word.span, replacement))
        })
        .into_iter()
        .collect()
}

fn simple_test_condition_operand<'a>(
    simple_test: &'a crate::SimpleTestFact<'a>,
) -> Option<&'a Word> {
    match simple_test.effective_shape() {
        SimpleTestShape::Truthy => simple_test.effective_operands().first().copied(),
        SimpleTestShape::Unary
            if simple_test.effective_operator_family() == SimpleTestOperatorFamily::StringUnary =>
        {
            simple_test.effective_operands().get(1).copied()
        }
        SimpleTestShape::Unary
        | SimpleTestShape::Empty
        | SimpleTestShape::Binary
        | SimpleTestShape::Other => None,
    }
}

fn collect_conditional_diagnostics(
    conditional: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<(Span, String)> {
    let mut diagnostics = Vec::new();
    let unary_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary) => unary.operand().word().map(|word| word.span),
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .collect::<Vec<_>>();
    let comparison_operand_spans = conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() != ConditionalOperatorFamily::Logical =>
            {
                Some([
                    binary.left().word().map(|word| word.span),
                    binary.right().word().map(|word| word.span),
                ])
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .flatten()
        .flatten()
        .collect::<Vec<_>>();

    match conditional.root() {
        ConditionalNodeFact::BareWord(bare_word)
            if conditional_quoted_pipeline_replacement(bare_word.operand(), source).is_some() =>
        {
            if let Some(word) = bare_word.operand().word()
                && let Some(replacement) =
                    conditional_quoted_pipeline_replacement(bare_word.operand(), source)
            {
                diagnostics.push((word.span, replacement));
            }
        }
        ConditionalNodeFact::Unary(unary)
            if conditional_unary_checks_literal_string(unary)
                && conditional_quoted_pipeline_replacement(unary.operand(), source).is_some() =>
        {
            if let Some(word) = unary.operand().word()
                && let Some(replacement) =
                    conditional_quoted_pipeline_replacement(unary.operand(), source)
            {
                diagnostics.push((word.span, replacement));
            }
        }
        ConditionalNodeFact::BareWord(_)
        | ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Binary(_)
        | ConditionalNodeFact::Other(_) => {}
    }

    for node in conditional.nodes().iter().skip(1) {
        match node {
            ConditionalNodeFact::BareWord(bare_word)
                if bare_word.operand().word().is_some_and(|word| {
                    !unary_operand_spans.contains(&word.span)
                        && !comparison_operand_spans.contains(&word.span)
                        && conditional_quoted_pipeline_replacement(bare_word.operand(), source)
                            .is_some()
                }) =>
            {
                if let Some(word) = bare_word.operand().word()
                    && let Some(replacement) =
                        conditional_quoted_pipeline_replacement(bare_word.operand(), source)
                {
                    diagnostics.push((word.span, replacement));
                }
            }
            ConditionalNodeFact::Unary(unary)
                if conditional_unary_checks_literal_string(unary)
                    && conditional_quoted_pipeline_replacement(unary.operand(), source)
                        .is_some() =>
            {
                if let Some(word) = unary.operand().word()
                    && let Some(replacement) =
                        conditional_quoted_pipeline_replacement(unary.operand(), source)
                {
                    diagnostics.push((word.span, replacement));
                }
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => {}
        }
    }

    diagnostics
}

fn conditional_unary_checks_literal_string(unary: &crate::ConditionalUnaryFact<'_>) -> bool {
    unary.operator_family() == ConditionalOperatorFamily::StringUnary
        || unary.op() == ConditionalUnaryOp::Not
}

fn simple_test_quoted_pipeline_replacement(
    checker: &Checker<'_>,
    span: Span,
    source: &str,
) -> Option<String> {
    checker
        .facts()
        .word_fact(
            span,
            WordFactContext::Expansion(ExpansionContext::CommandArgument),
        )
        .and_then(|fact| quoted_pipeline_replacement_from_word_fact(fact, source))
}

fn quoted_pipeline_replacement_from_word_fact(
    fact: crate::WordOccurrenceRef<'_, '_>,
    source: &str,
) -> Option<String> {
    if fact.classification().quote != WordQuote::FullyQuoted
        || !fact.classification().is_fixed_literal()
    {
        return None;
    }

    let text = fact.static_text_cow(source)?;
    looks_like_quoted_pipeline_literal(&text).then(|| command_substitution_replacement(&text))
}

fn conditional_quoted_pipeline_replacement(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<String> {
    operand
        .word()
        .zip(operand.word_classification())
        .and_then(|(word, classification)| {
            (classification.quote == WordQuote::FullyQuoted && classification.is_fixed_literal())
                .then(|| shuck_ast::static_word_text(word, source))
        })
        .flatten()
        .and_then(|text| {
            looks_like_quoted_pipeline_literal(&text)
                .then(|| command_substitution_replacement(&text))
        })
}

fn command_substitution_replacement(text: &str) -> String {
    format!("\"$({text})\"")
}

fn looks_like_quoted_pipeline_literal(text: &str) -> bool {
    if text.contains("||") {
        return false;
    }

    let segments = text.split('|').map(str::trim).collect::<Vec<_>>();

    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .split_ascii_whitespace()
                    .any(looks_like_command_word)
        })
}

fn looks_like_command_word(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_quoted_pipeline_literals_in_simple_tests() {
        let source = "\
#!/bin/sh
[ \"lsmod | grep v4l2loopback\" ]
[ -n \"modprobe | grep snd\" ]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"lsmod | grep v4l2loopback\"", "\"modprobe | grep snd\""]
        );
    }

    #[test]
    fn reports_negated_quoted_pipeline_literals_in_simple_tests() {
        let source = "\
#!/bin/sh
[ ! \"lsmod | grep v4l2loopback\" ]
test ! \"modprobe | grep snd\"
[ ! -n \"lsmod | grep usb\" ]
test ! -z \"modprobe | grep snd_hda\"
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"lsmod | grep v4l2loopback\"",
                "\"modprobe | grep snd\"",
                "\"lsmod | grep usb\"",
                "\"modprobe | grep snd_hda\""
            ]
        );
    }

    #[test]
    fn reports_nested_quoted_pipeline_literals_in_conditionals() {
        let source = "\
#!/bin/bash
[[ \"$ok\" && \"lsmod | grep v4l2loopback\" ]]
[[ -n \"lsmod | grep v4l2loopback\" && -n \"$ok\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"lsmod | grep v4l2loopback\"",
                "\"lsmod | grep v4l2loopback\""
            ]
        );
    }

    #[test]
    fn reports_negated_quoted_pipeline_literals_in_conditionals() {
        let source = "\
#!/bin/bash
[[ ! \"lsmod | grep v4l2loopback\" ]]
[[ \"$ok\" && ! \"modprobe | grep snd\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"lsmod | grep v4l2loopback\"", "\"modprobe | grep snd\""]
        );
    }

    #[test]
    fn ignores_non_pipeline_literals_and_binary_comparisons() {
        let source = "\
#!/bin/sh
[ \"echo hi\" ]
[ ! \"echo hi\" ]
[ \"foo | bar\" = x ]
[[ \"grep foo file\" = x ]]
[[ -n \"echo hi\" ]]
[[ ! \"echo hi\" ]]
[[ -f \"foo | bar\" ]]
[[ -x \"foo | bar\" ]]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::QuotedCommandInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_quoted_pipeline_literals() {
        let source = "\
#!/bin/sh
[ \"lsmod | grep v4l2loopback\" ]
[ -n \"modprobe | grep snd\" ]
[[ \"$ok\" && \"dmesg | grep usb\" ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedCommandInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
[ \"$(lsmod | grep v4l2loopback)\" ]
[ -n \"$(modprobe | grep snd)\" ]
[[ \"$ok\" && \"$(dmesg | grep usb)\" ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn preserves_inner_quotes_when_fixing_quoted_pipeline_literals() {
        let source = "\
#!/bin/sh
[ \"printf \\\"%s\\\" foo | grep foo\" ]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedCommandInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
[ \"$(printf \"%s\" foo | grep foo)\" ]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_pipeline_literals_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
[ \"echo hi\" ]
[ \"foo | bar\" = x ]
[[ -f \"foo | bar\" ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedCommandInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C089.sh").as_path(),
            &LinterSettings::for_rule(Rule::QuotedCommandInTest),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C089_fix_C089.sh", result);
        Ok(())
    }
}
