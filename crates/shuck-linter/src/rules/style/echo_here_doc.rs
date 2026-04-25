use crate::{Checker, Rule, Violation};

pub struct EchoHereDoc;

impl Violation for EchoHereDoc {
    fn rule() -> Rule {
        Rule::EchoHereDoc
    }

    fn message(&self) -> String {
        "here-document input on `echo` is ignored".to_owned()
    }
}

pub fn echo_here_doc(checker: &mut Checker) {
    checker.report_fact_slice_dedup(|facts| facts.echo_here_doc_spans(), || EchoHereDoc);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_heredoc_attached_to_echo() {
        let source = "\
#!/bin/sh
echo <<EOF
hi
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo <<EOF");
    }

    #[test]
    fn reports_tab_stripping_heredoc_attached_to_echo() {
        let source = "\
#!/bin/sh
echo <<-EOF
\thi
\tEOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "echo <<-EOF");
    }

    #[test]
    fn ignores_heredoc_attached_to_non_echo_commands() {
        let source = "\
#!/bin/sh
cat <<EOF
hi
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EchoHereDoc));

        assert!(diagnostics.is_empty());
    }
}
