use rustc_hash::FxHashSet;
use shuck_semantic::UninitializedCertainty;

use crate::{Checker, Rule, Violation};

use super::variable_reference_common::{
    VariableReferenceFilter, has_same_name_defining_bindings, is_reportable_variable_reference,
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
        if checker
            .facts()
            .is_suppressed_subscript_reference(reference.span)
        {
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
        if has_same_name_defining_bindings(checker, &reference.name) {
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

    #[test]
    fn reports_index_arithmetic_subscript_references() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${arr[$read_idx]}\"
[[ -v arr[bare_check] ]]
[[ -v arr[$dynamic_check] ]]
arr[bare_target]=value
arr[$dynamic_target]=value
arr+=([amazoncorretto]=value)
arr+=([$compound_key]=value)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$dynamic_check",
                "bare_target",
                "$dynamic_target",
                "amazoncorretto",
                "$compound_key"
            ]
        );
    }

    #[test]
    fn suppresses_read_and_string_key_bare_subscript_references() {
        let source = "\
#!/bin/bash
declare -A map
printf '%s\\n' \"${arr[$read_idx]}\" \"${map[$assoc_read_idx]}\"
[[ -v arr[bare_check] ]]
map+=([assoc_bare_key]=value)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn subscript_suppression_does_not_hide_later_plain_uses() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${arr[$read_idx]}\"
[[ -v arr[bare_check] ]]
unset arr[$unset_idx]
printf '%s\\n' \"$read_idx\" \"$bare_check\" \"$unset_idx\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$read_idx", "$bare_check", "$unset_idx"]
        );
    }

    #[test]
    fn reports_expansion_references_in_string_key_writes() {
        let source = "\
#!/bin/bash
declare -A map
map[$target_key]=value
map[$id/has_newer]=value
map+=([$compound_key]=value)
declare -A declared=([$declared_key]=value)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target_key", "$id", "$compound_key", "$declared_key"]
        );
    }
}
