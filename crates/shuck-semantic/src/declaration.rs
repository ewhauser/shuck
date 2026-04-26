use shuck_ast::{Name, Span};

use crate::AssignmentValueOrigin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationBuiltin {
    Declare,
    Local,
    Export,
    Readonly,
    Typeset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub builtin: DeclarationBuiltin,
    pub span: Span,
    pub operands: Vec<DeclarationOperand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationOperand {
    Flag {
        flag: char,
        flags: String,
        span: Span,
    },
    Name {
        name: Name,
        span: Span,
    },
    Assignment {
        name: Name,
        operand_span: Span,
        target_span: Span,
        name_span: Span,
        value_span: Span,
        append: bool,
        value_origin: AssignmentValueOrigin,
        has_command_substitution: bool,
        has_command_or_process_substitution: bool,
    },
    DynamicWord {
        span: Span,
    },
}
