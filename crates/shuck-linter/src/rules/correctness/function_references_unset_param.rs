use compact_str::CompactString;
use rustc_hash::FxHashSet;
use shuck_ast::Span;
use shuck_semantic::BindingId;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionReferencesUnsetParam {
    pub name: CompactString,
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
        if checker.is_suppressed_at(
            Rule::FunctionCalledWithoutArgs,
            header.function_span_in_source(checker.source()),
        ) {
            continue;
        }

        let positional = checker
            .facts()
            .function_positional_parameter_facts(function_scope);
        if !positional.uses_positional_parameters() || positional.resets_positional_parameters() {
            continue;
        }
        if zsh_function_arity_is_externally_defined(checker, function_scope) {
            continue;
        }
        let call_arity = header.call_arity();
        if !call_arity.called_only_without_args() {
            continue;
        }

        violations.extend(
            call_arity
                .zero_arg_diagnostic_spans()
                .iter()
                .copied()
                .map(|span| (span, name.as_str().into())),
        );
    }

    for (span, name) in violations {
        checker.report(FunctionReferencesUnsetParam { name }, span);
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
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;

    use crate::test::test_snippet;
    use crate::{
        LinterSettings, Rule, ShellCheckCodeMap, ShellDialect, lint_file_with_directives,
        parse_directives,
    };

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
    fn ignores_zsh_widget_and_hook_callbacks_with_optional_arguments() {
        let source = "\
#!/bin/zsh
single_operand_widget() {
  print -r -- \"$1\"
}
_zsh_autosuggest_widget_accept() {
  local original_widget=\"$1\"
  shift || true
  \"$original_widget\" \"$@\"
}
precmd_refresh() {
  (( $# )) && print -r -- \"$1\"
}
zle -N autosuggest-accept _zsh_autosuggest_widget_accept
zle -N single_operand_widget
add-zsh-hook precmd precmd_refresh
_zsh_autosuggest_widget_accept
single_operand_widget
precmd_refresh
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_standard_hook_call_sites() {
        let source = "\
#!/bin/zsh
preexec() { print -r -- \"$1\"; }
preexec
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_directory_name_call_sites() {
        let source = "\
#!/bin/zsh
zsh_directory_name() { print -r -- \"$1\"; }
zsh_directory_name
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_call_sites_missing_required_arguments() {
        let source = "\
#!/bin/zsh
greet() { print -r -- \"$1 $2\"; }
greet
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "greet");
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn reports_zsh_call_sites_removed_from_hooks() {
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
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "removed_precmd");
        assert_eq!(diagnostics[0].span.start.line, 7);
        assert_eq!(diagnostics[1].span.slice(source), "removed_chpwd");
        assert_eq!(diagnostics[1].span.start.line, 8);
    }

    #[test]
    fn reports_zsh_call_sites_removed_from_hooks_by_pattern() {
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
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cb_two");
        assert_eq!(diagnostics[0].span.start.line, 10);
    }

    #[test]
    fn reports_zsh_call_sites_removed_from_hooks_by_richer_patterns() {
        let source = "\
#!/bin/zsh
cb_1() { print -r -- \"$1\"; }
cb_2() { print -r -- \"$1\"; }
cb_10() { print -r -- \"$1\"; }
cb_keep() { print -r -- \"$1\"; }
add-zsh-hook precmd cb_1
add-zsh-hook precmd cb_2
add-zsh-hook precmd cb_10
add-zsh-hook precmd cb_keep
add-zsh-hook -D precmd 'cb_[12]'
add-zsh-hook -D precmd 'cb_<->'
cb_1
cb_2
cb_10
cb_keep
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.slice(source), "cb_1");
        assert_eq!(diagnostics[0].span.start.line, 12);
        assert_eq!(diagnostics[1].span.slice(source), "cb_2");
        assert_eq!(diagnostics[1].span.start.line, 13);
        assert_eq!(diagnostics[2].span.slice(source), "cb_10");
        assert_eq!(diagnostics[2].span.start.line, 14);
    }

    #[test]
    fn reports_zsh_call_sites_removed_from_widgets() {
        let source = "\
#!/bin/zsh
removed_widget() { print -r -- \"$1\"; }
zle -N removed-widget removed_widget
zle -D removed-widget
removed_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "removed_widget");
        assert_eq!(diagnostics[0].span.start.line, 5);
    }

    #[test]
    fn ignores_zsh_call_sites_still_registered_as_widgets() {
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
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_call_sites_registered_as_zle_hook_widgets() {
        let source = "\
#!/bin/zsh
line_init_widget() { print -r -- \"$1\"; }
add-zle-hook-widget line-init line_init_widget
line_init_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_call_sites_registered_by_unexecuted_setup_functions() {
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
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "latent_widget");
        assert_eq!(diagnostics[0].span.start.line, 6);
        assert_eq!(diagnostics[1].span.slice(source), "latent_hook");
        assert_eq!(diagnostics[1].span.start.line, 7);
    }

    #[test]
    fn reports_zsh_call_sites_with_dynamic_widget_registration() {
        let source = "\
#!/bin/zsh
dynamic_widget() { print -r -- \"$1 $2\"; }
zle -N \"$widget_name\" dynamic_widget
dynamic_widget
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "dynamic_widget");
        assert_eq!(diagnostics[0].span.start.line, 4);
    }

    #[test]
    fn ignores_zsh_call_sites_for_special_positional_forwarders() {
        let source = "\
#!/bin/zsh
forward() { print -r -- \"$@\"; }
forward
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_call_sites_for_argument_count_checks() {
        let source = "\
#!/bin/zsh
has_args() { (( $# )) && print -r -- ok; }
has_args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam)
                .with_shell(ShellDialect::Zsh),
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

    #[test]
    fn respects_suppressed_paired_function_argument_warning() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2120
greet() { echo \"$1\"; }
greet
";
        let parse_result = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &parse_result);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let diagnostics = lint_file_with_directives(
            &parse_result,
            source,
            &indexer,
            &LinterSettings::for_rule(Rule::FunctionReferencesUnsetParam),
            &directives,
            None,
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
