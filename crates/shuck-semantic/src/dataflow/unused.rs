use super::*;

/// Result of the unused-assignment analysis before callers choose their shape.
///
/// Linter-facing code usually wants just binding ids, while tests also assert
/// the reason. Keeping both here avoids rerunning the same backward liveness
/// pass for snippets such as:
///
/// ```sh
/// tmp=old
/// tmp=new
/// printf '%s\n' "$tmp"
/// ```
///
/// where the first write is unused because every live path sees the overwrite
/// before any read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnusedAssignmentsResult {
    pub(super) unused_assignments: Vec<UnusedAssignment>,
    pub(super) unused_assignment_ids: Vec<BindingId>,
}

fn build_bindings_by_name(bindings: &[Binding]) -> FxHashMap<Name, SmallVec<[BindingId; 2]>> {
    let mut bindings_by_name: FxHashMap<Name, SmallVec<[BindingId; 2]>> = FxHashMap::default();
    for binding in bindings {
        bindings_by_name
            .entry(binding.name.clone())
            .or_default()
            .push(binding.id);
    }
    bindings_by_name
}

pub(super) fn analyze_unused_assignments_exact(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    options: UnusedAssignmentAnalysisOptions,
) -> UnusedAssignmentsResult {
    let reference_name_ids = context
        .references
        .iter()
        .map(|reference| {
            let Some(name_id) = exact.names.get(&reference.name) else {
                unreachable!("reference name interned");
            };
            name_id
        })
        .collect::<Vec<_>>();
    let synthetic_read_name_ids = context
        .synthetic_reads
        .iter()
        .map(|read| {
            let Some(name_id) = exact.names.get(&read.name) else {
                unreachable!("synthetic read name interned");
            };
            name_id
        })
        .collect::<Vec<_>>();
    let (read_plans, callers_by_callee) = build_scope_read_plans(
        context.cfg,
        context.scopes,
        context.references,
        context.synthetic_reads,
        &exact.reference_blocks,
        &reference_name_ids,
        &synthetic_read_name_ids,
        context.call_sites,
        context.visible_function_call_bindings,
        context.function_body_scopes,
        exact.names.len(),
    );
    let transitive_reads =
        compute_transitive_read_sets(&read_plans, context.scopes, exact.names.len());

    let mut used_bindings = DenseBitSet::new(context.bindings.len());
    for binding in context.bindings {
        if !binding.references.is_empty()
            || binding
                .attributes
                .contains(BindingAttributes::SELF_REFERENTIAL_READ)
            || binding
                .attributes
                .contains(BindingAttributes::EXTERNALLY_CONSUMED)
            || context.runtime.is_always_used_binding(&binding.name)
        {
            used_bindings.insert(binding.id.index());
        }
    }

    mark_used_bindings_with_backward_liveness(
        context,
        exact,
        options,
        &reference_name_ids,
        &synthetic_read_name_ids,
        &read_plans,
        &transitive_reads,
        &mut used_bindings,
    );

    if context.bindings.iter().any(|binding| {
        is_function_escape_candidate(binding, context.scopes)
            || resolved_binding_shadows_name_without_initializing(Some(binding))
    }) {
        let compatibility_reads = compute_compatibility_read_sets(
            &read_plans,
            &callers_by_callee,
            &transitive_reads,
            exact.names.len(),
        );
        let next_local_shadows = next_shadowing_local_declarations(context.bindings);
        for binding in context.bindings {
            if is_function_escape_candidate(binding, context.scopes)
                && binding_has_future_reads_before_local_shadow(
                    binding,
                    exact.binding_data.binding_name_ids[binding.id.index()],
                    context.bindings,
                    &next_local_shadows,
                    context.cfg,
                    &exact.binding_blocks,
                    &read_plans,
                    &transitive_reads,
                    &compatibility_reads.future_reads,
                    &compatibility_reads.escape_reads,
                )
            {
                used_bindings.insert(binding.id.index());
            }
            if resolved_binding_shadows_name_without_initializing(Some(binding))
                && binding_has_future_reads_before_local_shadow(
                    binding,
                    exact.binding_data.binding_name_ids[binding.id.index()],
                    context.bindings,
                    &next_local_shadows,
                    context.cfg,
                    &exact.binding_blocks,
                    &read_plans,
                    &transitive_reads,
                    &compatibility_reads.future_reads,
                    &compatibility_reads.escape_reads,
                )
            {
                used_bindings.insert(binding.id.index());
            }
        }
    }

    let mut unused_assignments = Vec::new();
    for binding in context.bindings {
        let Some(block_id) = exact.binding_blocks[binding.id.index()] else {
            continue;
        };
        if matches!(
            binding.kind,
            BindingKind::FunctionDefinition | BindingKind::Imported
        ) || context.runtime.is_always_used_binding(&binding.name)
            || (exact.unreachable_blocks.contains(block_id.index())
                && !options.report_unreachable_assignments)
            || used_bindings.contains(binding.id.index())
        {
            continue;
        }

        let reason = exact.binding_data.next_overwrite[binding.id.index()]
            .map(|by| UnusedReason::Overwritten { by })
            .unwrap_or(UnusedReason::ScopeEnd);
        if binding
            .attributes
            .contains(BindingAttributes::EMPTY_INITIALIZER)
            && let UnusedReason::Overwritten { by } = reason
            && (binding.attributes.contains(BindingAttributes::LOCAL)
                || exact.binding_blocks[by.index()].is_some_and(|overwrite_block| {
                    is_straight_line_overwrite(context.cfg, block_id, overwrite_block)
                }))
        {
            continue;
        }
        unused_assignments.push(UnusedAssignment {
            binding: binding.id,
            reason,
        });
    }
    let no_unreachable_blocks = DenseBitSet::new(context.cfg.blocks().len());
    let unreachable_blocks = if options.report_unreachable_assignments {
        &no_unreachable_blocks
    } else {
        &exact.unreachable_blocks
    };
    let unused_assignment_ids = collapse_redundant_branch_unused_assignment_ids(
        context.cfg,
        context.bindings,
        &exact.binding_blocks,
        unreachable_blocks,
        &unused_assignments,
    );

    UnusedAssignmentsResult {
        unused_assignments,
        unused_assignment_ids,
    }
}

