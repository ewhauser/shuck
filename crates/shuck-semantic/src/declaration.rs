use compact_str::CompactString;
use shuck_ast::{Name, Span};

use crate::AssignmentValueOrigin;

/// Declaration builtin that introduced a declaration record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationBuiltin {
    /// `declare`
    Declare,
    /// `local`
    Local,
    /// `export`
    Export,
    /// `readonly`
    Readonly,
    /// `typeset`
    Typeset,
}

/// One declaration command together with its parsed operands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Builtin command that owns the declaration.
    pub builtin: DeclarationBuiltin,
    /// Span of the declaration command.
    pub span: Span,
    /// Ordered operands after the builtin name.
    pub operands: Vec<DeclarationOperand>,
}

/// Operand recorded for a declaration builtin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationOperand {
    /// A flag operand such as `-x`.
    Flag {
        /// Canonical single-character flag represented by the operand.
        flag: char,
        /// Raw flag text, preserving grouped characters.
        flags: CompactString,
        /// Span of the operand.
        span: Span,
    },
    /// A bare name operand.
    Name {
        /// Declared name.
        name: Name,
        /// Span of the operand.
        span: Span,
    },
    /// A name/value assignment operand.
    Assignment {
        /// Assigned name.
        name: Name,
        /// Span of the whole operand.
        operand_span: Span,
        /// Span of the assignment target portion.
        target_span: Span,
        /// Span of the name token.
        name_span: Span,
        /// Span of the value portion.
        value_span: Span,
        /// Whether the operand uses append syntax like `+=`.
        append: bool,
        /// Classification of the assigned value.
        value_origin: AssignmentValueOrigin,
        /// Whether the value contains command substitution syntax.
        has_command_substitution: bool,
        /// Whether the value contains command or process substitution syntax.
        has_command_or_process_substitution: bool,
    },
    /// An operand whose runtime word shape is too dynamic to normalize further.
    DynamicWord {
        /// Span of the dynamic operand.
        span: Span,
    },
}
