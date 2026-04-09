use shuck_ast::BinaryOp;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct PipeStderrInSh;

impl Violation for PipeStderrInSh {
    fn rule() -> Rule {
        Rule::PipeStderrInSh
    }

    fn message(&self) -> String {
        "the `|&` pipeline operator is not portable in `sh`".to_owned()
    }
}

pub fn pipe_stderr_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .pipelines()
        .iter()
        .filter_map(|pipeline| {
            (pipeline.command().op == BinaryOp::PipeAll).then(|| pipeline.command().op_span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PipeStderrInSh);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_pipe_stderr_in_sh() {
        let source = "\
#!/bin/sh
echo test |& grep -q foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipeStderrInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "|&");
    }

    #[test]
    fn ignores_pipe_stderr_in_bash() {
        let source = "\
#!/bin/bash
echo test |& grep -q foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PipeStderrInSh));

        assert!(diagnostics.is_empty());
    }
}
