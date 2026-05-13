use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct HeredocMissingEnd;

impl Violation for HeredocMissingEnd {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::HeredocMissingEnd
    }

    fn message(&self) -> String {
        "this here-document is missing its closing marker".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append the missing heredoc closing marker".to_owned())
    }
}

pub fn heredoc_missing_end(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .source_facts()
        .heredoc_missing_end_spans()
        .iter()
        .copied()
        .map(|span| {
            let diagnostic = Diagnostic::new(HeredocMissingEnd, span);
            match heredoc_closer_fix(span, source) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            }
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn heredoc_closer_fix(span: Span, source: &str) -> Option<Fix> {
    let marker = heredoc_marker_from_redirect_span(span, source)?;
    if marker.is_empty() {
        return None;
    }
    let prefix = if source.is_empty() || source.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    Some(Fix::safe_edit(Edit::insertion(
        source.len(),
        format!("{prefix}{marker}\n"),
    )))
}

fn heredoc_marker_from_redirect_span(span: Span, source: &str) -> Option<String> {
    let text = span.slice(source).trim();
    let marker = text
        .strip_prefix("<<-")
        .or_else(|| text.strip_prefix("<<"))?
        .trim();
    Some(
        marker
            .trim_matches(|ch| matches!(ch, '\'' | '"' | '\\'))
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_heredoc_without_a_closing_marker() {
        let source = "\
#!/bin/sh
cat <<MARK
line
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "<<MARK");
    }

    #[test]
    fn applies_safe_fix_by_appending_missing_heredoc_marker() {
        let source = "#!/bin/sh\ncat <<MARK\nline\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::HeredocMissingEnd),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ncat <<MARK\nline\nMARK\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn reports_unclosed_empty_delimiter_heredoc() {
        let source = "\
#!/bin/sh
cat <<''
line
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "<<''");
    }

    #[test]
    fn ignores_closed_heredoc_even_when_delimiter_has_no_trailing_newline() {
        let source = "#!/bin/sh\ncat <<MARK\nline\nMARK";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocMissingEnd));

        assert!(diagnostics.is_empty());
    }
}