fn collapse_redundant_branch_unused_assignment_ids(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_blocks: &[Option<BlockId>],
    unreachable_blocks: &DenseBitSet,
    unused_assignments: &[UnusedAssignment],
) -> Vec<BindingId> {
    if unused_assignments.len() < 2 {
        return unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect();
    }

    if cfg_has_no_branching_edges(cfg) {
        return unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect();
    }

    let bindings_by_name = build_bindings_by_name(bindings);
    let unused_binding_ids = unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<FxHashSet<_>>();
    let mut reachability_cache = vec![None; cfg.blocks().len()];
    let mut suppression_context = RedundantBranchUnusedAssignmentContext {
        cfg,
        bindings,
        bindings_by_name: &bindings_by_name,
        binding_blocks,
        unreachable_blocks,
        unused_binding_ids: &unused_binding_ids,
        reachability_cache: &mut reachability_cache,
    };

    unused_assignments
        .iter()
        .filter_map(|unused| {
            (!should_suppress_redundant_branch_unused_assignment(
                unused.binding,
                &mut suppression_context,
            ))
            .then_some(unused.binding)
        })
        .collect()
}

fn cfg_has_no_branching_edges(cfg: &ControlFlowGraph) -> bool {
    cfg.blocks().iter().all(|block| {
        cfg.predecessors(block.id).len() <= 1
            && cfg.successors(block.id).len() <= 1
            && cfg
                .successors(block.id)
                .iter()
                .all(|(_, edge)| matches!(edge, EdgeKind::Sequential))
    })
}

struct RedundantBranchUnusedAssignmentContext<'a> {
    cfg: &'a ControlFlowGraph,
    bindings: &'a [Binding],
    bindings_by_name: &'a FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    binding_blocks: &'a [Option<BlockId>],
    unreachable_blocks: &'a DenseBitSet,
    unused_binding_ids: &'a FxHashSet<BindingId>,
    reachability_cache: &'a mut [Option<DenseBitSet>],
}

