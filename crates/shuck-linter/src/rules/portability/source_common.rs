use shuck_ast::{ArenaFileCommandKind, RedirectNode, Span};

use crate::{Checker, CommandFactRef, ShellDialect};

pub(super) fn source_command_spans_in_sh(checker: &Checker<'_>) -> Vec<Span> {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return Vec::new();
    }

    let source = checker.source();
    checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("source"))
        .map(|fact| source_anchor_span_for_command_fact(fact, source))
        .collect()
}

pub(super) fn source_anchor_span_for_command_fact(
    fact: CommandFactRef<'_, '_>,
    source: &str,
) -> Span {
    source_anchor_span(fact, source)
}

fn source_anchor_span(fact: CommandFactRef<'_, '_>, source: &str) -> Span {
    match fact.command_kind() {
        ArenaFileCommandKind::Simple => clip_first_line(simple_command_span(fact), source),
        _ => clip_first_line(fact.span(), source),
    }
}

fn simple_command_span(fact: CommandFactRef<'_, '_>) -> Span {
    let command = fact
        .arena_command()
        .and_then(|command| command.simple())
        .expect("simple command fact should have a simple arena command");
    let start = command
        .assignments()
        .first()
        .map_or(command.name().span().start, |assignment| {
            assignment.span.start
        });
    let mut end = command
        .args()
        .last()
        .map_or(command.name().span().end, |word| word.span().end);
    if let Some(redirect) = fact.arena_redirects().and_then(last_redirect) {
        end = redirect.span.end;
    }
    Span::from_positions(start, end)
}

fn last_redirect(redirects: &[RedirectNode]) -> Option<&RedirectNode> {
    redirects.last()
}

fn clip_first_line(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let Some(line_end) = text.find('\n') else {
        return span;
    };

    let first_line = text[..line_end].trim_end_matches('\r');
    Span::from_positions(span.start, span.start.advanced_by(first_line))
}
