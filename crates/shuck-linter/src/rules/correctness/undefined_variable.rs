use rustc_hash::FxHashSet;
use shuck_semantic::UninitializedCertainty;

use crate::{Checker, Rule, Violation};

use super::variable_reference_common::{
    VariableReferenceFilter, has_same_name_defining_bindings, is_reportable_variable_reference,
    is_sc2154_defining_binding,
};

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
    let mut uninitialized_references = checker
        .semantic_analysis()
        .uninitialized_references()
        .to_vec();
    uninitialized_references.sort_by_key(|uninitialized| {
        let reference = checker.semantic().reference(uninitialized.reference);
        (reference.span.start.offset, reference.span.end.offset)
    });

    let mut reported_names = FxHashSet::default();
    let mut suppressed_names = FxHashSet::default();

    for uninitialized in uninitialized_references {
        let reference = checker.semantic().reference(uninitialized.reference);
        if reported_names.contains(&reference.name) || suppressed_names.contains(&reference.name) {
            continue;
        }
        if !is_reportable_variable_reference(
            checker,
            reference,
            VariableReferenceFilter {
                suppress_environment_style_names: true,
            },
        ) {
            continue;
        }
        if has_same_name_defining_bindings(checker, &reference.name)
            && !checker
                .semantic()
                .bindings_for(&reference.name)
                .iter()
                .copied()
                .filter(|binding_id| {
                    is_sc2154_defining_binding(checker.semantic().binding(*binding_id).kind)
                })
                .all(|binding_id| {
                    let binding = checker.semantic().binding(binding_id);
                    binding.span.start.line == reference.span.start.line
                        && binding.span.start.offset < reference.span.start.offset
                })
        {
            suppressed_names.insert(reference.name.clone());
            continue;
        }
        if !reported_names.insert(reference.name.clone()) {
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
