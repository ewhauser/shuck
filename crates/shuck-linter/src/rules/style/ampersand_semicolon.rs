use shuck_ast::{BackgroundOperator, Span, StmtTerminator};

use crate::{Checker, Rule, Violation};

pub struct AmpersandSemicolon;

impl Violation for AmpersandSemicolon {
    fn rule() -> Rule {
        Rule::AmpersandSemicolon
    }

    fn message(&self) -> String {
        "background command should not be followed by `;`".to_owned()
    }
}

pub fn ampersand_semicolon(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| ampersand_semicolon_span(command, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AmpersandSemicolon);
}

fn ampersand_semicolon_span(command: &crate::CommandFact<'_>, source: &str) -> Option<Span> {
    if command.stmt().terminator != Some(StmtTerminator::Background(BackgroundOperator::Plain)) {
        return None;
    }
    let terminator_span = command.stmt().terminator_span?;
    if terminator_span.slice(source) != "&" {
        return None;
    }

    let mut semicolon_offset = None;
    for (relative, ch) in source[terminator_span.end.offset..].char_indices() {
        if matches!(ch, ' ' | '\t' | '\r') {
            continue;
        }
        if ch == '\n' || ch == '#' {
            return None;
        }
        if ch == ';' {
            semicolon_offset = Some(terminator_span.end.offset + relative);
        }
        break;
    }
    let semicolon_offset = semicolon_offset?;
    let start = position_at_offset(source, semicolon_offset)?;
    let end = start.advanced_by(";");
    Some(Span::from_positions(start, end))
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<shuck_ast::Position> {
    if target_offset > source.len() {
        return None;
    }
    let mut position = shuck_ast::Position::new();
    for ch in source[..target_offset].chars() {
        position.advance(ch);
    }
    Some(position)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_background_followed_by_semicolon() {
        let source = "#!/bin/sh\necho x &;\necho y & ;\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), ";");
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.slice(source), ";");
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn ignores_background_without_semicolon() {
        let source = "#!/bin/sh\necho x &\nwait\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert!(diagnostics.is_empty());
    }
}
