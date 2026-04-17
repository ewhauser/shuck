use shuck_ast::Span;

use crate::{Checker, CommandSubstitutionKind, Rule, ShellDialect, Violation};

pub struct LsInSubstitution;

impl Violation for LsInSubstitution {
    fn rule() -> Rule {
        Rule::LsInSubstitution
    }

    fn message(&self) -> String {
        "avoid capturing `ls` output in command substitutions; use a glob or `find` instead"
            .to_owned()
    }
}

pub fn ls_in_substitution(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .flat_map(|substitution| {
            (substitution.kind() == CommandSubstitutionKind::Command
                && substitution.stdout_is_captured())
            .then(|| processed_ls_pipeline_spans(checker, substitution.span()))
            .into_iter()
            .flatten()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LsInSubstitution);
}

fn processed_ls_pipeline_spans(checker: &Checker, substitution_span: Span) -> Vec<Span> {
    checker
        .facts()
        .pipelines()
        .iter()
        .filter(|pipeline| span_contains(substitution_span, pipeline.span()))
        .flat_map(|pipeline| {
            pipeline
                .segments()
                .windows(2)
                .enumerate()
                .filter(|(_, pair)| {
                    left_segment_is_s047_ls_candidate(checker, pair[0].command_id())
                        && !matches!(pair[1].static_utility_name(), Some("grep" | "xargs"))
                })
                .map(|(index, _)| pipeline_ls_command_span(checker, pipeline, index))
        })
        .collect()
}

fn left_segment_is_s047_ls_candidate(
    checker: &Checker,
    command_id: crate::facts::CommandId,
) -> bool {
    let command = checker.facts().command(command_id);

    command.literal_name() == Some("ls")
        && command.wrappers().is_empty()
        && !command_has_leading_glob_operand(checker, command)
}

fn pipeline_ls_command_span(
    checker: &Checker,
    pipeline: &crate::PipelineFact<'_>,
    segment_index: usize,
) -> Span {
    let command = checker
        .facts()
        .command(pipeline.segments()[segment_index].command_id());
    let span = Span {
        start: command.span_in_source(checker.source()).start,
        end: pipeline.operators()[segment_index].span().start,
    };

    trim_trailing_whitespace(span, checker.source())
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn command_has_leading_glob_operand(checker: &Checker, command: &crate::CommandFact<'_>) -> bool {
    command
        .file_operand_words()
        .first()
        .and_then(|word| word.span.slice(checker.source()).chars().next())
        .is_some_and(|ch| matches!(ch, '*' | '?' | '['))
}

fn trim_trailing_whitespace(span: Span, source: &str) -> Span {
    let trimmed = span.slice(source).trim_end();
    Span {
        start: span.start,
        end: span.start.advanced_by(trimmed),
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_processed_ls_command_substitutions() {
        let source = "\
#!/bin/bash
LAYOUTS=\"$(ls layout.*.h | cut -d. -f2 | xargs echo)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsInSubstitution));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "ls layout.*.h");
    }

    #[test]
    fn ignores_wrapped_ls_and_non_command_substitutions() {
        let source = "\
#!/bin/sh
plain=\"$(command ls)\"
quiet=\"$(ls >/dev/null)\"
empty=\"$(printf foo)\"
bare=\"$(ls)\"
grep=\"$(ls | grep foo)\"
escaped_grep=\"$(ls | \\grep foo)\"
wrapped=\"$(command ls /tmp | head -n 1)\"
globbed=\"$(ls *.html | xargs -I% basename % | head -1)\"
xargs_only=\"$(ls /tmp | xargs -n 1 basename)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsInSubstitution));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_ls_pipelines_with_non_grep_consumers() {
        let source = "\
#!/bin/sh
count=\"$(ls | wc -l)\"
legacy=\"$(ls | egrep foo)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsInSubstitution));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls", "ls"]
        );
    }
}
