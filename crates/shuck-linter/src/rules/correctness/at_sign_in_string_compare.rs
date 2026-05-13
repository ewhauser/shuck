use shuck_ast::Span;

use crate::facts::word_spans;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct AtSignInStringCompare;

impl Violation for AtSignInStringCompare {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AtSignInStringCompare
    }

    fn message(&self) -> String {
        "positional-parameter at-splats fold arguments when used as test operands".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("use positional `*` splats".to_owned())
    }
}

pub fn at_sign_in_string_compare(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.simple_test())
        .flat_map(|simple_test| simple_test_spans(simple_test, source))
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        checker.report_diagnostic_dedup(Diagnostic::new(AtSignInStringCompare, span).with_fix(fix));
    }
}

fn simple_test_spans(fact: &crate::SimpleTestFact<'_>, source: &str) -> Vec<(Span, Fix)> {
    fact.operator_expression_operand_words(source)
        .into_iter()
        .filter_map(|word| {
            let edits = word_spans::word_positional_at_splat_spans_in_source(word, source)
                .into_iter()
                .filter_map(at_splat_edit)
                .collect::<Vec<_>>();
            (!edits.is_empty()).then(|| (word.span, Fix::unsafe_edits(edits)))
        })
        .collect()
}

fn at_splat_edit(span: Span) -> Option<Edit> {
    let start = span.start.offset;
    let offset = match span.end.offset.saturating_sub(start) {
        2 => start + 1,
        _ => start + 2,
    };
    (offset < span.end.offset).then(|| Edit::replacement_at(offset, offset + 1, "*"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_positional_at_splats_in_test_operands() {
        let source = "\
#!/bin/bash
if [ -z \"$@\" ]; then :; fi
if test -n \"${@:-fallback}\"; then :; fi
if [ -d \"$@\" ]; then :; fi
if [ \"_$@\" = \"_--version\" ]; then :; fi
if [ \"$@\" = \"--version\" ]; then :; fi
if [ ! \"$@\" = \"x\" ]; then :; fi
if [ -n foo -a \"${@:-lhs}\" = \"${@:-rhs}\" ]; then :; fi
if [ -d \"$@\" -o \"${@:-fallback}\" = \"x\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$@\"",
                "\"${@:-fallback}\"",
                "\"$@\"",
                "\"_$@\"",
                "\"$@\"",
                "\"$@\"",
                "\"${@:-lhs}\"",
                "\"${@:-rhs}\"",
                "\"$@\"",
                "\"${@:-fallback}\"",
            ]
        );
    }

    #[test]
    fn ignores_non_positional_truthy_double_bracket_and_escaped_tests() {
        let source = "\
#!/bin/bash
if [ \"$@\" ]; then :; fi
if test \"${@:-fallback}\"; then :; fi
if [ \"_${arr[@]}\" = \"_x\" ]; then :; fi
if [ \"_${arr[@]:1}\" = \"_x\" ]; then :; fi
if [ \"\\$@\" = \"x\" ]; then :; fi
if [[ \"_$@\" == \"_x\" ]]; then :; fi
if [ ! \"\\$@\" = \"x\" ]; then :; fi
if [ \"_$*\" = \"_--version\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_positional_at_splats_in_test_operands() {
        let source = "\
#!/bin/bash
if [ -z \"$@\" ]; then :; fi
if test -n \"${@:-fallback}\"; then :; fi
if [ \"_$@:${@:-fallback}\" = x ]; then :; fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
if [ -z \"$*\" ]; then :; fi
if test -n \"${*:-fallback}\"; then :; fi
if [ \"_$*:${*:-fallback}\" = x ]; then :; fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_positional_at_splats_unchanged() {
        let source = "#!/bin/bash\nif [ -z \"$@\" ]; then :; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C111.sh").as_path(),
            &LinterSettings::for_rule(Rule::AtSignInStringCompare),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C111_fix_C111.sh", result);
        Ok(())
    }
}
