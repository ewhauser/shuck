use shuck_ast::{Command, Redirect, SimpleCommand, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SourceBuiltinInSh;

impl Violation for SourceBuiltinInSh {
    fn rule() -> Rule {
        Rule::SourceBuiltinInSh
    }

    fn message(&self) -> String {
        "`source` is not portable in `sh` scripts".to_owned()
    }
}

pub fn source_builtin_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("source"))
        .map(|fact| {
            source_anchor_span(
                fact.command(),
                fact.redirects(),
                fact.span(),
                checker.source(),
            )
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || SourceBuiltinInSh);
}

fn source_anchor_span(
    command: &Command,
    redirects: &[Redirect],
    fallback: Span,
    source: &str,
) -> Span {
    match command {
        Command::Simple(command) => {
            clip_first_line(simple_command_span(command, redirects), source)
        }
        _ => clip_first_line(fallback, source),
    }
}

fn simple_command_span(command: &SimpleCommand, redirects: &[Redirect]) -> Span {
    let start = command
        .assignments
        .first()
        .map_or(command.name.span.start, |assignment| assignment.span.start);
    let mut end = command
        .args
        .last()
        .map_or(command.name.span.end, |word| word.span.end);
    if let Some(redirect) = redirects.last() {
        end = redirect.span.end;
    }
    Span::from_positions(start, end)
}

fn clip_first_line(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let Some(line_end) = text.find('\n') else {
        return span;
    };

    let first_line = text[..line_end].trim_end_matches('\r');
    Span::from_positions(span.start, span.start.advanced_by(first_line))
}
