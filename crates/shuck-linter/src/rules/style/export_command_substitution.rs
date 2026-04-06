use shuck_ast::{AssignmentValue, Command, WordPart};

use crate::rules::common::query::{self, CommandWalkOptions};
use crate::{Checker, Rule, Violation};

pub struct ExportCommandSubstitution {
    pub name: String,
}

impl Violation for ExportCommandSubstitution {
    fn rule() -> Rule {
        Rule::ExportCommandSubstitution
    }

    fn message(&self) -> String {
        format!("assign command output before exporting `{}`", self.name)
    }
}

pub fn export_command_substitution(checker: &mut Checker) {
    let mut diagnostics = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            let Command::Decl(command) = command else {
                return;
            };

            if command.variant.as_ref() != "export" {
                return;
            }

            for operand in &command.operands {
                let shuck_ast::DeclOperand::Assignment(assignment) = operand else {
                    continue;
                };

                let AssignmentValue::Scalar(word) = &assignment.value else {
                    continue;
                };

                if word
                    .parts
                    .iter()
                    .any(|part| matches!(part, WordPart::CommandSubstitution(_)))
                {
                    diagnostics.push((assignment.span, assignment.name.to_string()));
                }
            }
        },
    );

    for (span, name) in diagnostics {
        checker.report(ExportCommandSubstitution { name }, span);
    }
}
