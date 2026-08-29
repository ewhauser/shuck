use crate::{FixAvailability, Rule};

/// Rule-specific payload used to create a [`crate::Diagnostic`].
pub trait Violation: Sized {
    /// Whether diagnostics created from this violation type can offer fixes.
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::None;

    /// Returns the rule associated with this violation type.
    fn rule() -> Rule;

    /// Formats the diagnostic message for this violation.
    fn message(&self) -> String;

    /// Returns a human-readable fix label when the violation supports one.
    fn fix_title(&self) -> Option<String> {
        None
    }
}
