use crate::{Rule, Violation};

pub struct StrayClosingKeyword;

impl Violation for StrayClosingKeyword {
    fn rule() -> Rule {
        Rule::StrayClosingKeyword
    }

    fn message(&self) -> String {
        "this control keyword has no matching opener".to_owned()
    }
}
