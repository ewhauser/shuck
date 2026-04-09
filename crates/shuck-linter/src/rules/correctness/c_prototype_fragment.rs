use shuck_ast::{BackgroundOperator, Span, StmtTerminator};

use crate::{Checker, Rule, Violation};

pub struct CPrototypeFragment;

impl Violation for CPrototypeFragment {
    fn rule() -> Rule {
        Rule::CPrototypeFragment
    }

    fn message(&self) -> String {
        "add a space after `&` when using background execution".to_owned()
    }
}

pub fn c_prototype_fragment(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| attached_background_ampersand_span(command, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CPrototypeFragment);
}

fn attached_background_ampersand_span(
    command: &crate::CommandFact<'_>,
    source: &str,
) -> Option<Span> {
    if command.stmt().terminator != Some(StmtTerminator::Background(BackgroundOperator::Plain)) {
        return None;
    }
    let terminator_span = command.stmt().terminator_span?;
    if terminator_span.slice(source) != "&" {
        return None;
    }

    let next = source[terminator_span.end.offset..].chars().next()?;
    if !matches!(next, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9') {
        return None;
    }

    Some(Span::from_positions(
        terminator_span.start,
        terminator_span.start,
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_attached_ampersand_tokens() {
        let source = "#!/bin/sh\nX &NextItem ();\necho foo &bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CPrototypeFragment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 3);
        assert_eq!(diagnostics[1].span.start.line, 3);
        assert_eq!(diagnostics[1].span.start.column, 10);
    }

    #[test]
    fn ignores_escaped_or_quoted_ampersands() {
        let source = "#!/bin/sh\necho foo \\&bar\necho '&bar'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CPrototypeFragment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_background_with_space_after_ampersand() {
        let source = "#!/bin/sh\necho foo & bar\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CPrototypeFragment));

        assert!(diagnostics.is_empty());
    }
}
