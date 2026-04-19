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
    pipeline
        .segments()
        .iter()
        .zip(pipeline.operators().iter())
        .filter(|(_, operator)| operator.op() == BinaryOp::Pipe)
        .flat_map(|(segment, _)| {
            let fact = checker.facts().command(segment.command_id());
            fact.redirect_facts()
                .iter()
                .filter_map(|redirect| {
                    stdout_redirect_span_before_pipe(redirect, checker.source()).filter(|_| {
                        !has_independent_stderr_to_stdout_dup(fact.redirect_facts(), redirect)
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn has_independent_stderr_to_stdout_dup(
    redirects: &[crate::RedirectFact<'_>],
    stdout_redirect: &crate::RedirectFact<'_>,
) -> bool {
    redirects.iter().any(|redirect| {
        is_stderr_to_stdout_dup(redirect)
            && !is_synthetic_append_both_dup(redirect, stdout_redirect)
    })
}

fn is_stderr_to_stdout_dup(redirect: &crate::RedirectFact<'_>) -> bool {
    redirect.redirect().kind == RedirectKind::DupOutput
        && redirect.redirect().fd == Some(2)
        && redirect
            .analysis()
            .is_some_and(|analysis| analysis.numeric_descriptor_target == Some(1))
}

fn is_synthetic_append_both_dup(
    dup_redirect: &crate::RedirectFact<'_>,
    stdout_redirect: &crate::RedirectFact<'_>,
) -> bool {
    stdout_redirect.redirect().kind == RedirectKind::Append
        && dup_redirect.redirect().span.start == stdout_redirect.redirect().span.start
}

fn stdout_redirect_span_before_pipe(
    redirect: &crate::RedirectFact<'_>,
    source: &str,
) -> Option<Span> {
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

    redirect
        .analysis()?
        .is_file_target()
        .then(|| redirect_operator_span(redirect, source))
        .flatten()
}

fn redirect_operator_span(redirect: &crate::RedirectFact<'_>, source: &str) -> Option<Span> {
    let target_span = redirect.target_span()?;
    let operator_slice =
        source.get(redirect.redirect().span.start.offset..target_span.start.offset)?;
    let operator_start = operator_slice.find('>')?;
    let operator_end = operator_slice.rfind(|ch: char| !ch.is_whitespace())? + 1;
    let operator_start_pos = redirect
        .redirect()
        .span
        .start
        .advanced_by(&operator_slice[..operator_start]);
    let operator_end_pos = redirect
        .redirect()
        .span
        .start
        .advanced_by(&operator_slice[..operator_end]);

    Some(Span::from_positions(operator_start_pos, operator_end_pos))
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
cmd >>out | next
cmd >|out | next
cmd 1>out | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", ">", ">", ">>", ">|", ">"]
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
cmd >/dev/null 2>&1 | next
cmd 2>&1 1>/dev/null | next
cmd 2>&1 >/dev/null | next
cmd <>file | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_bash_both_redirects_without_explicit_dups() {
        let source = "\
#!/bin/bash
cmd &>out | next
cmd &>>out | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", ">>"]
        );
    }

    #[test]
    fn ignores_stdout_redirects_when_stderr_is_duplicated_to_stdout() {
        let source = "\
#!/bin/sh
cmd >out 2>&1 | next
cmd 2>&1 >out | next
cmd >>out 2>&1 | next
cmd 2>&1 >>out | next
cmd >|out 2>&1 | next
cmd 2>&1 >|out | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn still_reports_unrelated_descriptor_redirects() {
        let source = "\
#!/bin/sh
cmd >out 1>&2 | next
cmd >out 3>&1 | next
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::RedirectBeforePipe));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", ">"]
        );
    }
}
