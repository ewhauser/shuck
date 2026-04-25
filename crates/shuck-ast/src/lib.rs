#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! AST, token, and span types shared across the Shuck workspace.
//!
//! `shuck-parser` produces these data structures, while crates such as `shuck-indexer`,
//! `shuck-linter`, `shuck-semantic`, and `shuck-formatter` consume them.

#[allow(missing_docs)]
mod arena;
#[allow(missing_docs)]
mod arena_ast;
#[allow(missing_docs)]
mod ast;
#[allow(missing_docs)]
mod command_resolution;
#[allow(missing_docs)]
mod name;
#[allow(missing_docs)]
mod span;
#[allow(missing_docs)]
mod tokens;

/// Compact typed arena index and list storage utilities.
pub use arena::{IdRange, Idx, ListArena};
/// ID-backed parsed AST storage and borrowed views.
pub use arena_ast::*;
/// Parsed shell AST nodes and related syntax tree types.
pub use ast::*;
/// Static command-resolution helpers shared by semantic analysis and lint facts.
pub use command_resolution::*;
/// Identifier names used throughout the shell AST.
pub use name::Name;
/// Source positions, spans, and text range utilities.
pub use span::{Position, Span, TextRange, TextSize};
/// Token kinds emitted by the lexer.
pub use tokens::TokenKind;
