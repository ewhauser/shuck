use crate::{Checker, Rule, Violation};

pub struct CommentedContinuationLine;

impl Violation for CommentedContinuationLine {
    fn rule() -> Rule {
        Rule::CommentedContinuationLine
    }

    fn message(&self) -> String {
        "line continuation is followed by a comment-only line".to_owned()
    }
}

pub fn commented_continuation_line(checker: &mut Checker) {
    checker.report_all(
        checker
            .facts()
            .commented_continuation_comment_spans()
            .to_vec(),
        || CommentedContinuationLine,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_comment_line_after_continuation() {
        let source = "#!/bin/bash\necho hello \\\n  #world \\\n  foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 3);
        assert_eq!(diagnostics[0].span.slice(source), "#");
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
    fn reports_comment_line_without_its_own_backslash() {
        let source = "#!/bin/bash\necho hello \\\n  #world\n  foo\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommentedContinuationLine),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
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
}
