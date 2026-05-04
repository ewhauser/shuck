use crate::facts::{ListFact, ListSegmentKind, MixedShortCircuitKind};
use crate::{Checker, Rule, Violation};
use shuck_ast::BinaryOp;
use std::cmp::Reverse;

pub struct ChainedTestBranches;

impl Violation for ChainedTestBranches {
    fn rule() -> Rule {
        Rule::ChainedTestBranches
    }

    fn message(&self) -> String {
        "chaining `&&` and `||` makes the fallback depend on the middle command status".to_owned()
    }
}

pub fn chained_test_branches(checker: &mut Checker) {
    let mut lists = checker
        .facts()
        .lists()
        .iter()
        .filter(|list| matches_mixed_short_circuit(checker, list))
        .collect::<Vec<_>>();

    lists.sort_by_key(|list| {
        (
            list.span().start.offset,
            Reverse(list.span().end.offset - list.span().start.offset),
        )
    });

    let mut reported_lists = Vec::new();
    let mut spans = Vec::new();

    for list in lists {
        if reported_lists
            .iter()
            .any(|reported| span_strictly_contains(*reported, list.span()))
        {
            continue;
        }

        reported_lists.push(list.span());
        if let Some(span) = list.mixed_short_circuit_span() {
            spans.push(span);
        }
    }

    checker.report_all_dedup(spans, || ChainedTestBranches);
}

fn span_strictly_contains(outer: shuck_ast::Span, inner: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset
        && inner.end.offset <= outer.end.offset
        && (outer.start.offset < inner.start.offset || inner.end.offset < outer.end.offset)
}

fn matches_mixed_short_circuit(checker: &Checker<'_>, list: &ListFact<'_>) -> bool {
    if !matches_and_then_or_chain(list) || list_runs_as_if_or_elif_condition(checker, list) {
        return false;
    }

    match list.mixed_short_circuit_kind() {
        Some(MixedShortCircuitKind::TestChain) => false,
        Some(MixedShortCircuitKind::Fallthrough) => !list_exempts_warning(checker, list),
        _ => false,
    }
}

fn matches_and_then_or_chain(list: &ListFact<'_>) -> bool {
    let mut saw_and = false;
    let mut saw_or = false;

    for operator in list.operators() {
        match operator.op() {
            BinaryOp::And if !saw_or => saw_and = true,
            BinaryOp::Or if saw_and => saw_or = true,
            BinaryOp::And | BinaryOp::Or | BinaryOp::Pipe | BinaryOp::PipeAll => return false,
        }
    }

    saw_and && saw_or
}

fn list_runs_as_if_or_elif_condition(checker: &Checker<'_>, list: &ListFact<'_>) -> bool {
    list.segments().iter().all(|segment| {
        checker
            .facts()
            .is_if_condition_command(segment.command_id())
            || checker
                .facts()
                .is_elif_condition_command(segment.command_id())
    })
}

fn list_exempts_warning(checker: &Checker<'_>, list: &ListFact<'_>) -> bool {
    if list.segments().iter().all(|segment| {
        checker
            .facts()
            .command_is_in_completion_registered_function(segment.command_id())
    }) {
        return true;
    }

    if matches_status_propagation_assignment(list) {
        return true;
    }

    if matches_condition_guard_fallback(list) {
        return true;
    }

    if matches_exempt_fallback_branch(checker, list) {
        return true;
    }

    false
}

fn matches_status_propagation_assignment(list: &ListFact<'_>) -> bool {
    if !matches_and_or_ternary(list) {
        return false;
    }

    let segments = list.segments();
    let [first, _, last] = segments else {
        return false;
    };

    first.kind() == ListSegmentKind::AssignmentOnly
        && last.kind() == ListSegmentKind::AssignmentOnly
        && first.assignment_target().is_some()
        && first.assignment_target() == last.assignment_target()
}

fn matches_exempt_fallback_branch(checker: &Checker<'_>, list: &ListFact<'_>) -> bool {
    let Some(last_operator) = list.operators().last() else {
        return false;
    };
    if last_operator.op() != BinaryOp::Or {
        return false;
    }

    let Some(last_segment) = list.segments().last() else {
        return false;
    };

    if last_segment.kind() == ListSegmentKind::AssignmentOnly {
        return !last_segment.assignment_is_declaration();
    }

    matches!(
        checker
            .facts()
            .command(last_segment.command_id())
            .effective_or_literal_name()
            .map(command_basename),
        Some("return" | "exit" | "true" | ":" | "echo" | "printf")
    )
}

fn matches_condition_guard_fallback(list: &ListFact<'_>) -> bool {
    let Some(first_or_index) = list
        .operators()
        .iter()
        .position(|operator| operator.op() == BinaryOp::Or)
    else {
        return false;
    };

    let Some(then_branch) = list.segments().get(first_or_index) else {
        return false;
    };
    let Some(else_branch) = list.segments().last() else {
        return false;
    };

    then_branch.kind() == ListSegmentKind::Condition
        && else_branch.kind() != ListSegmentKind::Condition
}

