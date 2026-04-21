use crate::{FixAvailability, Rule};

pub trait Violation: Sized {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::None;

    fn rule() -> Rule;

    fn message(&self) -> String;

    fn fix_title(&self) -> Option<String> {
        None
    }
}
