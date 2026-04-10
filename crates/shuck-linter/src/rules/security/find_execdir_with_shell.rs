use crate::{Checker, Rule, Violation};

pub struct FindExecDirWithShell;

impl Violation for FindExecDirWithShell {
    fn rule() -> Rule {
        Rule::FindExecDirWithShell
    }

    fn message(&self) -> String {
        "shell command text passed through `find -execdir` can inject filenames".to_owned()
    }
}

pub fn find_execdir_with_shell(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.options().find_execdir())
        .flat_map(|fact| fact.shell_command_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FindExecDirWithShell);
}
