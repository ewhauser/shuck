use crate::{Checker, Rule, Violation};

use super::unchecked_directory_change::unchecked_directory_change_in_function_spans;

pub struct UncheckedDirectoryChangeInFunction {
    pub command: &'static str,
}

impl Violation for UncheckedDirectoryChangeInFunction {
    fn rule() -> Rule {
        Rule::UncheckedDirectoryChangeInFunction
    }

    fn message(&self) -> String {
        format!(
            "`{}` inside a function should check whether the directory change succeeded",
            self.command
        )
    }
}

pub fn unchecked_directory_change_in_function(checker: &mut Checker) {
    for (command, span) in unchecked_directory_change_in_function_spans(checker) {
        checker.report(UncheckedDirectoryChangeInFunction { command }, span);
    }
}
