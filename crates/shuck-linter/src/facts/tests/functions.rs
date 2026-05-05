use super::*;
use shuck_ast::Command;

#[test]
fn function_header_fact_span_in_source_stops_at_header() {
    let source = "#!/bin/bash\nfunction wrapped()\n{\n  printf '%s\\n' hi\n}\n";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .first()
            .expect("expected function header fact");

        assert_eq!(
            header.span_in_source(source).slice(source),
            "function wrapped()"
        );
    });
}

#[test]
fn function_header_fact_tracks_binding_scope_and_call_arity() {
    let source = "#!/bin/sh\ngreet ok\ngreet() { echo \"$1\"; }\ngreet\n";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");

        assert!(header.binding_id().is_some());
        assert!(header.function_scope().is_some());
        assert_eq!(header.call_arity().call_count(), 2);
        assert_eq!(header.call_arity().min_arg_count(), Some(0));
        assert_eq!(header.call_arity().max_arg_count(), Some(1));
        assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
        assert_eq!(
            header.call_arity().zero_arg_call_spans()[0].slice(source),
            "greet"
        );
    });
}

#[test]
fn function_definition_command_lookup_returns_header_command() {
    let source = "#!/bin/sh\ngreet() { echo hi; }\n";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");
        let scope = header
            .function_scope()
            .expect("expected greet function scope");
        let definition_command = facts
            .function_definition_command(scope)
            .expect("expected definition command");

        assert_eq!(definition_command.id(), header.command_id());
        assert!(matches!(definition_command.command(), Command::Function(_)));
    });
}

#[test]
fn command_for_name_word_span_resolves_definition_and_call_commands() {
    let source = "#!/bin/sh\ngreet() { echo hi; }\ngreet\n";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");
        let definition_command = facts.command(header.command_id());
        let call_name_span = header.call_arity().zero_arg_call_spans()[0];

        assert_eq!(
            facts
                .command_for_name_word_span(definition_command.span())
                .map(|command| command.id()),
            Some(header.command_id())
        );
        assert_eq!(
            facts
                .command_for_name_word_span(call_name_span)
                .and_then(|command| command.body_name_word().map(|word| word.span.slice(source))),
            Some("greet")
        );
    });
}

#[test]
fn function_header_fact_tracks_call_arity_inside_parameter_expansion_defaults() {
    let source = "\
#!/usr/bin/env bash
GetBuildVersion() {
  local build_revision=\"${1}\"
  printf '%s\n' \"$build_revision\"
}
BUILD_VERSION=\"${BUILD_VERSION:-\"$(GetBuildVersion \"${BUILD_REVISION}\")\"}\"
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "GetBuildVersion")
            })
            .expect("expected GetBuildVersion header fact");

        assert_eq!(header.call_arity().call_count(), 1);
        assert_eq!(header.call_arity().min_arg_count(), Some(1));
        assert_eq!(header.call_arity().max_arg_count(), Some(1));
        assert!(header.call_arity().zero_arg_call_spans().is_empty());
    });
}

#[test]
fn function_header_fact_ignores_wrapper_resolved_targets_for_call_arity() {
    let source = "\
#!/usr/bin/env bash
greet() { printf '%s\n' \"$1\"; }
command greet ok
greet
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");

        assert_eq!(header.call_arity().call_count(), 1);
        assert_eq!(header.call_arity().min_arg_count(), Some(0));
        assert_eq!(header.call_arity().max_arg_count(), Some(0));
        assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
        assert_eq!(
            header.call_arity().zero_arg_call_spans()[0].slice(source),
            "greet"
        );
    });
}

#[test]
fn function_header_fact_counts_quoted_static_calls_in_call_arity() {
    let source = "\
#!/usr/bin/env bash
greet() { printf '%s\n' \"$1\"; }
\"greet\" ok
greet
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");

        assert_eq!(header.call_arity().call_count(), 2);
        assert_eq!(header.call_arity().min_arg_count(), Some(0));
        assert_eq!(header.call_arity().max_arg_count(), Some(1));
        assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
        assert_eq!(
            header.call_arity().zero_arg_call_spans()[0].slice(source),
            "greet"
        );
    });
}

