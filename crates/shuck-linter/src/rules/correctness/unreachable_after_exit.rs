use crate::{Checker, Rule, Violation};

pub struct UnreachableAfterExit;

impl Violation for UnreachableAfterExit {
    fn rule() -> Rule {
        Rule::UnreachableAfterExit
    }

    fn message(&self) -> String {
        "code is unreachable".to_owned()
    }
}

pub fn unreachable_after_exit(checker: &mut Checker) {
    for dead_code in checker.semantic().dead_code() {
        for span in &dead_code.unreachable {
            checker.report(UnreachableAfterExit, *span);
        }
    }
}
