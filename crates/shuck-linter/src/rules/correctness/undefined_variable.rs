use compact_str::CompactString;
use rustc_hash::FxHashSet;
use shuck_semantic::UninitializedCertainty;

use crate::{Checker, Rule, Violation};

use super::variable_reference_common::{
    VariableReferenceFilter, has_same_name_defining_bindings, is_reportable_variable_reference,
};

pub struct UndefinedVariable {
    pub name: CompactString,
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
        if checker
            .facts()
            .is_backtick_double_escaped_parameter_reference(reference.span)
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
                name: reference.name.as_str().into(),
                certainty: uninitialized.certainty,
            },
            reference.span,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn prior_defaulting_parameter_operands_suppress_later_plain_uses() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"${missing_assign:=$seed_name}\" \"${missing_error:?$hint_name}\"
printf '%s\\n' \"$seed_name\" \"$hint_name\" \"$plain_missing\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$plain_missing"]
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
guard_function() { printf '%s\\n' \"${cross_scope:?missing}\"; }
read_function() { printf '%s\\n' \"$cross_scope\"; }
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
printf '%s\\n' \"${other:+${nested_replacement:+alt}}\" \"$other\" \"$nested_replacement\" \"$plain_missing\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$plain_missing"]
        );
    }

    #[test]
    fn later_parameter_guards_do_not_suppress_earlier_reads() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$before_default\" \"$before_error\"
printf '%s\\n' \"${before_default:-fallback}\" \"${before_error:?missing}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$before_default", "$before_error"]
        );
    }

    #[test]
    fn nested_presence_tests_suppress_same_name_c006_reports() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$late_guarded\"
options=(
  no_ask \"$( [[ -n \"$no_ask\" ]] && printf true || printf false)\"
  truthy \"$( [ \"$truthy\" ] && printf true || printf false)\"
)
printf '%s\\n' \"$( [[ -n \"$late_guarded\" ]] && printf true)\"
printf '%s\\n' \"$no_ask\" \"$truthy\"
printf '%s\\n' \"$(test -n \"$plain_test\" && printf true)\"
printf '%s\\n' \"$( [[ -s \"$file_test\" ]] && printf true)\"
printf '%s\\n' \"$plain_test\" \"$file_test\" \"$still_missing\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$plain_test", "$file_test", "$still_missing"]
        );
    }

    #[test]
    fn nested_presence_tests_suppress_same_name_c006_reports_across_functions() {
        let source = "\
#!/bin/bash
guarded_flag() {
  printf '%s\\n' \"$( [[ -n \"$shared_flag\" ]] && printf true || printf false)\"
}
read_flag() {
  printf '%s\\n' \"$shared_flag\" \"$unrelated_flag\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UndefinedVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$unrelated_flag"]
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
    fn suppresses_zsh_option_map_key_arithmetic_references() {
        let source = "\
#!/bin/zsh
f() {
  local quiet=0
  ( (( !OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn suppresses_zsh_existence_test_fake_variable_references() {
        let source = "\
#!/bin/zsh
if (( $+commands[git] )); then
  :
fi
if (( ${+functions[zdot_warn]} )); then
  :
fi
if (( $+ZINIT_CNORM )); then
  :
fi
if (( $+commands[$cmd] )); then
  :
fi
if (( ${+functions[$fn]} )); then
  :
fi
if (( $+arr[i+1] )); then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$cmd", "$fn", "i"]
        );
    }

    #[test]
    fn suppresses_zsh_associative_key_fake_variable_references() {
        let source = "\
#!/bin/zsh
typeset -A ZINIT ICE
ZINIT[ice-list]=x
ICE[ps-on-update]=x
functions[iterm2_precmd]=x
print -r -- ${functions[iterm2_precmd]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn zparseopts_targets_initialize_option_arrays() {
        let source = "\
#!/bin/zsh
zparseopts -D -E -F -a all -A optmap -- \\
  h=help -help=help \\
  v+:=verbose -verbose+:=verbose \\
  o:=output -output:=output
printf '%s\\n' \"$all\" \"$optmap\" \"$help\" \"$verbose\" \"$output\" \"$missing\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$missing"]
        );
    }

    #[test]
    fn zparseopts_attached_array_targets_are_arrays() {
        let source = "\
#!/bin/zsh
zparseopts -aall -Aassoc -- x:=xout y=yout
printf '%s\\n' \"${all[1]}\" \"${assoc[-x]}\" \"${xout[1]}\" \"${yout[1]}\" \"$missing\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$missing"]
        );
    }

    #[test]
    fn zparseopts_dynamic_targets_still_report_dynamic_names() {
        let source = "\
#!/bin/zsh
zparseopts -a$aggregate -- x=$target_name
printf '%s\\n' \"$aggregate\" \"$target_name\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$aggregate", "$target_name"]
        );
    }

    #[test]
    fn zparseopts_targets_do_not_initialize_names_in_bash() {
        let source = "\
#!/bin/bash
zparseopts -- x=target
printf '%s\\n' \"$target\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Bash),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn zparseopts_stacked_looking_specs_initialize_targets() {
        let source = "\
#!/bin/zsh
zparseopts -- -DEK=dest
printf '%s\\n' \"$dest\" \"$missing\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$missing"]
        );
    }

    #[test]
    fn zparseopts_mapping_does_not_initialize_spec_alias_names() {
        let source = "\
#!/bin/zsh
zparseopts -A bar -M a=foo b+: c:=b
printf '%s\\n' \"$bar\" \"$foo\" \"$b\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UndefinedVariable).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$b"]
        );
    }

    #[test]
    fn subscript_suppression_hides_later_same_name_uses() {
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
            vec!["$bare_check", "$unset_idx"]
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
