use super::*;

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
fn function_cli_dispatch_facts_track_case_positional_entrypoints_with_top_level_exit_status() {
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
fn function_cli_dispatch_facts_ignore_case_positional_entrypoints_without_top_level_exit_status() {
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
            vec!["[[ -n \"$x\" ]]"]
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
