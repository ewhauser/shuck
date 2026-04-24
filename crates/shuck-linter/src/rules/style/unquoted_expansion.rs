use rustc_hash::FxHashSet;
use shuck_ast::Span;

use crate::{
    Checker, ExpansionContext, Rule, SafeValueIndex, SafeValueQuery, ShellDialect, Violation,
    WordOccurrenceRef,
};

pub struct UnquotedExpansion;

impl Violation for UnquotedExpansion {
    fn rule() -> Rule {
        Rule::UnquotedExpansion
    }

    fn message(&self) -> String {
        "quote parameter expansions to avoid word splitting and globbing".to_owned()
    }
}

pub fn unquoted_expansion(checker: &mut Checker) {
    let source = checker.source();
    let colon_command_ids = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is(":"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();
    let mut safe_values = SafeValueIndex::build(
        checker.semantic(),
        checker.semantic_analysis(),
        checker.facts(),
        source,
    );
    let array_assignment_split_spans = collect_array_assignment_split_candidate_spans(checker);

    let mut spans = Vec::new();
    for fact in checker.facts().word_facts() {
        collect_word_fact_reports(
            checker,
            &colon_command_ids,
            &mut safe_values,
            &mut spans,
            source,
            fact,
        );
        collect_array_assignment_split_reports(
            &mut safe_values,
            &mut spans,
            source,
            fact,
            &array_assignment_split_spans,
        );
    }
    for escaped in checker.facts().backtick_escaped_parameters() {
        if escaped.standalone_command_name {
            continue;
        }
        if !escaped.name.as_ref().is_some_and(|name| {
            safe_values.name_reference_is_safe(name, escaped.reference_span, SafeValueQuery::Argv)
        }) {
            spans.push(escaped.diagnostic_span);
        }
    }
    for span in spans {
        checker.report_dedup(UnquotedExpansion, span);
    }
}

fn collect_word_fact_reports(
    checker: &Checker,
    colon_command_ids: &FxHashSet<crate::facts::core::CommandId>,
    safe_values: &mut SafeValueIndex<'_>,
    spans: &mut Vec<shuck_ast::Span>,
    source: &str,
    fact: WordOccurrenceRef<'_, '_>,
) {
    let Some(context) = fact.host_expansion_context() else {
        return;
    };
    if !should_check_context(context, checker.shell()) {
        return;
    }
    report_word_expansions(
        spans,
        safe_values,
        source,
        fact,
        context,
        colon_command_ids.contains(&fact.command_id()),
    );
}

fn collect_array_assignment_split_candidate_spans(checker: &Checker) -> Vec<Span> {
    let mut spans = checker
        .facts()
        .array_assignment_split_word_facts()
        .flat_map(|fact| {
            let command_substitution_spans = fact.command_substitution_spans();
            fact.array_assignment_split_scalar_expansion_spans()
                .iter()
                .copied()
                .filter(move |span| {
                    command_substitution_spans
                        .iter()
                        .any(|outer| span_contains(*outer, *span))
                })
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn collect_array_assignment_split_reports(
    safe_values: &mut SafeValueIndex<'_>,
    spans: &mut Vec<Span>,
    source: &str,
    fact: WordOccurrenceRef<'_, '_>,
    candidate_spans: &[Span],
) {
    if candidate_spans.is_empty() {
        return;
    }
    let Some(context) = fact.host_expansion_context() else {
        return;
    };
    if !array_assignment_split_context_is_checked(context) {
        return;
    }
    if context == ExpansionContext::CommandName
        && !fact.has_literal_affixes()
        && fact.parts_len() == 1
    {
        return;
    }

    for (part, part_span) in fact.parts_with_spans() {
        if !candidate_spans.contains(&part_span) {
            continue;
        }
        let affixed_command_name =
            context == ExpansionContext::CommandName && fact.has_literal_affixes();
        if safe_values.part_is_safe(part, part_span, SafeValueQuery::Argv) && !affixed_command_name
        {
            continue;
        }

        spans.push(fact.diagnostic_part_span(part, part_span, source));
    }
}

fn array_assignment_split_context_is_checked(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::HereString
            | ExpansionContext::RedirectTarget(_)
    )
}

fn should_check_context(context: ExpansionContext, shell: ShellDialect) -> bool {
    match context {
        ExpansionContext::CommandName
        | ExpansionContext::CommandArgument
        | ExpansionContext::HereString
        | ExpansionContext::RedirectTarget(_) => true,
        ExpansionContext::DeclarationAssignmentValue => shell != ShellDialect::Bash,
        _ => false,
    }
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn report_word_expansions(
    spans: &mut Vec<Span>,
    safe_values: &mut SafeValueIndex<'_>,
    source: &str,
    fact: WordOccurrenceRef<'_, '_>,
    context: ExpansionContext,
    in_colon_command: bool,
) {
    if !fact.analysis().hazards.field_splitting && !fact.analysis().hazards.pathname_matching {
        return;
    }

    let scalar_spans = fact.scalar_expansion_spans();
    let assign_default_spans = if in_colon_command && context == ExpansionContext::CommandArgument {
        fact.unquoted_assign_default_spans()
    } else {
        Default::default()
    };
    let use_replacement_spans = fact.use_replacement_spans();
    let star_spans = fact.unquoted_star_parameter_spans();
    if scalar_spans.is_empty() && star_spans.is_empty() {
        return;
    }
    if context == ExpansionContext::CommandName
        && !fact.has_literal_affixes()
        && fact.parts_len() == 1
    {
        return;
    }
    let Some(query) = SafeValueQuery::from_context(context) else {
        return;
    };

    for (part, part_span) in fact.parts_with_spans() {
        let report_unquoted_star = star_spans.contains(&part_span);
        if !scalar_spans.contains(&part_span) && !report_unquoted_star {
            continue;
        }
        if assign_default_spans.contains(&part_span) {
            continue;
        }
        if use_replacement_spans.contains(&part_span) {
            continue;
        }
        if safe_values.part_is_safe(part, part_span, query) {
            continue;
        }

        spans.push(fact.diagnostic_part_span(part, part_span, source));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_scalar_expansion_parts_instead_of_whole_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${name}suffix ${arr[0]} ${arr[@]}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}", "${arr[0]}"]
        );
    }

    #[test]
    fn descends_into_nested_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$(echo $name)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_early_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; :; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_assigned_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { FOO=1 exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_negated_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { ! exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn reports_literal_bindings_after_extra_arg_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0 1; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_pre_definition_exit_like_calls_before_function_definitions() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit
Exit() { exit 0; }
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_returning_helpers_with_unreachable_trailing_exit() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { return 0; exit 1; }
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_conditionally_returning_exit_like_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { if [ \"$SKIP\" ]; then return 0; fi; exit 1; }
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_all_branch_returning_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() {
  if [ \"$SKIP\" ]; then
    return 0
  else
    return 1
  fi
  exit 0
}
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_literal_bindings_after_conditionally_exiting_exit_like_helpers() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { if [ \"$SKIP\" ]; then exit 1; fi; exit 0; }
Exit
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_safe_bindings_after_conditional_exit_like_helper_calls() {
        let source = "\
#!/bin/sh
LIBDIRSUFFIX=64
warn_accounts() { exit 1; }
if false; then
  warn_accounts
fi
echo /usr/lib${LIBDIRSUFFIX}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_literal_bindings_after_redirected_exit_like_function_calls() {
        let source = "\
#!/bin/sh
OPTION_BINARY_FILE=\"../lynis\"
Exit() { exit 0; }
Exit >/dev/null
OPENBSD_CONTENTS=\"openbsd/+CONTENTS\"
FIND=$(sh -n ${OPTION_BINARY_FILE} ; echo $?)
echo x >> ${OPENBSD_CONTENTS}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${OPTION_BINARY_FILE}", "${OPENBSD_CONTENTS}"]
        );
    }

    #[test]
    fn ignores_safe_bindings_after_background_exit_like_helper_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
Exit &
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_backgrounded_brace_group_exit_like_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
{ Exit; } &
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_shadowed_exit_like_helper_names() {
        let source = "\
#!/bin/sh
Exit() { exit 0; }
Exit() { :; }
SAFE=foo
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_safe_bindings_after_conditionally_defined_exit_like_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
if false; then
  Exit() { exit 0; }
fi
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_safe_bindings_after_backgrounded_exit_like_helper_definitions() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; } &
Exit
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn collapses_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "\
#!/bin/sh
mkdir_umask=`expr $umask + 22 \\
  - $umask % 100 % 40 + $umask % 20 \\
  - $umask % 10 % 4 + $umask % 2
`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 19, 2, 25),
                (2, 35, 2, 41),
                (2, 55, 2, 61),
                (2, 71, 2, 77),
                (2, 89, 2, 95),
            ]
        );
    }

    #[test]
    fn collapses_tab_indented_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "\
#!/bin/sh
\t    mkdir_umask=`expr $umask + 22 \\
\t      - $umask % 100 % 40 + $umask % 20 \\
\t      - $umask % 10 % 4 + $umask % 2
\t    `
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 24, 2, 30),
                (2, 50, 2, 56),
                (2, 70, 2, 76),
                (2, 98, 2, 104),
                (2, 116, 2, 122),
            ]
        );
    }

    #[test]
    fn collapses_crlf_multiline_backtick_spans_to_shellcheck_columns() {
        let source = "#!/bin/sh\r\nmkdir_umask=`expr $umask + 22 \\\r\n  - $umask % 100 % 40 + $umask % 20 \\\r\n  - $umask % 10 % 4 + $umask % 2\r\n`\r\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (2, 19, 2, 25),
                (2, 35, 2, 41),
                (2, 55, 2, 61),
                (2, 71, 2, 77),
                (2, 89, 2, 95),
            ]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_brace_group_wrapped_exit_like_calls() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
{ Exit; }
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn ignores_exit_like_helper_calls_in_uncalled_function_bodies() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
wrapper() {
  Exit
}
echo /tmp/$SAFE
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unquoted_expansions_after_exit_like_calls_inside_same_function_body() {
        let source = "\
#!/bin/sh
SAFE=foo
Exit() { exit 0; }
wrapper() {
  Exit
  echo /tmp/$SAFE
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_inline_returns_in_same_function_body() {
        let source = "\
#!/bin/sh
wrapper() {
  SAFE=foo
  return 0
  echo /tmp/$SAFE
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_after_deferred_exit_like_calls_resolved_by_later_helpers() {
        let source = "\
#!/bin/sh
SAFE=foo
wrapper() {
  Exit
  echo /tmp/$SAFE
}
Exit() { exit 0; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SAFE"]
        );
    }

    #[test]
    fn ignores_expansions_inside_quoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
exec dbus-send --bus=\"unix:path=$XDG_RUNTIME_DIR/bus\" / org.freedesktop.DBus.Peer.Ping
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_unquoted_fragments_of_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix\"$HOME\"/$suffix
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix"]
        );
    }

    #[test]
    fn reports_unquoted_expansions_in_case_cli_dispatch_entry_functions_with_top_level_exit_status()
    {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid
configfile=/etc/collectd.conf

start() {
  [ -x $exec ] || exit 5
  [ -f $pidfile ] && rm $pidfile
  $exec -P $pidfile -C $configfile
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$exec", "$pidfile", "$pidfile", "$pidfile", "$configfile"]
        );
    }

    #[test]
    fn reports_collectd_style_case_cli_dispatch_entry_functions() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
prog=$(basename $exec)
configfile=/etc/collectd.conf
pidfile=/var/run/collectd.pid

start() {
  [ -x $exec ] || exit 5
  if [ -f $pidfile ]; then
    echo \"Seems that an active process is up and running with pid $(cat $pidfile)\"
    echo \"If this is not true try first to remove pidfile $pidfile\"
    exit 5
  fi
  echo $\"Starting $prog\"
  $exec -P $pidfile -C $configfile
}

stop() {
  if [ -e $pidfile ]; then
    echo \"Stopping $prog\"
    kill -QUIT $(cat $pidfile) 2>/dev/null
    rm $pidfile
  fi
}

status() {
  echo -n \"$prog is \"
  CHECK=$(ps aux | grep $exec | grep -v grep)
  STATUS=$?
  if [ \"$STATUS\" == \"1\" ]; then
    echo \"not running\"
  else
    echo \"running\"
  fi
}

restart() {
  stop
  start
}

case \"$1\" in
  start)
    $1
    ;;
  stop)
    $1
    ;;
  restart)
    $1
    ;;
  status)
    $1
    ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$exec",
                "$pidfile",
                "$pidfile",
                "$pidfile",
                "$configfile",
                "$pidfile",
                "$pidfile",
                "$pidfile",
                "$exec",
            ]
        );
    }

    #[test]
    fn reports_spice_vdagent_style_case_cli_dispatch_entry_functions() {
        let source = "\
#!/bin/sh
exec=\"/usr/sbin/spice-vdagentd\"
prog=\"spice-vdagentd\"
port=\"/dev/virtio-ports/com.redhat.spice.0\"
pid=\"/var/run/spice-vdagentd/spice-vdagentd.pid\"
lockfile=/var/lock/subsys/$prog

start() {
  /sbin/modprobe uinput > /dev/null 2>&1
  /usr/bin/rm -f /var/run/spice-vdagentd/spice-vdagent-sock
  /usr/bin/mkdir -p /var/run/spice-vdagentd
  /usr/bin/echo \"Starting $prog: \"
  $exec -s $port
  retval=$?
  /usr/bin/echo
  [ $retval -eq 0 ] && echo \"$(pidof $prog)\" > $pid && /usr/bin/touch $lockfile
  return $retval
}

stop() {
  if [ \"$(pidof $prog)\" ]; then
    /usr/bin/echo \"Stopping $prog: \"
    /bin/kill $(cat $pid)
  else
    /usr/bin/echo \"$prog not running\"
    return 1
  fi
  retval=$?
  /usr/bin/echo
  [ $retval -eq 0 ] && rm -f $lockfile $pid
  return $retval
}

restart() {
  stop
  start
}

case \"$1\" in
  start)
    $1
    ;;
  stop)
    $1
    ;;
  restart)
    $1
    ;;
  *)
    /usr/bin/echo $\"Usage: $0 {start|stop|restart}\"
    exit 2
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));
        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans.iter().filter(|span| **span == "$port").count(), 1);
        assert_eq!(spans.iter().filter(|span| **span == "$retval").count(), 4);
        assert_eq!(spans.iter().filter(|span| **span == "$prog").count(), 2);
        assert_eq!(spans.iter().filter(|span| **span == "$pid").count(), 3);
        assert_eq!(spans.iter().filter(|span| **span == "$lockfile").count(), 2);
    }

    #[test]
    fn reports_case_cli_dispatch_entry_function_local_arguments() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_dispatch_entry_function_local_arguments_with_literal_exit() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}

