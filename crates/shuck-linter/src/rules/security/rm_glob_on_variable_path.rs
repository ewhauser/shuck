use crate::{Checker, Rule, Violation};

pub struct RmGlobOnVariablePath;

impl Violation for RmGlobOnVariablePath {
    fn rule() -> Rule {
        Rule::RmGlobOnVariablePath
    }

    fn message(&self) -> String {
        "recursive `rm` on a variable path can delete more than intended".to_owned()
    }
}

pub fn rm_glob_on_variable_path(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("rm"))
        .filter_map(|fact| fact.options().rm())
        .flat_map(|rm| rm.dangerous_path_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || RmGlobOnVariablePath);
}
