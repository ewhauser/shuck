use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};
use smallvec::SmallVec;

use crate::cfg::{CommandId, RecordedCommandKind, RecordedCommandRange, RecordedProgram};
use crate::scope::{ancestor_scopes, enclosing_scope_matching};
use crate::{Binding, BindingId, BindingKind, CallSite, Scope, ScopeId, SpanKey};

pub(crate) struct FunctionBindingLookup<'a> {
    pub(crate) program: &'a RecordedProgram,
    pub(crate) scopes: &'a [Scope],
    pub(crate) bindings: &'a [Binding],
    pub(crate) call_sites: &'a FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    pub(crate) unconditional_function_bindings: &'a FxHashSet<BindingId>,
    pub(crate) function_bindings_by_scope: &'a FxHashMap<ScopeId, SmallVec<[BindingId; 2]>>,
    /// Per call scope, the offset from which each function scope can already have been entered
    /// (see `entry_thresholds_for_call_scope`), computed on first use.
    pub(crate) entry_thresholds: RefCell<FxHashMap<ScopeId, FxHashMap<ScopeId, usize>>>,
}

pub(crate) struct ResolvedFunctionCall<'a> {
    pub(crate) site: &'a CallSite,
    pub(crate) binding: BindingId,
    pub(crate) callee_scope: ScopeId,
}

pub(crate) fn call_payloads_by_callee_scope<'a, I, P>(
    lookup: &FunctionBindingLookup<'_>,
    function_body_scopes: &FxHashMap<BindingId, ScopeId>,
    calls: I,
) -> FxHashMap<ScopeId, Vec<P>>
where
    I: IntoIterator<Item = (&'a Name, ScopeId, usize, P)>,
{
    let mut grouped = FxHashMap::default();

    for (name, scope, offset, payload) in calls {
        let Some(function_binding) = lookup.visible_function_binding(name, scope, offset) else {
            continue;
        };
        let Some(callee_scope) = function_body_scopes.get(&function_binding).copied() else {
            continue;
        };
        grouped
            .entry(callee_scope)
            .or_insert_with(Vec::new)
            .push(payload);
    }

    grouped
}

impl FunctionBindingLookup<'_> {
    pub(crate) fn visible_function_call_bindings(&self) -> FxHashMap<SpanKey, BindingId> {
        let mut call_bindings = FxHashMap::default();

        for (name, sites) in self.call_sites {
            for site in sites {
                let Some(binding) =
                    self.visible_function_binding(name, site.scope, site.span.start.offset())
                else {
                    continue;
                };
                call_bindings.insert(SpanKey::new(site.name_span), binding);
            }
        }

        call_bindings
    }
}

pub(crate) fn resolved_function_calls_with_callee_scope<'a>(
    call_sites: &'a FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &'a FxHashMap<SpanKey, BindingId>,
    function_body_scopes: &'a FxHashMap<BindingId, ScopeId>,
) -> impl Iterator<Item = ResolvedFunctionCall<'a>> + 'a {
    call_sites
        .values()
        .flat_map(|sites| sites.iter())
        .filter_map(move |site| {
            let binding = visible_function_call_bindings
                .get(&SpanKey::new(site.name_span))
                .copied()?;
            let callee_scope = function_body_scopes.get(&binding).copied()?;

            Some(ResolvedFunctionCall {
                site,
                binding,
                callee_scope,
            })
        })
}

pub(crate) fn function_bindings_by_scope(
    program: &RecordedProgram,
) -> FxHashMap<ScopeId, SmallVec<[BindingId; 2]>> {
    let mut bindings_by_scope: FxHashMap<ScopeId, SmallVec<[BindingId; 2]>> = FxHashMap::default();
    for (&binding, &scope) in &program.function_body_scopes {
        bindings_by_scope.entry(scope).or_default().push(binding);
    }
    bindings_by_scope
}

pub(crate) fn collect_unconditional_function_bindings(
    program: &RecordedProgram,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    bindings: &[Binding],
) -> FxHashSet<BindingId> {
    let mut unconditional = FxHashSet::default();
    collect_sequence_function_bindings(
        program,
        program.file_commands(),
        command_bindings,
        bindings,
        &mut unconditional,
    );
    for commands in program.function_bodies().values().copied() {
        collect_sequence_function_bindings(
            program,
            commands,
            command_bindings,
            bindings,
            &mut unconditional,
        );
    }
    unconditional
}

fn collect_sequence_function_bindings(
    program: &RecordedProgram,
    commands: RecordedCommandRange,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    bindings: &[Binding],
    unconditional: &mut FxHashSet<BindingId>,
) {
    for &command_id in program.commands_in(commands) {
        collect_command_function_bindings(
            program,
            command_id,
            command_bindings,
            bindings,
            unconditional,
        );
    }
}

