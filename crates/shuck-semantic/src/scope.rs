use rustc_hash::FxHashMap;
use shuck_ast::{Name, Span};

use crate::BindingId;

/// Stable identifier for one semantic scope in a [`crate::SemanticModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub(crate) u32);

impl ScopeId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

pub(crate) fn ancestor_scopes(
    scopes: &[Scope],
    start: ScopeId,
) -> impl Iterator<Item = ScopeId> + '_ {
    std::iter::successors(Some(start), move |scope| scopes[scope.index()].parent)
}

pub(crate) fn enclosing_scope_matching<F>(
    scopes: &[Scope],
    start: ScopeId,
    mut matches_scope: F,
) -> Option<ScopeId>
where
    F: FnMut(ScopeId, &Scope) -> bool,
{
    ancestor_scopes(scopes, start).find(|scope| matches_scope(*scope, &scopes[scope.index()]))
}

pub(crate) fn enclosing_function_scope(scopes: &[Scope], start: ScopeId) -> Option<ScopeId> {
    enclosing_scope_matching(scopes, start, |_, scope| {
        matches!(scope.kind, ScopeKind::Function(_))
    })
}

/// Lexical scope discovered during semantic traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Unique identifier for this scope.
    pub id: ScopeId,
    /// Semantic kind of scope.
    pub kind: ScopeKind,
    /// Lexical parent scope, when one exists.
    pub parent: Option<ScopeId>,
    /// Source span covered by the scope.
    pub span: Span,
    /// Bindings introduced directly in this scope, grouped by name.
    pub bindings: FxHashMap<Name, Vec<BindingId>>,
}

/// Additional classification for function scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionScopeKind {
    /// A function with one or more static names visible from source.
    Named(Vec<Name>),
    /// A function-like scope whose name is resolved dynamically.
    Dynamic,
    /// A function body without a user-visible static function name.
    Anonymous,
}

impl FunctionScopeKind {
    /// Returns the statically known function names for this scope, if any.
    pub fn static_names(&self) -> &[Name] {
        match self {
            Self::Named(names) => names.as_slice(),
            Self::Dynamic | Self::Anonymous => &[],
        }
    }

    /// Returns whether `name` is one of this scope's statically known names.
    pub fn contains_name(&self, name: &Name) -> bool {
        self.static_names()
            .iter()
            .any(|candidate| candidate == name)
    }

    /// Returns whether `name` matches one of this scope's statically known names.
    pub fn contains_name_str(&self, name: &str) -> bool {
        self.static_names()
            .iter()
            .any(|candidate| candidate.as_str() == name)
    }

    /// Returns whether this function scope is dynamically named.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic)
    }

    /// Returns whether this function scope has no user-visible static name.
    pub fn is_anonymous(&self) -> bool {
        matches!(self, Self::Anonymous)
    }
}

/// High-level category for a semantic scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    /// The file-level scope.
    File,
    /// A function body scope.
    Function(FunctionScopeKind),
    /// A subshell scope such as `( ... )`.
    Subshell,
    /// A command substitution scope such as `$( ... )`.
    CommandSubstitution,
    /// A pipeline component scope whose side effects may be transient.
    Pipeline,
}
