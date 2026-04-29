use shuck_ast::{Name, Span};

use crate::ScopeId;

/// Stable identifier for a semantic reference recorded in a [`crate::SemanticModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(pub(crate) u32);

impl ReferenceId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// One semantic read-like use of a shell name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Unique identifier for this reference.
    pub id: ReferenceId,
    /// Referenced shell name.
    pub name: Name,
    /// Semantic category for the use.
    pub kind: ReferenceKind,
    /// Scope active at the use site.
    pub scope: ScopeId,
    /// Source span of the use.
    pub span: Span,
}

/// Semantic category for a shell name use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// The declared name operand in a command such as `local foo`.
    DeclarationName,
    /// A normal parameter expansion such as `$foo` or `${foo}`.
    Expansion,
    /// A read modeled implicitly by semantic analysis rather than direct syntax.
    ImplicitRead,
    /// A parameter expansion that participates in operator semantics.
    ParameterExpansion,
    /// A parameter-length expansion such as `${#foo}`.
    Length,
    /// An array element or slice access.
    ArrayAccess,
    /// An indirect expansion such as `${!name}`.
    IndirectExpansion,
    /// A read inside arithmetic evaluation.
    ArithmeticRead,
    /// A prompt-string expansion.
    PromptExpansion,
    /// A variable use inside a trap action.
    TrapAction,
    /// A parameter use inside a pattern operator such as `${x%$pat}`.
    ParameterPattern,
    /// A variable use inside arithmetic that computes parameter slicing bounds.
    ParameterSliceArithmetic,
    /// A use inside a conditional command operand.
    ConditionalOperand,
    /// A required runtime read introduced by contracts or semantic modeling.
    RequiredRead,
}
