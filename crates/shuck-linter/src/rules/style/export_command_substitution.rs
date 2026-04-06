use shuck_ast::{AssignmentValue, Command, WordPart};

use crate::{Checker, Rule, Violation};

use super::syntax::walk_commands;

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

    walk_commands(&checker.ast().commands, &mut |command| {
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
    });

    for (span, name) in diagnostics {
        checker.report(ExportCommandSubstitution { name }, span);
    }
}
