use crate::{Rule, Violation};

pub struct DanglingElse;

impl Violation for DanglingElse {
    fn rule() -> Rule {
        Rule::DanglingElse
    }

    fn message(&self) -> String {
        "this `else` branch has no command body".to_owned()
    }
}
