use shuck_semantic::{BindingAttributes, BindingKind};

use crate::{Checker, Rule, Violation};

pub struct UnusedAssignment {
    pub name: String,
}

impl Violation for UnusedAssignment {
    fn rule() -> Rule {
        Rule::UnusedAssignment
    }

    fn message(&self) -> String {
        format!("variable `{}` is assigned but never used", self.name)
    }
}

pub fn unused_assignment(checker: &mut Checker) {
    for &binding_id in checker.semantic().unused_assignments() {
        let binding = checker.semantic().binding(binding_id);

        // Exported variables are consumed by child processes.
        if binding.attributes.contains(BindingAttributes::EXPORTED) {
            continue;
        }

        // Namerefs redirect to another variable; the binding itself is not
        // a conventional assignment.
        if matches!(binding.kind, BindingKind::Nameref) {
            continue;
        }

        checker.report(
            UnusedAssignment {
                name: binding.name.to_string(),
            },
            binding.span,
        );
    }
}
