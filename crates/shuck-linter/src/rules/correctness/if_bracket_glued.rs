use crate::{FixAvailability, Rule, Violation};

pub struct IfBracketGlued;

impl Violation for IfBracketGlued {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IfBracketGlued
    }

    fn message(&self) -> String {
        "this `if` keyword is glued to a `[` test".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("insert a space after `if`".to_owned())
    }
}
