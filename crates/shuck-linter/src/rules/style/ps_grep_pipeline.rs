use shuck_ast::Span;

use crate::{Checker, CommandFactRef, PipelineFact, Rule, Violation};

pub struct PsGrepPipeline;

impl Violation for PsGrepPipeline {
    fn rule() -> Rule {
        Rule::PsGrepPipeline
    }

    fn message(&self) -> String {
        "prefer `pgrep` over piping `ps` into `grep`".to_owned()
    }
}

pub fn ps_grep_pipeline(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| unsafe_ps_grep_pipeline_spans(checker, pipeline))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PsGrepPipeline);
}

fn unsafe_ps_grep_pipeline_spans(checker: &Checker<'_>, pipeline: &PipelineFact) -> Vec<Span> {
    pipeline
        .segments()
        .windows(2)
        .filter_map(|pair| {
            let left = checker.facts().command(pair[0].command_id());
            let right = checker.facts().command(pair[1].command_id());

            if !is_raw_utility_named(left, "ps") || !is_raw_utility_named(right, "grep") {
                return None;
            }

            if left.options().ps().is_some_and(|ps| ps.has_pid_selector) {
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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_adjacent_ps_to_grep_segments() {
        let source = "\
ps aux | grep foo
ps aux | grep -v grep
ps aux | grep foo | awk '{print $1}'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PsGrepPipeline));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ps aux", "ps aux", "ps aux"]
        );
    }

    #[test]
    fn ignores_wrapped_commands_and_non_grep_tools() {
        let source = "\
command ps aux | grep foo
ps aux | command grep foo
ps aux | egrep foo
ps aux | fgrep foo
ps aux | awk '/foo/'
ps aux -p 1 -o comm= | grep -q systemd
ps ax -q 1 -o comm= | grep -q systemd
ps -p 1 -o comm= | grep -q systemd
ps p 123 -o comm= | grep -q systemd
ps 1 | grep -q systemd
ps 1,2 | grep -q systemd
ps -o command= -p \"$parent\" | grep -F -- \"-f\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PsGrepPipeline));

        assert!(diagnostics.is_empty());
    }
}
