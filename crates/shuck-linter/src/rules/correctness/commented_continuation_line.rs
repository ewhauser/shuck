use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CommentedContinuationLine;

impl Violation for CommentedContinuationLine {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::CommentedContinuationLine
    }

    fn message(&self) -> String {
        "line continuation is followed by a comment-only line".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the trailing `\\` from the comment-only line".to_owned())
    }
}

pub fn commented_continuation_line(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commented_continuation_comment_spans()
        .to_vec();
    for span in spans {
        let backslash_offset = span
            .start
            .offset
            .checked_sub(1)
            .expect("commented continuation anchors should follow a trailing backslash");
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(CommentedContinuationLine, span).with_fix(Fix::unsafe_edit(
                Edit::deletion_at(backslash_offset, span.start.offset),
            )),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_comment_line_after_continuation() {
        let source = "#!/bin/bash\necho hello \\\n  #world \\\n  foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 11);
        assert_eq!(diagnostics[0].span.start, diagnostics[0].span.end);
        assert_eq!(
            &source[diagnostics[0].span.start.offset - 1..diagnostics[0].span.start.offset],
            "\\"
        );
    }

    #[test]
    fn exposes_unsafe_fix_metadata_for_commented_continuation_lines() {
        let source = "#!/bin/bash\necho hello \\\n  #world \\\n  foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("remove the trailing `\\` from the comment-only line")
        );
    }

    #[test]
    fn ignores_regular_continuation_lines() {
        let source = "#!/bin/bash\necho hello \\\n  world\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_hash_on_non_continuation_lines() {
        let source = "#!/bin/bash\necho hello\n  #world\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_comment_line_without_its_own_backslash() {
        let source = "#!/bin/bash\necho hello \\\n  #world\n  foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_commented_continuation_lines() {
        let source = "#!/bin/bash\necho hello \\\n  #world \\\n  foo\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\necho hello \\\n  #world \n  foo\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_comment_breaks_in_conditional_chains() {
        let source = "#!/bin/bash\ncommand -v brew && \\\n  # allow stubbed brew in tests\n  brew --prefix\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_comment_breaks_in_conditional_chains_unchanged_when_fixing() {
        let source = "#!/bin/bash\ncommand -v brew && \\\n  # allow stubbed brew in tests\n  brew --prefix\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C076.sh").as_path(),
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C076_fix_C076.sh", result);
        Ok(())
    }
}