#[test]
fn function_header_fact_tracks_zero_arg_backtick_calls() {
    let source = "\
#!/bin/sh
greet() { printf '%s\n' \"$1\"; }
value=\"`greet`\"
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");

        assert_eq!(header.call_arity().call_count(), 1);
        assert_eq!(header.call_arity().min_arg_count(), Some(0));
        assert_eq!(header.call_arity().max_arg_count(), Some(0));
        assert_eq!(header.call_arity().zero_arg_call_spans().len(), 1);
        assert_eq!(
            header.call_arity().zero_arg_call_spans()[0].slice(source),
            "greet"
        );
    });
}

#[test]
fn function_header_fact_expands_zero_arg_call_spans_over_redirects() {
    let source = "\
#!/bin/sh
greet() { printf '%s\n' \"$1\"; }
greet >out
greet <<EOF
hi
EOF
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "greet")
            })
            .expect("expected greet header fact");

        let spans = header
            .call_arity()
            .zero_arg_diagnostic_spans()
            .iter()
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["greet >out", "greet <<EOF"]);
    });
}

#[test]
fn function_cli_dispatch_facts_track_case_positional_entrypoints_with_top_level_exit() {
    let source = "\
#!/bin/sh
start() { echo hi; }
stop() { echo bye; }
case \"$1\" in
  start) $1 ;;
  stop) \"$1\" ;;
esac
exit $?
";

    with_facts(source, None, |_, facts| {
        for name in ["start", "stop"] {
            let header = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(candidate, _)| candidate == name)
                })
                .expect("expected dispatched function header");
            let scope = header.function_scope().expect("expected function scope");
            let dispatch = facts.function_cli_dispatch_facts(scope);

            assert!(
                dispatch.exported_from_case_cli(),
                "expected {name} to be marked"
            );
            assert_eq!(
                dispatch
                    .dispatcher_span()
                    .expect("expected dispatcher span")
                    .slice(source),
                if name == "start" { "$1" } else { "\"$1\"" }
            );
        }
    });
}

#[test]
fn case_cli_reachable_function_scopes_track_dispatchable_functions() {
    let source = "\
#!/bin/sh
start() { echo hi; }
case \"$1\" in
  start) \"$1\" ;;
esac
exit $?
late() { echo later; }
";

    with_facts(source, None, |_, facts| {
        let start = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "start")
            })
            .expect("expected start header fact");
        let late = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(name, _)| name == "late")
            })
            .expect("expected late header fact");

        assert!(facts.is_case_cli_reachable_function_scope(
            start.function_scope().expect("expected start scope")
        ));
        assert!(!facts.is_case_cli_reachable_function_scope(
            late.function_scope().expect("expected late scope")
        ));
    });
}

#[test]
fn case_cli_reachable_function_scopes_count_anonymous_parent_functions() {
    let source = "\
#!/bin/zsh
case \"$1\" in
  start) \"$1\" ;;
esac
exit $?
function {
  helper() { echo helper; }
}
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let helper = facts
                .function_headers()
                .iter()
                .find(|header| {
                    header
                        .static_name_entry()
                        .is_some_and(|(name, _)| name == "helper")
                })
                .expect("expected helper header fact");

            assert!(facts.is_case_cli_reachable_function_scope(
                helper.function_scope().expect("expected helper scope")
            ));
        },
    );
}

#[test]
fn function_cli_dispatch_facts_track_case_positional_entrypoints_with_literal_top_level_exit() {
    let source = "\
#!/bin/sh
start() { echo hi; }
case \"$1\" in
  start) $1 ;;
esac
exit 0
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(candidate, _)| candidate == "start")
            })
            .expect("expected start header");
        let scope = header.function_scope().expect("expected function scope");

        assert!(
            facts
                .function_cli_dispatch_facts(scope)
                .exported_from_case_cli()
        );
    });
}

#[test]
fn function_cli_dispatch_facts_ignore_case_positional_entrypoints_without_top_level_exit() {
    let source = "\
#!/bin/sh
start() { echo hi; }
case \"$1\" in
  start) $1 ;;
