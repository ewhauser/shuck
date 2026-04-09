use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BraceFdRedirection;

impl Violation for BraceFdRedirection {
    fn rule() -> Rule {
        Rule::BraceFdRedirection
    }

    fn message(&self) -> String {
        "brace-based file-descriptor redirects are not portable in `sh`".to_owned()
    }
}

pub fn brace_fd_redirection(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter_map(|redirect| brace_fd_span(redirect.redirect()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BraceFdRedirection);
}

fn brace_fd_span(redirect: &shuck_ast::Redirect) -> Option<Span> {
    redirect.fd_var_span
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_brace_fd_redirection_in_sh() {
        let source = "\
#!/bin/sh
exec {fd}>/dev/null
exec {docfd}<<EOF
hello
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceFdRedirection));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "fd");
        assert_eq!(diagnostics[1].span.slice(source), "docfd");
    }

    #[test]
    fn ignores_brace_fd_redirection_in_bash() {
        let source = "\
#!/bin/bash
exec {fd}>/dev/null
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceFdRedirection));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_braced_arguments_before_spaced_heredocs() {
        let source = "\
#!/bin/sh
echo {fd} <<EOF
hello
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceFdRedirection));

        assert!(diagnostics.is_empty());
    }
}
