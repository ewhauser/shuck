use bitflags::bitflags;
use shuck_ast::{Name, Span};

use crate::{DeclarationBuiltin, ReferenceId, ScopeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub(crate) u32);

impl BindingId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub id: BindingId,
    pub name: Name,
    pub kind: BindingKind,
    pub scope: ScopeId,
    pub span: Span,
    pub references: Vec<ReferenceId>,
    pub attributes: BindingAttributes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Assignment,
    AppendAssignment,
    ArrayAssignment,
    Declaration(DeclarationBuiltin),
    FunctionDefinition,
    LoopVariable,
    ReadTarget,
    MapfileTarget,
    PrintfTarget,
    GetoptsTarget,
    ArithmeticAssignment,
    Nameref,
    Imported,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BindingAttributes: u16 {
        const EXPORTED               = 0b0000_0001;
        const READONLY               = 0b0000_0010;
        const LOCAL                  = 0b0000_0100;
        const INTEGER                = 0b0000_1000;
        const ARRAY                  = 0b0001_0000;
        const ASSOC                  = 0b0010_0000;
        const NAMEREF                = 0b0100_0000;
        const LOWERCASE              = 0b1000_0000;
        const UPPERCASE              = 0b0001_0000_0000;
        const DECLARATION_INITIALIZED = 0b0010_0000_0000;
    }
}
