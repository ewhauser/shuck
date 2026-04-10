use crate::{Checker, CommandSubstitutionKind, Rule, Violation};

pub struct EchoInsideCommandSubstitution;

impl Violation for EchoInsideCommandSubstitution {
    fn rule() -> Rule {
        Rule::EchoInsideCommandSubstitution
    }

    fn message(&self) -> String {
        "avoid echo inside command substitutions".to_owned()
    }
}

pub fn echo_inside_command_substitution(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .filter(|substitution| {
            substitution.kind() == CommandSubstitutionKind::Command
                && substitution.body_contains_echo()
        })
        .map(|substitution| substitution.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EchoInsideCommandSubstitution);
}
