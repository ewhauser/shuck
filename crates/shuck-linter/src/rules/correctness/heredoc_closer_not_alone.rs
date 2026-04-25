use crate::{Checker, Rule, Violation};

pub struct HeredocCloserNotAlone;

impl Violation for HeredocCloserNotAlone {
    fn rule() -> Rule {
        Rule::HeredocCloserNotAlone
    }

    fn message(&self) -> String {
        "this here-document closer must be on its own line".to_owned()
    }
}

pub fn heredoc_closer_not_alone(checker: &mut Checker) {
    checker.report_fact_slice_dedup(
        |facts| facts.heredoc_closer_not_alone_spans(),
        || HeredocCloserNotAlone,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_content_prefixed_terminator_lines() {
        let source = "\
#!/bin/sh
cat <<EOF
x EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn reports_content_prefixed_terminators_for_tab_stripped_heredocs() {
        let source = "\
#!/bin/sh
cat <<-EOF
\tx EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "EOF");
    }

    #[test]
    fn ignores_properly_closed_heredocs() {
        let source = "\
#!/bin/sh
cat <<EOF
x
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HeredocCloserNotAlone),
        );

        assert!(diagnostics.is_empty());
    }
}
