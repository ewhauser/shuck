use crate::{Rule, Violation};

pub struct ZshAlwaysBlock;

impl Violation for ZshAlwaysBlock {
    fn rule() -> Rule {
        Rule::ZshAlwaysBlock
    }

    fn message(&self) -> String {
        "`always` blocks are zsh-only syntax".to_owned()
    }
}
