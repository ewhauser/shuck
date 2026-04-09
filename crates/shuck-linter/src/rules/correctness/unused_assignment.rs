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
    let unused_bindings = checker.semantic_analysis().unused_assignments().to_vec();

    for binding_id in unused_bindings {
        let binding = checker.semantic().binding(binding_id);

        if binding.name.as_str() == "_" {
            continue;
        }

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

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

fn is_reportable_unused_assignment(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::AppendAssignment | BindingKind::ParameterDefaultAssignment => false,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_variable_name_span() {
        let source = "#!/bin/sh\nunused=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }
}
