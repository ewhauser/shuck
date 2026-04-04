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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub id: ScopeId,
    pub kind: ScopeKind,
    pub parent: Option<ScopeId>,
    pub span: Span,
    pub bindings: FxHashMap<Name, Vec<BindingId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Function(Name),
    Subshell,
    CommandSubstitution,
    Pipeline,
}
