use crate::{Checker, Rule, Violation};

pub struct LsGrepPipeline;

impl Violation for LsGrepPipeline {
    fn rule() -> Rule {
        Rule::LsGrepPipeline
    }

    fn message(&self) -> String {
        "avoid piping `ls` into `grep`; match files with globs instead".to_owned()
    }
}

pub fn ls_grep_pipeline(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline.segments().windows(2).filter_map(|pair| {
                let left = checker.facts().command(pair[0].command_id());
                let right = checker.facts().command(pair[1].command_id());

                if !is_raw_utility_named(left, "ls") || !is_raw_utility_named(right, "grep") {
                    return None;
                }

                left.body_word_span()
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LsGrepPipeline);
}

fn is_raw_utility_named(fact: &crate::CommandFact<'_>, name: &str) -> bool {
    fact.literal_name() == Some(name) && fact.wrappers().is_empty()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_adjacent_ls_to_grep_segments() {
        let source = "\
ls | grep foo
ls -1A /tmp | grep foo
ls | grep -v foo | wc -l
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsGrepPipeline));

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls", "ls", "ls"]
        );
    }

    #[test]
    fn ignores_wrapped_commands_and_non_grep_tools() {
        let source = "\
command ls | grep foo
ls | command grep foo
ls | egrep foo
ls | fgrep foo
ls | awk '/foo/'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsGrepPipeline));

        assert!(diagnostics.is_empty());
    }
}
