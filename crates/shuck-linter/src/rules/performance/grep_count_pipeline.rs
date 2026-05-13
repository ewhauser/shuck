use shuck_ast::{Span, static_word_text};

use crate::{
    Checker, CommandFactRef, Diagnostic, Edit, Fix, FixAvailability, PipelineFact, Rule, Violation,
};

pub struct GrepCountPipeline;

impl Violation for GrepCountPipeline {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GrepCountPipeline
    }

    fn message(&self) -> String {
        "use `grep -c` instead of piping `grep` into `wc -l`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the pipeline with `grep -c`".to_owned())
    }
}

pub fn grep_count_pipeline(checker: &mut Checker) {
    let reports = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| unsafe_grep_count_pipeline_reports(checker, pipeline))
        .collect::<Vec<_>>();

    for report in reports {
        let diagnostic = Diagnostic::new(GrepCountPipeline, report.diagnostic_span);
        checker.report_diagnostic_dedup(match report.fix {
            Some(fix) => diagnostic.with_fix(fix),
            None => diagnostic,
        });
    }
}

struct GrepCountPipelineReport {
    diagnostic_span: Span,
    fix: Option<Fix>,
}

fn unsafe_grep_count_pipeline_reports(
    checker: &Checker<'_>,
    pipeline: &PipelineFact<'_>,
) -> Vec<GrepCountPipelineReport> {
    pipeline
        .segments()
        .windows(2)
        .zip(pipeline.operators())
        .filter_map(|(pair, operator)| {
            let left_segment = &pair[0];
            let right_segment = &pair[1];
            let left = checker.facts().command(left_segment.command_id());
            let right = checker.facts().command(right_segment.command_id());

            if !is_raw_utility_named(left, "grep") || !is_raw_utility_named(right, "wc") {
                return None;
            }

            if left
                .options()
                .grep()
                .is_some_and(|grep| grep.uses_only_matching)
            {
                return None;
            }

            if !wc_uses_line_count(right, checker.source()) {
                return None;
            }

            Some(GrepCountPipelineReport {
                diagnostic_span: command_body_span(left)?,
                fix: wc_uses_only_line_count(right, checker.source())
                    .then(|| {
                        grep_count_pipeline_fix(checker.source(), left, right, operator.span())
                    })
                    .flatten(),
            })
        })
        .collect()
}

fn grep_count_pipeline_fix(
    source: &str,
    grep: CommandFactRef<'_, '_>,
    wc: CommandFactRef<'_, '_>,
    operator_span: Span,
) -> Option<Fix> {
    let grep_name = grep.body_name_word()?;
    let wc_span = wc.span_in_source(source);
    let delete_start = pipeline_delete_start(source, operator_span.start.offset);
    Some(Fix::unsafe_edits([
        Edit::insertion(grep_name.span.end.offset, " -c"),
        Edit::deletion_at(delete_start, wc_span.end.offset),
    ]))
}

fn command_body_span(fact: CommandFactRef<'_, '_>) -> Option<Span> {
    let body_name = fact.body_name_word()?;
    let mut end = body_name.span.end;

    for word in fact.body_args() {
        if word.span.end.offset > end.offset {
            end = word.span.end;
        }
    }

    for redirect in fact.redirect_facts() {
        let redirect_end = redirect.redirect().span.end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(body_name.span.start, end))
}

fn is_raw_utility_named(fact: CommandFactRef<'_, '_>, name: &str) -> bool {
    fact.literal_name() == Some(name) && fact.wrappers().is_empty()
}

fn wc_uses_line_count(fact: CommandFactRef<'_, '_>, source: &str) -> bool {
    fact.body_args().iter().any(|word| {
        static_word_text(word, source).is_some_and(|text| {
            text == "-l" || text == "--lines" || (text.starts_with('-') && text[1..].contains('l'))
        })
    })
}

fn wc_uses_only_line_count(fact: CommandFactRef<'_, '_>, source: &str) -> bool {
    if !fact.redirect_facts().is_empty() {
        return false;
    }

    let mut args = fact.body_args().iter();
    let Some(arg) = args.next() else {
        return false;
    };
    args.next().is_none()
        && static_word_text(arg, source).is_some_and(|text| text == "-l" || text == "--lines")
}

fn pipeline_delete_start(source: &str, operator_start: usize) -> usize {
    let offset = preceding_space_start(source, operator_start);
    line_continuation_delete_start(source, offset).unwrap_or(offset)
}

fn line_continuation_delete_start(source: &str, offset: usize) -> Option<usize> {
    if offset == 0 || source.as_bytes().get(offset - 1) != Some(&b'\n') {
        return None;
    }

    let backslash = offset.checked_sub(2)?;
    if source.as_bytes().get(backslash) != Some(&b'\\') {
        return None;
    }

    Some(preceding_space_start(source, backslash))
}

fn preceding_space_start(source: &str, mut offset: usize) -> usize {
    while offset > 0 && matches!(source.as_bytes().get(offset - 1), Some(b' ' | b'\t')) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_the_grep_pipeline_segment() {
        let source = "\
#!/bin/sh
grep foo file | wc -l
grep foo file 2>/dev/null | wc --lines
grep -o foo file | wc -l
grep -on foo file | wc -l
grep foo file | grep bar | wc -l
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GrepCountPipeline));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["grep foo file", "grep foo file 2>/dev/null", "grep bar",]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_replace_grep_wc_pipeline_with_grep_count() {
        let source = "\
#!/bin/sh
grep foo file | wc -l
grep foo file 2>/dev/null | wc --lines
grep foo file | grep bar | wc -l
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GrepCountPipeline),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep -c foo file
grep -c foo file 2>/dev/null
grep foo file | grep -c bar
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_multiline_continued_grep_wc_pipeline() {
        let source = "\
#!/bin/sh
grep foo file \\
  | wc -l
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GrepCountPipeline),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
grep -c foo file
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_wc_invocations_with_extra_count_modes_or_operands() {
        let source = "\
#!/bin/sh
grep foo file | wc -cl
grep foo file | wc -l other.txt
grep foo file | wc -l > count
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GrepCountPipeline),
            Applicability::Unsafe,
        );

        assert_eq!(result.diagnostics.len(), 3);
        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 3);
    }
}
