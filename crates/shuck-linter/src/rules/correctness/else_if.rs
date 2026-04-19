use shuck_ast::{Command, CompoundCommand, Position, Span};

use crate::{Checker, Rule, Violation};

pub struct ElseIf;

impl Violation for ElseIf {
    fn rule() -> Rule {
        Rule::ElseIf
    }

    fn message(&self) -> String {
        "use `elif` instead of `else if`".to_owned()
    }
}

pub fn else_if(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let Command::Compound(CompoundCommand::If(command)) = fact.command() else {
                return None;
            };
            let branch = command.else_branch.as_ref()?;
            let first = branch.stmts.first()?;
            let Command::Compound(CompoundCommand::If(_)) = &first.command else {
                return None;
            };

            else_if_span(first.span.start, checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || ElseIf);
}

fn else_if_span(nested_if_start: Position, source: &str) -> Option<Span> {
    let line_start_offset = source[..nested_if_start.offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let line_prefix = &source[line_start_offset..nested_if_start.offset];
    let trimmed = line_prefix.trim_end_matches([' ', '\t']);
    if !trimmed.ends_with("else") {
        return None;
    }

    let else_start_in_line = trimmed.len().saturating_sub("else".len());
    let line_start = Position {
        line: nested_if_start.line,
        column: 1,
        offset: line_start_offset,
    };
    let else_start = line_start.advanced_by(&line_prefix[..else_start_in_line]);
    Some(Span::at(else_start))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_same_line_else_if() {
        let source = "#!/bin/sh\nif true; then\n  :\nelse if true; then\n  :\nfi\nfi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ElseIf));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_nested_else_with_newline_if() {
        let source = "#!/bin/sh\nif true; then\n  :\nelse\n  if true; then\n    :\n  fi\nfi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ElseIf));

        assert!(diagnostics.is_empty());
    }
}
