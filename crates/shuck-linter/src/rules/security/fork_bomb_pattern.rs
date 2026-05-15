use crate::{Checker, Rule, Violation};

pub struct ForkBombPattern;

impl Violation for ForkBombPattern {
    fn rule() -> Rule {
        Rule::ForkBombPattern
    }

    fn message(&self) -> String {
        "function recursively pipes itself in the background".to_owned()
    }
}

pub fn fork_bomb_pattern(checker: &mut Checker) {
    let spans = checker.facts().command_facts().fork_bomb_pattern_spans();
    checker.report_all(spans, || ForkBombPattern);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_backgrounded_recursive_self_pipe() {
        let source = "#!/bin/sh\n:(){ :|:& };:\nboom() { boom | boom & }\nboom\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ForkBombPattern));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![":()", "boom()"]
        );
    }

    #[test]
    fn ignores_non_backgrounded_or_non_recursive_pipelines() {
        let source = "#!/bin/sh\nok() { ok | cat; }\nworker() { left | right & }\nonly_call() { only_call; }\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ForkBombPattern));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
