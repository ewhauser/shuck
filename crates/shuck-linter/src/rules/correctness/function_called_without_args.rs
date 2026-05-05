use compact_str::CompactString;
use rustc_hash::FxHashSet;
use shuck_ast::Span;
use shuck_semantic::BindingId;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionCalledWithoutArgs {
    pub name: CompactString,
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
    let mut violations = Vec::<(Span, CompactString)>::new();

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
        if zsh_function_arity_is_externally_defined(checker, function_scope) {
            continue;
        }
        if !header.call_arity().called_only_without_args() {
            continue;
        }

        violations.push((
            header.function_span_in_source(checker.source()),
            name.as_str().into(),
        ));
    }

    for (span, name) in violations {
        checker.report(FunctionCalledWithoutArgs { name }, span);
    }
}

fn zsh_function_arity_is_externally_defined(
    checker: &Checker<'_>,
    function_scope: shuck_semantic::ScopeId,
) -> bool {
    checker.shell() == ShellDialect::Zsh
        && (checker
            .facts()
            .function_is_external_entrypoint(function_scope)
            || checker
                .facts()
                .function_positional_parameter_facts(function_scope)
                .required_arg_count()
                == 0)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

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
    fn ignores_zsh_callback_functions_that_read_optional_positional_parameters() {
        let source = "\
#!/bin/zsh
single_operand_widget() {
  print -r -- \"$1\"
}
_zsh_autosuggest_toggle() {
  [[ -n \"$1\" ]] && BUFFER=\"$1\"
  printf '%s\n' \"$@\"
}
zle -N single_operand_widget
zle -N autosuggest-toggle _zsh_autosuggest_toggle
single_operand_widget
_zsh_autosuggest_toggle
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_hook_functions_registered_with_flags() {
        let source = "\
#!/bin/zsh
precmd_refresh() { print -r -- \"$1\"; }
add-zsh-hook -Uz precmd precmd_refresh
precmd_refresh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_standard_hook_functions() {
        let source = "\
#!/bin/zsh
precmd() { print -r -- \"$1\"; }
precmd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_directory_name_hook_function() {
        let source = "\
#!/bin/zsh
zsh_directory_name() { print -r -- \"$1\"; }
zsh_directory_name
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_functions_called_without_required_arguments() {
        let source = "\
#!/bin/zsh
greet() { print -r -- \"$1\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "greet() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn reports_zsh_functions_removed_from_hooks() {
        let source = "\
#!/bin/zsh
removed_precmd() { print -r -- \"$1\"; }
removed_chpwd() { print -r -- \"$1\"; }
add-zsh-hook chpwd removed_chpwd
add-zsh-hook -d precmd removed_precmd
add-zsh-hook -UD chpwd removed_chpwd
removed_precmd
removed_chpwd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "removed_precmd() { print -r -- \"$1\"; }"
        );
        assert_eq!(
            diagnostics[1].span.slice(source),
            "removed_chpwd() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn reports_zsh_functions_removed_from_hooks_by_pattern() {
        let source = "\
#!/bin/zsh
cb_one() { print -r -- \"$1\"; }
cb_two() { print -r -- \"$1\"; }
cb_keep() { print -r -- \"$1\"; }
add-zsh-hook precmd cb_one
add-zsh-hook precmd cb_two
add-zsh-hook precmd cb_keep
add-zsh-hook -D precmd 'cb_t*'
cb_one
cb_two
cb_keep
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "cb_two() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn reports_zsh_functions_removed_from_widgets() {
        let source = "\
#!/bin/zsh
removed_widget() { print -r -- \"$1\"; }
zle -N removed-widget removed_widget
zle -D removed-widget
removed_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "removed_widget() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn ignores_zsh_functions_still_registered_as_widgets() {
        let source = "\
#!/bin/zsh
shared_widget() { print -r -- \"$1\"; }
zle -N first-widget shared_widget
zle -N second-widget shared_widget
zle -D first-widget
shared_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_functions_registered_as_zle_hook_widgets() {
        let source = "\
#!/bin/zsh
line_init_widget() { print -r -- \"$1\"; }
line_init_impl() { print -r -- \"$1\"; }
zle -N line-init-alias line_init_impl
add-zle-hook-widget line-init line_init_widget
add-zle-hook-widget line-init line-init-alias
line_init_widget
line_init_impl
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_functions_registered_by_unexecuted_setup_functions() {
        let source = "\
#!/bin/zsh
latent_widget() { print -r -- \"$1\"; }
latent_hook() { print -r -- \"$1\"; }
setup_widget() { zle -N latent-widget latent_widget; }
setup_hook() { add-zsh-hook precmd latent_hook; }
latent_widget
latent_hook
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "latent_widget() { print -r -- \"$1\"; }"
        );
        assert_eq!(
            diagnostics[1].span.slice(source),
            "latent_hook() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn reports_zsh_functions_removed_from_hooks_by_richer_patterns() {
        let source = "\
#!/bin/zsh
cb_1() { print -r -- \"$1\"; }
cb_2() { print -r -- \"$1\"; }
cb_10() { print -r -- \"$1\"; }
cb_a() { print -r -- \"$1\"; }
cb_x() { print -r -- \"$1\"; }
cb_keep() { print -r -- \"$1\"; }
add-zsh-hook precmd cb_1
add-zsh-hook precmd cb_2
add-zsh-hook precmd cb_10
add-zsh-hook precmd cb_a
add-zsh-hook precmd cb_x
add-zsh-hook precmd cb_keep
add-zsh-hook -D precmd 'cb_[12]'
add-zsh-hook -D precmd 'cb_<->'
add-zsh-hook -D precmd 'cb_[^x]'
cb_1
cb_2
cb_10
cb_a
cb_x
cb_keep
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 4);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "cb_1() { print -r -- \"$1\"; }"
        );
        assert_eq!(
            diagnostics[1].span.slice(source),
            "cb_2() { print -r -- \"$1\"; }"
        );
        assert_eq!(
            diagnostics[2].span.slice(source),
            "cb_10() { print -r -- \"$1\"; }"
        );
        assert_eq!(
            diagnostics[3].span.slice(source),
            "cb_a() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn reports_zsh_functions_with_dynamic_widget_registration() {
        let source = "\
#!/bin/zsh
dynamic_widget() { print -r -- \"$1\"; }
zle -N \"$widget_name\" dynamic_widget
dynamic_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "dynamic_widget() { print -r -- \"$1\"; }"
        );
    }

    #[test]
    fn ignores_zsh_functions_that_only_forward_special_positionals() {
        let source = "\
#!/bin/zsh
forward() { print -r -- \"$@\"; }
forward
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_functions_that_only_read_argument_count() {
        let source = "\
#!/bin/zsh
has_args() { (( $# )) && print -r -- ok; }
has_args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionCalledWithoutArgs)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
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
    fn earlier_calls_in_same_scope_still_count_toward_mixed_arity() {
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

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
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
