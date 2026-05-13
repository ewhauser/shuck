use crate::{FixAvailability, Rule, Violation};

pub struct LoopWithoutEnd;

impl Violation for LoopWithoutEnd {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LoopWithoutEnd
    }

    fn message(&self) -> String {
        "this loop is missing a closing `done`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append a closing `done`".to_owned())
    }
}
