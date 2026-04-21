use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct ShebangNotOnFirstLine;

impl Violation for ShebangNotOnFirstLine {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ShebangNotOnFirstLine
    }

    fn message(&self) -> String {
        "move the shebang to the first line of the file".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move the shebang line to the top of the file".to_owned())
    }
}

pub fn shebang_not_on_first_line(checker: &mut Checker) {
    let source = checker.source();
    if let Some((span, fix_span)) = checker
        .facts()
        .shebang_not_on_first_line_span()
        .zip(checker.facts().shebang_not_on_first_line_fix_span())
    {
        let preferred_newline = checker
            .facts()
            .shebang_not_on_first_line_preferred_newline()
            .unwrap_or("\n");
        let moved_line = moved_shebang_line(fix_span.slice(source), preferred_newline);
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(ShebangNotOnFirstLine, span).with_fix(Fix::unsafe_edits([
                Edit::insertion(0, moved_line),
                Edit::deletion(fix_span),
            ])),
        );
    }
}

fn moved_shebang_line(shebang_line: &str, preferred_newline: &str) -> String {
    if shebang_line.ends_with('\n') {
        shebang_line.to_owned()
    } else {
        format!("{shebang_line}{preferred_newline}")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_shebang_on_second_line() {
        let source = "\n#!/bin/sh\n:\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn reports_second_line_shebang_after_other_prelude_text() {
        let source = "# comment\n#!/bin/sh\n:\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn reports_shebang_after_multiple_header_lines() {
        let source = "# comment\n\n#!/bin/sh\n:\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.end.column, 1);
    }

    #[test]
    fn ignores_non_header_second_line_shebangs() {
        for source in ["echo hi\n#!/bin/sh\n:\n", "cat <<EOF\n#!/bin/sh\nEOF\n"] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
            );
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn exposes_unsafe_fix_metadata_for_non_first_line_shebangs() {
        let source = "# comment\n#!/bin/sh\n:\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("move the shebang line to the top of the file")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_shebang_after_header_comment() {
        let source = "# comment\n#!/bin/sh\n:\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n# comment\n:\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn adds_a_newline_when_moving_an_eof_shebang_to_the_top() {
        let source = "# comment\r\n#!/bin/sh";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\r\n# comment\r\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_first_line_or_non_header_later_shebangs() {
        for source in [
            "#!/bin/sh\n:\n",
            "echo hi\n#!/bin/sh\n:\n",
            "cat <<EOF\n#!/bin/sh\nEOF\n",
        ] {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
            );
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn leaves_non_header_later_shebangs_unchanged_when_fixing() {
        for source in ["echo hi\n#!/bin/sh\n:\n", "cat <<EOF\n#!/bin/sh\nEOF\n"] {
            let result = test_snippet_with_fix(
                source,
                &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
                Applicability::Unsafe,
            );

            assert_eq!(result.fixes_applied, 0);
            assert_eq!(result.fixed_source, source);
            assert!(result.fixed_diagnostics.is_empty());
        }
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C075.sh").as_path(),
            &LinterSettings::for_rule(Rule::ShebangNotOnFirstLine),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C075_fix_C075.sh", result);
        Ok(())
    }
}
