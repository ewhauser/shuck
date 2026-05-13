use shuck_ast::Span;

use crate::{
    ArithmeticLiteralKind, Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect,
    Violation,
};

pub struct LeadingZeroArithmetic;

impl Violation for LeadingZeroArithmetic {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LeadingZeroArithmetic
    }

    fn message(&self) -> String {
        "this arithmetic literal is not valid octal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove leading zeroes from the arithmetic literal".to_owned())
    }
}

pub fn leading_zero_arithmetic(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let word_facts = checker.facts().words();
    let suppressed_subscript_spans = word_facts.arithmetic_only_suppressed_subscript_spans();
    let arithmetic_expansion_spans = word_facts.arithmetic_expansion_spans();
    let arithmetic_command_substitution_spans = word_facts.arithmetic_command_substitution_spans();

    let diagnostics = word_facts
        .arithmetic_literal_facts()
        .iter()
        .filter(|fact| fact.kind() == ArithmeticLiteralKind::LeadingZeroInteger)
        .filter(|fact| contains_invalid_octal_digit(fact.span().slice(source)))
        .filter(|fact| {
            !is_adjacent_to_runtime_expansion(
                source,
                fact.span(),
                arithmetic_command_substitution_spans,
            )
        })
        .filter(|fact| {
            !is_plain_suppressed_subscript_literal(
                fact.span(),
                suppressed_subscript_spans,
                arithmetic_expansion_spans,
            )
        })
        .map(|fact| diagnostic_for_leading_zero(fact.span(), source))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostic_for_leading_zero(span: Span, source: &str) -> Diagnostic {
    Diagnostic::new(LeadingZeroArithmetic, span).with_fix(Fix::unsafe_edit(Edit::replacement(
        decimal_replacement(span.slice(source)),
        span,
    )))
}

fn decimal_replacement(text: &str) -> &str {
    let trimmed = text.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn contains_invalid_octal_digit(text: &str) -> bool {
    text.bytes().any(|byte| matches!(byte, b'8' | b'9'))
}

fn is_adjacent_to_runtime_expansion(
    source: &str,
    span: Span,
    command_substitution_spans: &[Span],
) -> bool {
    let Some(previous) = span
        .start
        .offset
        .checked_sub(1)
        .and_then(|offset| source.as_bytes().get(offset))
    else {
        return false;
    };

    *previous == b'}'
        || *previous == b'`'
        || command_substitution_spans
            .iter()
            .any(|substitution| substitution.end.offset == span.start.offset)
}

fn is_plain_suppressed_subscript_literal(
    span: Span,
    suppressed_subscript_spans: &[Span],
    arithmetic_expansion_spans: &[Span],
) -> bool {
    span_is_within_any(span, suppressed_subscript_spans)
        && !span_is_within_any(span, arithmetic_expansion_spans)
}

fn span_is_within_any(span: Span, containers: &[Span]) -> bool {
    containers.iter().any(|container| {
        container.start.offset <= span.start.offset && span.end.offset <= container.end.offset
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_invalid_octal_literals_in_arithmetic() {
        let source = "\
#!/bin/bash
: $((08 + 009 + 010 + 000))
declare -a values
values[018]=x
printf '%s\n' \"${value:-$((008))}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["08", "009", "018", "008"]
        );
    }

    #[test]
    fn ignores_valid_octal_hex_explicit_base_and_substrings() {
        let source = "\
#!/bin/bash
: $((0 + 7 + 010 + 000 + 0x10 + 10#08))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_invalid_octal_literals_inside_substring_arithmetic() {
        let source = "#!/bin/bash\nprintf '%s\n' \"${value:$((08)):1}\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "08");
    }

    #[test]
    fn ignores_runtime_concatenated_literals_and_associative_keys() {
        let source = "\
#!/bin/bash
count=3
: $(( ${count}08 / 2 ))
: $(( $(printf '%s' 1)08 / 2 ))
declare -A checksums
checksums[008]=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_arithmetic_expansions_inside_associative_subscripts() {
        let source = "\
#!/bin/bash
declare -A checksums
checksums[$((08))]=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "08");
    }

    #[test]
    fn ignores_plain_associative_keys_near_unrelated_arithmetic_text() {
        let source = "\
#!/bin/bash
# $((
declare -A checksums
checksums[008]=value
printf '%s\\n' \"))\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn distinguishes_runtime_substitutions_from_arithmetic_grouping() {
        let source = "\
#!/bin/bash
: $(( $(printf '%s' 1)08 / 2 ))
: $(( `printf '%s' 1`09 / 2 ))
: $(( (1)08 / 2 ))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "08");
    }

    #[test]
    fn ignores_zsh_arithmetic_literals() {
        let source = "\
#!/bin/zsh
: $((08 + 000))
setopt octal_zeroes
: $((09))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\n: $((08))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove leading zeroes from the arithmetic literal")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_leading_zero_literals() {
        let source = "\
#!/bin/bash
: $((08 + 009 + 010))
declare -a values
values[018]=x
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\n: $((8 + 9 + 010))\ndeclare -a values\nvalues[18]=x\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C023.sh").as_path(),
            &LinterSettings::for_rule(Rule::LeadingZeroArithmetic),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C023_fix_C023.sh", result);
        Ok(())
    }
}
