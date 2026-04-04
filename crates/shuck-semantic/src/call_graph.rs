use rustc_hash::FxHashSet;
use shuck_ast::{Name, Span};

use crate::{BindingId, ScopeId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub callee: Name,
    pub span: Span,
    pub scope: ScopeId,
    pub arg_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallGraph {
    pub reachable: FxHashSet<Name>,
    pub uncalled: Vec<BindingId>,
    pub overwritten: Vec<OverwrittenFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverwrittenFunction {
    pub name: Name,
    pub first: BindingId,
    pub second: BindingId,
    pub first_called: bool,
}
