use shuck_ast::Span;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct IndentedHeredocClose;

impl Violation for IndentedHeredocClose {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IndentedHeredocClose
    }

    fn message(&self) -> String {
        "move this here-document terminator to the start of the line".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove indentation before the here-document terminator".to_owned())
    }
}

pub fn indented_heredoc_close(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let diagnostics = checker
        .facts()
        .source_facts()
        .indented_heredoc_close_facts()
        .iter()
        .map(|&(span, indent_span)| diagnostic_for_indented_heredoc_close(span, indent_span))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostic_for_indented_heredoc_close(span: Span, indent_span: Span) -> crate::Diagnostic {
    crate::Diagnostic::new(IndentedHeredocClose, span)
        .with_fix(Fix::unsafe_edit(Edit::deletion(indent_span)))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_indented_plain_heredoc_terminators() {
        let source = "\
#!/bin/sh
cat <<EOF
hi
 EOF
  EOF
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IndentedHeredocClose),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(4, 1), (5, 1)]
        );
    }

    #[test]
    fn ignores_tab_stripped_and_non_terminator_lines() {
        let source = "\
#!/bin/sh
cat <<-EOF
hi
\tEOF
cat <<EOF
hi EOF
EOF
cat <<EOF
hi
EOF 
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IndentedHeredocClose),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn skips_zsh() {
        let source = "#!/bin/zsh\ncat <<EOF\nhi\n EOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::IndentedHeredocClose).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_by_deleting_indentation() {
        let source = "\
#!/bin/sh
cat <<EOF
hi
 EOF
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IndentedHeredocClose),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
cat <<EOF
hi
EOF
"
        );
    }
}
