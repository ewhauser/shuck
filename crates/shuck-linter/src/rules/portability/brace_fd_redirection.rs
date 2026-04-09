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
        .filter_map(|redirect| brace_fd_span(redirect.redirect(), checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BraceFdRedirection);
}

fn brace_fd_span(redirect: &shuck_ast::Redirect, source: &str) -> Option<Span> {
    let fd_var_span = redirect.fd_var_span?;
    let brace_close = fd_var_span.end.advanced_by("}");
    let gap = &source[brace_close.offset..redirect.span.start.offset];
    brace_fd_gap_allows_attachment(gap).then_some(fd_var_span)
}

fn brace_fd_gap_allows_attachment(gap: &str) -> bool {
    if gap.is_empty() {
        return true;
    }

    let mut remaining = gap;
    while !remaining.is_empty() {
        if let Some(stripped) = remaining.strip_prefix("\\\r\n") {
            remaining = stripped;
            continue;
        }
        if let Some(stripped) = remaining.strip_prefix("\\\n") {
            remaining = stripped;
            continue;
        }
        return false;
    }

    true
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
exec {contfd}\
<<EOF
hello
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceFdRedirection));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].span.slice(source), "fd");
        assert_eq!(diagnostics[1].span.slice(source), "docfd");
        assert_eq!(diagnostics[2].span.slice(source), "contfd");
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
echo {fd} >/tmp/out
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceFdRedirection));

        assert!(diagnostics.is_empty());
    }
}
