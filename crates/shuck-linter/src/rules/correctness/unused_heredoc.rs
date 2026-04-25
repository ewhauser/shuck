use crate::{Checker, Rule, Violation};

pub struct UnusedHeredoc;

impl Violation for UnusedHeredoc {
    fn rule() -> Rule {
        Rule::UnusedHeredoc
    }

    fn message(&self) -> String {
        "this here-document has no command to consume it".to_owned()
    }
}

pub fn unused_heredoc(checker: &mut Checker) {
    checker.report_fact_slice_dedup(|facts| facts.unused_heredoc_spans(), || UnusedHeredoc);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_heredocs_without_a_consuming_command() {
        let source = "\
#!/bin/sh
<<EOF
alpha
EOF

x=1 <<EOF
beta
EOF

>out <<EOF
gamma
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedHeredoc));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 6, 10]
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["<<EOF", "<<EOF", "<<EOF"]
        );
    }

    #[test]
    fn ignores_heredocs_attached_to_commands() {
        let source = "\
#!/bin/sh
cat <<EOF
alpha
EOF

: <<EOF
beta
EOF

\"\" <<EOF
gamma
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedHeredoc));

        assert!(diagnostics.is_empty());
    }
}
