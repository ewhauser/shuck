use shuck_ast::{RedirectKind, Span};

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
        .filter_map(|redirect| {
            let redirect = redirect.redirect();
            (redirect.fd_var.is_some()
                && !matches!(redirect.kind, RedirectKind::HereDoc | RedirectKind::HereDocStrip))
                .then(|| brace_fd_span(redirect))
                .flatten()
        })
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
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceFdRedirection),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "fd");
    }

    #[test]
    fn ignores_brace_fd_redirection_in_bash() {
        let source = "\
#!/bin/bash
exec {fd}>/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceFdRedirection),
        );

        assert!(diagnostics.is_empty());
    }
}
