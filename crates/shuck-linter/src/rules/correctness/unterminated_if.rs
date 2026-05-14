use crate::{Rule, Violation};

pub struct UnterminatedIf;

impl Violation for UnterminatedIf {
    fn rule() -> Rule {
        Rule::UnterminatedIf
    }

    fn message(&self) -> String {
        "this `if` block is not closed with `fi`".to_owned()
    }
}