case \"$1\" in
  start) $1 ;;
esac
exit 0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_reachable_helper_local_arguments() {
        let source = "\
#!/bin/sh
foo() {
  local n=0
  echo $n
}

bound() { foo; }
renew() { foo; }
deconfig() { :; }

case \"$1\" in
  deconfig|renew|bound) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_case_cli_pre_dispatch_function_local_arguments() {
        let source = "\
#!/bin/sh
foo() {
  local n=0
  echo $n
}

bar() { :; }

case \"$1\" in
  bar) $1 ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn does_not_broaden_dynamic_case_dispatch_without_top_level_exit_status() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd
pidfile=/var/run/collectd.pid

start() {
  [ -x $exec ] || exit 5
  $exec -P $pidfile
}

case \"$1\" in
  start) $1 ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_function_local_arguments_before_plain_top_level_exit() {
        let source = "\
#!/bin/sh
start() {
  local n=0
  echo $n
}
exit 0
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$n"]
        );
    }

    #[test]
    fn reports_nested_function_return_arguments_without_top_level_exit() {
        let source = "\
#!/bin/bash
outer() {
  inner() {
    local good=0
    return $good
  }
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$good"]
        );
    }

    #[test]
    fn keeps_unknown_option_bundle_warnings_without_broad_command_name_reports() {
        let source = "\
#!/bin/sh
BLUEALSA_BIN=/usr/bin/bluealsa

start() {
  $BLUEALSA_BIN $BLUEALSA_OPTS
}

case \"$1\" in
  start) $1 ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BLUEALSA_OPTS"]
        );
    }

    #[test]
    fn ignores_static_case_dispatch_calls() {
        let source = "\
#!/bin/sh
exec=/usr/sbin/collectd

start() {
  $exec
}

case \"$1\" in
  start) start ;;
