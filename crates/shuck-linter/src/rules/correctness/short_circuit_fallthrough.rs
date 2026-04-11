use crate::facts::{ListFact, ListSegmentKind, MixedShortCircuitKind};
use crate::{Checker, Rule, Violation};

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
    let segments = list.segments();
    if segments
        .first()
        .is_some_and(|segment| segment.kind() == ListSegmentKind::AssignmentOnly)
        || segments
            .last()
            .is_some_and(|segment| segment.kind() == ListSegmentKind::AssignmentOnly)
    {
        return true;
    }

    let branch_names = segments
        .iter()
        .skip(1)
        .map(|segment| {
            checker
                .facts()
                .command(segment.command_id())
                .effective_or_literal_name()
        })
        .collect::<Vec<_>>();

    if branch_names
        .iter()
        .flatten()
        .any(|name| matches!(*name, "return" | "exit"))
    {
        return true;
    }

    matches!(
        branch_names.as_slice(),
        [Some("echo"), Some("echo")] | [Some("printf"), Some("printf")]
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
