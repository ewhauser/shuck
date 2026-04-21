use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct LinebreakInTest;

impl Violation for LinebreakInTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LinebreakInTest
    }

    fn message(&self) -> String {
        "`[` test spans lines without a trailing `\\` before the newline".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a trailing `\\` before the newline".to_owned())
    }
}

pub fn linebreak_in_test(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let anchor_span = fact.linebreak_in_test_anchor_span()?;
            let insert_offset = fact.linebreak_in_test_insert_offset()?;
            Some(
                crate::Diagnostic::new(LinebreakInTest, anchor_span)
                    .with_fix(Fix::unsafe_edit(Edit::insertion(insert_offset, "\\"))),
            )
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_linebreak_between_open_and_close_brackets() {
        let source = "#!/bin/sh\nif [ \"$x\" = y\n]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 14);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
    }

    #[test]
    fn ignores_backslash_continued_test_lines() {
        let source = "#!/bin/sh\nif [ \"$x\" = y \\\n]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_regular_single_line_bracket_tests() {
        let source = "#!/bin/sh\nif [ \"$x\" = y ]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_other_missing_closing_bracket_shapes() {
        let source = "#!/bin/sh\nif [ \"$x\" = y; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\nif [ \"$x\" = y\n]; then :; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("insert a trailing `\\` before the newline")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_split_bracket_tests() {
        let source = "\
#!/bin/sh
if [ \"$x\" = y
]; then :; fi

if [ \"$x\" = z
]; then :; fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LinebreakInTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
if [ \"$x\" = y\\
]; then :; fi

if [ \"$x\" = z\\
]; then :; fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C040.sh").as_path(),
            &LinterSettings::for_rule(Rule::LinebreakInTest),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C040_fix_C040.sh", result);
        Ok(())
    }

    #[test]
    fn ignores_recovered_command_ordering_regression() {
        let source = concat!(
            "#!/bin/bash\n\n",
            "# Invalid: the quoted home-relative path stays literal in `[ ]`.\n",
            "[ \"$profile\" = \"~/.bashrc\" ]\n\n",
            "# Invalid: either side of the string comparison can carry the quoted `~/...`.\n",
            "[ \"~/.bash_profile\n",
            "[[ \"$profile\" == \"~/.zshrc\" ]]\n\n",
            "# Invalid: single quotes still prevent tilde expansion.\n",
            "[ \"$porfile\" != '~/.config/fish/config.fish' ]\n\n",
            "# Valid: an unquoted tilde expands before the comparison.\n",
            "[ \"$profile\" = ~/.bashr` ]\n\n",
            "# Valid: `~user` is a different lookup and not interchangeable printf '%s\\n' stamp)suffix\n\n",
            "printf '%s\\n' \"$(print`f '%s\\n' 'a b')\"\n",
            "stamp=$(printf '%s\\n' nowith `$HOME`.\n",
            "[ \"$profile\" = \"~user/.bashrc\" ]\n",
        );

        let _ = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakInTest));
    }
}
