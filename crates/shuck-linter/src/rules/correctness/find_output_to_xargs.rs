use shuck_ast::{Command, Span, Stmt};

use crate::rules::common::query;
use crate::{Checker, CommandFact, Rule, Violation};

pub struct FindOutputToXargs;

impl Violation for FindOutputToXargs {
    fn rule() -> Rule {
        Rule::FindOutputToXargs
    }

    fn message(&self) -> String {
        "raw `find` output piped to `xargs` can break on whitespace".to_owned()
    }
}

pub fn find_output_to_xargs(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| query::pipeline_segments(fact.command()))
        .flat_map(|pipeline| unsafe_find_to_xargs_spans(checker, &pipeline))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(FindOutputToXargs, span);
    }
}

fn unsafe_find_to_xargs_spans(checker: &Checker<'_>, pipeline: &[&Stmt]) -> Vec<Span> {
    pipeline
        .windows(2)
        .filter_map(|pair| {
            let left = checker.facts().command_for_stmt(pair[0])?;
            let right = checker.facts().command_for_stmt(pair[1])?;

            if !left.effective_name_is("find") || !right.effective_name_is("xargs") {
                return None;
            }

            if left.options().find().is_some_and(|find| find.has_print0)
                && right
                    .options()
                    .xargs()
                    .is_some_and(|xargs| xargs.uses_null_input)
            {
                return None;
            }

            Some(find_command_span(pair[0], left))
        })
        .collect()
}

fn find_command_span(command: &Stmt, fact: &CommandFact<'_>) -> Span {
    match &command.command {
        Command::Simple(simple) => {
            let end = command
                .redirects
                .last()
                .map(|redirect| redirect.span.end)
                .or_else(|| simple.args.last().map(|word| word.span.end))
                .unwrap_or(simple.name.span.end);
            Span::from_positions(fact.body_span().start, end)
        }
        _ => fact.body_span(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_effective_find_command_name() {
        let source = "command find . -type f | xargs wc -l\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
    }

    #[test]
    fn anchors_on_multiline_find_segment_before_pipe() {
        let source = "find . -type f \\\n  -name '*.txt' | xargs rm\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "find . -type f \\\n  -name '*.txt'"
        );
    }

    #[test]
    fn accepts_null_delimited_find_xargs_pairs_and_reports_wrapped_find() {
        let source = "\
find . -type f -print0 | xargs -0 rm
command find . -type f | xargs rm
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
    }
}
