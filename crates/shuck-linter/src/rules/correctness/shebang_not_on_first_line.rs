use crate::{Checker, Rule, Violation};

pub struct ShebangNotOnFirstLine;

impl Violation for ShebangNotOnFirstLine {
    fn rule() -> Rule {
        Rule::ShebangNotOnFirstLine
    }

    fn message(&self) -> String {
        "move the shebang to the first line of the file".to_owned()
    }
}

pub fn shebang_not_on_first_line(checker: &mut Checker) {
    if let Some(span) = checker.facts().shebang_not_on_first_line_span() {
        checker.report(ShebangNotOnFirstLine, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
