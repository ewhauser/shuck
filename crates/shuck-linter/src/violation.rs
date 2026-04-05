use crate::Rule;

pub trait Violation: Sized {
    fn rule() -> Rule;

    fn message(&self) -> String;
}
