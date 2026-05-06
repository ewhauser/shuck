use super::*;

/// A resolved function call attached to the callee's scope.
///
/// Scope-read summaries need call edges in source order. In:
///
/// ```sh
/// use_flag() { printf '%s\n' "$flag"; }
/// flag=1
/// use_flag
/// ```
///
/// the call creates a future read of `flag` from the caller's scope, even
/// though the textual read is inside the function body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedCallSite {
    pub(super) offset: usize,
    pub(super) span: Span,
    pub(super) callee_scope: ScopeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopeReadEventKind {
    Direct(NameId),
    Call(ScopeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScopeReadEvent {
    pub(super) offset: usize,
    pub(super) block: Option<BlockId>,
    pub(super) kind: ScopeReadEventKind,
}

#[derive(Debug, Clone)]
pub(super) struct ScopeReadPlan {
    /// Names read directly inside the scope, ignoring calls to other scopes.
    pub(super) direct_reads: DenseBitSet,
    /// Calls made by this scope, kept in source order for future-read queries.
    pub(super) calls: Vec<ResolvedCallSite>,
    /// Interleaved direct reads and calls, also source ordered.
    pub(super) events: Vec<ScopeReadEvent>,
    /// Whether the scope is a function body rather than file or compound scope.
    pub(super) is_function: bool,
}

impl ScopeReadPlan {
    pub(super) fn new(name_count: usize, is_function: bool) -> Self {
        Self {
            direct_reads: DenseBitSet::new(name_count),
            calls: Vec::new(),
            events: Vec::new(),
            is_function,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ScopeFutureReads {
    pub(super) suffix_reads: Vec<DenseBitSet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallerReadSite {
    pub(super) caller_scope: ScopeId,
    pub(super) offset: usize,
}

#[derive(Debug, Clone)]
pub(super) struct CompatibilityReadSets {
    pub(super) escape_reads: Vec<DenseBitSet>,
    pub(super) future_reads: Vec<ScopeFutureReads>,
}

pub(crate) fn summarize_scope_provided_bindings(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    scope: ScopeId,
) -> Vec<ProvidedBinding> {
    let exit_blocks = exit_blocks_for_scope(context.cfg, exact.scope_components(context), scope);
    if exit_blocks.is_empty() {
        return Vec::new();
    }

    let eligible_names = context
        .bindings
        .iter()
        .filter(|binding| {
            binding.scope == scope
                && !binding.attributes.contains(BindingAttributes::LOCAL)
                && binding_initializes_name(binding).is_some()
        })
        .map(|binding| binding.name.clone())
        .collect::<FxHashSet<_>>();

    let initialized_states = exact.initialized_name_states(context);
    let mut maybe_counts = FxHashMap::<Name, usize>::default();
    let mut definite_counts = FxHashMap::<Name, usize>::default();

    for exit_block in &exit_blocks {
        for name in &eligible_names {
            let Some(name_id) = exact.names.get(name) else {
                continue;
            };
            if initialized_states
                .maybe_out
                .contains(exit_block.index(), name_id.index())
            {
                *maybe_counts.entry(name.clone()).or_default() += 1;
            }
            if initialized_states
                .definite_out
                .contains(exit_block.index(), name_id.index())
            {
                *definite_counts.entry(name.clone()).or_default() += 1;
            }
        }
    }

    let exit_count = exit_blocks.len();
    let mut provided = Vec::new();
    for (name, maybe_count) in maybe_counts {
        let definite_count = definite_counts.get(&name).copied().unwrap_or_default();
        let certainty = if definite_count == exit_count {
            ContractCertainty::Definite
        } else if maybe_count > 0 {
            ContractCertainty::Possible
        } else {
            continue;
        };
        provided.push(ProvidedBinding::new(
            name,
            ProvidedBindingKind::Variable,
            certainty,
        ));
    }

    provided.sort_by(|left, right| {
        left.name
            .as_str()
            .cmp(right.name.as_str())
            .then_with(|| (left.kind as u8).cmp(&(right.kind as u8)))
    });
    provided
}

pub(crate) fn summarize_scope_provided_functions(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    scope: ScopeId,
) -> Vec<ProvidedBinding> {
    let reaching_definitions = exact.reaching_definitions(context);
    let exit_blocks = exit_blocks_for_scope(context.cfg, exact.scope_components(context), scope);
    if exit_blocks.is_empty() {
        return Vec::new();
    }

    let eligible_names = context
        .bindings
        .iter()
        .filter(|binding| binding.scope == scope && function_binding_certainty(binding).is_some())
        .map(|binding| binding.name.clone())
        .collect::<FxHashSet<_>>();

    let mut maybe_counts = FxHashMap::<Name, usize>::default();
    let mut definite_counts = FxHashMap::<Name, usize>::default();

    for exit_block in &exit_blocks {
        let reaching = &reaching_definitions.reaching_out[exit_block.index()];

        for name in &eligible_names {
            let Some(name_id) = exact.names.get(name) else {
                continue;
            };
            let mut maybe_present = false;
            let mut definite_present = false;
            for binding_index in exact.binding_data.bindings_for_name[name_id.index()].iter_ones() {
                if !reaching.contains(binding_index) {
                    continue;
                }
                let binding = &context.bindings[binding_index];
                if binding.scope != scope {
                    continue;
                }
                let Some(certainty) = function_binding_certainty(binding) else {
                    continue;
                };
                maybe_present = true;
                definite_present |= certainty == ContractCertainty::Definite;
            }
            if maybe_present {
                *maybe_counts.entry(name.clone()).or_default() += 1;
            }
            if definite_present {
                *definite_counts.entry(name.clone()).or_default() += 1;
            }
        }
    }

    let exit_count = exit_blocks.len();
    let mut provided = Vec::new();
    for (name, maybe_count) in maybe_counts {
        let definite_count = definite_counts.get(&name).copied().unwrap_or_default();
        let certainty = if definite_count == exit_count {
            ContractCertainty::Definite
        } else if maybe_count > 0 {
            ContractCertainty::Possible
        } else {
            continue;
        };
        provided.push(ProvidedBinding::new(
            name,
            ProvidedBindingKind::Function,
            certainty,
        ));
    }

    provided.sort_by(|left, right| {
        left.name
            .as_str()
            .cmp(right.name.as_str())
            .then_with(|| (left.kind as u8).cmp(&(right.kind as u8)))
    });
    provided
}

fn exit_blocks_for_scope(
    cfg: &ControlFlowGraph,
    scope_components: &[ExactScopeComponent],
    scope: ScopeId,
) -> Vec<BlockId> {
    let component = &scope_components[scope.index()];
    if let Some(scope_exits) = cfg.scope_exits(scope) {
        return scope_exits.to_vec();
    }

    component
        .blocks
        .iter_ones()
        .filter_map(|block_index| {
            let block_id = BlockId(block_index as u32);
            block_exits_component(cfg, &component.blocks, block_id).then_some(block_id)
        })
        .collect()
}

pub(super) fn reachable_blocks_dense(
    cfg: &ControlFlowGraph,
    entry: BlockId,
    block_count: usize,
) -> DenseBitSet {
    let mut visited = DenseBitSet::new(block_count);
    let mut stack = vec![entry];
    while let Some(block_id) = stack.pop() {
        if visited.contains(block_id.index()) {
            continue;
        }
        visited.insert(block_id.index());
        stack.extend(
            cfg.successors(block_id)
                .iter()
                .map(|(successor, _)| *successor),
        );
    }
    visited
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_scope_read_plans(
    cfg: &ControlFlowGraph,
    scopes: &[Scope],
    references: &[Reference],
    synthetic_reads: &[SyntheticRead],
    reference_blocks: &[Option<BlockId>],
    reference_name_ids: &[NameId],
    synthetic_read_name_ids: &[NameId],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
    function_body_scopes: &FxHashMap<BindingId, ScopeId>,
    name_count: usize,
) -> (Vec<ScopeReadPlan>, Vec<Vec<CallerReadSite>>) {
    let calls_by_scope = resolved_calls_by_scope(
        call_sites,
        visible_function_call_bindings,
        function_body_scopes,
    );
    let mut plans = scopes
        .iter()
        .map(|scope| ScopeReadPlan::new(name_count, matches!(scope.kind, ScopeKind::Function(_))))
        .collect::<Vec<_>>();
    let mut callers_by_callee = vec![Vec::new(); scopes.len()];

    for (reference_index, reference) in references.iter().enumerate() {
        let plan = &mut plans[reference.scope.index()];
        let name_id = reference_name_ids[reference_index];
        plan.direct_reads.insert(name_id.index());
        plan.events.push(ScopeReadEvent {
            offset: reference.span.start.offset,
            block: reference_blocks[reference_index],
            kind: ScopeReadEventKind::Direct(name_id),
        });
    }

    for (read_index, synthetic_read) in synthetic_reads.iter().enumerate() {
        let plan = &mut plans[synthetic_read.scope.index()];
        let name_id = synthetic_read_name_ids[read_index];
        plan.direct_reads.insert(name_id.index());
        plan.events.push(ScopeReadEvent {
            offset: synthetic_read.span.start.offset,
            block: command_block_for_span(cfg, synthetic_read.span),
            kind: ScopeReadEventKind::Direct(name_id),
        });
    }

    for (scope_id, calls) in calls_by_scope {
        let plan = &mut plans[scope_id.index()];
        for call in &calls {
            callers_by_callee[call.callee_scope.index()].push(CallerReadSite {
                caller_scope: scope_id,
                offset: call.offset,
            });
            plan.events.push(ScopeReadEvent {
                offset: call.offset,
                block: command_block_for_span(cfg, call.span),
                kind: ScopeReadEventKind::Call(call.callee_scope),
            });
        }
        plan.calls = calls;
    }

    for plan in &mut plans {
        plan.events.sort_by_key(|event| event.offset);
    }

    (plans, callers_by_callee)
}

pub(super) fn compute_transitive_read_sets(
    read_plans: &[ScopeReadPlan],
    scopes: &[Scope],
    name_count: usize,
) -> Vec<DenseBitSet> {
    let nested_child_scopes = nested_non_function_child_scopes(scopes);
    let mut transitive_reads = vec![DenseBitSet::new(name_count); read_plans.len()];
    let mut reads = DenseBitSet::new(name_count);
    loop {
        let mut changed = false;
        for (scope_index, plan) in read_plans.iter().enumerate() {
            reads.copy_from(&plan.direct_reads);
            for &child_scope in &nested_child_scopes[scope_index] {
                reads.union_with(&transitive_reads[child_scope.index()]);
            }
            for call in &plan.calls {
                reads.union_with(&transitive_reads[call.callee_scope.index()]);
            }
            if transitive_reads[scope_index].replace_if_changed(&reads) {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    transitive_reads
}

pub(super) fn compute_compatibility_read_sets(
    read_plans: &[ScopeReadPlan],
    callers_by_callee: &[Vec<CallerReadSite>],
    transitive_reads: &[DenseBitSet],
    name_count: usize,
) -> CompatibilityReadSets {
    let future_reads = build_future_read_summaries(read_plans, transitive_reads, name_count);
    let mut escape_reads = vec![DenseBitSet::new(name_count); read_plans.len()];
    let mut reads = DenseBitSet::new(name_count);
    loop {
        let mut changed = false;
        for (scope_index, plan) in read_plans.iter().enumerate() {
            if !plan.is_function {
                continue;
            }

            reads.clear();
            for caller in &callers_by_callee[scope_index] {
                future_reads_union_after(
                    &mut reads,
                    caller.caller_scope,
                    caller.offset,
                    read_plans,
                    &future_reads,
                    &escape_reads,
                );
            }
            if escape_reads[scope_index].replace_if_changed(&reads) {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    CompatibilityReadSets {
        escape_reads,
        future_reads,
    }
}

fn nested_non_function_child_scopes(scopes: &[Scope]) -> Vec<Vec<ScopeId>> {
    let mut children = vec![Vec::new(); scopes.len()];
    for scope in scopes {
        let Some(parent) = scope.parent else {
            continue;
        };
        if matches!(scope.kind, ScopeKind::Function(_)) {
            continue;
        }
        children[parent.index()].push(scope.id);
    }
    children
}

fn build_future_read_summaries(
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
    name_count: usize,
) -> Vec<ScopeFutureReads> {
    read_plans
        .iter()
        .map(|plan| {
            let mut suffix_reads = vec![DenseBitSet::new(name_count); plan.events.len() + 1];
            for event_index in (0..plan.events.len()).rev() {
                let (current_and_before, next_and_after) =
                    suffix_reads.split_at_mut(event_index + 1);
                let current = &mut current_and_before[event_index];
                current.copy_from(&next_and_after[0]);
                match plan.events[event_index].kind {
                    ScopeReadEventKind::Direct(name_id) => {
                        current.insert(name_id.index());
                    }
                    ScopeReadEventKind::Call(callee_scope) => {
                        current.union_with(&transitive_reads[callee_scope.index()]);
                    }
                }
            }
            ScopeFutureReads { suffix_reads }
        })
        .collect()
}

fn future_reads_union_after(
    destination: &mut DenseBitSet,
    scope: ScopeId,
    offset: usize,
    read_plans: &[ScopeReadPlan],
    future_reads: &[ScopeFutureReads],
    escape_reads: &[DenseBitSet],
) {
    let plan = &read_plans[scope.index()];
    let index = plan.events.partition_point(|event| event.offset <= offset);
    destination.union_with(&future_reads[scope.index()].suffix_reads[index]);
    if plan.is_function {
        destination.union_with(&escape_reads[scope.index()]);
    }
}

fn future_reads_contain_after(
    scope: ScopeId,
    offset: usize,
    name_id: NameId,
    read_plans: &[ScopeReadPlan],
    future_reads: &[ScopeFutureReads],
    escape_reads: &[DenseBitSet],
) -> bool {
    let plan = &read_plans[scope.index()];
    let index = plan.events.partition_point(|event| event.offset <= offset);
    future_reads[scope.index()].suffix_reads[index].contains(name_id.index())
        || (plan.is_function && escape_reads[scope.index()].contains(name_id.index()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn binding_has_future_reads_before_local_shadow(
    binding: &Binding,
    name_id: NameId,
    bindings: &[Binding],
    next_local_shadows: &[Option<BindingId>],
    cfg: &ControlFlowGraph,
    binding_blocks: &[Option<BlockId>],
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
    future_reads: &[ScopeFutureReads],
    escape_reads: &[DenseBitSet],
) -> bool {
    let shadow = next_local_shadows[binding.id.index()].map(|shadow| &bindings[shadow.index()]);
    let escape_reads_visible = read_plans[binding.scope.index()].is_function
        && escape_reads[binding.scope.index()].contains(name_id.index());

    if let Some(shadow) = shadow {
        if let (Some(binding_block), Some(shadow_block)) = (
            binding_blocks[binding.id.index()],
            binding_blocks[shadow.id.index()],
        ) {
            escape_reads_visible
                || future_reads_contain_after_without_shadow(
                    binding.scope,
                    binding.span.start.offset,
                    shadow.span.start.offset,
                    binding_block,
                    shadow_block,
                    name_id,
                    read_plans,
                    transitive_reads,
                    cfg,
                )
        } else {
            escape_reads_visible
                || future_reads_contain_after_until(
                    binding.scope,
                    binding.span.start.offset,
                    shadow.span.start.offset,
                    name_id,
                    read_plans,
                    transitive_reads,
                )
        }
    } else {
        future_reads_contain_after(
            binding.scope,
            binding.span.start.offset,
            name_id,
            read_plans,
            future_reads,
            escape_reads,
        )
    }
}

pub(super) fn next_shadowing_local_declarations(bindings: &[Binding]) -> Vec<Option<BindingId>> {
    let mut next_shadows = vec![None; bindings.len()];
    let mut next_by_scope_and_name: FxHashMap<(ScopeId, Name), BindingId> = FxHashMap::default();

    for binding in bindings.iter().rev() {
        let key = (binding.scope, binding.name.clone());
        next_shadows[binding.id.index()] = next_by_scope_and_name.get(&key).copied();

        if matches!(binding.kind, BindingKind::Declaration(_))
            && binding.attributes.contains(BindingAttributes::LOCAL)
        {
            next_by_scope_and_name.insert(key, binding.id);
        }
    }

    next_shadows
}

pub(super) fn future_reads_contain_after_until(
    scope: ScopeId,
    after_offset: usize,
    before_offset: usize,
    name_id: NameId,
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
) -> bool {
    if before_offset <= after_offset {
        return false;
    }

    let plan = &read_plans[scope.index()];
    let start = plan
        .events
        .partition_point(|event| event.offset <= after_offset);
    let end = plan
        .events
        .partition_point(|event| event.offset < before_offset);

    plan.events[start..end]
        .iter()
        .any(|event| match event.kind {
            ScopeReadEventKind::Direct(candidate) => candidate == name_id,
            ScopeReadEventKind::Call(callee_scope) => {
                transitive_reads[callee_scope.index()].contains(name_id.index())
            }
        })
}

#[allow(clippy::too_many_arguments)]
fn future_reads_contain_after_without_shadow(
    scope: ScopeId,
    after_offset: usize,
    shadow_offset: usize,
    binding_block: BlockId,
    shadow_block: BlockId,
    name_id: NameId,
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
    cfg: &ControlFlowGraph,
) -> bool {
    let plan = &read_plans[scope.index()];
    let start = plan
        .events
        .partition_point(|event| event.offset <= after_offset);

    plan.events[start..].iter().any(|event| {
        let uses_name = match event.kind {
            ScopeReadEventKind::Direct(candidate) => candidate == name_id,
            ScopeReadEventKind::Call(callee_scope) => {
                transitive_reads[callee_scope.index()].contains(name_id.index())
            }
        };
        if !uses_name {
            return false;
        }
        if event.offset < shadow_offset {
            return true;
        }

        let Some(event_block) = event.block else {
            return true;
        };
        if event_block == shadow_block {
            return false;
        }

        block_reaches_without(cfg, binding_block, event_block, shadow_block)
    })
}

pub(super) fn binding_initializes_name(binding: &Binding) -> Option<ContractCertainty> {
    match binding.kind {
        BindingKind::Declaration(_) | BindingKind::Nameref => binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
            .then_some(ContractCertainty::Definite),
        BindingKind::FunctionDefinition => None,
        BindingKind::Imported => {
            if binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FUNCTION)
            {
                None
            } else if binding
                .attributes
                .contains(BindingAttributes::IMPORTED_POSSIBLE)
            {
                Some(ContractCertainty::Possible)
            } else {
                Some(ContractCertainty::Definite)
            }
        }
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment => Some(ContractCertainty::Definite),
    }
}

fn function_binding_certainty(binding: &Binding) -> Option<ContractCertainty> {
    match binding.kind {
        BindingKind::FunctionDefinition => Some(ContractCertainty::Definite),
        BindingKind::Imported
            if binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FUNCTION) =>
        {
            if binding
                .attributes
                .contains(BindingAttributes::IMPORTED_POSSIBLE)
            {
                Some(ContractCertainty::Possible)
            } else {
                Some(ContractCertainty::Definite)
            }
        }
        _ => None,
    }
}

pub(super) fn resolved_calls_by_scope(
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    visible_function_call_bindings: &FxHashMap<SpanKey, BindingId>,
    function_scopes: &FxHashMap<BindingId, ScopeId>,
) -> FxHashMap<ScopeId, Vec<ResolvedCallSite>> {
    let mut calls_by_scope: FxHashMap<ScopeId, Vec<ResolvedCallSite>> = FxHashMap::default();
    for call in resolved_function_calls_with_callee_scope(
        call_sites,
        visible_function_call_bindings,
        function_scopes,
    ) {
        calls_by_scope
            .entry(call.site.scope)
            .or_default()
            .push(ResolvedCallSite {
                offset: call.site.span.start.offset,
                span: call.site.span,
                callee_scope: call.callee_scope,
            });
    }
    for calls in calls_by_scope.values_mut() {
        calls.sort_by_key(|call| call.offset);
    }
    calls_by_scope
}

pub(super) fn is_function_escape_candidate(binding: &Binding, scopes: &[Scope]) -> bool {
    matches!(scopes[binding.scope.index()].kind, ScopeKind::Function(_))
        && !binding.attributes.contains(BindingAttributes::LOCAL)
        && !matches!(
            binding.kind,
            BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref
        )
}
