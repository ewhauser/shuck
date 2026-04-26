use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct MultiVarForLoop;

impl Violation for MultiVarForLoop {
    fn rule() -> Rule {
        Rule::MultiVarForLoop
    }

    fn message(&self) -> String {
        "portable `for` loops bind a single variable per iteration".to_owned()
    }
}

pub fn multi_var_for_loop(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter_map(|fact| fact.target_spans().get(1).copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MultiVarForLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_single_target_for_loops() {
        let source = "#!/bin/sh\nfor key in a b; do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MultiVarForLoop));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_multi_target_loops() {
        let source = "#!/bin/zsh\nfor key val in a 1 b 2; do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MultiVarForLoop).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}
