#![warn(missing_docs)]

//! AST, token, and span types for parsed bash scripts.

#[allow(missing_docs)]
mod ast;
#[allow(missing_docs)]
mod name;
#[allow(missing_docs)]
mod span;
#[allow(missing_docs)]
mod tokens;

/// Parsed shell AST nodes and related syntax tree types.
pub use ast::*;
/// Identifier names used throughout the shell AST.
pub use name::Name;
/// Source positions, spans, and text range utilities.
pub use span::{Position, Span, TextRange, TextSize};
/// Token kinds emitted by the lexer.
pub use tokens::TokenKind;
