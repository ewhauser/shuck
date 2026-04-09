use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct DeclareCommand;

impl Violation for DeclareCommand {
    fn rule() -> Rule {
        Rule::DeclareCommand
    }

    fn message(&self) -> String {
        "`declare` is not portable in `sh` scripts".to_owned()
    }
}

pub fn declare_command(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("declare"))
        .map(|fact| {
            fact.declaration().map_or(fact.body_span(), |declaration| {
                declare_anchor_span(declaration.span, checker.source())
            })
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || DeclareCommand);
}

fn declare_anchor_span(span: Span, source: &str) -> Span {
    let text = span.slice(source);
    let Some(line_end) = text.find('\n') else {
        return span;
    };

    let first_line = text[..line_end].trim_end_matches('\r');
    Span::from_positions(span.start, span.start.advanced_by(first_line))
}
