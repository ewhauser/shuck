use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_ast::Span;

use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BlockId, CallSite, ControlFlowGraph,
    IndirectTargetHint, Reference, ReferenceId, ReferenceKind, Scope, ScopeId, ScopeKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachingDefinitions {
    pub reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>>,
    pub reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedAssignment {
    pub binding: BindingId,
    pub reason: UnusedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnusedReason {
    Overwritten { by: BindingId },
    ScopeEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninitializedReference {
    pub reference: ReferenceId,
    pub certainty: UninitializedCertainty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninitializedCertainty {
    Definite,
    Possible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadCode {
    pub unreachable: Vec<Span>,
    pub cause: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataflowResult {
    pub reaching_definitions: ReachingDefinitions,
    pub unused_assignments: Vec<UnusedAssignment>,
    pub uninitialized_references: Vec<UninitializedReference>,
    pub dead_code: Vec<DeadCode>,
    pub(crate) unused_assignment_ids: Vec<BindingId>,
}

impl DataflowResult {
    pub fn unused_assignment_ids(&self) -> &[BindingId] {
        &self.unused_assignment_ids
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn analyze(
    cfg: &ControlFlowGraph,
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
) -> DataflowResult {
    let block_ids = cfg
        .blocks()
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    let mut reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>> = FxHashMap::default();
    let mut reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>> = FxHashMap::default();

    let gen_sets = block_ids
        .iter()
        .map(|block_id| (*block_id, gen_set(cfg, *block_id, bindings)))
        .collect::<FxHashMap<_, _>>();
    let kill_sets = block_ids
        .iter()
        .map(|block_id| (*block_id, kill_set(cfg, *block_id, bindings)))
        .collect::<FxHashMap<_, _>>();

    let mut changed = true;
    while changed {
        changed = false;
        for block_id in &block_ids {
            let incoming = cfg
                .predecessors(*block_id)
                .iter()
                .flat_map(|predecessor| {
                    reaching_out.get(predecessor).into_iter().flatten().copied()
                })
                .collect::<FxHashSet<_>>();
            let outgoing = gen_sets
                .get(block_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .chain(incoming.iter().copied().filter(|binding| {
                    !kill_sets
                        .get(block_id)
                        .is_some_and(|kills| kills.contains(binding))
                }))
                .collect::<FxHashSet<_>>();

            if reaching_in.get(block_id) != Some(&incoming) {
                reaching_in.insert(*block_id, incoming);
                changed = true;
            }
            if reaching_out.get(block_id) != Some(&outgoing) {
                reaching_out.insert(*block_id, outgoing);
                changed = true;
            }
        }
    }

    let reaching_definitions = ReachingDefinitions {
        reaching_in,
        reaching_out,
    };

    let reference_blocks = reference_blocks(cfg);
    let binding_blocks = binding_blocks(cfg);
    let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
    let scope_components = scope_components(cfg, &reaching_definitions);

    let maybe_defined = block_ids
        .iter()
        .map(|block_id| {
            (
                *block_id,
                names_from_bindings(
                    reaching_definitions
                        .reaching_in
                        .get(block_id)
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .copied(),
                    bindings,
                ),
            )
        })
        .collect::<FxHashMap<_, _>>();

    let definitely_defined = block_ids
        .iter()
        .map(|block_id| {
            let predecessors = cfg.predecessors(*block_id);
            if predecessors.is_empty() {
                return (*block_id, FxHashSet::default());
            }
            let mut predecessor_sets = predecessors
                .iter()
                .map(|predecessor| {
                    names_from_bindings(
                        reaching_definitions
                            .reaching_out
                            .get(predecessor)
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .copied(),
                        bindings,
                    )
                })
                .collect::<Vec<_>>();
            let first = predecessor_sets.pop().unwrap_or_default();
            let intersection = predecessor_sets
                .into_iter()
                .fold(first, |acc, set| acc.intersection(&set).cloned().collect());
            (*block_id, intersection)
        })
        .collect::<FxHashMap<_, _>>();

    let mut uninitialized_references = Vec::new();
    for reference in references {
        if matches!(reference.kind, ReferenceKind::ImplicitRead) {
            continue;
        }
        let Some(block_id) = reference_blocks.get(&reference.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) {
            continue;
        }
        let maybe = maybe_defined
            .get(&block_id)
            .is_some_and(|names| names.contains(&reference.name));
        let definite = definitely_defined
            .get(&block_id)
            .is_some_and(|names| names.contains(&reference.name));

        if !maybe {
            uninitialized_references.push(UninitializedReference {
                reference: reference.id,
                certainty: UninitializedCertainty::Definite,
            });
        } else if !definite {
            uninitialized_references.push(UninitializedReference {
                reference: reference.id,
                certainty: UninitializedCertainty::Possible,
            });
        }
    }

    let mut used_bindings = bindings
        .iter()
        .filter(|binding| !binding.references.is_empty())
        .map(|binding| binding.id)
        .collect::<FxHashSet<_>>();
    used_bindings.extend(
        bindings
            .iter()
            .filter(|binding| binding.name == "IFS")
            .map(|binding| binding.id),
    );
    for reference in references {
        let Some(block_id) = reference_blocks.get(&reference.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) {
            continue;
        }
        if let Some(incoming) = reaching_definitions.reaching_in.get(&block_id) {
            for binding in incoming {
                if bindings[binding.index()].name == reference.name {
                    used_bindings.insert(*binding);
                }
            }
        }

        let Some(resolved_binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        let resolved_binding = &bindings[resolved_binding_id.index()];
        if let Some(component) = scope_components.get(&resolved_binding.scope)
            && !component.blocks.contains(&block_id)
        {
            for binding in &component.exit_defs {
                if bindings[binding.index()].scope == resolved_binding.scope
                    && bindings[binding.index()].name == reference.name
                {
                    used_bindings.insert(*binding);
                }
            }
        }

        if indirect_expansion_refs.contains(&reference.id)
            && let Some(hint) = indirect_target_hints.get(&resolved_binding_id)
        {
            if let Some(incoming) = reaching_definitions.reaching_in.get(&block_id) {
                mark_indirect_targets_used(
                    &mut used_bindings,
                    incoming.iter().copied(),
                    bindings,
                    hint,
                );
            }

            if let Some(component) = scope_components.get(&resolved_binding.scope)
                && !component.blocks.contains(&block_id)
            {
                mark_indirect_targets_used(
                    &mut used_bindings,
                    component.exit_defs.iter().copied(),
                    bindings,
                    hint,
                );
            }
        }
    }
    used_bindings.extend(interprocedural_function_uses(
        scopes, bindings, references, call_sites,
    ));

    let mut unused_assignments = Vec::new();
    let mut unused_assignment_ids = Vec::new();
    for binding in bindings {
        let Some(block_id) = binding_blocks.get(&binding.id).copied() else {
            continue;
        };
        if unreachable.contains(&block_id) || used_bindings.contains(&binding.id) {
            continue;
        }

        let reason = next_overwrite(binding, bindings)
            .map(|by| UnusedReason::Overwritten { by })
            .unwrap_or(UnusedReason::ScopeEnd);
        unused_assignments.push(UnusedAssignment {
            binding: binding.id,
            reason,
        });
        unused_assignment_ids.push(binding.id);
    }

    let mut dead_code_by_cause: FxHashMap<(usize, usize), (Span, Vec<Span>)> = FxHashMap::default();
    for block_id in cfg.unreachable() {
        let block = cfg.block(*block_id);
        if block.commands.is_empty() {
            continue;
        }
        let cause = cfg
            .unreachable_cause(*block_id)
            .unwrap_or_else(|| block.commands[0]);
        dead_code_by_cause
            .entry((cause.start.offset, cause.end.offset))
            .or_insert_with(|| (cause, Vec::new()))
            .1
            .extend(block.commands.iter().copied());
    }
    let dead_code = dead_code_by_cause
        .into_iter()
        .map(|(_, (cause, unreachable))| DeadCode { unreachable, cause })
        .collect();

    DataflowResult {
        reaching_definitions,
        unused_assignments,
        uninitialized_references,
        dead_code,
        unused_assignment_ids,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedCallSite {
    offset: usize,
    callee_scope: ScopeId,
}

#[derive(Debug, Clone)]
struct ScopedName {
    offset: usize,
    name: Name,
}

#[derive(Debug, Clone, Default)]
struct ScopeComponent {
    blocks: FxHashSet<BlockId>,
    exit_defs: FxHashSet<BindingId>,
}

fn gen_set(
    cfg: &ControlFlowGraph,
    block_id: BlockId,
    bindings: &[Binding],
) -> FxHashSet<BindingId> {
    let mut generated = FxHashSet::default();
    for binding in &cfg.block(block_id).bindings {
        let binding_data = &bindings[binding.index()];
        if matches!(binding_data.kind, BindingKind::AppendAssignment) {
            generated.insert(*binding);
            continue;
        }

        generated.retain(|candidate| bindings[candidate.index()].name != binding_data.name);
        generated.insert(*binding);
    }
    generated
}

fn kill_set(
    cfg: &ControlFlowGraph,
    block_id: BlockId,
    bindings: &[Binding],
) -> FxHashSet<BindingId> {
    let block = cfg.block(block_id);
    let overwritten_names = block
        .bindings
        .iter()
        .filter(|binding| {
            !matches!(
                bindings[binding.index()].kind,
                BindingKind::AppendAssignment
            )
        })
        .map(|binding| bindings[binding.index()].name.clone())
        .collect::<FxHashSet<_>>();
    bindings
        .iter()
        .filter(|binding| {
            overwritten_names.contains(&binding.name) && !block.bindings.contains(&binding.id)
        })
        .map(|binding| binding.id)
        .collect()
}

fn names_from_bindings(
    bindings_iter: impl Iterator<Item = BindingId>,
    bindings: &[Binding],
) -> FxHashSet<shuck_ast::Name> {
    bindings_iter
        .map(|binding| bindings[binding.index()].name.clone())
        .collect()
}

fn reference_blocks(cfg: &ControlFlowGraph) -> FxHashMap<ReferenceId, BlockId> {
    let mut map = FxHashMap::default();
    for block in cfg.blocks() {
        for reference in &block.references {
            map.insert(*reference, block.id);
        }
    }
    map
}

fn binding_blocks(cfg: &ControlFlowGraph) -> FxHashMap<BindingId, BlockId> {
    let mut map = FxHashMap::default();
    for block in cfg.blocks() {
        for binding in &block.bindings {
            map.insert(*binding, block.id);
        }
    }
    map
}

fn scope_components(
    cfg: &ControlFlowGraph,
    reaching_definitions: &ReachingDefinitions,
) -> FxHashMap<ScopeId, ScopeComponent> {
    cfg.scope_entries
        .iter()
        .map(|(scope, entry)| {
            let blocks = reachable_blocks(cfg, *entry);
            let exit_defs = blocks
                .iter()
                .copied()
                .filter(|block| {
                    cfg.successors(*block)
                        .iter()
                        .all(|(successor, _)| !blocks.contains(successor))
                })
                .flat_map(|block| {
                    reaching_definitions
                        .reaching_out
                        .get(&block)
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect();
            (*scope, ScopeComponent { blocks, exit_defs })
        })
        .collect()
}

fn reachable_blocks(cfg: &ControlFlowGraph, entry: BlockId) -> FxHashSet<BlockId> {
    let mut visited = FxHashSet::default();
    let mut stack = vec![entry];
    while let Some(block) = stack.pop() {
        if !visited.insert(block) {
            continue;
        }
        stack.extend(
            cfg.successors(block)
                .iter()
                .map(|(successor, _)| *successor),
        );
    }
    visited
}

fn next_overwrite(binding: &Binding, bindings: &[Binding]) -> Option<BindingId> {
    bindings
        .iter()
        .filter(|candidate| {
            candidate.name == binding.name
                && candidate.span.start.offset > binding.span.start.offset
        })
        .min_by_key(|candidate| candidate.span.start.offset)
        .map(|candidate| candidate.id)
}

fn mark_indirect_targets_used(
    used_bindings: &mut FxHashSet<BindingId>,
    candidates: impl Iterator<Item = BindingId>,
    bindings: &[Binding],
    hint: &IndirectTargetHint,
) {
    for binding in candidates {
        if indirect_target_matches(hint, &bindings[binding.index()]) {
            used_bindings.insert(binding);
        }
    }
}

fn indirect_target_matches(hint: &IndirectTargetHint, binding: &Binding) -> bool {
    match hint {
        IndirectTargetHint::Exact { name, array_like } => {
            binding.name == *name && (!array_like || is_array_like_binding(binding))
        }
        IndirectTargetHint::Pattern {
            prefix,
            suffix,
            array_like,
        } => {
            let name = binding.name.as_str();
            name.starts_with(prefix)
                && name.ends_with(suffix)
                && (!array_like || is_array_like_binding(binding))
        }
    }
}

fn is_array_like_binding(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || matches!(binding.kind, BindingKind::ArrayAssignment)
}

fn interprocedural_function_uses(
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
) -> FxHashSet<BindingId> {
    let function_scopes = function_scopes_by_binding(scopes, bindings);
    let calls_by_scope = resolved_calls_by_scope(scopes, bindings, call_sites, &function_scopes);
    let refs_by_scope = references_by_scope(references);
    let scope_ids = scopes.iter().map(|scope| scope.id).collect::<Vec<_>>();

    let mut transitive_reads = FxHashMap::default();
    loop {
        let mut changed = false;
        for scope_id in &scope_ids {
            let mut reads = refs_by_scope
                .get(scope_id)
                .into_iter()
                .flatten()
                .map(|reference| reference.name.clone())
                .collect::<FxHashSet<_>>();
            if let Some(calls) = calls_by_scope.get(scope_id) {
                for call in calls {
                    reads.extend(
                        transitive_reads
                            .get(&call.callee_scope)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                }
            }
            if transitive_reads.get(scope_id) != Some(&reads) {
                transitive_reads.insert(*scope_id, reads);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut escape_reads = FxHashMap::default();
    loop {
        let mut changed = false;
        for scope in scopes {
            if !matches!(scope.kind, ScopeKind::Function(_)) {
                continue;
            }
            let mut reads = FxHashSet::default();
            for (caller_scope, calls) in &calls_by_scope {
                for call in calls {
                    if call.callee_scope == scope.id {
                        reads.extend(names_after_offset(
                            scopes,
                            *caller_scope,
                            call.offset,
                            &refs_by_scope,
                            &calls_by_scope,
                            &transitive_reads,
                            &escape_reads,
                        ));
                    }
                }
            }
            if escape_reads.get(&scope.id) != Some(&reads) {
                escape_reads.insert(scope.id, reads);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    bindings
        .iter()
        .filter(|binding| is_function_escape_candidate(binding, scopes))
        .filter(|binding| {
            names_after_offset(
                scopes,
                binding.scope,
                binding.span.start.offset,
                &refs_by_scope,
                &calls_by_scope,
                &transitive_reads,
                &escape_reads,
            )
            .contains(&binding.name)
        })
        .map(|binding| binding.id)
        .collect()
}

fn function_scopes_by_binding(
    scopes: &[Scope],
    bindings: &[Binding],
) -> FxHashMap<BindingId, ScopeId> {
    let mut bindings_by_parent_and_name: FxHashMap<(ScopeId, Name), Vec<BindingId>> =
        FxHashMap::default();
    for binding in bindings {
        if matches!(binding.kind, BindingKind::FunctionDefinition) {
            bindings_by_parent_and_name
                .entry((binding.scope, binding.name.clone()))
                .or_default()
                .push(binding.id);
        }
    }
    for binding_ids in bindings_by_parent_and_name.values_mut() {
        binding_ids.sort_by_key(|binding| bindings[binding.index()].span.start.offset);
    }

    let mut scopes_by_parent_and_name: FxHashMap<(ScopeId, Name), Vec<ScopeId>> =
        FxHashMap::default();
    for scope in scopes {
        if let ScopeKind::Function(name) = &scope.kind
            && let Some(parent) = scope.parent
        {
            scopes_by_parent_and_name
                .entry((parent, name.clone()))
                .or_default()
                .push(scope.id);
        }
    }
    for scope_ids in scopes_by_parent_and_name.values_mut() {
        scope_ids.sort_by_key(|scope| scopes[scope.index()].span.start.offset);
    }

    let mut function_scopes = FxHashMap::default();
    for (key, binding_ids) in bindings_by_parent_and_name {
        let Some(scope_ids) = scopes_by_parent_and_name.get(&key) else {
            continue;
        };
        for (binding_id, scope_id) in binding_ids.into_iter().zip(scope_ids.iter().copied()) {
            function_scopes.insert(binding_id, scope_id);
        }
    }
    function_scopes
}

fn resolved_calls_by_scope(
    scopes: &[Scope],
    bindings: &[Binding],
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    function_scopes: &FxHashMap<BindingId, ScopeId>,
) -> FxHashMap<ScopeId, Vec<ResolvedCallSite>> {
    let mut calls_by_scope: FxHashMap<ScopeId, Vec<ResolvedCallSite>> = FxHashMap::default();
    for (name, sites) in call_sites {
        for site in sites {
            let Some(function_binding) = visible_function_binding(
                scopes,
                bindings,
                name,
                site.scope,
                site.span.start.offset,
            ) else {
                continue;
            };
            let Some(callee_scope) = function_scopes.get(&function_binding).copied() else {
                continue;
            };
            calls_by_scope
                .entry(site.scope)
                .or_default()
                .push(ResolvedCallSite {
                    offset: site.span.start.offset,
                    callee_scope,
                });
        }
    }
    for calls in calls_by_scope.values_mut() {
        calls.sort_by_key(|call| call.offset);
    }
    calls_by_scope
}

fn visible_function_binding(
    scopes: &[Scope],
    bindings: &[Binding],
    name: &Name,
    scope: ScopeId,
    offset: usize,
) -> Option<BindingId> {
    for scope_id in ancestor_scopes(scopes, scope) {
        let Some(candidates) = scopes[scope_id.index()].bindings.get(name) else {
            continue;
        };
        for binding in candidates.iter().rev().copied() {
            let candidate = &bindings[binding.index()];
            if matches!(candidate.kind, BindingKind::FunctionDefinition)
                && candidate.span.start.offset <= offset
            {
                return Some(binding);
            }
        }
    }
    None
}

fn references_by_scope(references: &[Reference]) -> FxHashMap<ScopeId, Vec<ScopedName>> {
    let mut refs_by_scope: FxHashMap<ScopeId, Vec<ScopedName>> = FxHashMap::default();
    for reference in references {
        refs_by_scope
            .entry(reference.scope)
            .or_default()
            .push(ScopedName {
                offset: reference.span.start.offset,
                name: reference.name.clone(),
            });
    }
    for references in refs_by_scope.values_mut() {
        references.sort_by_key(|reference| reference.offset);
    }
    refs_by_scope
}

fn names_after_offset(
    scopes: &[Scope],
    scope: ScopeId,
    offset: usize,
    refs_by_scope: &FxHashMap<ScopeId, Vec<ScopedName>>,
    calls_by_scope: &FxHashMap<ScopeId, Vec<ResolvedCallSite>>,
    transitive_reads: &FxHashMap<ScopeId, FxHashSet<Name>>,
    escape_reads: &FxHashMap<ScopeId, FxHashSet<Name>>,
) -> FxHashSet<Name> {
    let mut names = refs_by_scope
        .get(&scope)
        .into_iter()
        .flatten()
        .filter(|reference| reference.offset > offset)
        .map(|reference| reference.name.clone())
        .collect::<FxHashSet<_>>();

    if let Some(calls) = calls_by_scope.get(&scope) {
        for call in calls {
            if call.offset > offset {
                names.extend(
                    transitive_reads
                        .get(&call.callee_scope)
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
        }
    }

    if matches!(scopes[scope.index()].kind, ScopeKind::Function(_)) {
        names.extend(escape_reads.get(&scope).into_iter().flatten().cloned());
    }

    names
}

fn is_function_escape_candidate(binding: &Binding, scopes: &[Scope]) -> bool {
    matches!(scopes[binding.scope.index()].kind, ScopeKind::Function(_))
        && !binding.attributes.contains(BindingAttributes::LOCAL)
        && !matches!(
            binding.kind,
            BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref
        )
}

fn ancestor_scopes(scopes: &[Scope], start: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
    std::iter::successors(Some(start), move |scope| scopes[scope.index()].parent)
}
