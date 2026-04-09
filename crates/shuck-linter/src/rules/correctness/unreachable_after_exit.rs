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
    let unreachable_spans = checker
        .semantic_analysis()
        .dead_code()
        .iter()
        .flat_map(|dead_code| dead_code.unreachable.iter().copied())
        .collect::<Vec<_>>();

    for span in unreachable_spans {
        checker.report(UnreachableAfterExit, span);
    }
}
