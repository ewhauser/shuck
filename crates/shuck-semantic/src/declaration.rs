use shuck_ast::{Name, Span};

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
    Flag { flag: char, span: Span },
    Name { name: Name, span: Span },
    Assignment {
        name: Name,
        name_span: Span,
        value_span: Span,
        append: bool,
    },
    DynamicWord { span: Span },
}
