use crate::{Rule, Violation};

pub struct UntilMissingDo;

impl Violation for UntilMissingDo {
    fn rule() -> Rule {
        Rule::UntilMissingDo
    }

    fn message(&self) -> String {
        "this `until` loop is missing `do` before its body".to_owned()
    }
}
