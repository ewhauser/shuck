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
    pub origin: BindingOrigin,
    pub scope: ScopeId,
    pub span: Span,
    pub references: Vec<ReferenceId>,
    pub attributes: BindingAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingOrigin {
    Assignment {
        definition_span: Span,
        value: AssignmentValueOrigin,
    },
    LoopVariable {
        definition_span: Span,
        items: LoopValueOrigin,
    },
    ParameterDefaultAssignment {
        definition_span: Span,
    },
    Imported {
        definition_span: Span,
    },
    FunctionDefinition {
        definition_span: Span,
    },
    BuiltinTarget {
        definition_span: Span,
        kind: BuiltinBindingTargetKind,
    },
    ArithmeticAssignment {
        definition_span: Span,
        target_span: Span,
    },
    Declaration {
        definition_span: Span,
    },
    Nameref {
        definition_span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentValueOrigin {
    PlainScalarAccess,
    StaticLiteral,
    ParameterOperator,
    Transformation,
    IndirectExpansion,
    CommandOrProcessSubstitution,
    MixedDynamic,
    ArrayOrCompound,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopValueOrigin {
    StaticWords,
    ExpandedWords,
    ImplicitArgv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinBindingTargetKind {
    Read,
    Mapfile,
    Printf,
    Getopts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Assignment,
    ParameterDefaultAssignment,
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

pub(crate) fn is_array_like_binding(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
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
        const IMPORTED_POSSIBLE      = 0b0100_0000_0000;
        const IMPORTED_FUNCTION      = 0b1000_0000_0000;
        const EMPTY_INITIALIZER      = 0b0001_0000_0000_0000;
        const IMPORTED_FILE_ENTRY    = 0b0010_0000_0000_0000;
        const SELF_REFERENTIAL_READ  = 0b0100_0000_0000_0000;
    }
}