esac
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(candidate, _)| candidate == "start")
            })
            .expect("expected start header");
        let scope = header.function_scope().expect("expected function scope");

        assert!(
            !facts
                .function_cli_dispatch_facts(scope)
                .exported_from_case_cli()
        );
    });
}

#[test]
fn function_cli_dispatch_facts_ignore_static_case_calls() {
    let source = "\
#!/bin/sh
start() { echo hi; }
case \"$1\" in
  start) start ;;
esac
exit $?
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(candidate, _)| candidate == "start")
            })
            .expect("expected start header");
        let scope = header.function_scope().expect("expected function scope");

        assert!(
            !facts
                .function_cli_dispatch_facts(scope)
                .exported_from_case_cli()
        );
    });
}

#[test]
fn function_cli_dispatch_facts_ignore_later_defined_functions() {
    let source = "\
#!/bin/sh
case \"$1\" in
  start) $1 ;;
esac
exit $?

start() { echo hi; }
";

    with_facts(source, None, |_, facts| {
        let header = facts
            .function_headers()
            .iter()
            .find(|header| {
                header
                    .static_name_entry()
                    .is_some_and(|(candidate, _)| candidate == "start")
            })
            .expect("expected start header");
        let scope = header.function_scope().expect("expected function scope");

        assert!(
            !facts
                .function_cli_dispatch_facts(scope)
                .exported_from_case_cli()
        );
    });
}

