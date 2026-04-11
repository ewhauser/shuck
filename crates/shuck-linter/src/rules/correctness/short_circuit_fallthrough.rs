use crate::facts::{ListFact, ListSegmentKind, MixedShortCircuitKind};
use crate::{Checker, Rule, Violation};
use shuck_ast::BinaryOp;

pub struct ShortCircuitFallthrough;

impl Violation for ShortCircuitFallthrough {
    fn rule() -> Rule {
        Rule::ShortCircuitFallthrough
    }

    fn message(&self) -> String {
        "mixing `&&` and `||` here can make the fallback depend on the middle command".to_owned()
    }
}

pub fn short_circuit_fallthrough(checker: &mut Checker) {
    let spans = checker
        .facts()
        .lists()
        .iter()
        .filter(|list| list.mixed_short_circuit_kind() == Some(MixedShortCircuitKind::Fallthrough))
        .filter(|list| !list_exempts_warning(checker, list))
        .filter_map(|list| list.mixed_short_circuit_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ShortCircuitFallthrough);
}

fn list_exempts_warning(checker: &Checker<'_>, list: &ListFact<'_>) -> bool {
    if matches_status_propagation_assignment(list) {
        return true;
    }

    let Some(branch_names) = ternary_branch_names(checker, list) else {
        return false;
    };

    if branch_names
        .iter()
        .all(|name| matches!(name, Some("return" | "exit")))
    {
        return true;
    }

    matches!(
        &branch_names,
        [Some("echo"), Some("echo")] | [Some("printf"), Some("printf")]
    )
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

fn ternary_branch_names<'a>(
    checker: &'a Checker<'a>,
    list: &ListFact<'a>,
) -> Option<[Option<&'a str>; 2]> {
    if !matches_and_or_ternary(list) {
        return None;
    }

    let [_, then_branch, else_branch] = list.segments() else {
        return None;
    };

    Some([
        checker
            .facts()
            .command(then_branch.command_id())
            .effective_or_literal_name(),
        checker
            .facts()
            .command(else_branch.command_id())
            .effective_or_literal_name(),
    ])
}

fn matches_and_or_ternary(list: &ListFact<'_>) -> bool {
    list.segments().len() == 3
        && matches!(
            list.operators()
                .iter()
                .map(|operator| operator.op())
                .collect::<Vec<_>>()
                .as_slice(),
            [BinaryOp::And, BinaryOp::Or]
        )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_fallthrough_chains_but_skips_other_mixed_chain_kinds() {
        let source = "\
[ \"$dir\" = vendor ] && mv go-* \"$dir\" || mv pkg-* \"$dir\"
true && false || printf '%s\\n' fallback
[ \"$x\" = foo ] && [ \"$x\" = bar ] || [ \"$x\" = baz ]
[ -n \"$x\" ] && out=foo || out=bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["&&", "&&"]
        );
    }

    #[test]
    fn ignores_status_propagation_and_formatter_idioms() {
        let source = "\
cond && return 0 || return 1
return_code=0 && cmd >out 2>err || return_code=$?
is_tty && printf '%s\\n' a || printf '%s\\n' b
flag && echo enabled || echo disabled
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_ignores_three_segment_status_propagation_shapes() {
        let source = "\
rc=0 || run && rc=$?
rc=0 && run || fallback && rc=$?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["||", "&&"]
        );
    }

    #[test]
    fn only_ignores_three_segment_return_and_formatter_shapes() {
        let source = "\
cond && run || cleanup && exit 1
cond || printf '%s\\n' a && printf '%s\\n' b
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["&&", "||"]
        );
    }

    #[test]
    fn still_reports_when_only_the_fallback_is_an_assignment() {
        let source = "\
[ -n \"$x\" ] && run_task || fallback=1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }

    #[test]
    fn still_reports_when_only_the_middle_branch_is_an_assignment() {
        let source = "\
cmd && ok=1 || fallback
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShortCircuitFallthrough),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }
}
