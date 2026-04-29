use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CStyleComment;

impl Violation for CStyleComment {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::CStyleComment
    }

    fn message(&self) -> String {
        "C-style comment syntax is not valid shell syntax".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert `# ` before the C-style comment".to_owned())
    }
}

pub fn c_style_comment(checker: &mut Checker) {
    for index in 0..checker.facts().commands().len() {
        let diagnostic = {
            let Some(command) = checker.facts().commands().get(index) else {
                continue;
            };
            let Some(name) = command.body_name_word() else {
                continue;
            };
            name.span
                .slice(checker.source())
                .starts_with("/*")
                .then(|| {
                    crate::Diagnostic::new(CStyleComment, name.span).with_fix(Fix::unsafe_edit(
                        Edit::insertion(name.span.start.offset, "# "),
                    ))
                })
        };

        if let Some(diagnostic) = diagnostic {
            checker.report_diagnostic(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_c_style_comment_tokens() {
        let source = "#!/bin/sh\n/* note */\n/*compact*/\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleComment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "/*");
        assert_eq!(diagnostics[1].span.slice(source), "/*compact*/");
    }

    #[test]
    fn ignores_quoted_comment_like_text() {
        let source = "#!/bin/sh\necho '/* note */'\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleComment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\n/* note */\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleComment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("insert `# ` before the C-style comment")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_c_style_comment_commands() {
        let source = "#!/bin/sh\n/* note */\n/*compact*/\necho '/* note */'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CStyleComment),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n# /* note */\n# /*compact*/\necho '/* note */'\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C041.sh").as_path(),
            &LinterSettings::for_rule(Rule::CStyleComment),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C041_fix_C041.sh", result);
        Ok(())
    }
}