fn collect_command_function_bindings(
    program: &RecordedProgram,
    command_id: CommandId,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    bindings: &[Binding],
    unconditional: &mut FxHashSet<BindingId>,
) {
    let command = program.command(command_id);
    collect_direct_function_bindings(command.span, command_bindings, bindings, unconditional);

    match command.kind {
        RecordedCommandKind::List { first, .. } => collect_command_function_bindings(
            program,
            first,
            command_bindings,
            bindings,
            unconditional,
        ),
        RecordedCommandKind::BraceGroup { body } => collect_sequence_function_bindings(
            program,
            body,
            command_bindings,
            bindings,
            unconditional,
        ),
        RecordedCommandKind::Always { body, always_body } => {
            collect_sequence_function_bindings(
                program,
                body,
                command_bindings,
                bindings,
                unconditional,
            );
            collect_sequence_function_bindings(
                program,
                always_body,
                command_bindings,
                bindings,
                unconditional,
            );
        }
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit
        | RecordedCommandKind::If { .. }
        | RecordedCommandKind::While { .. }
        | RecordedCommandKind::Until { .. }
        | RecordedCommandKind::For { .. }
        | RecordedCommandKind::Select { .. }
        | RecordedCommandKind::ArithmeticFor { .. }
        | RecordedCommandKind::Case { .. }
        | RecordedCommandKind::Subshell { .. }
        | RecordedCommandKind::Pipeline { .. } => {}
    }
}

fn collect_direct_function_bindings(
    span: Span,
    command_bindings: &FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    bindings: &[Binding],
    unconditional: &mut FxHashSet<BindingId>,
) {
    let key = SpanKey::new(span);
    let Some(command_bindings) = command_bindings.get(&key) else {
        return;
    };
    unconditional.extend(command_bindings.iter().copied().filter(|binding| {
        matches!(
            bindings[binding.index()].kind,
            BindingKind::FunctionDefinition
        )
    }));
}

