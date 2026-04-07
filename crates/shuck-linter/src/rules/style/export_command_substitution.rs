use shuck_ast::AssignmentValue;

use crate::rules::common::{
    command::{self, DeclarationKind},
    query::{self, CommandWalkOptions},
    span,
    word::classify_word,
};
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

    query::walk_commands(
        &checker.ast().commands,
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            let normalized = command::normalize_command(command, source);
            let Some(declaration) = normalized.declaration.as_ref() else {
                return;
            };

            if !matches!(
                declaration.kind,
                DeclarationKind::Export
                    | DeclarationKind::Local
                    | DeclarationKind::Declare
                    | DeclarationKind::Typeset
            ) {
                return;
            }

            for assignment in &declaration.assignment_operands {
                let AssignmentValue::Scalar(word) = &assignment.value else {
                    continue;
                };

                if classify_word(word, source).has_command_substitution() {
                    checker.report_dedup(
                        ExportCommandSubstitution {
                            name: assignment.target.name.to_string(),
                        },
                        span::assignment_name_span(assignment),
                    );
                }
            }
        },
    );
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
