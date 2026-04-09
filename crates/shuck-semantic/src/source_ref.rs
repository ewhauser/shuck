use shuck_ast::{Name, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: SourceRefKind,
    pub span: Span,
    pub path_span: Span,
    pub resolution: SourceRefResolution,
    pub explicitly_provided: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefKind {
    Literal(String),
    Directive(String),
    DirectiveDevNull,
    Dynamic,
    SingleVariableStaticTail { variable: Name, tail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRefResolution {
    Unchecked,
    Resolved,
    Unresolved,
}
