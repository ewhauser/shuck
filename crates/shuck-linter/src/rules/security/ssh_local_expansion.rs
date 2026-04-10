use crate::{Checker, Rule, Violation};

pub struct SshLocalExpansion;

impl Violation for SshLocalExpansion {
    fn rule() -> Rule {
        Rule::SshLocalExpansion
    }

    fn message(&self) -> String {
        "ssh command text is expanded locally before the remote shell sees it".to_owned()
    }
}

pub fn ssh_local_expansion(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.options().ssh())
        .flat_map(|fact| fact.local_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SshLocalExpansion);
}
