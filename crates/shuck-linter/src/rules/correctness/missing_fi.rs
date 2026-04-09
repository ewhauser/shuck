use crate::{Rule, Violation};

pub struct MissingFi;

impl Violation for MissingFi {
    fn rule() -> Rule {
        Rule::MissingFi
    }

    fn message(&self) -> String {
        "this `if` block is missing a closing `fi`".to_owned()
    }
}
