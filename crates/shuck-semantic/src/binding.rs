use bitflags::bitflags;
use shuck_ast::{Name, Span};

use crate::{DeclarationBuiltin, ReferenceId, ScopeId};

/// Stable identifier for a semantic binding recorded in a [`crate::SemanticModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(pub(crate) u32);

impl BindingId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// One semantic name definition discovered in the analyzed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Unique identifier for this binding.
    pub id: BindingId,
    /// Bound shell name.
    pub name: Name,
    /// High-level binding category.
    pub kind: BindingKind,
    /// Provenance details describing how the binding was introduced.
    pub origin: BindingOrigin,
    /// Scope that owns the binding.
    pub scope: ScopeId,
    /// Source span that defines or introduces the binding.
    pub span: Span,
    /// References that resolved to this binding.
    pub references: Vec<ReferenceId>,
    /// Attribute flags inferred for the binding.
    pub attributes: BindingAttributes,
}

/// Provenance details for how a binding entered the semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingOrigin {
    /// A normal assignment form such as `foo=bar`.
    Assignment {
        /// Span of the definition site.
        definition_span: Span,
        /// Classification of the assigned value.
        value: AssignmentValueOrigin,
    },
    /// A loop variable such as `for foo in ...`.
    LoopVariable {
        /// Span of the variable definition site.
        definition_span: Span,
        /// Classification of the loop item source.
        items: LoopValueOrigin,
    },
    /// An assignment produced by a parameter operator such as `${x:=value}`.
    ParameterDefaultAssignment {
        /// Span of the operator site that introduces the binding.
        definition_span: Span,
    },
    /// A binding imported from a sourced file or ambient contract.
    Imported {
        /// Span associated with the import site.
        definition_span: Span,
    },
    /// A function definition.
    FunctionDefinition {
        /// Span of the function definition.
        definition_span: Span,
    },
    /// A builtin target such as `read foo` or `mapfile arr`.
    BuiltinTarget {
        /// Span of the target definition site.
        definition_span: Span,
        /// Builtin command that created the target.
        kind: BuiltinBindingTargetKind,
    },
    /// An arithmetic assignment such as `(( foo += 1 ))`.
    ArithmeticAssignment {
        /// Span of the full arithmetic assignment.
        definition_span: Span,
        /// Span of the assignment target within the expression.
        target_span: Span,
    },
    /// A declaration builtin operand such as `local foo`.
    Declaration {
        /// Span of the declaration operand.
        definition_span: Span,
    },
    /// A nameref declaration such as `declare -n ref=target`.
    Nameref {
        /// Span of the nameref definition site.
        definition_span: Span,
    },
}

/// Coarse classification for the right-hand side of an assignment-like binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentValueOrigin {
    /// A plain scalar read or concatenation of scalar reads.
    PlainScalarAccess,
    /// A purely static literal value.
    StaticLiteral,
    /// A parameter operator result such as `${x:-fallback}`.
    ParameterOperator,
    /// A transformation that preserves variable identity in a rule-relevant way.
    Transformation,
    /// An indirect expansion such as `${!name}`.
    IndirectExpansion,
    /// A command or process substitution.
    CommandOrProcessSubstitution,
    /// A mixed or partially dynamic value that does not fit a narrower category.
    MixedDynamic,
    /// An array literal, array expansion, or compound assignment.
    ArrayOrCompound,
    /// The origin could not be classified precisely.
    Unknown,
}

/// Coarse classification for the item source of a loop variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopValueOrigin {
    /// The loop iterates a statically known word list.
    StaticWords,
    /// The loop iterates words produced by expansion.
    ExpandedWords,
    /// The loop iterates the implicit positional parameter list.
    ImplicitArgv,
}

/// Builtin command kind that creates a target binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinBindingTargetKind {
    /// `read`
    Read,
    /// `mapfile` or `readarray`
    Mapfile,
    /// `printf -v`
    Printf,
    /// `getopts`
    Getopts,
    /// `zparseopts`
    Zparseopts,
    /// `zstyle -a`, `zstyle -s`, or `zstyle -b`
    Zstyle,
    /// zsh `_arguments`
    ZshArguments,
}

/// High-level category for a semantic binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// A normal assignment.
    Assignment,
    /// A parameter-default assignment from `${x:=...}`.
    ParameterDefaultAssignment,
    /// An append assignment such as `foo+=bar`.
    AppendAssignment,
    /// An array assignment or compound assignment.
    ArrayAssignment,
    /// A declaration builtin target.
    Declaration(DeclarationBuiltin),
    /// A function definition binding.
    FunctionDefinition,
    /// A loop variable binding.
    LoopVariable,
    /// A target created by `read`.
    ReadTarget,
    /// A target created by `mapfile` or `readarray`.
    MapfileTarget,
    /// A target created by `printf -v`.
    PrintfTarget,
    /// A target created by `getopts`.
    GetoptsTarget,
    /// A target created by `zparseopts`.
    ZparseoptsTarget,
    /// An arithmetic assignment binding.
    ArithmeticAssignment,
    /// A nameref binding.
    Nameref,
    /// A binding imported from another file or ambient contract.
    Imported,
}

pub(crate) fn is_array_like_binding(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment
                | BindingKind::MapfileTarget
                | BindingKind::ZparseoptsTarget
        )
}

bitflags! {
    /// Attribute flags inferred for a binding.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BindingAttributes: u32 {
        /// The binding is exported to child processes.
        const EXPORTED               = 0b0000_0001;
        /// The binding is readonly.
        const READONLY               = 0b0000_0010;
        /// The binding is local to a function-like scope.
        const LOCAL                  = 0b0000_0100;
        /// The binding has integer semantics.
        const INTEGER                = 0b0000_1000;
        /// The binding is array-like.
        const ARRAY                  = 0b0001_0000;
        /// The binding is associative-array-like.
        const ASSOC                  = 0b0010_0000;
        /// The binding is a nameref.
        const NAMEREF                = 0b0100_0000;
        /// The binding applies lowercase transformation.
        const LOWERCASE              = 0b1000_0000;
        /// The binding applies uppercase transformation.
        const UPPERCASE              = 0b0001_0000_0000;
        /// A declaration builtin definitely initializes the binding.
        const DECLARATION_INITIALIZED = 0b0010_0000_0000;
        /// The binding may have been imported rather than locally defined.
        const IMPORTED_POSSIBLE      = 0b0100_0000_0000;
        /// The imported binding is a function.
        const IMPORTED_FUNCTION      = 0b1000_0000_0000;
        /// The binding was initialized to an empty value.
        const EMPTY_INITIALIZER      = 0b0001_0000_0000_0000;
        /// The binding comes from a file-entry contract.
        const IMPORTED_FILE_ENTRY    = 0b0010_0000_0000_0000;
        /// The binding's initializer reads its own previous value.
        const SELF_REFERENTIAL_READ  = 0b0100_0000_0000_0000;
        /// The imported file-entry binding is definitely initialized.
        const IMPORTED_FILE_ENTRY_INITIALIZED = 0b1000_0000_0000_0000;
        /// The binding is consumed by runtime behavior outside direct syntax reads.
        const EXTERNALLY_CONSUMED    = 0b0001_0000_0000_0000_0000;
    }
}
