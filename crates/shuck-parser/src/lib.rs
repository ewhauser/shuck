#![warn(missing_docs)]

//! Shell lexer and parser APIs for the Shuck workspace.
//!
//! `shuck-parser` turns shell source text into `shuck-ast` syntax trees and also exposes a
//! source-backed lexer for lower-level tooling.

#[allow(missing_docs)]
mod error;
#[allow(missing_docs)]
/// Parsing entrypoints, lexer types, and shell-profile configuration.
pub mod parser;

/// Error types returned by parser operations.
pub use error::{Error, Result};
/// Shell dialect, profile, and option types exposed by the parser.
pub use parser::{
    OptionValue, ShellDialect, ShellProfile, ZshEmulationMode, ZshOptionState,
    text_is_self_contained_arithmetic_expression, text_looks_like_nontrivial_arithmetic_expression,
};
