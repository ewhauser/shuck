use crate::{Checker, Rule, Violation};

pub struct HeredocEndSpace;

impl Violation for HeredocEndSpace {
    fn rule() -> Rule {
        Rule::HeredocEndSpace
    }

    fn message(&self) -> String {
        "remove trailing whitespace after this here-document terminator".to_owned()
    }
}

pub fn heredoc_end_space(checker: &mut Checker) {
    checker.report_fact_slice_dedup(|facts| facts.heredoc_end_space_spans(), || HeredocEndSpace);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_trailing_space_after_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.line, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn reports_trailing_tab_after_heredoc_terminator() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF\t\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn reports_trailing_space_for_tab_stripped_heredoc_terminator() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tvalue
\tEOF 
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 5);
        assert_eq!(diagnostics[0].span.end.column, 5);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn anchors_at_the_first_trailing_whitespace_character() {
        let source = "#!/bin/sh\ncat <<EOF\nok\nEOF  \t\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.line, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }

    #[test]
    fn ignores_properly_terminated_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
ok
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_the_first_bad_terminator_per_heredoc() {
        let source = "\
#!/bin/sh
cat <<EOF
value
EOF 
EOF\t
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HeredocEndSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.start.column, 4);
        assert_eq!(diagnostics[0].span.end.column, 4);
        assert_eq!(diagnostics[0].span.slice(source), "");
    }
}
