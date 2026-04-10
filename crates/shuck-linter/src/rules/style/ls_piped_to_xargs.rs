use crate::{Checker, Rule, Violation};

pub struct LsPipedToXargs;

impl Violation for LsPipedToXargs {
    fn rule() -> Rule {
        Rule::LsPipedToXargs
    }

    fn message(&self) -> String {
        "avoid piping `ls` into `xargs`; use globs or `find` instead".to_owned()
    }
}

pub fn ls_piped_to_xargs(checker: &mut Checker) {
    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .flat_map(|pipeline| {
            pipeline.segments().windows(2).filter_map(|pair| {
                let left = checker.facts().command(pair[0].command_id());
                let right = checker.facts().command(pair[1].command_id());

                if !is_raw_command_named(left, "ls") || !is_raw_command_named(right, "xargs") {
                    return None;
                }

                left.body_word_span()
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LsPipedToXargs);
}

fn is_raw_command_named(fact: &crate::CommandFact<'_>, name: &str) -> bool {
    fact.literal_name() == Some(name) && fact.wrappers().is_empty()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_raw_ls_to_xargs_pipelines() {
        let source = "\
ls *.txt | xargs -n1 wc
ls -1A /tmp | xargs wc -l
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsPipedToXargs));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["ls", "ls"]
        );
    }

    #[test]
    fn ignores_wrapped_commands_and_non_xargs_tools() {
        let source = "\
command ls | xargs wc
ls | command xargs wc
ls | grep foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LsPipedToXargs));

        assert!(diagnostics.is_empty());
    }
}
