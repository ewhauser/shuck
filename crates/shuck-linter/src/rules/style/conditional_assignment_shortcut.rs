use crate::facts::{ListFact, ListSegmentKind};
use crate::{Checker, Rule, Violation};

pub struct ConditionalAssignmentShortcut;

impl Violation for ConditionalAssignmentShortcut {
    fn rule() -> Rule {
        Rule::ConditionalAssignmentShortcut
    }

    fn message(&self) -> String {
        "assignment hidden inside a `&&`/`||` shortcut is harder to follow than an explicit `if`"
            .to_owned()
    }
}

pub fn conditional_assignment_shortcut(checker: &mut Checker) {
    let spans = checker
        .facts()
        .lists()
        .iter()
        .filter_map(|list| conditional_assignment_span(checker, list))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ConditionalAssignmentShortcut);
}

fn conditional_assignment_span(checker: &Checker, list: &ListFact<'_>) -> Option<shuck_ast::Span> {
    let segments = list.segments();
    let [first, second, rest @ ..] = segments else {
        return None;
    };

    if rest.is_empty()
        && first.kind() == ListSegmentKind::AssignmentOnly
        && second.kind() != ListSegmentKind::AssignmentOnly
    {
        return Some(command_span_in_source(checker, first));
    }

    segments
        .windows(3)
        .find_map(|window| match window {
            [previous, current, next]
                if previous.kind() == ListSegmentKind::Condition
                    && current.kind() == ListSegmentKind::AssignmentOnly
                    && next.kind() != ListSegmentKind::AssignmentOnly =>
            {
                Some(command_span_in_source(checker, current))
            }
            _ => None,
        })
        .or_else(|| {
            match (
                segments.get(segments.len().wrapping_sub(2)),
                segments.last(),
            ) {
                (Some(previous), Some(last))
                    if previous.kind() == ListSegmentKind::Condition
                        && last.kind() == ListSegmentKind::AssignmentOnly =>
                {
                    Some(command_span_in_source(checker, last))
                }
                _ => None,
            }
        })
}

fn command_span_in_source(
    checker: &Checker,
    segment: &crate::facts::ListSegmentFact,
) -> shuck_ast::Span {
    if let Some(span) = segment.assignment_span() {
        return span;
    }

    checker
        .facts()
        .command(segment.command_id())
        .span_in_source(checker.source())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_shortcuts_but_skips_assignment_ternaries_and_generic_commands() {
        let source = "\
#!/bin/sh
false || remove=set
true && remove=set
true && declare -x chosen=set
remove=set || echo nope
true && remove=set && echo later
[ -n \"$x\" ] && domain=$domain || domain=$str
echo ok && remove=set
foo=bar && baz=qux
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ConditionalAssignmentShortcut),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "remove=set",
                "remove=set",
                "chosen=set",
                "remove=set",
                "remove=set",
            ]
        );
    }
}