#[test]
fn builds_function_style_spans() {
    let source = "\
#!/bin/bash
f() [[ -n \"$x\" ]]
p() if true; then :; fi
q() case x in x) :;; esac
r() for x in y; do :; done
s() while true; do :; done
t() until false; do :; done
u() ( echo hi )
v() (( x++ ))
g() {
  if cond; then
    false
    return $?
  fi
}
h() {
  if cond; then
    false
    return $?
  fi
  echo done
}
i() {
  false
  return $? 5
}
j() {
  false
  x=1 return $?
}
k() {
  false
  return $? >out
}
l() {
  ! {
    false
    return $?
  }
}
m() {
  {
    false
    return $?
  } &
}
n() {
  if cond; then
    false
  fi
  return $?
}
o() {
  : | false
  return $?
}
";

    with_facts(source, None, |_, facts| {
        assert_eq!(
            facts
                .function_body_without_braces_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "[[ -n \"$x\" ]]",
                "if true; then :; fi\n",
                "case x in x) :;; esac\n",
                "for x in y; do :; done",
                "while true; do :; done\n",
                "until false; do :; done\n",
            ]
        );
        assert_eq!(
            facts
                .redundant_return_status_spans()
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    });
}

#[test]
fn marks_commands_inside_completion_registration_chain_functions() {
    let source = "\
#!/bin/bash
_comp_cmd_hostname() {
  [[ $cur == -* ]] && _comp_compgen_help || _comp_compgen_usage
} &&
  complete -F _comp_cmd_hostname hostname
_comp_cmd_later() {
  [[ $cur == -* ]] && _comp_compgen_help || _comp_compgen_usage
}
complete -F _comp_cmd_later later
";

    with_facts(source, None, |_, facts| {
        let flagged_lines = facts
            .commands()
            .iter()
            .filter(|command| facts.command_is_in_completion_registered_function(command.id()))
            .map(|command| command.span().start.line)
            .collect::<Vec<_>>();

        assert!(!flagged_lines.is_empty());
        assert!(flagged_lines.iter().all(|line| *line == 3));
    });
}

#[test]
fn marks_commands_inside_zsh_compdef_registered_functions() {
    let source = "\
#!/bin/zsh
__grunt() {
  print -r -- $verbose
}
compdef __grunt grunt
";

    with_facts(source, None, |_, facts| {
        let flagged_lines = facts
            .commands()
            .iter()
            .filter(|command| facts.command_is_in_completion_registered_function(command.id()))
            .map(|command| command.span().start.line)
            .collect::<Vec<_>>();

        assert!(!flagged_lines.is_empty());
        assert!(flagged_lines.iter().all(|line| *line == 3));
    });
}

#[test]
fn ignores_zsh_compdef_registrations_inside_function_bodies() {
    let source = "\
#!/bin/zsh
__grunt() {
  print -r -- $verbose
}
setup_completion() {
  compdef __grunt grunt
}
";

    with_facts(source, None, |_, facts| {
        let flagged_lines = facts
            .commands()
            .iter()
            .filter(|command| facts.command_is_in_completion_registered_function(command.id()))
            .map(|command| command.span().start.line)
            .collect::<Vec<_>>();

        assert_eq!(flagged_lines, Vec::<usize>::new());
    });
}

#[test]
fn marks_zsh_widget_and_hook_functions_as_external_entrypoints() {
    let source = "\
#!/bin/zsh
single_operand_widget() { print -r -- \"$1\"; }
widget_impl() { print -r -- \"$1\"; }
precmd_refresh() { (( $# )) && print -r -- \"$1\"; }
precmd() { print -r -- \"$1\"; }
zsh_directory_name() { print -r -- \"$1\"; }
not_external() { print -r -- \"$1\"; }
dynamic_widget() { print -r -- \"$1\"; }
removed_precmd() { print -r -- \"$1\"; }
removed_chpwd() { print -r -- \"$1\"; }
deleted_widget_impl() { print -r -- \"$1\"; }
shared_widget_impl() { print -r -- \"$1\"; }
latent_widget_impl() { print -r -- \"$1\"; }
latent_hook_impl() { print -r -- \"$1\"; }
pattern_removed_hook() { print -r -- \"$1\"; }
pattern_kept_hook() { print -r -- \"$1\"; }
zle_hook_widget_impl() { print -r -- \"$1\"; }
zle_hook_backing_impl() { print -r -- \"$1\"; }
bracket_removed_hook() { print -r -- \"$1\"; }
number_removed_hook_12() { print -r -- \"$1\"; }
negated_class_removed_hook() { print -r -- \"$1\"; }
negated_class_kept_hook() { print -r -- \"$1\"; }
zle -N single_operand_widget
zle -N widget-name widget_impl
zle -N deleted-widget deleted_widget_impl
zle -D deleted-widget
zle -N first-widget shared_widget_impl
zle -N second-widget shared_widget_impl
zle -D first-widget
add-zsh-hook -Uz precmd precmd_refresh
add-zsh-hook chpwd removed_chpwd
add-zsh-hook -d precmd removed_precmd
add-zsh-hook -UD chpwd removed_chpwd
add-zsh-hook precmd pattern_removed_hook
add-zsh-hook precmd pattern_kept_hook
add-zsh-hook -D precmd 'pattern_removed_*'
add-zle-hook-widget line-init zle_hook_widget_impl
zle -N zle-hook-widget-alias zle_hook_backing_impl
add-zle-hook-widget line-init zle-hook-widget-alias
add-zsh-hook precmd bracket_removed_hook
add-zsh-hook precmd number_removed_hook_12
add-zsh-hook precmd negated_class_removed_hook
add-zsh-hook precmd negated_class_kept_hook
add-zsh-hook -D precmd 'bracket_removed_[hr]ook'
add-zsh-hook -D precmd 'number_removed_hook_<->'
add-zsh-hook -D precmd 'negated_class_[^k]*'
zle -N \"$widget_name\" dynamic_widget
setup_widget() { zle -N latent-widget latent_widget_impl; }
setup_hook() { add-zsh-hook precmd latent_hook_impl; }
";

    with_facts_dialect(
        source,
        None,
        ParseShellDialect::Zsh,
        ShellDialect::Zsh,
        |_, facts| {
            let external_names = facts
                .function_headers()
                .iter()
                .filter_map(|header| {
                    let scope = header.function_scope()?;
                    facts
                        .function_is_external_entrypoint(scope)
                        .then(|| header.static_name_entry().map(|(name, _)| name.as_str()))
                        .flatten()
                })
                .collect::<Vec<_>>();

            assert_eq!(
                external_names,
                vec![
                    "single_operand_widget",
                    "widget_impl",
                    "precmd_refresh",
                    "precmd",
                    "zsh_directory_name",
                    "shared_widget_impl",
                    "pattern_kept_hook",
                    "zle_hook_widget_impl",
                    "zle_hook_backing_impl",
                    "negated_class_kept_hook"
                ]
            );
        },
    );
}