esac
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_for_lists_but_reports_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
for item in $first \"$second\"; do :; done
cat <<< $here >$out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$here", "$out"]
        );
    }

    #[test]
    fn skips_assignment_values_and_descriptor_dup_targets() {
        let source = "\
#!/bin/bash
value=$name
printf '%s\\n' ok >&$fd
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_zsh_parameter_modifiers() {
        let source = "\
#!/usr/bin/env zsh
print ${~foo}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${~foo}"]
        );
    }

    #[test]
    fn reports_dynamic_command_names() {
        let source = "\
#!/bin/bash
$HOME/bin/tool $arg
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOME", "$arg"]
        );
    }

    #[test]
    fn reports_unquoted_star_selector_expansions() {
        let source = "\
#!/bin/bash
RSYNC_OPTIONS=(-a -v)
rsync ${RSYNC_OPTIONS[*]} src dst
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${RSYNC_OPTIONS[*]}"]
        );
    }

    #[test]
    fn reports_bourne_transformations_in_command_arguments() {
        let source = "\
#!/bin/bash
printf '%s\\n' ${name@U}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name@U}"]
        );
    }

    #[test]
    fn reports_bindings_derived_from_parameter_operations() {
        let source = "\
#!/bin/bash
PRGNAM=Fennel
SRCNAM=${PRGNAM,}
release=1.0.0
VERSION=${release:-fallback}
rm -rf $SRCNAM-$VERSION
printf '%s\\n' ${PRGNAM,} ${release:-fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SRCNAM", "$VERSION"]
        );
    }

    #[test]
    fn reports_bindings_from_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