impl FunctionBindingLookup<'_> {
    pub(crate) fn visible_function_binding(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> Option<BindingId> {
        for scope_id in ancestor_scopes(self.scopes, scope) {
            let Some(candidates) = self.scopes[scope_id.index()].bindings.get(name) else {
                continue;
            };

            for binding in candidates.iter().rev().copied() {
                let candidate = &self.bindings[binding.index()];
                if !matches!(candidate.kind, BindingKind::FunctionDefinition) {
                    continue;
                }

                if scope_id == scope {
                    if candidate.span.start.offset() <= offset
                        && self.unconditional_function_bindings.contains(&binding)
                    {
                        return Some(binding);
                    }
                    continue;
                }

                if !self.unconditional_function_bindings.contains(&binding) {
                    return None;
                }

                return self
                    .parent_scope_binding_available_before_scope_runs(candidate, scope)
                    .then_some(binding);
            }
        }

        None
    }

    fn parent_scope_binding_available_before_scope_runs(
        &self,
        candidate: &Binding,
        scope: ScopeId,
    ) -> bool {
        if candidate.span.start.offset() <= self.scopes[scope.index()].span.start.offset() {
            return true;
        }

        !self.scope_has_known_entry_before_offset(
            scope,
            candidate.scope,
            candidate.span.start.offset(),
        )
    }

    /// Returns whether some chain of known callers can enter function scope `scope` before
    /// execution reaches `offset` in `call_scope`.
    fn scope_has_known_entry_before_offset(
        &self,
        scope: ScopeId,
        call_scope: ScopeId,
        offset: usize,
    ) -> bool {
        if let Some(thresholds) = self.entry_thresholds.borrow().get(&call_scope) {
            return thresholds
                .get(&scope)
                .is_some_and(|threshold| offset >= *threshold);
        }

        let thresholds = self.entry_thresholds_for_call_scope(call_scope);
        let has_entry = thresholds
            .get(&scope)
            .is_some_and(|threshold| offset >= *threshold);
        self.entry_thresholds
            .borrow_mut()
            .insert(call_scope, thresholds);
        has_entry
    }

    /// Computes, for every function scope, the smallest offset in `call_scope` from which a
    /// chain of known callers can already have entered that scope. Scopes that no caller chain
    /// enters early are absent.
    ///
    /// Whether a call site can run before an offset only gets easier as the offset grows, so
    /// every scope has such a threshold. A call site that runs inside another function is only
    /// as early as its enclosing function's own threshold, which makes this a bottleneck
    /// shortest-path problem over the reversed call graph: thresholds are settled in increasing
    /// order with a priority queue, so every caller chain is explored once per `call_scope`
    /// regardless of how many offsets are queried or how deep the chains are.
    fn entry_thresholds_for_call_scope(&self, call_scope: ScopeId) -> FxHashMap<ScopeId, usize> {
        let mut best: FxHashMap<ScopeId, usize> = FxHashMap::default();
        let mut callers_of: FxHashMap<ScopeId, Vec<(ScopeId, usize)>> = FxHashMap::default();

        for (&scope, function_bindings) in self.function_bindings_by_scope {
            for binding in function_bindings.iter().copied() {
                let function = &self.bindings[binding.index()];
                let Some(sites) = self.call_sites.get(&function.name) else {
                    continue;
                };
                for site in sites {
                    let Some(reference_threshold) =
                        self.call_site_reference_threshold(site, binding)
                    else {
                        continue;
                    };
                    match self.call_site_entry_threshold(scope, site, call_scope) {
                        CallSiteEntry::Never => {}
                        CallSiteEntry::From(run_threshold) => {
                            let threshold = reference_threshold.max(run_threshold);
                            let entry = best.entry(scope).or_insert(threshold);
                            *entry = (*entry).min(threshold);
                        }
                        CallSiteEntry::ThroughCaller(caller) => callers_of
                            .entry(caller)
                            .or_default()
                            .push((scope, reference_threshold)),
                    }
                }
            }
        }

        let mut queue = best
            .iter()
            .map(|(&scope, &threshold)| Reverse((threshold, scope.0)))
            .collect::<BinaryHeap<_>>();
        let mut settled = FxHashMap::default();
        while let Some(Reverse((threshold, scope))) = queue.pop() {
            let scope = ScopeId(scope);
            if settled.contains_key(&scope) {
                continue;
            }
            settled.insert(scope, threshold);
            let Some(callees) = callers_of.get(&scope) else {
                continue;
            };
            for &(callee, reference_threshold) in callees {
                if settled.contains_key(&callee) {
                    continue;
                }
                let candidate = threshold.max(reference_threshold);
                if best.get(&callee).is_none_or(|current| candidate < *current) {
                    best.insert(callee, candidate);
                    queue.push(Reverse((candidate, callee.0)));
                }
            }
        }

        settled
    }

    /// Returns the smallest query offset at which `site` may call `binding`, or `None` when it
    /// never does.
    fn call_site_reference_threshold(&self, site: &CallSite, binding: BindingId) -> Option<usize> {
        let target = &self.bindings[binding.index()];
        if !self.scope_has_ancestor(site.scope, target.scope) {
            return None;
        }

        if target.scope == site.scope {
            return (self.lexically_visible_function_binding_in_scope(
                &target.name,
                site.scope,
                site.span.start.offset(),
            ) == Some(binding))
            .then_some(0);
        }

        Some(target.span.start.offset() + 1)
    }

    fn call_site_entry_threshold(
        &self,
        target_scope: ScopeId,
        site: &CallSite,
        call_scope: ScopeId,
    ) -> CallSiteEntry {
        let command_start = || {
            crate::cfg::recorded_command_span_for_call_site(self.program, site)
                .start
                .offset()
        };
        let Some(enclosing_function) = self.enclosing_function_scope(site.scope) else {
            if self.scope_has_ancestor(site.scope, call_scope) {
                return CallSiteEntry::From(command_start() + 1);
            }
            return if self.scope_has_ancestor(call_scope, target_scope) {
                CallSiteEntry::From(0)
            } else {
                CallSiteEntry::Never
            };
        };

        if enclosing_function == call_scope {
            return CallSiteEntry::From(command_start() + 1);
        }

        CallSiteEntry::ThroughCaller(enclosing_function)
    }

    fn enclosing_function_scope(&self, scope: ScopeId) -> Option<ScopeId> {
        enclosing_scope_matching(self.scopes, scope, |scope_id, _| {
            self.function_bindings_by_scope.contains_key(&scope_id)
        })
    }

    fn scope_has_ancestor(&self, scope: ScopeId, ancestor: ScopeId) -> bool {
        ancestor_scopes(self.scopes, scope).any(|scope_id| scope_id == ancestor)
    }

    fn lexically_visible_function_binding_in_scope(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> Option<BindingId> {
        lexically_visible_function_binding_in_scope(
            self.scopes,
            self.bindings,
            name,
            scope,
            scope,
            offset,
        )
    }
}

enum CallSiteEntry {
    /// The call site never runs before the offset.
    Never,
    /// The call site runs before any offset at or after this one.
    From(usize),
    /// The call site runs early exactly when its enclosing function can be entered early.
    ThroughCaller(ScopeId),
}

pub(crate) fn lexically_visible_function_binding_in_scope(
    scopes: &[Scope],
    bindings: &[Binding],
    name: &Name,
    target_scope: ScopeId,
    call_scope: ScopeId,
    offset: usize,
) -> Option<BindingId> {
    let candidates = scopes[target_scope.index()].bindings.get(name)?;
    if target_scope != call_scope {
        return candidates.iter().rev().copied().find(|binding| {
            matches!(
                bindings[binding.index()].kind,
                BindingKind::FunctionDefinition
            )
        });
    }

    candidates.iter().rev().copied().find(|binding| {
        let candidate = &bindings[binding.index()];
        matches!(candidate.kind, BindingKind::FunctionDefinition)
            && candidate.span.start.offset() <= offset
    })
}
