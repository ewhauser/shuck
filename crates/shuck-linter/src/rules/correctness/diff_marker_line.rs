use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct DiffMarkerLine;

impl Violation for DiffMarkerLine {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::DiffMarkerLine
    }

    fn message(&self) -> String {
        "this dash-prefixed command looks like leftover patch text".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("comment out the dash-prefixed line".to_owned())
    }
}

pub fn diff_marker_line(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let mut diagnostics = checker
        .facts()
        .command_facts()
        .structural_commands()
        .filter(|command| command.command_name_starts_with_literal_dash(source))
        .filter(|command| !command.command_name_follows_escaped_semicolon(source))
        .filter_map(|command| {
            let name = command.command_name_word()?;
            Some(
                Diagnostic::new(DiffMarkerLine, name.span).with_fix(Fix::unsafe_edit(
                    Edit::insertion(name.span.start.offset, "# "),
                )),
            )
        })
        .collect::<Vec<_>>();
    diagnostics.extend(
        checker
            .facts()
            .source_facts()
            .escaped_dash_command_name_spans()
            .iter()
            .copied()
            .map(|span| {
                Diagnostic::new(DiffMarkerLine, span)
                    .with_fix(Fix::unsafe_edit(Edit::insertion(span.start.offset, "# ")))
            }),
    );

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_dash_prefixed_command_names() {
        let source = "\
#!/bin/sh
--- a/sample.txt
  --- indented.txt
--help
\\-n foo
-$tool foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["---", "---", "--help", "\\-n", "-$tool"]
        );
    }

    #[test]
    fn ignores_arguments_comments_quoted_names_and_zsh() {
        let source = "\
#!/bin/sh
echo --- a/sample.txt
# --- comment.txt
cat <<'DOC'
--- heredoc.txt
DOC
\"-n\" foo
'-n' foo
command -n foo
find . -exec chmod 755 {}\\; -o -name '*.txt'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");

        let zsh_diagnostics = test_snippet(
            "#!/bin/zsh\n--- a/sample.txt\n",
            &LinterSettings::for_rule(Rule::DiffMarkerLine).with_shell(ShellDialect::Zsh),
        );
        assert!(
            zsh_diagnostics.is_empty(),
            "diagnostics: {zsh_diagnostics:?}"
        );
    }

    #[test]
    fn applies_unsafe_fix_before_the_command_name() {
        let source = "#!/bin/sh\n  --- a/sample.txt\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n  # --- a/sample.txt\n");
    }
}
