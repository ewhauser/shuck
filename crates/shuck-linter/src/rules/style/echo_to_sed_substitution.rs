use crate::{Checker, Rule, ShellDialect, Violation};

pub struct EchoToSedSubstitution;

impl Violation for EchoToSedSubstitution {
    fn rule() -> Rule {
        Rule::EchoToSedSubstitution
    }

    fn message(&self) -> String {
        "prefer a shell rewrite over piping echo into sed for one substitution".to_owned()
    }
}

pub fn echo_to_sed_substitution(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Ksh) {
        return;
    }

    checker.report_all_dedup(
        checker.facts().echo_to_sed_substitution_spans().to_vec(),
        || EchoToSedSubstitution,
    );
}
