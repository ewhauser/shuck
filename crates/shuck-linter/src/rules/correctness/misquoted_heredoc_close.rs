use crate::{Checker, Rule, Violation};

pub struct MisquotedHeredocClose;

impl Violation for MisquotedHeredocClose {
    fn rule() -> Rule {
        Rule::MisquotedHeredocClose
    }

    fn message(&self) -> String {
        "this here-document closing marker is only a near match".to_owned()
    }
}

pub fn misquoted_heredoc_close(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.misquoted_heredoc_close_spans(),
        || MisquotedHeredocClose,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_close_match_for_quoted_delimiter() {
        let source = "\
#!/bin/bash
cat <<'BLOCK'
x
'BLOCK'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn ignores_content_plus_delimiter_lines_to_avoid_c144_overlap() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_properly_closed_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MisquotedHeredocClose),
        );

        assert!(diagnostics.is_empty());
    }
}
