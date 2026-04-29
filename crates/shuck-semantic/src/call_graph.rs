use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};
use smallvec::SmallVec;

use crate::binding::Binding;
use crate::scope::ancestor_scopes;
use crate::{BindingId, Scope, ScopeId, ScopeKind};

/// One syntactic function call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Callee name as written at the site.
    pub callee: Name,
    /// Span of the containing command.
    pub span: Span,
    /// Span of the callee token itself.
    pub name_span: Span,
    /// Scope active at the call site.
    pub scope: ScopeId,
    /// Number of positional arguments passed at the site.
    pub arg_count: usize,
}

/// Summary call graph derived from discovered function definitions and call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallGraph {
    /// Function names reachable from top-level execution.
    pub reachable: FxHashSet<Name>,
    /// Function-definition bindings that were never reached from the top level.
    pub uncalled: Vec<BindingId>,
    /// Pairs of same-name function definitions where a later definition overwrites an earlier one.
    pub overwritten: Vec<OverwrittenFunction>,
}

/// Two same-name function definitions where the second overwrites the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverwrittenFunction {
    /// Function name shared by both definitions.
    pub name: Name,
    /// Earlier function-definition binding.
    pub first: BindingId,
    /// Later function-definition binding.
    pub second: BindingId,
    /// Whether the earlier definition was called before being overwritten.
    pub first_called: bool,
}

/// Reason a function definition is considered unreached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreachedFunctionReason {
    /// The definition appears only in unreachable control-flow.
    UnreachableDefinition,
    /// The script terminates before control can reach the definition.
    ScriptTerminates,
    /// The enclosing function is not itself reached.
    EnclosingFunctionUnreached,
}

/// One unreached function definition together with the reason it is unreached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreachedFunction {
    /// Function name.
    pub name: Name,
    /// Binding for the unreached definition.
    pub binding: BindingId,
    /// Why the function is unreached.
    pub reason: UnreachedFunctionReason,
}

pub(crate) fn build_call_graph(
    scopes: &[Scope],
    bindings: &[Binding],
    functions: &FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
) -> CallGraph {
    let mut callees_by_enclosing_function: FxHashMap<Name, Vec<Name>> = FxHashMap::default();
    let mut top_level_callees: Vec<Name> = Vec::new();
    for sites in call_sites.values() {
        for site in sites {
            let mut saw_function_ancestor = false;
            for ancestor in ancestor_scopes(scopes, site.scope) {
                if let ScopeKind::Function(function) = &scopes[ancestor.index()].kind {
                    saw_function_ancestor = true;
                    for fn_name in function.static_names() {
                        callees_by_enclosing_function
                            .entry(fn_name.clone())
                            .or_default()
                            .push(site.callee.clone());
                    }
                }
            }
            if !saw_function_ancestor {
                top_level_callees.push(site.callee.clone());
            }
        }
    }

    let mut reachable = FxHashSet::default();
    let mut worklist = top_level_callees;
    while let Some(name) = worklist.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if let Some(callees) = callees_by_enclosing_function.get(&name) {
            worklist.extend(callees.iter().cloned());
        }
    }

    let uncalled = functions
        .iter()
        .filter(|(name, _)| !reachable.contains(*name))
        .flat_map(|(_, bindings)| bindings.iter().copied())
        .collect();

    let overwritten = functions
        .iter()
        .flat_map(|(name, function_bindings)| {
            function_bindings
                .windows(2)
                .map(move |pair| OverwrittenFunction {
                    name: name.clone(),
                    first: pair[0],
                    second: pair[1],
                    first_called: call_sites
                        .get(name)
                        .into_iter()
                        .flat_map(|sites| sites.iter())
                        .any(|site| {
                            let first = bindings[pair[0].index()].span.start.offset;
                            let second = bindings[pair[1].index()].span.start.offset;
                            site.span.start.offset > first && site.span.start.offset < second
                        }),
                })
        })
        .collect();

    CallGraph {
        reachable,
        uncalled,
        overwritten,
    }
}
