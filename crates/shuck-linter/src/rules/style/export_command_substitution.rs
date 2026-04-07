use shuck_ast::AssignmentValue;

use crate::rules::common::{command::DeclarationKind, span, word::classify_word};
use crate::{Checker, Rule, Violation};

pub struct ExportCommandSubstitution {
    pub name: String,
}

impl Violation for ExportCommandSubstitution {
    fn rule() -> Rule {
        Rule::ExportCommandSubstitution
    }

    fn message(&self) -> String {
        format!("assign command output before declaring `{}`", self.name)
    }
}

pub fn export_command_substitution(checker: &mut Checker) {
    let source = checker.source();
    let findings = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| !fact.is_nested_word_command())
        .filter_map(|fact| fact.declaration())
        .filter(|declaration| {
            matches!(
                declaration.kind,
                DeclarationKind::Export
                    | DeclarationKind::Local
                    | DeclarationKind::Declare
                    | DeclarationKind::Typeset
            )
        })
        .flat_map(|declaration| declaration.assignment_operands.iter().copied())
        .filter_map(|assignment| {
            let AssignmentValue::Scalar(word) = &assignment.value else {
                return None;
            };

            classify_word(word, source)
                .has_command_substitution()
                .then(|| {
                    (
                        assignment.target.name.to_string(),
                        span::assignment_name_span(assignment),
                    )
                })
        })
        .collect::<Vec<_>>();

    for (name, span) in findings {
        checker.report_dedup(ExportCommandSubstitution { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_declaration_assignment_names() {
        let source = "\
#!/bin/bash
export greeting=$(printf '%s\\n' hi)
demo() {
  local temp=\"$(date)\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["greeting", "temp"]
        );
    }
}
