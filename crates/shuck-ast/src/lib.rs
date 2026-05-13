#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! AST, token, and span types shared across the Shuck workspace.
//!
//! `shuck-parser` produces these data structures, while crates such as `shuck-indexer`,
//! `shuck-linter`, `shuck-semantic`, and `shuck-formatter` consume them.

#[doc(hidden)]
mod arena;
#[doc(hidden)]
mod ast;
#[doc(hidden)]
mod command_resolution;
#[doc(hidden)]
mod name;
#[doc(hidden)]
mod span;
#[doc(hidden)]
mod tokens;

/// Compact typed arena index and list storage utilities.
pub use arena::{IdRange, Idx, ListArena};
#[doc(hidden)]
pub use ast::*;
#[doc(hidden)]
pub use command_resolution::*;
/// Identifier names used throughout the shell AST.
pub use name::Name;
/// Source positions, spans, and text range utilities.
pub use span::{Position, Span, TextRange, TextSize};
/// Token kinds emitted by the lexer.
pub use tokens::TokenKind;