check() { return 0; }
check && w='-w' || w=''
if check; then
  flag='-w'
else
  flag=''
fi
iptables $w -t nat -N chain
iptables $flag -t nat -N chain
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn reports_nested_guarded_short_circuit_assignment_ternaries() {
        let source = "\
#!/bin/bash
f() {
  [ \"$1\" = iptables ] && {
    true && w='-w' || w=''
  }
  [ \"$1\" = ip6tables ] && {
    true && w='-w' || w=''
  }
  iptables $w -t nat -N chain
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$w"]
        );
    }

    #[test]
    fn skips_colon_assign_default_expansions_but_keeps_regular_argument_cases() {
        let source = "\
#!/bin/bash
: ${x:=fallback} $other
printf '%s\\n' ${z:=fallback}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![(2, "$other"), (3, "${z:=fallback}")]
        );
    }

    #[test]
    fn keeps_colon_assign_default_reports_for_here_strings_and_redirect_targets() {
        let source = "\
#!/bin/bash
: <<< ${x:=fallback} >${y:=out}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${x:=fallback}", "${y:=out}"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_nested_command_substitution_arguments() {
        let source = "\
#!/bin/sh
PRGNAM=cproc
GIT_SHA=$( git rev-parse --short HEAD )
DATE=$( git log --date=format:%Y%m%d --format=%cd | head -1 )
VERSION=${DATE}_${GIT_SHA}
echo \"MD5SUM=\\\"$( md5sum $PRGNAM-$VERSION.tar.xz | cut -d' ' -f1 )\\\"\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$VERSION"]
        );
    }

    #[test]
    fn reports_dynamic_values_inside_parameter_replacement_command_substitutions() {
        let source = "\
#!/bin/bash
image_file='foo bar'
template='IMG_EXTRACT_SIZE IMG_SHA256 IMG_DOWNLOAD_SIZE'
template=\"${template/IMG_EXTRACT_SIZE/$(stat -c %s $image_file)}\"
template=\"${template/IMG_SHA256/$(sha256sum $image_file | cut -d' ' -f1)}\"
template=\"${template/IMG_DOWNLOAD_SIZE/$(stat -c %s ${image_file}.xz)}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$image_file", "$image_file", "${image_file}"]
        );
    }

    #[test]
    fn skips_dynamic_values_inside_parameter_replacement_arithmetic_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${template/IMG_OFFSET/$(( $(cat file) $1 step ))}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_dynamic_values_inside_parameter_default_arithmetic_expansions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${value:-$(( $(cat file) $1 step ))}\" \"${value:=$(( $2 + 1 ))}\" \"${value:+$(( $3 + 1 ))}\" \"${value:?$(( $4 + 1 ))}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_dynamic_values_inside_arithmetic_shell_words() {
        let source = "\
#!/bin/sh
printf '%s' \"$(( $(cat file) $1 step ))\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_scalar_expansions_that_split_through_array_assignments() {
        let source = "\
#!/bin/bash
MODE_ID+=($group-$id)
MODE_CUR=($(get_${HAS_MODESET}_mode_info \"${mode_id[*]}\"))
arr=(\"$(printf '%s\\n' $quoted_outer)\")
arr=($PPID $HOME)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${HAS_MODESET}", "$quoted_outer"]
        );
    }

    #[test]
    fn reports_possibly_uninitialized_static_branch_bindings_in_array_assignments() {
        let source = "\
#!/bin/bash
if command -v kms >/dev/null; then
  HAS_MODESET=kms
elif command -v xrandr >/dev/null; then
  HAS_MODESET=x11
fi

mode_switch() {
  if [ \"$HAS_MODESET\" = kms ]; then
    MODE_CUR=($(get_${HAS_MODESET}_mode_info))
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${HAS_MODESET}"]
        );
    }

    #[test]
    fn skips_standalone_safe_command_names_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
tool=/usr/bin/helper
items=($($tool list))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_standalone_dynamic_command_names_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
items=($($tool list))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_for_lists_in_array_assignment_substitutions() {
        let source = "\
#!/bin/bash
items=($(
  for keyword in $keywords; do
    printf '%s\\n' \"$keyword\"
  done
))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn treats_wrapper_targets_as_command_names() {
        let source = "\
#!/bin/sh
exec $SHELL $0 \"$@\"
exec pre$x arg
command $tool arg
builtin $name arg
case \"$1\" in
  restart) $0 stop; exec $0 start ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$x"]
        );
    }

    #[test]
    fn skips_safe_variables_that_share_names_with_functions() {
        let source = "\
#!/usr/bin/env bash
check_gitlab=0
check_gitlab() {
  [ $check_gitlab = 1 ] && return 0
}
if check_gitlab; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
SAFE=foo
printf '%s\\n' `echo \\$1 \\$HOME \\$SAFE \\$PPID`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(3, 21, 3, 23), (3, 24, 3, 29)]
        );
    }

    #[test]
    fn reports_unsafe_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
