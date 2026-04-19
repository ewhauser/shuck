use rustc_hash::FxHashSet;
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
    let pipeline_producer_command_ids = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline
                .segments()
                .split_last()
                .into_iter()
                .flat_map(|(_, producers)| producers.iter().map(|segment| segment.command_id()))
        })
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| !pipeline_producer_command_ids.contains(&fact.id()))
        .flat_map(|fact| {
            let redirects = fact.redirect_facts();
            redirects
                .iter()
                .enumerate()
                .filter_map(move |(index, redirect)| {
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

    #[test]
    fn ignores_pipeline_producers_but_keeps_pipeline_tail_reports() {
        let source = "\
#!/bin/sh
foo 2>&1 >/dev/null | sed 's/x/y/'
echo ok | foo 2>&1 >/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StderrBeforeStdoutRedirect),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }
}
