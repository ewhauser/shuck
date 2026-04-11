//! Bash parser library.

mod error;
pub mod parser;

pub use error::{Error, Result};
pub use parser::{OptionValue, ShellDialect, ShellProfile, ZshEmulationMode, ZshOptionState};
