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
            vec!["$?", "$?", "$?"]
        );
    });
}
