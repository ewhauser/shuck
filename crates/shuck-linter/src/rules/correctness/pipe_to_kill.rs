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
        .pipelines()
        .iter()
        .filter(|pipeline| {
            pipeline
                .last_segment()
                .is_some_and(|segment| segment.static_utility_name_is("kill"))
        })
        .map(|pipeline| pipeline.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PipeToKill);
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
