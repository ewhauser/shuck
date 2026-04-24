use crate::{Checker, CommandFact, ListFact, Rule, Violation};
use shuck_ast::{BinaryOp, Span};
use shuck_semantic::UnreachableCauseKind;

pub struct UnreachableAfterExit;

impl Violation for UnreachableAfterExit {
    fn rule() -> Rule {
        Rule::UnreachableAfterExit
    }

    fn message(&self) -> String {
        "code is unreachable".to_owned()
    }
}

pub fn unreachable_after_exit(checker: &mut Checker) {
    let source = checker.source();
    let short_circuit_guard_spans = short_circuit_exit_guard_spans(checker);
    let unreachable_spans = outermost_unreachable_spans(
        checker
            .semantic_analysis()
            .dead_code()
            .iter()
            .filter(|dead_code| dead_code.cause_kind != UnreachableCauseKind::LoopControl)
            .flat_map(|dead_code| dead_code.unreachable.iter().copied())
            .filter(|span| !short_circuit_guard_spans.contains(span))
            .collect::<Vec<_>>(),
    );

    for span in unreachable_spans
        .into_iter()
        .map(|span| trim_trailing_terminator(span, source))
    {
        checker.report(UnreachableAfterExit, span);
    }
}

fn short_circuit_exit_guard_spans(checker: &Checker) -> Vec<Span> {
    let commands = checker.facts().commands();
    checker
        .facts()
        .lists()
        .iter()
        .filter(|list| list_is_exit_guard(list, commands))
        .map(ListFact::span)
        .collect()
}

fn list_is_exit_guard(list: &ListFact<'_>, commands: &[CommandFact<'_>]) -> bool {
    if list
        .operators()
        .last()
        .is_none_or(|operator| operator.op() != BinaryOp::Or)
    {
        return false;
    }

    let Some(terminator_id) = list.segments().last().map(|segment| segment.command_id()) else {
        return false;
    };
    let Some(terminator) = commands
        .iter()
        .find(|command| command.id() == terminator_id)
    else {
        return false;
    };

    matches!(terminator.static_utility_name(), Some("exit" | "return"))
}

fn outermost_unreachable_spans(mut spans: Vec<shuck_ast::Span>) -> Vec<shuck_ast::Span> {
    spans.sort_by(|left, right| {
        left.start
            .offset
            .cmp(&right.start.offset)
            .then_with(|| right.end.offset.cmp(&left.end.offset))
    });

    let mut outermost = Vec::new();
    for span in spans {
        if outermost
            .iter()
            .any(|outer| span_contained_by(span, *outer))
        {
            continue;
        }
        if outermost.contains(&span) {
            continue;
        }
        outermost.push(span);
    }
    outermost
}

fn span_contained_by(inner: shuck_ast::Span, outer: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn trim_trailing_terminator(span: Span, source: &str) -> Span {
    let trimmed = span
        .slice(source)
        .trim_end_matches(char::is_whitespace)
        .trim_end_matches(';')
        .trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}
