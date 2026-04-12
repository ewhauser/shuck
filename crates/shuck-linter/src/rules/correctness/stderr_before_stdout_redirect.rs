use shuck_ast::RedirectKind;

use crate::{Checker, RedirectFact, Rule, Violation};

pub struct StderrBeforeStdoutRedirect;

impl Violation for StderrBeforeStdoutRedirect {
    fn rule() -> Rule {
        Rule::StderrBeforeStdoutRedirect
    }

    fn message(&self) -> String {
        "stderr is redirected before stdout is redirected".to_owned()
    }
}

pub fn stderr_before_stdout_redirect(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .flat_map(|fact| {
            let redirects = fact.redirect_facts();
            redirects.iter().enumerate().filter_map(move |(index, redirect)| {
                if !is_stderr_to_stdout_redirect(redirect) {
                    return None;
                }
                has_later_stdout_file_redirect(&redirects[index + 1..])
                    .then_some(redirect.redirect().span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || StderrBeforeStdoutRedirect);
}

fn is_stderr_to_stdout_redirect(redirect: &RedirectFact<'_>) -> bool {
    let Some(analysis) = redirect.analysis() else {
        return false;
    };

    redirect.redirect().kind == RedirectKind::DupOutput
        && redirect.redirect().fd == Some(2)
        && analysis.numeric_descriptor_target == Some(1)
}

fn has_later_stdout_file_redirect(redirects: &[RedirectFact<'_>]) -> bool {
    redirects.iter().any(|redirect| {
        let data = redirect.redirect();
        data.fd.unwrap_or(1) == 1
            && matches!(
                data.kind,
                RedirectKind::Output | RedirectKind::Clobber | RedirectKind::Append
            )
    })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_stdout_redirects_in_structural_commands_only() {
        let source = "\
#!/bin/sh
foo 2>&1 >/dev/null
out=$(bar 2>&1 >/dev/null)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }
}
