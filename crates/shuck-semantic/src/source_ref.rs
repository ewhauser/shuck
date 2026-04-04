use shuck_ast::{Name, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: SourceRefKind,
    pub span: Span,
    pub path_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefKind {
    Literal(String),
    Directive(String),
    DirectiveDevNull,
    Dynamic,
    SingleVariableStaticTail { variable: Name, tail: String },
}