fn should_suppress_redundant_branch_unused_assignment(
    binding_id: BindingId,
    context: &mut RedundantBranchUnusedAssignmentContext<'_>,
) -> bool {
    let binding = &context.bindings[binding_id.index()];
    if !participates_in_unused_assignment_family(binding.kind, binding.attributes) {
        return false;
    }

    let Some(binding_block) = context.binding_blocks[binding_id.index()] else {
        return false;
    };
    let Some(later_bindings) = context.bindings_by_name.get(&binding.name) else {
        return false;
    };

    let mut later_participants = later_bindings
        .iter()
        .copied()
        .filter(|candidate_id| candidate_id.index() > binding_id.index())
        .filter(|candidate_id| {
            let candidate = &context.bindings[candidate_id.index()];
            candidate.scope == binding.scope
                && participates_in_unused_assignment_family(candidate.kind, candidate.attributes)
        })
        .filter_map(|candidate_id| {
            let candidate_block = context.binding_blocks[candidate_id.index()]?;
            (!context.unreachable_blocks.contains(candidate_block.index()))
                .then_some((candidate_id, candidate_block))
        });

    let Some((next_binding_id, next_binding_block)) = later_participants.next() else {
        return false;
    };

    if block_can_reach(
        context.cfg,
        binding_block,
        next_binding_block,
        context.reachability_cache,
    ) {
        return false;
    }

    let next_binding = &context.bindings[next_binding_id.index()];
    if !context.unused_binding_ids.contains(&next_binding_id)
        || !can_survive_unused_assignment_branch_collapse(
            next_binding.kind,
            next_binding.attributes,
        )
    {
        return false;
    }

    if later_participants
        .any(|(candidate_id, _)| !context.unused_binding_ids.contains(&candidate_id))
    {
        return false;
    }

    true
}

fn participates_in_unused_assignment_family(
    kind: BindingKind,
    _attributes: BindingAttributes,
) -> bool {
    match kind {
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
        | BindingKind::ArithmeticAssignment
        | BindingKind::Declaration(_) => true,
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn can_survive_unused_assignment_branch_collapse(
    kind: BindingKind,
    attributes: BindingAttributes,
) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ZparseoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::Imported
        | BindingKind::Nameref => false,
    }
}

fn resolved_binding_shadows_name_without_initializing(binding: Option<&Binding>) -> bool {
    matches!(
        binding,
        Some(binding)
            if matches!(binding.kind, BindingKind::Declaration(_))
                && !binding
                    .attributes
                    .contains(BindingAttributes::DECLARATION_INITIALIZED)
    )
}

fn block_can_reach(
    cfg: &ControlFlowGraph,
    from: BlockId,
    to: BlockId,
    reachability_cache: &mut [Option<DenseBitSet>],
) -> bool {
    if from == to {
        return true;
    }

    if cfg
        .successors(from)
        .iter()
        .any(|(successor, _)| *successor == to)
    {
        return true;
    }

    if let Some(reachable) = &reachability_cache[from.index()] {
        return reachable.contains(to.index());
    }

    let mut reachable = DenseBitSet::new(cfg.blocks().len());
    let mut stack = vec![from];
    while let Some(block_id) = stack.pop() {
        for &(successor, _) in cfg.successors(block_id) {
            if reachable.contains(successor.index()) {
                continue;
            }
            reachable.insert(successor.index());
            stack.push(successor);
        }
    }

    let can_reach = reachable.contains(to.index());
    reachability_cache[from.index()] = Some(reachable);
    can_reach
}

