use crate::{Checker, Rule, Violation};

pub struct DollarInArithmetic;

impl Violation for DollarInArithmetic {
    fn rule() -> Rule {
        Rule::DollarInArithmetic
    }

    fn message(&self) -> String {
        "omit the `$` prefix inside arithmetic".to_owned()
    }
}

pub fn dollar_in_arithmetic(checker: &mut Checker) {
    let spans = checker.facts().dollar_in_arithmetic_spans().to_vec();

    checker.report_all_dedup(spans, || DollarInArithmetic);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_assignments() {
        let source = "#!/bin/bash\nn=1\nm=$(($n + 1))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$n");
    }

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_command_arguments() {
        let source = "#!/bin/bash\nn=1\nprintf '%s\\n' \"$(($n + 1))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$n");
    }

    #[test]
    fn reports_braced_arithmetic_variables_in_command_arguments() {
        let source = "#!/bin/bash\nx=1\nprintf '%s\\n' \"$((${x} + 1))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${x}");
    }

    #[test]
    fn reports_dollar_prefixed_arithmetic_variables_in_command_context() {
        let source = "#!/bin/bash\nx=1\n(( $x + 1 ))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$x");
    }

    #[test]
    fn reports_braced_arithmetic_variables_in_command_context() {
        let source = "#!/bin/bash\nx=1\n(( ${x} + 1 ))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "${x}");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_arithmetic_for_clauses() {
        let source = "#!/bin/bash\nlimit=3\nfor (( i=$limit; i > 0; i-- )); do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$limit");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_substring_offset_arithmetic() {
        let source =
            "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:$((${#rest}-$len))}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$len");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_substring_length_arithmetic() {
        let source = "#!/bin/bash\nstring=abcdef\nwidth=10\nprintf '%s\\n' \"${string:0:$(( $width - 4 ))}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$width");
    }

    #[test]
    fn ignores_plain_substring_offset_parameter_expansions() {
        let source = "#!/bin/bash\nrest=abcdef\nlen=2\nprintf '%s\\n' \"${rest:${len}:1}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_plain_positional_slice_parameter_expansions() {
        let source = "#!/bin/bash\nargs_offset=$#\nprintf '%s\\n' \"${@:1:${args_offset}}\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_parameter_replacement_arithmetic() {
        let source = "#!/bin/bash\noffset=1\nindex=2\nline=x\necho \"${line/ $index / $(($offset + $index)) }\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "$offset");
        assert_eq!(diagnostics[1].span.slice(source), "$index");
    }

    #[test]
    fn ignores_assignments_without_arithmetic_dollar_variables() {
        let source = "#!/bin/bash\nm=$((1 + 1))\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_positional_parameters_in_arithmetic_expressions() {
        let source = "#!/bin/bash\necho \"$(( $1 / 2 ))\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_simple_subscripted_parameter_accesses_in_arithmetic_expressions() {
        let source = "\
#!/bin/bash
declare -a ver
declare -A assoc
echo \"$(( ${ver[0]} + ${ver[i]} + ${assoc[key]} + 1 ))\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${ver[0]}", "${ver[i]}", "${assoc[key]}"]
        );
    }

    #[test]
    fn reports_dollar_prefixed_indexed_assignment_subscripts() {
        let source = "\
#!/bin/bash
declare -a arr
i=1
lang=en
arr[$i]=x
arr[$i+1]=y
arr[$i/repo_dir]=z
arr[${lang},27]=q
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i", "$i", "$i", "${lang}"]
        );
    }

    #[test]
    fn ignores_associative_assignment_subscripts_for_dollar_in_arithmetic() {
        let source = "\
#!/bin/bash
declare -A assoc
key=name
lang=en
assoc[$key]=x
assoc[${lang},27]=y
assoc[$key/sfx]=z
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_mixed_assoc_and_indexed_assignment_subscripts_in_branches() {
        let source = "\
#!/bin/bash
f() {
  if cond; then
    local -A arr
  else
    local -a arr
  fi
  idx=0
  arr[${idx}]=x
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_mixed_assoc_and_indexed_assignment_subscripts_in_linear_flow() {
        let source = "\
#!/bin/bash
f() {
  local -A arr
  local -a arr
  idx=0
  arr[${idx}]=x
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_caller_local_shadowing_after_assoc_declaration() {
        let source = "\
#!/bin/bash
helper() {
  map[$key]=1
}
main() {
  local key=name
  declare -A map
  unset map
  local map
  helper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$key"]
        );
    }

    #[test]
    fn ignores_quoted_indexed_assignment_subscripts() {
        let source = "\
#!/bin/bash
declare -a arr
wash_counter=1
arr[\"${wash_counter}\"]=x
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_command_substitutions_in_indexed_assignment_subscripts() {
        let source = "\
#!/bin/bash
declare -a arr
i=file
arr[$(printf '%s' \"$i\")]=x
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_multi_declared_associative_append_assignment_subscripts() {
        let source = "\
#!/bin/bash
declare -A one=() two=() seen=()
key=name
one[$key]+=x
two[$key]+=y
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_globally_declared_associative_assignment_subscripts() {
        let source = "\
#!/bin/bash
init() {
  declare -gA map
}
helper() {
  map[$key]=1
}
main() {
  key=name
  init
  helper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_globally_declared_associative_assignment_subscripts_with_combined_flags() {
        let source = "\
#!/bin/bash
init() {
  declare -Ag map=()
}
helper() {
  map[$key/field]=1
}
main() {
  key=name
  init
  helper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_dynamic_scope_associative_assignment_subscripts() {
        let source = "\
#!/bin/bash
helper() {
  map[${key}]=1
}
wrapper() {
  helper
}
main() {
  local key=name
  declare -A map
  wrapper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_shadowing_local_subscripts_even_when_callers_have_assoc_bindings() {
        let source = "\
#!/bin/bash
helper() {
  local map
  map[$key]=1
}
main() {
  local key=name
  declare -A map
  helper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$key");
    }

    #[test]
    fn ignores_repeated_dynamic_scope_associative_assignment_subscripts() {
        let source = "\
#!/bin/bash
helper() {
  map[${key}]=1
  map[${other}]=2
}
main() {
  local key=alpha
  local other=beta
  declare -A map
  helper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_wrapper_shadowing_local_subscripts_even_when_outer_callers_have_assoc_bindings() {
        let source = "\
#!/bin/bash
helper() {
  map[$key]=1
}
wrapper() {
  local map
  helper
}
main() {
  local key=name
  declare -A map
  wrapper
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$key");
    }

    #[test]
    fn ignores_associative_declaration_initializer_subscripts() {
        let source = "\
#!/bin/bash
declare -A map=([$key]=1)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_nested_arithmetic_in_array_access_subscripts() {
        let source = "\
#!/bin/bash
declare -a tools
choice=2
printf '%s\\n' \"${tools[$(($choice-1))]}\"\n\
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$choice"]
        );
    }

    #[test]
    fn reports_nested_arithmetic_in_associative_assignment_subscripts() {
        let source = "\
#!/bin/bash
declare -A assoc
choice=2
assoc[$(($choice-1))]=x
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$choice"]
        );
    }

    #[test]
    fn reports_dollar_prefixed_variables_in_eval_strings() {
        let source = "\
#!/bin/bash
i=0
eval \"rssi=\\\"\\\\$rssi${i}\\\"; i=$(( $i + 1 ))\"\n\
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i"]
        );
    }

    #[test]
    fn ignores_dynamic_and_compound_subscript_parameter_accesses_in_arithmetic_expressions() {
        let source = "\
#!/bin/bash
declare -a ver
declare -A assoc
i=0
key=name
echo \"$(( ${ver[$i]} + ${ver[i+1]} + ${ver[-1]} + ${assoc[$key]} + 1 ))\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DollarInArithmetic));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