if cond; then
  value=ok
fi
printf '%s\\n' `echo \\$value`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(5, 21, 5, 27)]
        );
    }

    #[test]
    fn reports_complex_escaped_parameters_in_legacy_backticks() {
        let source = "\
#!/bin/sh
printf '%s\\n' `echo \\${SAFE:-$fallback} \\${SAFE:+$fallback}`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.span.start.line,
                        diagnostic.span.start.column,
                        diagnostic.span.end.line,
                        diagnostic.span.end.column,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(2, 21, 2, 39)]
        );
    }

    #[test]
    fn skips_escaped_backtick_standalone_command_names() {
        let source = "\
#!/bin/sh
`\\$cmd`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_escaped_backtick_command_names_after_quoted_assignment_prefixes() {
        let source = "\
#!/bin/sh
`VAR=\"a b\" OTHER=$(printf '%s\\n' value) \\$cmd arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_backtick_command_arguments() {
        let source = "\
#!/bin/sh
`echo \\$arg`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn reports_affixed_escaped_backtick_command_names() {
        let source = "\
#!/bin/sh
`pre\\$cmd`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn skips_use_replacement_expansions() {
        let source = "\
#!/bin/bash
foo='a b'
arr=('left side' right)
printf '%s\\n' ${foo:+$foo} ${foo:+\"$foo\"} ${arr:+\"${arr[@]}\"}
tar ${foo:+-C \"$foo\"} -f archive.tar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn keeps_default_expansions_with_quoted_operands() {
        let source = "\
#!/bin/bash
foo='a b'
printf '%s\\n' ${foo:-\"$foo\"}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${foo:-\"$foo\"}"]
        );
    }

    #[test]
    fn skips_plain_expansion_command_names_but_reports_composite_command_words() {
        let source = "\
#!/bin/bash
$CC -c file.c
if $TERMUX_ON_DEVICE_BUILD; then
  :
fi
${CC}${FLAGS} file.c
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CC}", "${FLAGS}"]
        );
    }

    #[test]
    fn ignores_escaped_backticks_inside_double_quoted_assignments() {
        let source = "\
#!/bin/bash
NVM_TEST_VERSION=v0.42
EXPECTED=\"Found '$(pwd)/.nvmrc' with version <${NVM_TEST_VERSION}>
N/A: version \\\"${NVM_TEST_VERSION}\\\" is not yet installed.

You need to run \\`nvm install ${NVM_TEST_VERSION}\\` to install and use it.
No NODE_VERSION provided; no .nvmrc file found\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn reports_expansions_wrapped_in_escaped_literal_quotes() {
        let source = "\
#!/bin/bash
printf '%s\\n' -DPACKAGE_VERSION=\\\"$TERMUX_PKG_VERSION\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_VERSION"]
        );
    }

    #[test]
    fn reports_decl_assignment_values_in_sh_mode() {
        let source = "\
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$TERMUX_PKG_BUILDER_DIR", "$HOME", "$PATH"]
        );
    }

    #[test]
    fn reports_transformed_decl_assignment_values_in_sh_mode() {
        let source = "\
local upper=${TERMUX_ARCH@U}
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${TERMUX_ARCH@U}"]
        );
    }

    #[test]
    fn skips_decl_assignment_values_in_bash_mode() {
        let source = "\
#!/bin/bash
local _patch=$TERMUX_PKG_BUILDER_DIR/file.diff
export PATH=$HOME/bin:$PATH
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.bash"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unquoted_spans_inside_mixed_words() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'prefix:'$name':suffix'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn skips_safe_special_parameters() {
        let source = "\
#!/bin/bash
printf '%s\\n' $? $# $$ $! $- $0 $1 $* $@
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$1", "$*"]
        );
    }

    #[test]
    fn skips_bindings_with_safe_visible_values() {
        let source = "\
#!/bin/bash
n=42
s=abc
glob='*'
split='1 2'
copy=\"$n\"
alias=$s
printf '%s\\n' $n $s $glob $split $copy $alias
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$glob", "$split"]
        );
    }

    #[test]
    fn skips_initialized_declaration_bindings_with_safe_values() {
        let source = "\
#!/bin/bash
f() {
  local name=abc i=0
  printf '%s\\n' $name $i
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_literal_bindings_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
FILE=$(basename $URL)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_numeric_shell_variables() {
        let source = "\
#!/bin/bash
printf '%s\\n' $(ps -o comm= -p $PPID) $UID $EUID $RANDOM $OPTIND $SECONDS $LINENO $BASHPID $COLUMNS
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unknown_values_in_uncalled_function_bodies() {
        let source = "\
#!/bin/sh
unused() {
  echo $1 $value
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1", "$value"]
        );
    }

    #[test]
    fn keeps_called_function_arguments_unsafe() {
        let source = "\
#!/bin/sh
called() {
  echo $1 $value
}
called \"$@\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$1", "$value"]
        );
    }

    #[test]
    fn skips_status_capture_bindings_with_local_declarations() {
        let source = "\
#!/bin/bash
function first() {
  local return_value
  other \"$@\"
  return_value=$?
  if [ $return_value -eq 64 ]; then :; fi
  return $return_value
}
second() {
  local return_value
  for x in \"$@\"; do
    y=$(cat \"$x\")
    return_value=$?
    if [ $return_value -ne 0 ]; then return $return_value; fi
  done
}
first \"$@\"
second \"$@\"
exit $?
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_capture_declarations_with_initializers() {
        let source = "\
#!/bin/bash
cleanup() {
  rm -f -- \"$1\" || {
    \\typeset ret=$?
    return $ret
  }
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_status_capture_declarations_after_unsafe_reassignments() {
        let source = "\
#!/bin/bash
demo() {
  local ret=$?
  ret=$user_input
  echo $ret
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$ret"]
        );
    }

    #[test]
    fn uses_static_loop_values_from_static_function_call_sites() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
for linker in gold bfd lld; do
  run
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn uses_static_loop_values_after_prior_local_declarations() {
        let source = "\
#!/bin/bash
run() {
  LDFLAGS=\"-fuse-ld=${linker}\"
  cc ${CFLAGS} ${LDFLAGS}
}
main() {
  local linker
  for linker in gold bfd lld; do
    run
  done
}
main
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn uses_safe_top_level_bindings_at_static_function_call_sites() {
        let source = "\
#!/bin/sh
RETVAL=0
prog=daemon
start() {
  if [ $RETVAL -eq 0 ]; then
    touch /var/lock/subsys/$prog
  fi
  return $RETVAL
}
case \"$1\" in
  start) start ;;
esac
exit $RETVAL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn uses_safe_top_level_bindings_through_static_dispatch_helpers() {
        let source = "\
#!/bin/sh
RETVAL=0
prog=daemon
start () {
  RETVAL=$?
  if [ $RETVAL -eq 0 ]; then
    touch /var/lock/subsys/$prog
  fi
  return $RETVAL
}
restart () {
  start
}
case \"$1\" in
  start) start ;;
  restart) restart ;;
esac
exit $RETVAL
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_reassigned_ppid_in_sh_mode() {
        let source = "\
#!/bin/sh
PPID='a b'
printf '%s\\n' $PPID
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/pkg.sh"),
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PPID"]
        );
    }

    #[test]
    fn skips_safe_here_string_operands() {
        let source = "\
#!/bin/bash
URL=https://example.com/file.tgz
cat <<< $URL
cat <<< $PPID
v='a b'
cat <<< $v
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_safe_literal_loop_variables() {
        let source = "\
#!/bin/bash
for v in one two; do
  unset $v
done
for i in 16 32 64; do
  cmd ${i}x${i}! \"$i\"
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_static_loop_variables_after_the_loop_body() {
        let source = "\
#!/bin/bash
for i in castool chdman; do
  :
done
install $i /tmp
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i"]
        );
    }

    #[test]
    fn reports_loop_variables_derived_from_expanded_values() {
        let source = "\
#!/bin/bash
PRGNAM=neverball
BONUS=neverputt
for i in $PRGNAM $BONUS; do
  install -D ${i}.desktop /tmp/$i.png
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${i}", "$i"]
        );
    }

    #[test]
    fn reports_loop_variables_derived_from_at_slices() {
        let source = "\
#!/bin/bash
f() {
  for v in ${@:2}; do
    del $v
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$v"]
        );
    }

    #[test]
    fn skips_direct_at_slices_that_belong_to_array_split_handling() {
        let source = "\
#!/bin/bash
f() {
  dns_set ${@:2}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_bindings_derived_from_arithmetic_values() {
        let source = "\
#!/bin/bash
x=$((1 + 2))
y=\"$x\"
z=${x}
printf '%s\\n' $x $y $z
if [ $x -eq 0 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_safe_assign_default_expansions_when_name_is_already_safe() {
        let source = "\
#!/bin/bash
check_count() {
  local i=0
  [ ${i:=0} -eq 0 ]
  test $i -ge 0
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_default_operator_operands_when_existing_value_is_safe() {
        let source = "\
#!/bin/bash
value=abc
printf '%s\\n' ${value:=$1} ${value:-$2}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_assign_default_after_maybe_uninitialized_binding() {
        let source = "\
#!/bin/bash
if cond; then value=abc; fi
printf '%s\\n' ${value:=fallback}
printf '%s\\n' $value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${value:=fallback}", "$value"]
        );
    }

    #[test]
    fn reports_conditionally_sanitized_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
if [ \"$foo\" = \"\" ]; then
  foo=0
fi
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn reports_conditionally_initialized_bindings_with_unknown_fallbacks() {
        let source = "\
#!/bin/bash
if [ \"$1\" = yes ]; then
  foo=0
fi
printf '%s\\n' $foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn skips_straight_line_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
foo=0
if [ $foo -eq 1 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_in_test_operands() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=0
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_case_arm_safe_overwrites_even_with_nested_conditional_updates() {
        let source = "\
#!/bin/bash
foo=$BAR
case $1 in
  settings)
    foo=1
    while [ $# -gt 1 ]; do
      shift
      case $1 in
        --no) foo=0 ;;
      esac
    done
    if [ $foo -eq 1 ]; then :; fi
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
free ${humanreadable}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_if_else_safe_literal_bindings_inside_command_substitutions() {
        let source = "\
#!/bin/bash
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
value=\"$(free ${humanreadable} | awk '{print $2}')\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_helper_initialized_option_flags_after_intermediate_calls() {
        let source = "\
#!/bin/bash
fn_select_compression() {
  if command -v zstd >/dev/null 2>&1; then
    compressflag=--zstd
  elif command -v pigz >/dev/null 2>&1; then
    compressflag=--use-compress-program=pigz
  elif command -v gzip >/dev/null 2>&1; then
    compressflag=--gzip
  else
    compressflag=
  fi
}

fn_backup_check_lockfile() { :; }
fn_backup_create_lockfile() { :; }
fn_backup_init() { :; }
fn_backup_stop_server() { :; }
fn_backup_dir() { :; }

fn_backup_compression() {
  if [ -n \"${compressflag}\" ]; then
    tar ${compressflag} -hcf out.tar ./.
  else
    tar -hcf out.tar ./.
  fi
}

fn_select_compression
fn_backup_check_lockfile
fn_backup_create_lockfile
fn_backup_init
fn_backup_stop_server
fn_backup_dir
fn_backup_compression
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_top_level_arguments_after_transitively_unsafe_helper_calls() {
        let source = "\
#!/bin/bash
DISK_SIZE=\"32G\"

advanced_settings() {
  DISK_SIZE=\"$(get_size)\"
}

start_script() {
  advanced_settings
}

start_script
if [ -n \"$DISK_SIZE\" ]; then
  qm resize 100 scsi0 ${DISK_SIZE} >/dev/null
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${DISK_SIZE}"]
        );
    }

    #[test]
    fn reports_helper_initialized_bindings_when_other_callers_skip_the_helper() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  printf '%s\\n' ${flag}
}

safe_path() {
  init_flag
  render
}

unsafe_path() {
  render
}

safe_path
unsafe_path
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn reports_helper_globals_with_unsafe_caller_visible_bindings() {
        let source = "\
#!/bin/sh
SERVERNUM=99
find_free_servernum() {
  i=$SERVERNUM
  while [ -f /tmp/.X$i-lock ]; do
    i=$(($i + 1))
  done
  echo $i
}
set -- -n '1 2' -a --
while :; do
  case \"$1\" in
    -a|--auto-servernum) SERVERNUM=$(find_free_servernum) ;;
    -n|--server-num) SERVERNUM=\"$2\"; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
  shift
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$i", "$i"]
        );
    }

    #[test]
    fn reports_called_helper_mutations_even_when_callers_have_safe_bindings() {
        let source = "\
#!/bin/bash
flag=-n
set_flag() {
  flag=$1
}
render() {
  set_flag \"$1\"
  printf '%s\\n' $flag
}
render value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$flag"]
        );
    }

    #[test]
    fn reports_helper_globals_with_mixed_safe_and_unsafe_caller_branches() {
        let source = "\
#!/bin/sh
if [ -n \"$2\" ]; then
  UIPORT=\"$2\"
else
  UIPORT=\"8080\"
fi
do_start() {
  grep $UIPORT /dev/null
}
do_start
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$UIPORT"]
        );
    }

    #[test]
    fn reports_helper_globals_with_conditionally_initialized_caller_bindings() {
        let source = "\
#!/bin/bash
[ \"$1\" = 64 ] && extra=ENABLE_LIB64=1
run_make() {
  make $extra
}
run_make
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$extra"]
        );
    }

    #[test]
    fn reports_helper_bindings_when_initializers_are_guarded_by_conditionals() {
        let source = "\
#!/bin/bash
init_flag() {
  flag=-n
}

render() {
  if [ \"$1\" = yes ]; then
    init_flag
  fi
  printf '%s\\n' ${flag}
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${flag}"]
        );
    }

    #[test]
    fn skips_helper_initialized_bindings_when_all_callers_provide_distinct_values() {
        let source = "\
#!/bin/bash
init_flag_a() {
  flag=-a
}

init_flag_b() {
  flag=-b
}

render() {
  printf '%s\\n' ${flag}
}

safe_path_a() {
  init_flag_a
  render
}

safe_path_b() {
  init_flag_b
  render
}

safe_path_a
safe_path_b
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_tilde_path_bindings_shellcheck_treats_as_field_safe() {
        let source = "\
#!/bin/bash
file=~/.launch-bristol
if [ -e $file ]; then
  dflt=\"$(cat $file)\"
fi
> $file.new
mv $file.new $file
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_self_referential_static_suffix_bindings() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
LIBDIR=lib
if [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIR=\"$LIBDIR\"64
fi
mkdir -p /pkg/usr/$LIBDIR/clap
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_status_bindings_when_local_initializer_covers_helper_updates() {
        let source = "\
#!/bin/sh
deploy() {
  err_code=0
  if ! remote; then
    return $err_code
  fi
}

remote() {
  err_code=$?
  return $err_code
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_safe_bindings_after_exit_inside_nested_command_substitution() {
        let source = "\
#!/bin/bash
name=package-name
version=1
list=$(echo file || (echo error; exit 1))
mkdir -p /pkg/usr/doc/$name-$version
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_exhaustive_static_branch_bindings_inside_brace_expansion() {
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i586\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi
mkdir -p /pkg/usr/{bin,lib${LIBDIRSUFFIX}}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_definite_empty_overwrite_inside_brace_expansion() {
        let source = "\
#!/bin/sh
x=64
x=
echo {pre,$x}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x"]
        );
    }

    #[test]
    fn reports_ambient_contract_bindings_without_known_values() {
        let path = Path::new("/tmp/void-packages/common/build-style/example.sh");
        let source = "\
#!/bin/sh
helper() {
  printf '%s\\n' $wrksrc $pkgname
}
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$wrksrc", "$pkgname"]
        );
    }

    #[test]
    fn skips_static_suffix_bindings_in_slackbuild_subshell_paths() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
if [ \"$ARCH\" = \"i386\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i486\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i586\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"i686\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  MULTILIB=\"YES\"
  LIBDIRSUFFIX=\"64\"
elif [ \"$ARCH\" = \"armv7hl\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"s390\" ]; then
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
else
  MULTILIB=\"NO\"
  LIBDIRSUFFIX=\"\"
fi

if [ ${MULTILIB} = \"YES\" ]; then
  printf '%s\\n' multilib
fi

(
  ./configure \
    --libdir=/usr/lib${LIBDIRSUFFIX} \
    --with-python-dir=/lib${LIBDIRSUFFIX}/python2.7/site-packages \
    --with-java-home=/usr/lib${LIBDIRSUFFIX}/jvm/jre
)
";
        let diagnostics = test_snippet_at_path(
            path,
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn skips_static_suffix_bindings_in_realistic_slackbuild_path_words() {
        let path = Path::new("/tmp/example.SlackBuild");
        let source = "\
#!/bin/bash
if [ -z \"$ARCH\" ]; then
  ARCH=$(uname -m)
  export ARCH
fi

if [ \"$ARCH\" = \"x86_64\" ]; then
  MULTILIB=\"YES\"
else
  MULTILIB=\"NO\"
fi

if [ \"$ARCH\" = \"i386\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i486\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i586\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"i686\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=i386
elif [ \"$ARCH\" = \"s390\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=s390
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
  LIB_ARCH=amd64
elif [ \"$ARCH\" = \"armv7hl\" ]; then
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=armv7hl
else
  LIBDIRSUFFIX=\"\"
  LIB_ARCH=$ARCH
fi

(
  ./configure \
    --prefix=/usr \
    --libdir=/usr/lib$LIBDIRSUFFIX \
    --with-python-dir=/lib$LIBDIRSUFFIX/python2.7/site-packages \
    --with-java-home=/usr/lib$LIBDIRSUFFIX/jvm/jre \
    --with-jvm-root-dir=/usr/lib$LIBDIRSUFFIX/jvm \
    --with-jvm-jar-dir=/usr/lib$LIBDIRSUFFIX/jvm/jvm-exports \
    --with-arch-directory=$LIB_ARCH
)

if [ ! -r /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/specs ]; then
  cat stage1-gcc/specs > /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/specs
fi
if [ -d /pkg/usr/lib${LIBDIRSUFFIX} ]; then
  mv /pkg/usr/lib${LIBDIRSUFFIX}/lib* /pkg/usr/lib${LIBDIRSUFFIX}/gcc/x/y/
fi
";
        let settings = LinterSettings::for_rule(Rule::UnquotedExpansion)
            .with_analyzed_paths([path.to_path_buf()]);
        let diagnostics = test_snippet_at_path(path, source, &settings);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$LIB_ARCH"]
        );
    }

    #[test]
    fn keeps_safe_indirect_bindings_but_reports_parameter_operator_results() {
        let source = "\
#!/bin/bash
base=abc
name=base
upper=${base^^}
value='a b*'
quoted=${value@Q}
printf '%s\\n' ${!name} $upper $quoted
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$upper", "$quoted"]
        );
    }

    #[test]
    fn indirect_cycles_and_multi_field_targets_stay_unsafe() {
        let source = "\
#!/bin/bash
split='1 2'
name=split
a=$b
b=$a
printf '%s\\n' ${!name} $a
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!name}", "$a"]
        );
    }

    #[test]
    fn skips_plain_unquoted_scalars_in_native_zsh_mode() {
        let source = "print $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_unquoted_scalars_after_setopt_sh_word_split_in_zsh() {
        let source = "setopt sh_word_split\nprint $name\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$name"]
        );
    }

    #[test]
    fn reports_zsh_force_split_modifier_even_without_sh_word_split() {
        let source = "print ${=name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${=name}"]
        );
    }

    #[test]
    fn skips_zsh_double_tilde_modifier_when_it_forces_globbing_off() {
        let source = "print ${~~name}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedExpansion).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
