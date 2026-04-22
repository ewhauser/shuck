use rustc_hash::FxHashSet;
use shuck_ast::Span;
use shuck_semantic::BindingId;

use crate::{Checker, Rule, Violation};

pub struct FunctionReferencesUnsetParam {
    pub name: String,
}

impl Violation for FunctionReferencesUnsetParam {
    fn rule() -> Rule {
        Rule::FunctionReferencesUnsetParam
    }

    fn message(&self) -> String {
        format!("function `{}` is called with too few arguments", self.name)
    }
}

pub fn function_references_unset_param(checker: &mut Checker) {
    let mut reported = FxHashSet::<BindingId>::default();
    let mut violations = Vec::<(Span, String)>::new();

    for header in checker.facts().function_headers() {
        let Some((name, _)) = header.static_name_entry() else {
            continue;
        };
        let Some(binding_id) = header.binding_id() else {
            continue;
        };
        if !reported.insert(binding_id) {
            continue;
        }

        let Some(function_scope) = header.function_scope() else {
            continue;
        };

        let positional = checker
            .facts()
            .function_positional_parameter_facts(function_scope);
        if !positional.uses_positional_parameters() || positional.resets_positional_parameters() {
            continue;
        }
        let call_arity = header.call_arity();
        if !call_arity.called_only_without_args() {
            continue;
        }

        violations.extend(
            call_arity
                .zero_arg_call_spans()
                .iter()
                .copied()
                .map(|span| (span, name.to_string())),
        );
    }

    for (span, name) in violations {
        checker.report(FunctionReferencesUnsetParam { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_zero_argument_call_sites_for_functions_that_read_positional_parameters() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn reports_each_zero_argument_call_site_when_all_calls_omit_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[1].span.slice(source), "greet");
        assert_eq!(diagnostics[1].span.start.line, 4);
    }

    #[test]
    fn ignores_functions_called_with_enough_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet a b
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_with_mixed_arity_calls() {
        let source = "\
#!/bin/sh
greet() { echo \"$1 $2\"; }
greet a b
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_functions_that_read_special_positional_parameters() {
        let source = "\
#!/bin/sh
usage() { echo \"usage: ${##*/} <files>\"; }
usage
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "usage");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn ignores_guarded_or_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set -- hello
  echo \"${1:-default}\" \"$#\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_guarded_special_positional_parameters() {
        let source = "\
#!/bin/sh
greet() { printf '%s\n' \"${@:-fallback}\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_guarded_special_positional_parameters() {
        let source = "\
#!/bin/sh
greet() { printf '%s\n' \"${name:-${@}}\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn later_redefinitions_do_not_inherit_argumented_calls_from_earlier_bindings() {
        let source = "\
#!/bin/sh
greet() { echo \"$2\"; }
greet ok ok
greet() { echo \"$2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn calls_before_definition_do_not_suppress_zero_argument_call_reports() {
        let source = "\
#!/bin/sh
greet ok ok
greet() { echo \"$2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn nested_scopes_resolve_calls_to_the_visible_binding() {
        let source = "\
#!/bin/sh
foo() { echo \"$2\"; }
wrapper() {
  foo
  foo() { :; }
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn ignores_plain_set_operands_that_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set hello world
  echo \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zero_argument_call_sites_after_set_plus_option_toggle() {
        let source = "\
#!/bin/sh
greet() {
  set +x
  printf '%s\n' \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zero_argument_call_sites_after_bare_set_plus_o() {
        let source = "\
#!/bin/sh
greet() {
  set +o
  printf '%s\n' \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn still_reports_zero_argument_call_sites_after_set_minus_option_toggle() {
        let source = "\
#!/bin/sh
greet() {
  set -x
  printf '%s\n' \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 7);
    }

    #[test]
    fn same_name_functions_in_different_scopes_do_not_mask_each_other() {
        let source = "\
#!/bin/sh
outer_without_args() {
  inner() { echo \"$1 $2\"; }
  inner
}

outer_with_args() {
  inner() { echo \"$1 $2\"; }
  inner ok yes
}

outer_without_args
outer_with_args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "inner");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn nested_command_substitutions_still_count_toward_required_arg_count() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' \"$(printf '%s' \"$1 $2\")\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 6);
    }

    #[test]
    fn backtick_substitutions_still_count_as_zero_arg_calls() {
        let source = "\
#!/bin/sh
greet() { printf '%s\n' \"$1 $2\"; }
value=\"`greet`\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
    }

    #[test]
    fn calls_before_definition_do_not_count_toward_mixed_arity() {
        let source = "\
#!/bin/sh
greet ok yes
greet() { echo \"$1 $2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn wrapper_resolved_targets_do_not_suppress_direct_zero_arg_reports() {
        let source = "\
#!/usr/bin/env bash
greet() { echo \"$1 $2\"; }
command greet ok yes
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn quoted_static_calls_with_arguments_still_suppress_zero_arg_reports() {
        let source = "\
#!/usr/bin/env bash
greet() { echo \"$1 $2\"; }
\"greet\" ok yes
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn nested_set_commands_still_count_as_positional_parameter_resets() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' 'hello world' | while read -r first second; do
    set -- \"$first\" \"$second\"
    printf '%s\n' \"$2\"
  done
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn command_substitution_set_does_not_reset_outer_function_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  status=$(set -- hello world)
  echo \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 6);
    }

    #[test]
    fn command_substitution_local_reset_still_protects_nested_positional_parameter_use() {
        let source = "\
#!/bin/sh
greet() {
  status=\"$(
    set -- hello world
    printf '%s\n' \"$2\"
  )\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn pipeline_set_does_not_reset_outer_function_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' 'hello world' | while read -r first second; do
    set -- \"$first\" \"$second\"
  done
  echo \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 9);
    }

    #[test]
    fn ignores_dynamic_set_operands_that_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  line='hello world'
  set $line
  echo \"$2\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_calls_with_arguments_inside_parameter_expansion_defaults() {
        let source = "\
#!/usr/bin/env bash
GetBuildVersion() {
  local build_revision=\"${1}\"
  printf '%s\n' \"$build_revision\"
}
BUILD_VERSION=\"${BUILD_VERSION:-\"$(GetBuildVersion \"${BUILD_REVISION}\")\"}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
