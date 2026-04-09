use crate::{Checker, Rule, ShellDialect, Violation};

pub struct LetCommand;

impl Violation for LetCommand {
    fn rule() -> Rule {
        Rule::LetCommand
    }

    fn message(&self) -> String {
        "`let` is not portable in `sh` scripts".to_owned()
    }
}

pub fn let_command(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("let"))
        .map(|fact| fact.body_span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LetCommand);
}
