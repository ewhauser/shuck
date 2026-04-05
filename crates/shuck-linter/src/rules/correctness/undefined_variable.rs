use shuck_semantic::{ReferenceKind, UninitializedCertainty};

use crate::{Checker, Rule, Violation};

pub struct UndefinedVariable {
    pub name: String,
    pub certainty: UninitializedCertainty,
}

impl Violation for UndefinedVariable {
    fn rule() -> Rule {
        Rule::UndefinedVariable
    }

    fn message(&self) -> String {
        match self.certainty {
            UninitializedCertainty::Definite => {
                format!("variable `{}` is referenced before assignment", self.name)
            }
            UninitializedCertainty::Possible => {
                format!(
                    "variable `{}` may be referenced before assignment",
                    self.name
                )
            }
        }
    }
}

pub fn undefined_variable(checker: &mut Checker) {
    for uninitialized in checker.semantic().uninitialized_references() {
        let reference = checker.semantic().reference(uninitialized.reference);
        if matches!(
            reference.kind,
            ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
        ) {
            continue;
        }
        if is_shell_special_parameter(reference.name.as_str()) {
            continue;
        }

        checker.report(
            UndefinedVariable {
                name: reference.name.to_string(),
                certainty: uninitialized.certainty,
            },
            reference.span,
        );
    }
}

fn is_shell_special_parameter(name: &str) -> bool {
    matches!(name, "@" | "*" | "#" | "?" | "-" | "$" | "!" | "0")
        || (!name.is_empty() && name.chars().all(|char| char.is_ascii_digit()))
}
