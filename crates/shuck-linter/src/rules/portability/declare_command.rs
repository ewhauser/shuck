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
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("declare"))
        .map(|fact| declare_anchor_span(fact, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all(spans, || DeclareCommand);
}

fn declare_anchor_span(fact: &crate::CommandFact<'_>, source: &str) -> Span {
    if let Some(declaration) = fact.declaration() {
        let end = declaration_anchor_end(fact, declaration.head_span.end, source);

        return Span::from_positions(fact.span().start, end);
    }

    Span::from_positions(fact.span().start, command_anchor_end(fact, source))
}

fn declaration_anchor_end(
    fact: &crate::CommandFact<'_>,
    mut end: shuck_ast::Position,
    source: &str,
) -> shuck_ast::Position {
    for redirect in fact.redirects() {
        if redirect.span.end.offset > end.offset {
            end = redirect.span.end;
        }
    }

    clip_terminator(fact, end, source)
}

fn command_anchor_end(fact: &crate::CommandFact<'_>, source: &str) -> shuck_ast::Position {
    clip_terminator(fact, fact.span_in_source(source).end, source)
}

fn clip_terminator(
    fact: &crate::CommandFact<'_>,
    mut end: shuck_ast::Position,
    _source: &str,
) -> shuck_ast::Position {
    if let Some(terminator_span) = fact.stmt().terminator_span
        && terminator_span.start.offset < end.offset
    {
        end = terminator_span.start;
    }

    end
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn excludes_assignment_values_for_direct_declarations() {
        let source = "#!/bin/sh\nFOO=1 declare bar=baz\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "FOO=1 declare bar");
    }

    #[test]
    fn includes_attached_redirects_without_statement_terminators() {
        let source = "#!/bin/sh\nif declare -f pre_step >/dev/null; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "declare -f pre_step >/dev/null"
        );
    }

    #[test]
    fn anchors_wrapped_declare_on_the_full_command() {
        let source = "#!/bin/sh\ncommand declare wrapped=value\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "command declare wrapped=value"
        );
    }

    #[test]
    fn keeps_declaration_operands_after_interleaved_redirects() {
        let source = "#!/bin/sh\ndeclare >/tmp/out foo=bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DeclareCommand));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "declare >/tmp/out foo");
    }
}