fn command_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn matches_and_or_ternary(list: &ListFact<'_>) -> bool {
    list.segments().len() == 3 && matches_and_then_or_chain(list)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_operator_that_introduces_mixed_short_circuiting() {
        let source = "\
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
false || true && [ \"$x\" = baz ]
true && false; false || printf '%s\\n' ok
[ \"$dir\" = vendor ] && mv go-* \"$dir\" || mv pkg-* \"$dir\"
[ -n \"$x\" ] && out=foo || out=bar
check_one && check_two && check_three || cleanup
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["&&", "&&"]
        );
    }

    #[test]
    fn ignores_status_propagation_formatter_and_guard_idioms() {
        let source = "\
cond && return 0 || return 1
return_code=0 && cmd >out 2>err || return_code=$?
is_tty && printf '%s\\n' a || printf '%s\\n' b
flag && echo enabled || echo disabled
test -n \"$x\" && [ -f out ] || die
[ -n \"$x\" ] && [ -f out ] || rm -f out
[ -n \"$x\" ] && [ -n \"$y\" ] && [ -f out ] || die
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_literal_formatter_fallbacks_inside_command_substitutions() {
        let source = "\
echo \"\\\"$BUILDSCRIPT\\\" --library $(test \"${PKG_DIR%/*}\" = \"gpkg\" && echo \"glibc\" || echo \"bionic\")\"
config=$([[ \"$mode\" = prod ]] && echo true || echo false)
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_non_sc2015_longer_or_reversed_mixed_chains() {
        let source = "\
rc=0 || run && rc=$?
rc=0 && run || fallback && rc=$?
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_final_assignment_and_status_fallback_idioms() {
        let source = "\
check_download_exists \"$path\" && download_status=0 || download_status=$?
__rvm_select_set_variable_defaults && __rvm_select_after_parse || return $?
run \"Reloading\" sig USR2 &&
wait_pid_kill 5 &&
oldsig QUIT ||
oldsig TERM ||
cmd_restart ||
return $?
cmd_reload()
{
  run \"Reloading\" sig USR2 &&
  wait_pid_kill 5 &&
  oldsig QUIT ||
  oldsig TERM ||
  cmd_restart ||
  return $?
}
test -d x && mv x y || :
test -d x && mv x y || true
test -d x && mv x y || /bin/true
test -d x && chmod 755 y || echo fail
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_mixed_short_circuit_inside_completion_registration_chains() {
        let source = "\
_comp_cmd_hostname() {
  [[ $cur == -* ]] && _comp_compgen_help || _comp_compgen_usage
} &&
  complete -F _comp_cmd_hostname hostname
_comp_cmd_mussh() {
  [[ $cur == *@* ]] && _comp_complete_user_at_host \"$@\" || _comp_compgen_known_hosts -a -- \"$cur\"
} && echo ready &&
  complete -F _comp_cmd_mussh mussh
_comp_cmd_rcs() {
  [[ ${#COMPREPLY[@]} -eq 0 && $1 == *ci ]] && _comp_compgen -a filedir || _comp_compgen -a filedir -d
} ||
  complete -F _comp_cmd_rcs ci
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_when_complete_registration_is_not_in_the_same_chain() {
        let source = "\
_comp_cmd_hostname() {
  [[ $cur == -* ]] && _comp_compgen_help || _comp_compgen_usage
}
complete -F _comp_cmd_hostname hostname
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }

    #[test]
    fn ignores_when_only_the_fallback_is_an_assignment_or_list_runs_in_condition_position() {
        let source = "\
[ -n \"$x\" ] && run_task || fallback=1
[ \"$hidden\" ] && return 0 || len=${_width%,*}
if [ ! -d \"$cache_dir\" ] && ! mkdir -p -- \"$cache_dir\" || [ ! -w \"$cache_dir\" ]; then
  :
fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_when_the_fallback_is_not_an_exempt_control_flow_or_assignment() {
        let source = "\
[ -n \"$x\" ] && run_task || fallback
check && log_ok || log_fail
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
        assert_eq!(diagnostics[1].span.slice(source), "&&");
    }

    #[test]
    fn reports_return_guards_that_fall_back_to_declaration_assignments() {
        let source = "\
init_guard() {
  [[ ${FLAG:-} -eq 1 ]] && return || readonly FLAG=1
}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }

    #[test]
    fn suppresses_nested_lists_when_an_outer_chain_is_already_reported() {
        let source = "\
check && {
  nested && log_ok || log_fail
} || cleanup
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ChainedTestBranches));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }
}
