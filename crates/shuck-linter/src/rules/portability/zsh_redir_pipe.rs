use shuck_ast::{BackgroundOperator, StmtTerminator};

use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct ZshRedirPipe;

impl Violation for ZshRedirPipe {
    fn rule() -> Rule {
        Rule::ZshRedirPipe
    }

    fn message(&self) -> String {
        "`&|` is zsh-only syntax and is not portable to this shell".to_owned()
    }
}

pub fn zsh_redir_pipe(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| {
            (command.stmt_terminator()
                == Some(StmtTerminator::Background(BackgroundOperator::Pipe)))
            .then_some(command.stmt_terminator_span())
            .flatten()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshRedirPipe);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::test::{test_path, test_snippet};
    use crate::{LinterSettings, Rule, ShellDialect, assert_diagnostics};

    #[test_case(Rule::ZshRedirPipe, Path::new("X036.sh"))]
    fn fixtures(rule: Rule, path: &Path) -> anyhow::Result<()> {
        let snapshot = format!("{}_{}", rule.code(), path.display());
        let (diagnostics, source) = test_path(
            Path::new("portability").join(path).as_path(),
            &LinterSettings::for_rule(rule),
        )?;
        assert_diagnostics!(snapshot, diagnostics, &source);
        Ok(())
    }

    #[test]
    fn ignores_operator_in_zsh_dialect() {
        let source = "#!/bin/zsh\necho hi &|\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshRedirPipe).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_operator_in_mksh_dialect() {
        let source = "#!/bin/mksh\necho hi &|\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshRedirPipe).with_shell(ShellDialect::Mksh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ZshRedirPipe);
    }
}