fn is_straight_line_overwrite(cfg: &ControlFlowGraph, from: BlockId, to: BlockId) -> bool {
    if from == to {
        return true;
    }

    let mut current = from;
    let mut visited = DenseBitSet::new(cfg.blocks().len());
    visited.insert(current.index());
    loop {
        let successors = cfg.successors(current);
        if successors.len() != 1 {
            return false;
        }

        let (next, edge) = successors[0];
        if !matches!(edge, EdgeKind::Sequential) {
            return false;
        }
        if cfg.predecessors(next).len() != 1 {
            return false;
        }
        if next == to {
            return true;
        }
        if visited.contains(next.index()) {
            return false;
        }
        visited.insert(next.index());
        current = next;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SlotId(u32);

impl SlotId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone)]
struct UnusedAssignmentSlots {
    binding_slots: Vec<SlotId>,
    slots_for_name: Vec<SlotId>,
}

impl UnusedAssignmentSlots {
    fn new(binding_name_ids: &[NameId], name_count: usize) -> Self {
        let slots_for_name = (0..name_count)
            .map(|index| SlotId(index as u32))
            .collect::<Vec<_>>();
        let binding_slots = binding_name_ids
            .iter()
            .map(|name| slots_for_name[name.index()])
            .collect::<Vec<_>>();

        Self {
            binding_slots,
            slots_for_name,
        }
    }

    fn len(&self) -> usize {
        self.slots_for_name.len()
    }

    fn slot_for_name(&self, name: NameId) -> SlotId {
        self.slots_for_name[name.index()]
    }

