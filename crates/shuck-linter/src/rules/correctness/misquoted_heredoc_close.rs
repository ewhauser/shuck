use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct MisquotedHeredocClose;

impl Violation for MisquotedHeredocClose {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::MisquotedHeredocClose
    }

    fn message(&self) -> String {
        "this here-document closing marker is only a near match".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append the unquoted heredoc closing marker".to_owned())
    }
}

pub fn misquoted_heredoc_close(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .source_facts()
        .misquoted_heredoc_close_spans()
        .iter()
        .copied()
        .map(|span| {
            let diagnostic = Diagnostic::new(MisquotedHeredocClose, span);
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
    let text = text.trim_start_matches(|ch: char| ch.is_ascii_digit());
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
    fn reports_quoted_close_match_for_quoted_delimiter() {
        let source = "\
#!/bin/bash
cat <<'BLOCK'
x
'BLOCK'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn applies_safe_fix_by_appending_unquoted_heredoc_marker() {
        let source = "#!/bin/bash\ncat <<'BLOCK'\nx\n'BLOCK'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\ncat <<'BLOCK'\nx\n'BLOCK'\nBLOCK\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_fd_prefixed_heredoc_redirects() {
        let source = "#!/bin/bash\ncat 3<<'BLOCK'\nx\n'BLOCK'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\ncat 3<<'BLOCK'\nx\n'BLOCK'\nBLOCK\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_content_plus_delimiter_lines_to_avoid_c144_overlap() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_properly_closed_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }
}
