use shuck_ast::{BackgroundOperator, Span, StmtTerminator};

use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CPrototypeFragment;

impl Violation for CPrototypeFragment {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::CPrototypeFragment
    }

    fn message(&self) -> String {
        "add a space after `&` when using background execution".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space after `&`".to_owned())
    }
}

pub fn c_prototype_fragment(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| attached_background_ampersand_span(command, source))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(CPrototypeFragment, span)
                .with_fix(Fix::safe_edit(Edit::insertion(span.start.offset + 1, " "))),
        );
    }
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
    use crate::test::test_snippet_with_fix;
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

    #[test]
    fn attaches_safe_fix_metadata() {
        let source = "#!/bin/sh\nX &NextItem ();\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CPrototypeFragment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(crate::Applicability::Safe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("insert a space after `&`")
        );
    }

    #[test]
    fn applies_safe_fix_to_attached_background_ampersands() {
        let source = "#!/bin/sh\nX &NextItem ();\necho foo &bar\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CPrototypeFragment),
            crate::Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nX & NextItem ();\necho foo & bar\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_near_misses_unchanged_when_fixing() {
        let source = "#!/bin/sh\necho foo \\&bar\necho '&bar'\necho foo & bar\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CPrototypeFragment),
            crate::Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }
}
