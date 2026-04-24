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
                suppress_environment_style_names: !checker.report_environment_style_names(),
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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_defaulting_parameter_operands_until_later_plain_uses() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"${missing_assign:=$seed_name}\" \"${missing_error:?$hint_name}\"
printf '%s\\n' \"$seed_name\" \"$hint_name\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$seed_name", "$hint_name"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start.line == 3)
        );
    }

    #[test]
    fn parameter_guard_flow_suppresses_later_reads_of_the_guarded_name() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"${defaulted:-fallback}\" \"$defaulted\"
printf '%s\\n' \"${assigned:=fallback}\" \"$assigned\"
printf '%s\\n' \"${required:?missing}\" \"$required\"
printf '%s\\n' \"${replacement:+alt}\" \"$replacement\"
printf '%s\\n' \"$before_default\" \"${before_default:-fallback}\" \"$plain_missing\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$before_default", "$plain_missing"]
        );
    }

    #[test]
    fn parameter_guard_flow_does_not_escape_conditional_operands() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"${outer:+${nested_default:-fallback}}\" \"$outer\" \"$nested_default\"
printf '%s\\n' \"${other:+${nested_replacement:+alt}}\" \"$other\" \"$nested_replacement\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$nested_default", "$nested_replacement"]
        );
    }
}
