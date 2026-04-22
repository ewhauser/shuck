use rustc_hash::FxHashSet;
use shuck_ast::Span;
use shuck_semantic::BindingId;

use crate::{Checker, Rule, Violation};

pub struct FunctionCalledWithoutArgs {
    pub name: String,
}

impl Violation for FunctionCalledWithoutArgs {
    fn rule() -> Rule {
        Rule::FunctionCalledWithoutArgs
    }

    fn message(&self) -> String {
        format!("function `{}` is called without arguments", self.name)
    }
}

pub fn function_called_without_args(checker: &mut Checker) {
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

        if !positional.uses_positional_parameters() {
            continue;
        }
        if positional.resets_positional_parameters() {
            continue;
        }
        if !header.call_arity().called_only_without_args() {
            continue;
        }

        violations.push((
            header.function_span_in_source(checker.source()),
            name.to_string(),
        ));
    }

    for (span, name) in violations {
        checker.report(FunctionCalledWithoutArgs { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_functions_called_without_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1\"; }"
        );
    }

    #[test]
    fn ignores_functions_called_with_arguments() {
        let source = "\
#!/bin/sh
greet() { echo \"$@\"; }
greet world
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_without_calls() {
        let source = "\
#!/bin/sh
greet() { echo \"$#\"; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_guarded_positional_parameters() {
        let source = "\
#!/bin/sh
greet() { echo \"${1:-default}\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
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
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
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
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_that_reset_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  set -- hello
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_that_reset_positional_parameters_with_plain_operands() {
        let source = "\
#!/bin/sh
greet() {
  set hello
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_after_set_plus_option_toggle() {
        let source = "\
#!/bin/sh
greet() {
  set +x
  printf '%s\n' \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_functions_after_bare_set_plus_o() {
        let source = "\
#!/bin/sh
greet() {
  set +o
  printf '%s\n' \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn still_reports_functions_after_set_minus_option_toggle() {
        let source = "\
#!/bin/sh
greet() {
  set -x
  printf '%s\n' \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  set -x\n  printf '%s\n' \"$1\"\n}"
        );
    }

    #[test]
    fn reports_functions_that_use_variadic_positional_parameters() {
        let source = "\
#!/bin/sh
die() { echo \"$@\"; }
die
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "die() { echo \"$@\"; }");
    }

    #[test]
    fn reports_functions_that_use_argument_count() {
        let source = "\
#!/bin/sh
die() { echo \"$#\"; }
die
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "die() { echo \"$#\"; }");
    }

    #[test]
    fn ignores_call_sites_that_precede_the_definition_if_arguments_are_passed() {
        let source = "\
#!/bin/sh
main() {
  greet 1
  greet
}
main

greet() { echo \"$1\"; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn nested_functions_are_tracked_independently() {
        let source = "\
#!/bin/sh
outer() {
  inner() { echo \"$1\"; }
  inner
}
outer
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "inner() { echo \"$1\"; }"
        );
    }

    #[test]
    fn same_name_functions_in_different_scopes_do_not_mask_each_other() {
        let source = "\
#!/bin/sh
outer_without_args() {
  inner() { echo \"$1\"; }
  inner
}

outer_with_args() {
  inner() { echo \"$1\"; }
  inner ok
}

outer_without_args
outer_with_args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "inner() { echo \"$1\"; }"
        );
    }

    #[test]
    fn nested_command_substitutions_still_count_as_positional_parameter_use() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' \"$(printf '%s' \"$1\")\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  printf '%s\n' \"$(printf '%s' \"$1\")\"\n}"
        );
    }

    #[test]
    fn backtick_substitutions_still_count_as_zero_arg_calls() {
        let source = "\
#!/bin/sh
greet() { printf '%s\n' \"$1\"; }
value=\"`greet`\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { printf '%s\n' \"$1\"; }"
        );
    }

    #[test]
    fn calls_before_definition_do_not_count_toward_mixed_arity() {
        let source = "\
#!/bin/sh
greet ok
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1\"; }"
        );
    }

    #[test]
    fn later_redefinitions_do_not_inherit_argumented_calls_from_earlier_bindings() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet ok
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1\"; }"
        );
    }

    #[test]
    fn nested_scopes_resolve_calls_to_the_visible_binding() {
        let source = "\
#!/bin/sh
foo() { echo \"$1\"; }
wrapper() {
  foo
  foo() { :; }
}
wrapper
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo() { echo \"$1\"; }");
    }

    #[test]
    fn nested_set_commands_still_count_as_positional_parameter_resets() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' hello | while read -r word; do
    set -- \"$word\"
    printf '%s\n' \"$1\"
  done
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn subshell_set_does_not_reset_outer_function_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  (set -- hello)
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  (set -- hello)\n  echo \"$1\"\n}"
        );
    }

    #[test]
    fn subshell_local_reset_still_protects_nested_positional_parameter_use() {
        let source = "\
#!/bin/sh
greet() {
  (
    set -- hello
    printf '%s\n' \"$1\"
  )
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn pipeline_set_does_not_reset_outer_function_positional_parameters() {
        let source = "\
#!/bin/sh
greet() {
  printf '%s\n' hello | while read -r word; do
    set -- \"$word\"
  done
  echo \"$1\"
}
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  printf '%s\n' hello | while read -r word; do\n    set -- \"$word\"\n  done\n  echo \"$1\"\n}"
        );
    }
}
