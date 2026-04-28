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
    pub(crate) fn visible_function_binding(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
    ) -> Option<BindingId> {
        let mut resolver = FunctionCallResolver {
            program: self.program,
            scopes: self.scopes,
            bindings: self.bindings,
            call_sites: self.call_sites,
            unconditional_function_bindings: self.unconditional_function_bindings,
            function_bindings_by_scope: self.function_bindings_by_scope,
            entry_before_offset_cache: FxHashMap::default(),
        };
        resolver.visible_function_binding(name, scope, offset)
    }

    pub(crate) fn visible_function_call_bindings(&self) -> FxHashMap<SpanKey, BindingId> {
        let mut resolver = FunctionCallResolver {
            program: self.program,
            scopes: self.scopes,
            bindings: self.bindings,
            call_sites: self.call_sites,
            unconditional_function_bindings: self.unconditional_function_bindings,
            function_bindings_by_scope: self.function_bindings_by_scope,
            entry_before_offset_cache: FxHashMap::default(),
        };
        let mut call_bindings = FxHashMap::default();

        for (name, sites) in self.call_sites {
            for site in sites {
                let Some(binding) =
                    resolver.visible_function_binding(name, site.scope, site.span.start.offset)
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

struct FunctionCallResolver<'a> {
    program: &'a RecordedProgram,
    scopes: &'a [Scope],
    bindings: &'a [Binding],
    call_sites: &'a FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    unconditional_function_bindings: &'a FxHashSet<BindingId>,
    function_bindings_by_scope: &'a FxHashMap<ScopeId, SmallVec<[BindingId; 2]>>,
    entry_before_offset_cache: FxHashMap<(ScopeId, ScopeId, usize), bool>,
}

impl FunctionCallResolver<'_> {
    fn visible_function_binding(
        &mut self,
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
                    if candidate.span.start.offset <= offset
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
        &mut self,
        candidate: &Binding,
        scope: ScopeId,
    ) -> bool {
        if candidate.span.start.offset <= self.scopes[scope.index()].span.start.offset {
            return true;
        }

        let mut visiting = FxHashSet::default();
        !self.scope_has_known_entry_before_offset(
            scope,
            candidate.scope,
            candidate.span.start.offset,
            &mut visiting,
        )
    }

    fn scope_has_known_entry_before_offset(
        &mut self,
        scope: ScopeId,
        call_scope: ScopeId,
        offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let cache_key = (scope, call_scope, offset);
        let cacheable = visiting.is_empty();
        if cacheable && let Some(cached) = self.entry_before_offset_cache.get(&cache_key) {
            return *cached;
        }

        if !visiting.insert(scope) {
            return false;
        }

        let has_entry =
            self.function_bindings_by_scope
                .get(&scope)
                .is_some_and(|function_bindings| {
                    function_bindings.iter().copied().any(|binding| {
                        let function = &self.bindings[binding.index()];
                        let Some(sites) = self.call_sites.get(&function.name) else {
                            return false;
                        };
                        sites.iter().any(|site| {
                            self.call_site_may_reference_binding(site, binding, offset)
                                && self.call_site_can_run_before_offset(
                                    scope, site, call_scope, offset, visiting,
                                )
                        })
                    })
                });

        visiting.remove(&scope);
        if cacheable {
            self.entry_before_offset_cache.insert(cache_key, has_entry);
        }
        has_entry
    }

    fn call_site_can_run_before_offset(
        &mut self,
        target_scope: ScopeId,
        site: &CallSite,
        call_scope: ScopeId,
        offset: usize,
        visiting: &mut FxHashSet<ScopeId>,
    ) -> bool {
        let command_start = crate::cfg::recorded_command_span_for_call_site(self.program, site)
            .start
            .offset;
        let Some(enclosing_function) = self.enclosing_function_scope(site.scope) else {
            if self.scope_has_ancestor(site.scope, call_scope) {
                return command_start < offset;
            }
            return self.scope_has_ancestor(call_scope, target_scope);
        };

        if enclosing_function == call_scope {
            return command_start < offset;
        }

        self.scope_has_known_entry_before_offset(enclosing_function, call_scope, offset, visiting)
    }

    fn call_site_may_reference_binding(
        &self,
        site: &CallSite,
        binding: BindingId,
        offset: usize,
    ) -> bool {
        let target = &self.bindings[binding.index()];
        if !self.scope_has_ancestor(site.scope, target.scope) {
            return false;
        }

        if target.scope == site.scope {
            return self.lexically_visible_function_binding_in_scope(
                &target.name,
                site.scope,
                site.span.start.offset,
            ) == Some(binding);
        }

        target.span.start.offset < offset
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
            && candidate.span.start.offset <= offset
    })
}
