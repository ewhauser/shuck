use shuck_ast::{BinaryOp, RedirectKind, Span};

use crate::{Checker, PipelineFact, Rule, Violation};

pub struct RedirectBeforePipe;

impl Violation for RedirectBeforePipe {
    fn rule() -> Rule {
        Rule::RedirectBeforePipe
    }

    fn message(&self) -> String {
        "a stdout redirect before a pipe only affects the command on the left".to_owned()
    }
}

pub fn redirect_before_pipe(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| redirect_spans_for_pipeline(checker, pipeline))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RedirectBeforePipe);
}

fn redirect_spans_for_pipeline(checker: &Checker<'_>, pipeline: &PipelineFact<'_>) -> Vec<Span> {
    if pipeline
        .operators()
        .iter()
        .any(|operator| operator.op() == BinaryOp::PipeAll)
    {
        return Vec::new();
    }

    pipeline
        .segments()
        .iter()
        .take(pipeline.segments().len().saturating_sub(1))
        .flat_map(|segment| {
            let fact = checker.facts().command(segment.command_id());
            fact.redirect_facts()
                .iter()
                .enumerate()
                .filter_map(|(index, redirect)| {
                    stdout_redirect_span_before_pipe(redirect).filter(|_| {
                        !has_prior_stderr_to_stdout_dup(&fact.redirect_facts()[..index])
                    })
                })
        })
        .collect()
}

fn has_prior_stderr_to_stdout_dup(redirects: &[crate::RedirectFact<'_>]) -> bool {
    redirects.iter().any(|redirect| {
        redirect.redirect().kind == RedirectKind::DupOutput
            && redirect.redirect().fd == Some(2)
            && redirect
                .analysis()
                .is_some_and(|analysis| analysis.numeric_descriptor_target == Some(1))
    })
}

fn stdout_redirect_span_before_pipe(redirect: &crate::RedirectFact<'_>) -> Option<Span> {
    let data = redirect.redirect();
    if data.fd.unwrap_or(1) != 1 {
        return None;
    }

    if !matches!(
        data.kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::OutputBoth
    ) {
        return None;
    }

    redirect.analysis()?.is_file_target().then_some(data.span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_stdout_redirects_before_plain_pipes() {
        let source = "\
#!/bin/sh
cmd >/dev/null | next
cmd >out | next
left | mid >/dev/null | right
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">/dev/null", ">out", ">/dev/null"]
        );
    }

    #[test]
    fn ignores_stderr_only_descriptor_dups_and_pipeall() {
        let source = "\
#!/bin/sh
2>/dev/null | next
cmd | next >/dev/null
cmd >/dev/null |& next
cmd 1>&2 | next
cmd 2>&1 1>/dev/null | next
cmd <>file | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert!(diagnostics.is_empty());
    }
}
