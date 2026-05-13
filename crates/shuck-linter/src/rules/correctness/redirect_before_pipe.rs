use shuck_ast::{BinaryOp, Position, RedirectKind, Span};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, PipelineFact, Rule, Violation};

pub struct RedirectBeforePipe;

impl Violation for RedirectBeforePipe {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::RedirectBeforePipe
    }

    fn message(&self) -> String {
        "a stdout redirect before a pipe only affects the command on the left".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move the redirect after the pipeline".to_owned())
    }
}

pub fn redirect_before_pipe(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .command_facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| redirect_diagnostics_for_pipeline(checker, pipeline))
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        checker.report_diagnostic_dedup(Diagnostic::new(RedirectBeforePipe, span).with_fix(fix));
    }
}

fn redirect_diagnostics_for_pipeline(
    checker: &Checker<'_>,
    pipeline: &PipelineFact<'_>,
) -> Vec<(Span, Fix)> {
    let source = checker.source();
    pipeline
        .segments()
        .iter()
        .zip(pipeline.operators().iter())
        .filter(|(_, operator)| operator.op() == BinaryOp::Pipe)
        .flat_map(|(segment, _)| {
            let fact = checker
                .facts()
                .command_facts()
                .command(segment.command_id());
            fact.redirect_facts()
                .iter()
                .filter_map(|redirect| {
                    let operator_span =
                        stdout_redirect_span_before_pipe(redirect, source).filter(|_| {
                            !has_independent_stderr_to_stdout_dup(fact.redirect_facts(), redirect)
                        })?;
                    let fix = move_redirect_after_pipeline_fix(redirect, pipeline, source)?;
                    Some((operator_span, fix))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn move_redirect_after_pipeline_fix(
    redirect: &crate::RedirectFact<'_>,
    pipeline: &PipelineFact<'_>,
    source: &str,
) -> Option<Fix> {
    let redirect_span = redirect.redirect().span;
    let delete_span = redirect_deletion_span(redirect_span, source);
    let insertion_offset = trim_trailing_whitespace_offset(
        pipeline.last_segment()?.stmt().span.start.offset,
        pipeline.last_segment()?.stmt().span.end.offset,
        source,
    );
    let insertion = format!(" {}", redirect_span.slice(source).trim());
    Some(Fix::unsafe_edits([
        Edit::deletion(delete_span),
        Edit::insertion(insertion_offset, insertion),
    ]))
}

fn trim_trailing_whitespace_offset(start: usize, mut end: usize, source: &str) -> usize {
    while end > start {
        let previous = source.as_bytes()[end - 1];
        if !previous.is_ascii_whitespace() {
            break;
        }
        end -= 1;
    }
    end
}

fn redirect_deletion_span(span: Span, source: &str) -> Span {
    let mut start = span.start.offset;
    while start > 0 {
        let previous = source.as_bytes()[start - 1];
        if !matches!(previous, b' ' | b'\t') {
            break;
        }
        start -= 1;
    }

    Span::from_positions(
        Position {
            offset: start,
            line: span.start.line,
            column: span
                .start
                .column
                .saturating_sub(span.start.offset.saturating_sub(start)),
        },
        span.end,
    )
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
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn applies_unsafe_fix_by_moving_redirect_after_pipeline() {
        let source = "\
#!/bin/sh
cmd >/dev/null | next
cmd 1>out | next
left | mid >>log | right
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedirectBeforePipe),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
cmd | next >/dev/null
cmd | next 1>out
left | mid | right >>log
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_redirect_before_pipe_unchanged() {
        let source = "#!/bin/sh\ncmd >/dev/null | next\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedirectBeforePipe),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C119.sh").as_path(),
            &LinterSettings::for_rule(Rule::RedirectBeforePipe),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C119_fix_C119.sh", result);
        Ok(())
    }
}
