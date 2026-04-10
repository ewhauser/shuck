use crate::{Rule, Violation};

pub struct LoopWithoutEnd;

impl Violation for LoopWithoutEnd {
    fn rule() -> Rule {
        Rule::LoopWithoutEnd
    }

    fn message(&self) -> String {
        "this loop is missing a closing `done`".to_owned()
    }
}
