use shuck_ast::{Name, Span};

use crate::ScopeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(pub(crate) u32);

impl ReferenceId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub id: ReferenceId,
    pub name: Name,
    pub kind: ReferenceKind,
    pub scope: ScopeId,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    Expansion,
    ParameterExpansion,
    Length,
    ArrayAccess,
    IndirectExpansion,
    ArithmeticRead,
    ConditionalOperand,
    RequiredRead,
}
