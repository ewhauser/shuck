use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct HeredocEndSpace;

impl Violation for HeredocEndSpace {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::HeredocEndSpace
    }

    fn message(&self) -> String {
        "remove trailing whitespace after this here-document terminator".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the trailing heredoc terminator whitespace".to_owned())
    }
}

pub fn heredoc_end_space(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .source_facts()
        .heredoc_end_space_spans()
        .iter()
        .copied()
        .filter_map(|span| heredoc_end_space_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(HeredocEndSpace, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn heredoc_end_space_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let start = span.start.offset;
    let rest = source.get(start..)?;
    let len = rest
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .map(char::len_utf8)
        .sum::<usize>();
    (len > 0).then(|| {
        (
            span,
            Fix::unsafe_edit(Edit::deletion_at(start, start + len)),
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_trailing_space_after_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.line, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn reports_trailing_tab_after_heredoc_terminator() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF\t\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn reports_trailing_space_for_tab_stripped_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tvalue
\tEOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 5);
        assert_eq!(diagnostics[0].span.end.column, 5);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn anchors_at_the_first_trailing_whitespace_character() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF  \t\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.line, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn applies_unsafe_fix_by_deleting_trailing_heredoc_terminator_space() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF  \t\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::HeredocEndSpace),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ncat <<EOF\nok\nEOF\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_properly_terminated_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_the_first_bad_terminator_per_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
value
EOF 
EOF\t
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }
}
