use crate::{Checker, Rule, Violation};

pub struct ScriptSizeThreshold {
    line_count: usize,
    max_lines: usize,
    mode: LineCountMode,
}

impl Violation for ScriptSizeThreshold {
    fn rule() -> Rule {
        Rule::ScriptSizeThreshold
    }

    fn message(&self) -> String {
        format!(
            "script has {} {} lines, above configured maximum {}",
            self.line_count,
            self.mode.label(),
            self.max_lines
        )
    }
}

pub fn script_size_threshold(checker: &mut Checker) {
    let options = &checker.rule_options().s080;
    let Some(mode) = LineCountMode::from_config(&options.count) else {
        return;
    };
    let line_counts = checker.facts().source_facts().script_line_count();
    let line_count = mode.line_count(line_counts);

    if line_count > options.max_lines {
        checker.report(
            ScriptSizeThreshold {
                line_count,
                max_lines: options.max_lines,
                mode,
            },
            line_counts.report_span(),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineCountMode {
    Physical,
    NonCommentNonBlank,
}

impl LineCountMode {
    fn from_config(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "physical" => Some(Self::Physical),
            "non-comment-non-blank" => Some(Self::NonCommentNonBlank),
            _ => None,
        }
    }

    fn line_count(self, fact: crate::ScriptLineCountFact) -> usize {
        match self {
            Self::Physical => fact.physical_lines(),
            Self::NonCommentNonBlank => fact.non_comment_non_blank_lines(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Physical => "physical",
            Self::NonCommentNonBlank => "non-comment, non-blank",
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_scripts_above_the_configured_physical_line_threshold() {
        let source = "#!/bin/bash\necho one\necho two\necho three\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold).with_s080_max_lines(3),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["#!/bin/bash"]
        );
        assert_eq!(
            diagnostics[0].message,
            "script has 4 physical lines, above configured maximum 3"
        );
    }

    #[test]
    fn accepts_scripts_at_the_configured_threshold() {
        let source = "echo one\necho two\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold).with_s080_max_lines(2),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn does_not_count_a_trailing_newline_as_an_extra_physical_line() {
        let source = "echo one\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold).with_s080_max_lines(1),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn can_count_only_non_comment_non_blank_lines() {
        let source = "\
#!/bin/bash
# file header

echo one
  # section
echo two
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold)
                .with_s080_max_lines(2)
                .with_s080_count("non-comment-non-blank"),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_when_non_comment_non_blank_lines_exceed_threshold() {
        let source = "#!/bin/bash\n# file header\necho one\necho two\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold)
                .with_s080_max_lines(1)
                .with_s080_count("non-comment-non-blank"),
        );

        assert_eq!(
            diagnostics[0].message,
            "script has 2 non-comment, non-blank lines, above configured maximum 1"
        );
    }

    #[test]
    fn counts_hash_lines_inside_heredoc_bodies_as_content() {
        let source = "cat <<'EOF'\n# payload\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ScriptSizeThreshold)
                .with_s080_max_lines(2)
                .with_s080_count("non-comment-non-blank"),
        );

        assert_eq!(
            diagnostics[0].message,
            "script has 3 non-comment, non-blank lines, above configured maximum 2"
        );
    }
}
