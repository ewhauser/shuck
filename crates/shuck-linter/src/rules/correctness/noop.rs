use crate::{Checker, Rule, Violation};

/// Temporary placeholder rule used to stand up the dispatch pipeline.
pub struct NoopPlaceholder;

impl Violation for NoopPlaceholder {
    fn rule() -> Rule {
        Rule::NoopPlaceholder
    }

    fn message(&self) -> String {
        "temporary no-op linter rule".to_owned()
    }
}

pub fn noop(_checker: &mut Checker) {}
