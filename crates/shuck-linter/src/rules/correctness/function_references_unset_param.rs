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
        let required_arg_count = positional.required_arg_count();

        if required_arg_count <= 1 || positional.resets_positional_parameters() {
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
        checker.report(FunctionReferencesUnsetParam { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_functions_called_with_too_few_arguments() {
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$1 $2\"; }"
        );
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
    fn ignores_functions_with_only_one_required_argument() {
        let source = "\
#!/bin/sh
greet() { echo \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { echo \"$2\"; }"
        );
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
        assert_eq!(diagnostics[0].span.slice(source), "foo() { echo \"$2\"; }");
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "inner() { echo \"$1 $2\"; }"
        );
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  printf '%s\n' \"$(printf '%s' \"$1 $2\")\"\n}"
        );
    }

    #[test]
    fn earlier_calls_in_same_scope_still_count_toward_mixed_arity() {
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  status=$(set -- hello world)\n  echo \"$2\"\n}"
        );
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
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() {\n  printf '%s\n' 'hello world' | while read -r first second; do\n    set -- \"$first\" \"$second\"\n  done\n  echo \"$2\"\n}"
        );
    }
}
