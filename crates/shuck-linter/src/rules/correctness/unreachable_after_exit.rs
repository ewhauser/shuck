use crate::{Checker, Rule, Violation};
use shuck_ast::Span;

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
    let unreachable_spans = outermost_unreachable_spans(
        checker
            .semantic_analysis()
            .dead_code()
            .iter()
            .flat_map(|dead_code| dead_code.unreachable.iter().copied())
            .collect::<Vec<_>>(),
    );

    for span in unreachable_spans
        .into_iter()
        .map(|span| trim_trailing_whitespace(span, source))
    {
        checker.report(UnreachableAfterExit, span);
    }
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

fn trim_trailing_whitespace(span: Span, source: &str) -> Span {
    let trimmed = span.slice(source).trim_end_matches(char::is_whitespace);
    Span::from_positions(span.start, span.start.advanced_by(trimmed))
}
