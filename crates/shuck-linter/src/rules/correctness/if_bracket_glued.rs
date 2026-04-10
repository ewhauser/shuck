use crate::{Rule, Violation};

pub struct IfBracketGlued;

impl Violation for IfBracketGlued {
    fn rule() -> Rule {
        Rule::IfBracketGlued
    }

    fn message(&self) -> String {
        "this `if` keyword is glued to a `[` test".to_owned()
    }
}
