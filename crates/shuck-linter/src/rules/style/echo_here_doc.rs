use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Locator, Rule, Violation};

pub struct EchoHereDoc;

impl Violation for EchoHereDoc {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::EchoHereDoc
    }

    fn message(&self) -> String {
        "here-document input on `echo` is ignored".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the ignored here-document".to_owned())
    }
}

pub fn echo_here_doc(checker: &mut Checker) {
    let locator = checker.locator();
    let spans = checker.facts().echo_here_doc_spans().to_vec();
    for span in spans {
        let mut diagnostic = Diagnostic::new(EchoHereDoc, span);
        if let Some(fix) = echo_here_doc_fix(locator, span) {
            diagnostic = diagnostic.with_fix(fix);
        }
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn echo_here_doc_fix(locator: Locator<'_>, span: Span) -> Option<Fix> {
    let source = locator.source();
    let span_text = span.slice(source);
    let operator_offset = span_text.find("<<")?;
    let redirect_start = span.start.offset + operator_offset;
    let marker = heredoc_marker_from_redirect_text(&span_text[operator_offset..])?;
    if marker.name.is_empty() {
        return None;
    }

    let line_range = locator.line_range(span.start.line)?;
    let line_start = usize::from(line_range.start());
    let header_delete_start = preceding_space_start(source, redirect_start, line_start);
    let body_start = locator
        .line_index()
        .line_start(span.start.line + 1)
        .map(usize::from)
        .unwrap_or(source.len());
    let body_end = heredoc_body_end(locator, span.start.line + 1, &marker)?;

    Some(Fix::unsafe_edits([
        Edit::deletion_at(header_delete_start, span.end.offset),
        Edit::deletion_at(body_start, body_end),
    ]))
}

struct HeredocMarker {
    name: String,
    strip_tabs: bool,
}

fn heredoc_marker_from_redirect_text(text: &str) -> Option<HeredocMarker> {
    let mut marker = text.strip_prefix("<<")?;
    let strip_tabs = marker.starts_with('-');
    if strip_tabs {
        marker = &marker[1..];
    }
    let name = marker
        .trim()
        .trim_matches(|ch| matches!(ch, '\'' | '"' | '\\'))
        .to_owned();
    Some(HeredocMarker { name, strip_tabs })
}

fn heredoc_body_end(
    locator: Locator<'_>,
    mut line_number: usize,
    marker: &HeredocMarker,
) -> Option<usize> {
    let source = locator.source();
    loop {
        let line_range = locator.line_range(line_number)?;
        let line_start = usize::from(line_range.start());
        let line_end = trim_cr_end(source, usize::from(line_range.end()));
        let line = source.get(line_start..line_end)?;
        let candidate = if marker.strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line
        };
        if candidate == marker.name {
            return Some(
                locator
                    .line_index()
                    .line_start(line_number + 1)
                    .map(usize::from)
                    .unwrap_or(source.len()),
            );
        }
        line_number += 1;
    }
}

fn preceding_space_start(source: &str, mut offset: usize, floor: usize) -> usize {
    while offset > floor && matches!(source.as_bytes().get(offset - 1), Some(b' ' | b'\t')) {
        offset -= 1;
    }
    offset
}

fn trim_cr_end(source: &str, offset: usize) -> usize {
    if offset > 0 && matches!(source.as_bytes().get(offset - 1), Some(b'\r')) {
        offset - 1
    } else {
        offset
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_heredoc_attached_to_echo() {
        let source = "\
#!/bin/sh
echo <<EOF
hi
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo <<EOF");
    }

    #[test]
    fn reports_tab_stripping_heredoc_attached_to_echo() {
        let source = "\
#!/bin/sh
echo <<-EOF
\thi
\tEOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo <<-EOF");
    }

    #[test]
    fn ignores_heredoc_attached_to_non_echo_commands() {
        let source = "\
#!/bin/sh
cat <<EOF
hi
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_delete_echo_heredoc_redirect_and_body() {
        let source = "#!/bin/sh\necho <<EOF\nhi\nEOF\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::EchoHereDoc),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\necho\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
