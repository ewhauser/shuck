use rustc_hash::FxHashMap;
use shuck_ast::{Name, Span};

use crate::BindingId;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub id: ScopeId,
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub span: Span,
    pub bindings: FxHashMap<Name, Vec<BindingId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionScopeKind {
    Named(Vec<Name>),
    Dynamic,
    Anonymous,
}

impl FunctionScopeKind {
    pub fn static_names(&self) -> &[Name] {
        match self {
            Self::Named(names) => names.as_slice(),
            Self::Dynamic | Self::Anonymous => &[],
        }
    }

    pub fn contains_name(&self, name: &Name) -> bool {
        self.static_names()
            .iter()
            .any(|candidate| candidate == name)
    }

    pub fn contains_name_str(&self, name: &str) -> bool {
        self.static_names()
            .iter()
            .any(|candidate| candidate.as_str() == name)
    }

    pub fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic)
    }

    pub fn is_anonymous(&self) -> bool {
        matches!(self, Self::Anonymous)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Function(FunctionScopeKind),
    Subshell,
    CommandSubstitution,
    Pipeline,
}
