use shuck_ast::{BinaryOp, Command};

use crate::rules::common::query;
use crate::{Checker, Rule, Violation};

pub struct PipeToKill;

impl Violation for PipeToKill {
    fn rule() -> Rule {
        Rule::PipeToKill
    }

    fn message(&self) -> String {
        "piping data into `kill` has no effect".to_owned()
    }
}

pub fn pipe_to_kill(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let Command::Binary(command) = fact.command() else {
                return None;
            };
            if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
                return None;
            }

            let segments = query::pipeline_segments(fact.command())?;
            (segments.len() > 1)
                .then_some(segments.last().copied())
                .flatten()
                .and_then(|segment| checker.facts().command_for_stmt(segment))
                .filter(|fact| fact.effective_name_is("kill"))
                .map(|_| command.span)
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(PipeToKill, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_pipelines_whose_tail_effectively_runs_kill() {
        let source = "\
printf '%s\\n' $$ | kill
printf '%s\\n' $$ | command kill
printf '%s\\n' $$ | exec kill
printf '%s\\n' $$ | cat
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipeToKill));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}
