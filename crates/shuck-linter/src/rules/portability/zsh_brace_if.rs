use crate::{Rule, Violation};

pub struct ZshBraceIf;

impl Violation for ZshBraceIf {
    fn rule() -> Rule {
        Rule::ZshBraceIf
    }

    fn message(&self) -> String {
        "brace-style `if` bodies are zsh-only syntax".to_owned()
    }
}
