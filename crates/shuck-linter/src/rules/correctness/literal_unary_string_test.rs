use shuck_ast::ConditionalUnaryOp;

use crate::facts::word_spans;
use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Edit, Fix, FixAvailability, Rule,
    Violation,
};

pub struct LiteralUnaryStringTest;

impl Violation for LiteralUnaryStringTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LiteralUnaryStringTest
    }

    fn message(&self) -> String {
        "this string test checks a fixed literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the constant string-test operand with an explicit literal".to_owned())
    }
}

pub fn literal_unary_string_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut diagnostics = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                diagnostics.extend(simple_test_diagnostics(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                diagnostics.extend(conditional_diagnostics(conditional, source));
            }
            diagnostics
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn simple_test_diagnostics(
    fact: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    fact.string_unary_expression_words(source)
        .into_iter()
        .filter_map(|(operator, operand)| simple_test_diagnostic(fact, operator, operand, source))
        .collect()
}

fn simple_test_diagnostic(
    fact: &crate::SimpleTestFact<'_>,
    operator: &shuck_ast::Word,
    operand: &shuck_ast::Word,
    source: &str,
) -> Option<crate::Diagnostic> {
    if unary_operand_is_explicit_boolean_literal(operator, operand, source) {
        return None;
    }

    let index = fact
        .effective_operands()
        .iter()
        .position(|candidate| candidate.span == operand.span)?;

    if fact
        .effective_operand_class(index)
        .is_some_and(|class| class.is_fixed_literal())
    {
        let span = operand
            .quoted_content_span_in_source(source)
            .unwrap_or(operand.span);
        let replacement = explicit_literal_replacement_for_word(operand, source);
        return Some(
            crate::Diagnostic::new(LiteralUnaryStringTest, span).with_fix(Fix::unsafe_edit(
                Edit::replacement(replacement, operand.span),
            )),
        );
    }

    let span = word_spans::double_quoted_scalar_affix_span(operand)?;
    Some(
        crate::Diagnostic::new(LiteralUnaryStringTest, span)
            .with_fix(Fix::unsafe_edit(Edit::replacement("x", operand.span))),
    )
}

fn conditional_diagnostics(
    fact: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    fact.nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Unary(unary)
                if unary.operator_family() == ConditionalOperatorFamily::StringUnary =>
            {
                conditional_diagnostic(unary.op(), unary.operand(), source)
            }
            _ => None,
        })
        .collect()
}

fn conditional_diagnostic(
    op: ConditionalUnaryOp,
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<crate::Diagnostic> {
    if conditional_unary_operand_is_explicit_boolean_literal(op, operand, source) {
        return None;
    }

    if operand.class().is_fixed_literal() {
        let replacement = operand
            .word()
            .map(|word| explicit_literal_replacement_for_word(word, source))
            .unwrap_or_else(|| {
                explicit_literal_replacement_for_text(operand.expression().span().slice(source))
            });

        let span = operand
            .word()
            .map(|word| {
                word.quoted_content_span_in_source(source)
                    .unwrap_or(word.span)
            })
            .unwrap_or_else(|| operand.expression().span());
        let fix_span = operand
            .word()
            .map(|word| word.span)
            .unwrap_or_else(|| operand.expression().span());
        return Some(
            crate::Diagnostic::new(LiteralUnaryStringTest, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, fix_span))),
        );
    }

    let word = operand.word()?;
    let span = word_spans::double_quoted_scalar_affix_span(word)?;
    Some(
        crate::Diagnostic::new(LiteralUnaryStringTest, span)
            .with_fix(Fix::unsafe_edit(Edit::replacement("x", word.span))),
    )
}

const EMPTY_LITERAL_REPLACEMENT: &str = "\"\"";
const NON_EMPTY_LITERAL_REPLACEMENT: &str = "x";

fn explicit_literal_replacement_for_word(word: &shuck_ast::Word, source: &str) -> &'static str {
    explicit_literal_replacement_for_text(
        shuck_ast::static_word_text(word, source)
            .as_deref()
            .unwrap_or_else(|| word.span.slice(source)),
    )
}

fn explicit_literal_replacement_for_text(text: &str) -> &'static str {
    if text.is_empty() || quoted_literal_body(text).is_some_and(str::is_empty) {
        EMPTY_LITERAL_REPLACEMENT
    } else {
        NON_EMPTY_LITERAL_REPLACEMENT
    }
}

fn quoted_literal_body(text: &str) -> Option<&str> {
    let quote = text.chars().next()?;
    if !matches!(quote, '"' | '\'') {
        return None;
    }

    text.strip_prefix(quote)?.strip_suffix(quote)
}

fn unary_operand_is_explicit_boolean_literal(
    operator: &shuck_ast::Word,
    operand: &shuck_ast::Word,
    source: &str,
) -> bool {
    matches!(
        shuck_ast::static_word_text(operator, source).as_deref(),
        Some("-z" | "-n")
    ) && matches!(
        shuck_ast::static_word_text(operand, source).as_deref(),
        Some("" | "x")
    )
}

fn conditional_unary_operand_is_explicit_boolean_literal(
    op: ConditionalUnaryOp,
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> bool {
    matches!(
        op,
        ConditionalUnaryOp::EmptyString | ConditionalUnaryOp::NonEmptyString
    ) && conditional_operand_text(operand, source)
        .as_deref()
        .is_some_and(|text| matches!(text, "" | "x"))
}

fn conditional_operand_text(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<String> {
    if let Some(word) = operand.word() {
        return shuck_ast::static_word_text(word, source).map(|text| text.into_owned());
    }

    operand
        .class()
        .is_fixed_literal()
        .then(|| operand.expression().span().slice(source).to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_nested_unary_string_tests_in_simple_and_conditional_logical_chains() {
        let source = "\
#!/bin/bash
[ -z foo -o -z \"$path\" ]
[[ -z \"name\" || -z \"$path\" ]]
[[ ! -n bar ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "name", "bar"]
        );
    }

    #[test]
    fn reports_quoted_scalar_affixes_that_make_unary_string_tests_constant() {
        let source = "\
#!/bin/bash
[ -z \"${rootfs_path}_path\" ]
[[ -n \"prefix${rootfs_path}\" ]]
[ -n \"$rootfs_path\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["_path", "prefix"]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/bash\n[ -z foo ]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("replace the constant string-test operand with an explicit literal")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_fixed_and_affix_operands() {
        let source = "\
#!/bin/bash
[ -z foo -o -z \"$path\" ]
[[ -z \"name\" || -z \"$path\" ]]
[[ ! -n bar ]]
[ -z \"${rootfs_path}_path\" ]
[[ -n \"prefix${rootfs_path}\" ]]
test -n ''
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ -z x -o -z \"$path\" ]
[[ -z x || -z \"$path\" ]]
[[ ! -n x ]]
[ -z x ]
[[ -n x ]]
test -n ''
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_explicit_boolean_literals() {
        let source = "\
#!/bin/bash
[ -z x ]
[ -z \"\" ]
[ -n '' ]
[ -n \"\" ]
[[ -z x ]]
[[ -z \"\" ]]
[[ -n '' ]]
[[ -n \"\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_runtime_sensitive_unary_string_tests_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ -z \"$value\" ]
test -n \"$value\"
[[ -z $value ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C019.sh").as_path(),
            &LinterSettings::for_rule(Rule::LiteralUnaryStringTest),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C019_fix_C019.sh", result);
        Ok(())
    }
}
