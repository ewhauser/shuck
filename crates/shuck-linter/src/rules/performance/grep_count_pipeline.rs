use shuck_ast::Span;

use crate::{Checker, CommandFactRef, PipelineFact, Rule, Violation};

pub struct GrepCountPipeline;

impl Violation for GrepCountPipeline {
    fn rule() -> Rule {
        Rule::GrepCountPipeline
    }

    fn message(&self) -> String {
        "use `grep -c` instead of piping `grep` into `wc -l`".to_owned()
    }
}

pub fn grep_count_pipeline(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| unsafe_grep_count_pipeline_spans(checker, pipeline))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GrepCountPipeline);
}

fn unsafe_grep_count_pipeline_spans(checker: &Checker<'_>, pipeline: &PipelineFact) -> Vec<Span> {
    pipeline
        .segments()
        .windows(2)
        .filter_map(|pair| {
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

            command_body_span(left, checker.source())
        })
        .collect()
}

fn command_body_span(fact: CommandFactRef<'_, '_>, source: &str) -> Option<Span> {
    let body_name = fact.arena_body_name_word(source)?;
    let mut end = body_name.span().end;

    for word in fact.arena_body_args(source) {
        if word.span().end.offset > end.offset {
            end = word.span().end;
        }
    }

    for redirect in fact.redirect_facts() {
        let redirect_end = redirect.span().end;
        if redirect_end.offset > end.offset {
            end = redirect_end;
        }
    }

    Some(Span::from_positions(body_name.span().start, end))
}

fn is_raw_utility_named(fact: CommandFactRef<'_, '_>, name: &str) -> bool {
    fact.literal_name() == Some(name) && fact.wrappers().is_empty()
}

fn wc_uses_line_count(fact: CommandFactRef<'_, '_>, source: &str) -> bool {
    fact.arena_body_args(source).iter().any(|word| {
        word.static_text(source).is_some_and(|text| {
            text == "-l" || text == "--lines" || (text.starts_with('-') && text[1..].contains('l'))
        })
    })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