    fn slot_for_binding(&self, binding: BindingId) -> SlotId {
        self.binding_slots[binding.index()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnusedAssignmentEvent {
    offset: usize,
    order: u8,
    kind: UnusedAssignmentEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnusedAssignmentEventKind {
    Reference(ReferenceId),
    SyntheticRead(usize),
    Binding(BindingId),
    Call(ScopeId),
    FunctionDefinition(ScopeId),
}

#[derive(Debug, Clone)]
struct SlotLiveSet {
    words_per_set: usize,
    inline: usize,
    heap: Vec<usize>,
}

impl SlotLiveSet {
    fn new(bit_len: usize) -> Self {
        let words_per_set = bit_len.div_ceil(DenseBitSet::WORD_BITS);
        Self {
            words_per_set,
            inline: 0,
            heap: if words_per_set > 1 {
                vec![0; words_per_set]
            } else {
                Vec::new()
            },
        }
    }

    fn as_slice(&self) -> &[usize] {
        match self.words_per_set {
            0 => &[],
            1 => std::slice::from_ref(&self.inline),
            _ => &self.heap,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [usize] {
        match self.words_per_set {
            0 => &mut self.heap,
            1 => std::slice::from_mut(&mut self.inline),
            _ => &mut self.heap,
        }
    }

    fn clear(&mut self) {
        if self.words_per_set == 1 {
            self.inline = 0;
        } else {
            self.heap.fill(0);
        }
    }

    fn copy_from_slice(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words_per_set, words.len());
        self.as_mut_slice().copy_from_slice(words);
    }

    fn union_with_slice(&mut self, words: &[usize]) {
        debug_assert_eq!(self.words_per_set, words.len());
        for (destination, source) in self.as_mut_slice().iter_mut().zip(words) {
            *destination |= *source;
        }
    }

    fn insert(&mut self, index: usize) {
        let word = index / DenseBitSet::WORD_BITS;
        let bit = index % DenseBitSet::WORD_BITS;
        if self.words_per_set == 1 {
            self.inline |= 1usize << bit;
        } else if let Some(word) = self.heap.get_mut(word) {
            *word |= 1usize << bit;
        }
    }

    fn contains(&self, index: usize) -> bool {
        let word = index / DenseBitSet::WORD_BITS;
        let bit = index % DenseBitSet::WORD_BITS;
        if self.words_per_set == 1 {
            (self.inline & (1usize << bit)) != 0
        } else {
            self.heap
                .get(word)
                .is_some_and(|word| (word & (1usize << bit)) != 0)
        }
    }

    fn remove(&mut self, index: usize) {
        let word = index / DenseBitSet::WORD_BITS;
        let bit = index % DenseBitSet::WORD_BITS;
        if self.words_per_set == 1 {
            self.inline &= !(1usize << bit);
        } else if let Some(word) = self.heap.get_mut(word) {
            *word &= !(1usize << bit);
        }
    }
}

#[derive(Debug, Clone)]
struct SlotLiveMatrix {
    words_per_set: usize,
    words: Vec<usize>,
}

impl SlotLiveMatrix {
    fn new(set_count: usize, bit_len: usize) -> Self {
        let words_per_set = bit_len.div_ceil(DenseBitSet::WORD_BITS);
        Self {
            words_per_set,
            words: vec![0; set_count * words_per_set],
        }
    }

    fn set(&self, index: usize) -> &[usize] {
        let start = index * self.words_per_set;
        let end = start + self.words_per_set;
        &self.words[start..end]
    }

    fn replace_if_changed(&mut self, index: usize, source: &SlotLiveSet) -> bool {
        debug_assert_eq!(self.words_per_set, source.words_per_set);
        let start = index * self.words_per_set;
        let end = start + self.words_per_set;
        let destination = &mut self.words[start..end];
        let source = source.as_slice();
        if destination == source {
            false
        } else {
            destination.copy_from_slice(source);
            true
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mark_used_bindings_with_backward_liveness(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    options: UnusedAssignmentAnalysisOptions,
    reference_name_ids: &[NameId],
    synthetic_read_name_ids: &[NameId],
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
    used_bindings: &mut DenseBitSet,
) {
    let slots = UnusedAssignmentSlots::new(&exact.binding_data.binding_name_ids, exact.names.len());
    let events = build_unused_assignment_events(context, exact, read_plans);
    let block_count = context.cfg.blocks().len();
    let mut live_in = SlotLiveMatrix::new(block_count, slots.len());
    let mut live_out = SlotLiveMatrix::new(block_count, slots.len());
    let mut outgoing = SlotLiveSet::new(slots.len());
    let mut incoming = SlotLiveSet::new(slots.len());
    let backward_order = exact.backward_block_order(context.cfg);

    run_backward_dataflow_worklist(context.cfg, backward_order, |block_id| {
        let block_index = block_id.index();
        outgoing.clear();
        for (successor, _) in context.cfg.successors(block_id) {
            outgoing.union_with_slice(live_in.set(successor.index()));
        }

        incoming.copy_from_slice(outgoing.as_slice());
        if !exact.unreachable_blocks.contains(block_index) || options.report_unreachable_assignments
        {
            for event in events[block_index].iter().rev() {
                apply_unused_assignment_event(
                    context,
                    options,
                    reference_name_ids,
                    synthetic_read_name_ids,
                    transitive_reads,
                    &slots,
                    &mut incoming,
                    used_bindings,
                    event.kind,
                );
            }
        }

        live_out.replace_if_changed(block_index, &outgoing);
        live_in.replace_if_changed(block_index, &incoming)
    });
}

fn build_unused_assignment_events(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    read_plans: &[ScopeReadPlan],
) -> Vec<Vec<UnusedAssignmentEvent>> {
    let mut events = vec![Vec::new(); context.cfg.blocks().len()];

    for block in context.cfg.blocks() {
        let block_events = &mut events[block.id.index()];
        for reference_id in &block.references {
            let reference = &context.references[reference_id.index()];
            block_events.push(UnusedAssignmentEvent {
                offset: reference.span.start.offset,
                order: 0,
                kind: UnusedAssignmentEventKind::Reference(*reference_id),
            });
        }
        for binding_id in &block.bindings {
            let binding = &context.bindings[binding_id.index()];
            block_events.push(UnusedAssignmentEvent {
                offset: binding.span.start.offset,
                order: 1,
                kind: UnusedAssignmentEventKind::Binding(*binding_id),
            });
        }
    }

    for (read_index, synthetic_read) in context.synthetic_reads.iter().enumerate() {
        let Some(block_id) = command_block_for_span(context.cfg, synthetic_read.span) else {
            continue;
        };
        events[block_id.index()].push(UnusedAssignmentEvent {
            offset: synthetic_read.span.start.offset,
            order: 0,
            kind: UnusedAssignmentEventKind::SyntheticRead(read_index),
        });
    }

    for plan in read_plans {
        for call in &plan.calls {
            let Some(block_id) = command_block_for_span(context.cfg, call.span) else {
                continue;
            };
            events[block_id.index()].push(UnusedAssignmentEvent {
                offset: call.offset,
                order: 0,
                kind: UnusedAssignmentEventKind::Call(call.callee_scope),
            });
        }
    }

    for (&binding_id, &scope_id) in context.function_body_scopes {
        let Some(block_id) = exact.binding_blocks[binding_id.index()] else {
            continue;
        };
        let binding = &context.bindings[binding_id.index()];
        events[block_id.index()].push(UnusedAssignmentEvent {
            offset: binding.span.start.offset,
            order: 0,
            kind: UnusedAssignmentEventKind::FunctionDefinition(scope_id),
        });
    }

    for block_events in &mut events {
        block_events.sort_by_key(|event| (event.offset, event.order));
    }

    events
}

#[allow(clippy::too_many_arguments)]
fn apply_unused_assignment_event(
    context: &DataflowContext<'_>,
    options: UnusedAssignmentAnalysisOptions,
    reference_name_ids: &[NameId],
    synthetic_read_name_ids: &[NameId],
    transitive_reads: &[DenseBitSet],
    slots: &UnusedAssignmentSlots,
    live: &mut SlotLiveSet,
    used_bindings: &mut DenseBitSet,
    event: UnusedAssignmentEventKind,
) {
    match event {
        UnusedAssignmentEventKind::Reference(reference_id) => {
            let reference = &context.references[reference_id.index()];
            let name = reference_name_ids[reference_id.index()];
            live.insert(slots.slot_for_name(name).index());

            if (options.treat_indirect_expansion_targets_as_used
                || context
                    .array_like_indirect_expansion_refs
                    .contains(&reference.id))
                && let Some(candidates) = context.indirect_targets_by_reference.get(&reference.id)
            {
                for candidate in candidates {
                    live.insert(slots.slot_for_binding(*candidate).index());
                }
            }
        }
        UnusedAssignmentEventKind::SyntheticRead(read_index) => {
            let name = synthetic_read_name_ids[read_index];
            live.insert(slots.slot_for_name(name).index());
        }
        UnusedAssignmentEventKind::Call(callee_scope)
        | UnusedAssignmentEventKind::FunctionDefinition(callee_scope) => {
            union_name_reads_into_live_slots(live, &transitive_reads[callee_scope.index()], slots);
        }
        UnusedAssignmentEventKind::Binding(binding_id) => {
            apply_unused_assignment_binding_event(context, slots, live, used_bindings, binding_id);
        }
    }
}

fn apply_unused_assignment_binding_event(
    context: &DataflowContext<'_>,
    slots: &UnusedAssignmentSlots,
    live: &mut SlotLiveSet,
    used_bindings: &mut DenseBitSet,
    binding_id: BindingId,
) {
    let binding = &context.bindings[binding_id.index()];
    if !binding_writes_unused_assignment_slot(binding) {
        return;
    }

    let slot = slots.slot_for_binding(binding_id);
    if resolved_binding_shadows_name_without_initializing(Some(binding)) {
        if live.contains(slot.index()) {
            used_bindings.insert(binding_id.index());
        }
        return;
    }

    if live.contains(slot.index()) {
        used_bindings.insert(binding_id.index());
    }

    if matches!(binding.kind, BindingKind::AppendAssignment) {
        live.insert(slot.index());
        return;
    }

    live.remove(slot.index());
    if binding
        .attributes
        .contains(BindingAttributes::SELF_REFERENTIAL_READ)
    {
        live.insert(slot.index());
    }
}

fn binding_writes_unused_assignment_slot(binding: &Binding) -> bool {
    !matches!(
        binding.kind,
        BindingKind::FunctionDefinition | BindingKind::Imported
    ) && binding_initializes_name(binding).is_some()
}

fn union_name_reads_into_live_slots(
    live: &mut SlotLiveSet,
    reads: &DenseBitSet,
    slots: &UnusedAssignmentSlots,
) {
    for name_index in reads.iter_ones() {
        live.insert(slots.slot_for_name(NameId(name_index as u32)).index());
    }
}
