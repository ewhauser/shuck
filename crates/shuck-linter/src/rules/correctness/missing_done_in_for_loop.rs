use crate::{Rule, Violation};

pub struct MissingDoneInForLoop;

impl Violation for MissingDoneInForLoop {
    fn rule() -> Rule {
        Rule::MissingDoneInForLoop
    }

    fn message(&self) -> String {
        "this `for` loop is missing a closing `done`".to_owned()
    }
}
