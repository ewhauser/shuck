#![warn(missing_docs)]

//! Bash parser library.

#[allow(missing_docs)]
mod error;
/// Parser entry points and low-level parser data structures.
#[allow(missing_docs)]
pub mod parser;

/// Error types returned by parser operations.
pub use error::{Error, Result};
/// Shell dialect, profile, and option types exposed by the parser.
pub use parser::{OptionValue, ShellDialect, ShellProfile, ZshEmulationMode, ZshOptionState};
