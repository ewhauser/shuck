use shuck_ast::{DeclOperand, Span, static_word_text};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SpacedAssignment;

impl Violation for SpacedAssignment {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SpacedAssignment
    }

    fn message(&self) -> String {
        "remove spaces around `=` in this assignment".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove spaces around the assignment operator".to_owned())
    }
}

pub fn spaced_assignment(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.declaration())
        .flat_map(|declaration| spaced_assignment_diagnostics(declaration.operands, source))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn spaced_assignment_diagnostics(operands: &[DeclOperand], source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, pair) in operands.windows(2).enumerate() {
        let [DeclOperand::Name(name), DeclOperand::Dynamic(word)] = pair else {
            continue;
        };
        if !static_word_text(word, source).is_some_and(|text| text.starts_with('=')) {
            continue;
        }

        let mut edits = vec![Edit::deletion_at(
            name.span.end.offset,
            word.span.start.offset,
        )];
        if word.span.slice(source) == "="
            && let Some(next) = operands.get(index + 2).and_then(decl_operand_span)
        {
            edits.push(Edit::deletion_at(word.span.end.offset, next.start.offset));
        }
        diagnostics
            .push(Diagnostic::new(SpacedAssignment, word.span).with_fix(Fix::unsafe_edits(edits)));
    }
    diagnostics
}

fn decl_operand_span(operand: &DeclOperand) -> Option<Span> {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => Some(word.span),
        DeclOperand::Name(name) => Some(name.span),
        DeclOperand::Assignment(assignment) => Some(assignment.span),
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_the_stray_equals_word() {
        let source = "\
#!/bin/sh
export foo =bar
readonly bar = baz
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpacedAssignment));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["=bar", "="]
        );
    }

    #[test]
    fn ignores_tight_assignments_and_plain_commands() {
        let source = "\
#!/bin/sh
export foo=bar
foo =bar
foo= bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpacedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_by_removing_declaration_assignment_spaces() {
        let source = "#!/bin/sh\nexport foo =bar\nreadonly bar = baz\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SpacedAssignment),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nexport foo=bar\nreadonly bar=baz\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
