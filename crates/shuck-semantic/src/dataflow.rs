use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_ast::Span;

use crate::runtime::RuntimePrelude;
use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BlockId, CallSite, ControlFlowGraph,
    Reference, ReferenceId, ReferenceKind, Scope, ScopeId, ScopeKind, SpanKey, SyntheticRead,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnusedAssignmentsResult {
    unused_assignments: Vec<UnusedAssignment>,
    unused_assignment_ids: Vec<BindingId>,
}

#[derive(Debug, Clone, Default)]
struct BindingNameData {
    bindings_by_name: FxHashMap<Name, Vec<BindingId>>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn analyze_unused_assignments(
    cfg: &ControlFlowGraph,
    runtime: &RuntimePrelude,
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    indirect_targets_by_reference: &[Vec<BindingId>],
    synthetic_reads: &[SyntheticRead],
) -> Vec<BindingId> {
    analyze_unused_assignments_exact(
        cfg,
        runtime,
        scopes,
        bindings,
        references,
        resolved,
        call_sites,
        indirect_targets_by_reference,
        synthetic_reads,
    )
    .unused_assignment_ids
}

pub(crate) fn analyze_uninitialized_references(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    references: &[Reference],
    predefined_runtime_refs: &FxHashSet<ReferenceId>,
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_targets_by_reference: &[Vec<BindingId>],
) -> Vec<UninitializedReference> {
    let names = build_uninitialized_name_table(bindings, references);
    let binding_data = build_dense_binding_data_for_scope_count(
        bindings,
        bindings
            .iter()
            .map(|binding| binding.scope.index() + 1)
            .max()
            .unwrap_or(0),
        &names,
    );
    let reaching_definitions = compute_reaching_definitions_dense(cfg, bindings, &binding_data);
    analyze_uninitialized_references_dense(
        cfg,
        bindings,
        references,
        predefined_runtime_refs,
        resolved,
        indirect_targets_by_reference,
        &names,
        &binding_data,
        &reaching_definitions,
    )
}

pub(crate) fn analyze_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
    build_dead_code(cfg)
}

#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) fn analyze(
    cfg: &ControlFlowGraph,
    runtime: &RuntimePrelude,
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    predefined_runtime_refs: &FxHashSet<ReferenceId>,
    resolved: &FxHashMap<ReferenceId, BindingId>,
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    indirect_targets_by_reference: &[Vec<BindingId>],
    synthetic_reads: &[SyntheticRead],
) -> DataflowResult {
    let binding_name_data = build_binding_name_data(bindings);
    let reaching_definitions =
        compute_reaching_definitions(cfg, bindings, &binding_name_data.bindings_by_name);
    let names = build_name_table(bindings, references, synthetic_reads);
    let dense_binding_data = build_dense_binding_data(bindings, scopes, &names);
    let dense_reaching_definitions =
        compute_reaching_definitions_dense(cfg, bindings, &dense_binding_data);
    let unused_assignments = analyze_unused_assignments_exact(
        cfg,
        runtime,
        scopes,
        bindings,
        references,
        resolved,
        call_sites,
        indirect_targets_by_reference,
        synthetic_reads,
    );
    let uninitialized_references = analyze_uninitialized_references_dense(
        cfg,
        bindings,
        references,
        predefined_runtime_refs,
        resolved,
        indirect_targets_by_reference,
        &names,
        &dense_binding_data,
        &dense_reaching_definitions,
    );
    let dead_code = build_dead_code(cfg);

    DataflowResult {
        reaching_definitions,
        unused_assignments: unused_assignments.unused_assignments,
        uninitialized_references,
        dead_code,
        unused_assignment_ids: unused_assignments.unused_assignment_ids,
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_uninitialized_references_dense(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    references: &[Reference],
    predefined_runtime_refs: &FxHashSet<ReferenceId>,
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_targets_by_reference: &[Vec<BindingId>],
    names: &NameTable,
    binding_data: &DenseBindingData,
    reaching_definitions: &DenseReachingDefinitions,
) -> Vec<UninitializedReference> {
    let reference_blocks = build_reference_block_index(cfg, references.len());
    let unreachable = build_unreachable_block_set(cfg);
    let name_count = names.len();
    let initializing_name_ids = build_initializing_name_ids(bindings, binding_data);
    let maybe_defined = reaching_definitions
        .reaching_in
        .iter()
        .map(|incoming| initialized_names_from_dense(incoming, &initializing_name_ids, name_count))
        .collect::<Vec<_>>();
    let maybe_defined_out = reaching_definitions
        .reaching_out
        .iter()
        .map(|outgoing| initialized_names_from_dense(outgoing, &initializing_name_ids, name_count))
        .collect::<Vec<_>>();
    let definitely_defined = cfg
        .blocks()
        .iter()
        .map(|block| {
            let predecessors = cfg.predecessors(block.id);
            if predecessors.is_empty() {
                return DenseBitSet::new(name_count);
            }
            let mut intersection = maybe_defined_out[predecessors[0].index()].clone();
            for predecessor in predecessors.iter().skip(1) {
                intersection.intersect_with(&maybe_defined_out[predecessor.index()]);
            }
            intersection
        })
        .collect::<Vec<_>>();

    let mut uninitialized_references = Vec::new();
    for reference in references {
        if matches!(
            reference.kind,
            ReferenceKind::ImplicitRead | ReferenceKind::DeclarationName
        ) || predefined_runtime_refs.contains(&reference.id)
        {
            continue;
        }
        if matches!(reference.kind, ReferenceKind::IndirectExpansion)
            && (resolved.contains_key(&reference.id)
                || !indirect_targets_by_reference[reference.id.index()].is_empty())
        {
            continue;
        }
        let Some(block_id) = reference_blocks[reference.id.index()] else {
            continue;
        };
        if unreachable.contains(block_id.index()) {
            continue;
        }
        let Some(name_id) = names.get(&reference.name) else {
            continue;
        };
        let maybe = maybe_defined[block_id.index()].contains(name_id.index());
        let definite = definitely_defined[block_id.index()].contains(name_id.index());

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

    uninitialized_references
}

fn build_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
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
    dead_code_by_cause
        .into_iter()
        .map(|(_, (cause, unreachable))| DeadCode { unreachable, cause })
        .collect()
}

fn build_binding_name_data(bindings: &[Binding]) -> BindingNameData {
    let mut bindings_by_name: FxHashMap<Name, Vec<BindingId>> = FxHashMap::default();
    for binding in bindings {
        bindings_by_name
            .entry(binding.name.clone())
            .or_default()
            .push(binding.id);
    }

    BindingNameData { bindings_by_name }
}

fn compute_reaching_definitions(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    bindings_by_name: &FxHashMap<Name, Vec<BindingId>>,
) -> ReachingDefinitions {
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
        .map(|block_id| {
            (
                *block_id,
                kill_set(cfg, *block_id, bindings, bindings_by_name),
            )
        })
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

    ReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_unused_assignments_exact(
    cfg: &ControlFlowGraph,
    runtime: &RuntimePrelude,
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    indirect_targets_by_reference: &[Vec<BindingId>],
    synthetic_reads: &[SyntheticRead],
) -> UnusedAssignmentsResult {
    let names = build_name_table(bindings, references, synthetic_reads);
    let binding_data = build_dense_binding_data(bindings, scopes, &names);
    let reference_name_ids = references
        .iter()
        .map(|reference| names.get(&reference.name).expect("reference name interned"))
        .collect::<Vec<_>>();
    let synthetic_read_name_ids = synthetic_reads
        .iter()
        .map(|read| names.get(&read.name).expect("synthetic read name interned"))
        .collect::<Vec<_>>();
    let binding_blocks = build_binding_block_index(cfg, bindings.len());
    let reference_blocks = build_reference_block_index(cfg, references.len());
    let unreachable_blocks = build_unreachable_block_set(cfg);
    let reaching_definitions = compute_reaching_definitions_dense(cfg, bindings, &binding_data);
    let scope_components = compute_scope_components_dense(
        cfg,
        scopes.len(),
        cfg.blocks().len(),
        bindings.len(),
        &reaching_definitions.reaching_out,
    );
    let interprocedural_reads = if call_sites.is_empty() {
        None
    } else {
        let (read_plans, callers_by_callee) = build_scope_read_plans(
            scopes,
            bindings,
            references,
            synthetic_reads,
            &reference_name_ids,
            &synthetic_read_name_ids,
            call_sites,
            names.len(),
        );
        let interprocedural =
            compute_interprocedural_read_sets(&read_plans, &callers_by_callee, names.len());
        Some((read_plans, interprocedural))
    };

    let mut used_bindings = DenseBitSet::new(bindings.len());
    for binding in bindings {
        if !binding.references.is_empty() || runtime.is_always_used_binding(&binding.name) {
            used_bindings.insert(binding.id.index());
        }
    }

    for (reference_index, reference) in references.iter().enumerate() {
        let Some(block_id) = reference_blocks[reference_index] else {
            continue;
        };
        if unreachable_blocks.contains(block_id.index()) {
            continue;
        }

        let incoming = &reaching_definitions.reaching_in[block_id.index()];
        let name_id = reference_name_ids[reference_index];
        used_bindings
            .or_intersection_with(incoming, &binding_data.bindings_for_name[name_id.index()]);

        let Some(resolved_binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        let resolved_binding = &bindings[resolved_binding_id.index()];
        let component = &scope_components[resolved_binding.scope.index()];
        if !component.blocks.contains(block_id.index()) {
            used_bindings.or_intersection3_with(
                &component.exit_defs,
                &binding_data.bindings_for_name[name_id.index()],
                &binding_data.bindings_in_scope[resolved_binding.scope.index()],
            );
        }

        if let Some(candidates) = indirect_targets_by_reference.get(reference.id.index())
            && !candidates.is_empty()
        {
            mark_reaching_candidate_bindings_used(&mut used_bindings, incoming, candidates);
            if !component.blocks.contains(block_id.index()) {
                mark_reaching_candidate_bindings_used(
                    &mut used_bindings,
                    &component.exit_defs,
                    candidates,
                );
            }
        }
    }

    for (read_index, synthetic_read) in synthetic_reads.iter().enumerate() {
        let Some(block_id) = command_block_for_span(cfg, synthetic_read.span) else {
            continue;
        };
        if unreachable_blocks.contains(block_id.index()) {
            continue;
        }
        used_bindings.or_intersection_with(
            &reaching_definitions.reaching_in[block_id.index()],
            &binding_data.bindings_for_name[synthetic_read_name_ids[read_index].index()],
        );
    }

    if let Some((read_plans, interprocedural)) = &interprocedural_reads {
        for plan in read_plans {
            for call in &plan.calls {
                let Some(block_id) = command_block_for_span(cfg, call.span) else {
                    continue;
                };
                if unreachable_blocks.contains(block_id.index()) {
                    continue;
                }
                mark_reaching_defs_for_names_used(
                    &mut used_bindings,
                    &reaching_definitions.reaching_in[block_id.index()],
                    &binding_data.binding_name_ids,
                    &interprocedural.transitive_reads[call.callee_scope.index()],
                );
            }
        }

        for binding in bindings {
            if is_function_escape_candidate(binding, scopes)
                && future_reads_contain_after(
                    binding.scope,
                    binding.span.start.offset,
                    binding_data.binding_name_ids[binding.id.index()],
                    read_plans,
                    &interprocedural.future_reads,
                    &interprocedural.escape_reads,
                )
            {
                used_bindings.insert(binding.id.index());
            }
        }
    }

    let mut unused_assignments = Vec::new();
    for binding in bindings {
        let Some(block_id) = binding_blocks[binding.id.index()] else {
            continue;
        };
        if matches!(
            binding.kind,
            BindingKind::FunctionDefinition | BindingKind::Imported
        ) || runtime.is_always_used_binding(&binding.name)
            || unreachable_blocks.contains(block_id.index())
            || used_bindings.contains(binding.id.index())
        {
            continue;
        }

        let reason = binding_data.next_overwrite[binding.id.index()]
            .map(|by| UnusedReason::Overwritten { by })
            .unwrap_or(UnusedReason::ScopeEnd);
        unused_assignments.push(UnusedAssignment {
            binding: binding.id,
            reason,
        });
    }
    let unused_assignment_ids = collapse_redundant_branch_unused_assignment_ids(
        cfg,
        bindings,
        &binding_blocks,
        &unreachable_blocks,
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

    let binding_name_data = build_binding_name_data(bindings);
    let unused_binding_ids = unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<FxHashSet<_>>();
    let mut reachability_cache = vec![None; cfg.blocks().len()];
    let mut suppression_context = RedundantBranchUnusedAssignmentContext {
        cfg,
        bindings,
        bindings_by_name: &binding_name_data.bindings_by_name,
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

struct RedundantBranchUnusedAssignmentContext<'a> {
    cfg: &'a ControlFlowGraph,
    bindings: &'a [Binding],
    bindings_by_name: &'a FxHashMap<Name, Vec<BindingId>>,
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
    if !participates_in_unused_assignment_reporting(binding.kind, binding.attributes) {
        return false;
    }

    let Some(binding_block) = context.binding_blocks[binding_id.index()] else {
        return false;
    };
    let Some(later_bindings) = context.bindings_by_name.get(&binding.name) else {
        return false;
    };

    let mut later_reportable = later_bindings
        .iter()
        .copied()
        .filter(|candidate_id| candidate_id.index() > binding_id.index())
        .filter(|candidate_id| {
            let candidate = &context.bindings[candidate_id.index()];
            candidate.scope == binding.scope
                && participates_in_unused_assignment_reporting(candidate.kind, candidate.attributes)
        })
        .filter_map(|candidate_id| {
            let candidate_block = context.binding_blocks[candidate_id.index()]?;
            (!context.unreachable_blocks.contains(candidate_block.index()))
                .then_some((candidate_id, candidate_block))
        });

    let Some((next_binding_id, next_binding_block)) = later_reportable.next() else {
        return false;
    };

    if !context.unused_binding_ids.contains(&next_binding_id) {
        return false;
    }

    if later_reportable.any(|(candidate_id, _)| !context.unused_binding_ids.contains(&candidate_id))
    {
        return false;
    }

    !block_can_reach(
        context.cfg,
        binding_block,
        next_binding_block,
        context.reachability_cache,
    )
}

fn participates_in_unused_assignment_reporting(
    kind: BindingKind,
    attributes: BindingAttributes,
) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseBitSet {
    words: Vec<usize>,
}

impl DenseBitSet {
    const WORD_BITS: usize = usize::BITS as usize;

    fn new(bit_len: usize) -> Self {
        Self {
            words: vec![0; bit_len.div_ceil(Self::WORD_BITS)],
        }
    }

    fn insert(&mut self, index: usize) {
        let word = index / Self::WORD_BITS;
        let bit = index % Self::WORD_BITS;
        self.words[word] |= 1usize << bit;
    }

    fn contains(&self, index: usize) -> bool {
        let word = index / Self::WORD_BITS;
        let bit = index % Self::WORD_BITS;
        self.words
            .get(word)
            .is_some_and(|word| (word & (1usize << bit)) != 0)
    }

    fn union_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
            *word |= *other_word;
        }
    }

    fn subtract_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
            *word &= !*other_word;
        }
    }

    fn intersect_with(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        for (word, other_word) in self.words.iter_mut().zip(&other.words) {
            *word &= *other_word;
        }
    }

    fn or_intersection_with(&mut self, left: &Self, right: &Self) {
        debug_assert_eq!(self.words.len(), left.words.len());
        debug_assert_eq!(self.words.len(), right.words.len());
        for ((word, left_word), right_word) in
            self.words.iter_mut().zip(&left.words).zip(&right.words)
        {
            *word |= *left_word & *right_word;
        }
    }

    fn or_intersection3_with(&mut self, first: &Self, second: &Self, third: &Self) {
        debug_assert_eq!(self.words.len(), first.words.len());
        debug_assert_eq!(self.words.len(), second.words.len());
        debug_assert_eq!(self.words.len(), third.words.len());
        for (((word, first_word), second_word), third_word) in self
            .words
            .iter_mut()
            .zip(&first.words)
            .zip(&second.words)
            .zip(&third.words)
        {
            *word |= *first_word & *second_word & *third_word;
        }
    }

    fn iter_ones(&self) -> DenseBitSetIter<'_> {
        DenseBitSetIter {
            words: &self.words,
            word_index: 0,
            current_word: 0,
        }
    }
}

struct DenseBitSetIter<'a> {
    words: &'a [usize],
    word_index: usize,
    current_word: usize,
}

impl Iterator for DenseBitSetIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_word != 0 {
                let bit = self.current_word.trailing_zeros() as usize;
                self.current_word &= self.current_word - 1;
                return Some((self.word_index - 1) * DenseBitSet::WORD_BITS + bit);
            }

            let next_word = self.words.get(self.word_index).copied()?;
            self.current_word = next_word;
            self.word_index += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NameId(u32);

impl NameId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Default)]
struct NameTable {
    ids_by_name: FxHashMap<Name, NameId>,
}

impl NameTable {
    fn intern(&mut self, name: &Name) -> NameId {
        if let Some(id) = self.ids_by_name.get(name).copied() {
            return id;
        }

        let id = NameId(self.ids_by_name.len() as u32);
        self.ids_by_name.insert(name.clone(), id);
        id
    }

    fn get(&self, name: &Name) -> Option<NameId> {
        self.ids_by_name.get(name).copied()
    }

    fn len(&self) -> usize {
        self.ids_by_name.len()
    }
}

#[derive(Debug, Clone)]
struct DenseBindingData {
    binding_name_ids: Vec<NameId>,
    bindings_for_name: Vec<DenseBitSet>,
    bindings_in_scope: Vec<DenseBitSet>,
    next_overwrite: Vec<Option<BindingId>>,
}

#[derive(Debug, Clone)]
struct DenseReachingDefinitions {
    reaching_in: Vec<DenseBitSet>,
    reaching_out: Vec<DenseBitSet>,
}

#[derive(Debug, Clone)]
struct ExactScopeComponent {
    blocks: DenseBitSet,
    exit_defs: DenseBitSet,
}

impl ExactScopeComponent {
    fn new(block_count: usize, binding_count: usize) -> Self {
        Self {
            blocks: DenseBitSet::new(block_count),
            exit_defs: DenseBitSet::new(binding_count),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedCallSite {
    offset: usize,
    span: Span,
    callee_scope: ScopeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeReadEventKind {
    Direct(NameId),
    Call(ScopeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScopeReadEvent {
    offset: usize,
    kind: ScopeReadEventKind,
}

#[derive(Debug, Clone)]
struct ScopeReadPlan {
    direct_reads: DenseBitSet,
    calls: Vec<ResolvedCallSite>,
    events: Vec<ScopeReadEvent>,
    is_function: bool,
}

impl ScopeReadPlan {
    fn new(name_count: usize, is_function: bool) -> Self {
        Self {
            direct_reads: DenseBitSet::new(name_count),
            calls: Vec::new(),
            events: Vec::new(),
            is_function,
        }
    }
}

#[derive(Debug, Clone)]
struct ScopeFutureReads {
    suffix_reads: Vec<DenseBitSet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CallerReadSite {
    caller_scope: ScopeId,
    offset: usize,
}

#[derive(Debug, Clone)]
struct InterproceduralReadSets {
    transitive_reads: Vec<DenseBitSet>,
    escape_reads: Vec<DenseBitSet>,
    future_reads: Vec<ScopeFutureReads>,
}

fn build_name_table(
    bindings: &[Binding],
    references: &[Reference],
    synthetic_reads: &[SyntheticRead],
) -> NameTable {
    let mut names = NameTable::default();
    for binding in bindings {
        names.intern(&binding.name);
    }
    for reference in references {
        names.intern(&reference.name);
    }
    for synthetic_read in synthetic_reads {
        names.intern(&synthetic_read.name);
    }
    names
}

fn build_uninitialized_name_table(bindings: &[Binding], references: &[Reference]) -> NameTable {
    let mut names = NameTable::default();
    for binding in bindings {
        names.intern(&binding.name);
    }
    for reference in references {
        names.intern(&reference.name);
    }
    names
}

fn build_dense_binding_data(
    bindings: &[Binding],
    scopes: &[Scope],
    names: &NameTable,
) -> DenseBindingData {
    build_dense_binding_data_for_scope_count(bindings, scopes.len(), names)
}

fn build_dense_binding_data_for_scope_count(
    bindings: &[Binding],
    scope_count: usize,
    names: &NameTable,
) -> DenseBindingData {
    let name_count = names.len();
    let binding_count = bindings.len();
    let mut binding_name_ids = Vec::with_capacity(binding_count);
    let mut bindings_for_name = (0..name_count)
        .map(|_| DenseBitSet::new(binding_count))
        .collect::<Vec<_>>();
    let mut bindings_by_name = vec![Vec::new(); name_count];
    let mut bindings_in_scope = (0..scope_count)
        .map(|_| DenseBitSet::new(binding_count))
        .collect::<Vec<_>>();

    for binding in bindings {
        let name_id = names.get(&binding.name).expect("binding name interned");
        binding_name_ids.push(name_id);
        bindings_for_name[name_id.index()].insert(binding.id.index());
        bindings_by_name[name_id.index()].push(binding.id);
        if let Some(bindings_in_scope) = bindings_in_scope.get_mut(binding.scope.index()) {
            bindings_in_scope.insert(binding.id.index());
        }
    }

    let mut next_overwrite = vec![None; binding_count];
    for binding_ids in bindings_by_name {
        for pair in binding_ids.windows(2) {
            next_overwrite[pair[0].index()] = Some(pair[1]);
        }
    }

    DenseBindingData {
        binding_name_ids,
        bindings_for_name,
        bindings_in_scope,
        next_overwrite,
    }
}

fn build_binding_block_index(cfg: &ControlFlowGraph, binding_count: usize) -> Vec<Option<BlockId>> {
    let mut blocks = vec![None; binding_count];
    for block in cfg.blocks() {
        for binding in &block.bindings {
            blocks[binding.index()] = Some(block.id);
        }
    }
    blocks
}

fn build_reference_block_index(
    cfg: &ControlFlowGraph,
    reference_count: usize,
) -> Vec<Option<BlockId>> {
    let mut blocks = vec![None; reference_count];
    for block in cfg.blocks() {
        for reference in &block.references {
            blocks[reference.index()] = Some(block.id);
        }
    }
    blocks
}

fn build_unreachable_block_set(cfg: &ControlFlowGraph) -> DenseBitSet {
    let mut unreachable = DenseBitSet::new(cfg.blocks().len());
    for block in cfg.unreachable() {
        unreachable.insert(block.index());
    }
    unreachable
}

fn command_block_for_span(cfg: &ControlFlowGraph, span: Span) -> Option<BlockId> {
    cfg.command_blocks
        .get(&SpanKey::new(span))
        .and_then(|blocks| blocks.last())
        .copied()
}

fn compute_reaching_definitions_dense(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
) -> DenseReachingDefinitions {
    let block_count = cfg.blocks().len();
    let binding_count = bindings.len();
    let name_count = binding_data.bindings_for_name.len();
    let block_bindings = cfg
        .blocks()
        .iter()
        .map(|block| {
            let mut bitset = DenseBitSet::new(binding_count);
            for binding in &block.bindings {
                bitset.insert(binding.index());
            }
            bitset
        })
        .collect::<Vec<_>>();
    let gen_sets = cfg
        .blocks()
        .iter()
        .map(|block| {
            let mut generated = DenseBitSet::new(binding_count);
            for binding in &block.bindings {
                let binding_info = &bindings[binding.index()];
                if matches!(binding_info.kind, BindingKind::AppendAssignment) {
                    generated.insert(binding.index());
                    continue;
                }

                let name_id = binding_data.binding_name_ids[binding.index()];
                generated.subtract_with(&binding_data.bindings_for_name[name_id.index()]);
                generated.insert(binding.index());
            }
            generated
        })
        .collect::<Vec<_>>();
    let kill_sets = cfg
        .blocks()
        .iter()
        .enumerate()
        .map(|(block_index, block)| {
            let mut overwritten_names = DenseBitSet::new(name_count);
            for binding in &block.bindings {
                if !matches!(
                    bindings[binding.index()].kind,
                    BindingKind::AppendAssignment
                ) {
                    overwritten_names
                        .insert(binding_data.binding_name_ids[binding.index()].index());
                }
            }

            let mut killed = DenseBitSet::new(binding_count);
            for name_index in overwritten_names.iter_ones() {
                killed.union_with(&binding_data.bindings_for_name[name_index]);
            }
            killed.subtract_with(&block_bindings[block_index]);
            killed
        })
        .collect::<Vec<_>>();

    let mut reaching_in = vec![DenseBitSet::new(binding_count); block_count];
    let mut reaching_out = vec![DenseBitSet::new(binding_count); block_count];
    let mut changed = true;
    while changed {
        changed = false;
        for block in cfg.blocks() {
            let block_index = block.id.index();
            let mut incoming = DenseBitSet::new(binding_count);
            for predecessor in cfg.predecessors(block.id) {
                incoming.union_with(&reaching_out[predecessor.index()]);
            }

            let mut carried = incoming.clone();
            carried.subtract_with(&kill_sets[block_index]);
            let mut outgoing = gen_sets[block_index].clone();
            outgoing.union_with(&carried);

            if reaching_in[block_index] != incoming {
                reaching_in[block_index] = incoming;
                changed = true;
            }
            if reaching_out[block_index] != outgoing {
                reaching_out[block_index] = outgoing;
                changed = true;
            }
        }
    }

    DenseReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

fn compute_scope_components_dense(
    cfg: &ControlFlowGraph,
    scope_count: usize,
    block_count: usize,
    binding_count: usize,
    reaching_out: &[DenseBitSet],
) -> Vec<ExactScopeComponent> {
    let mut components = (0..scope_count)
        .map(|_| ExactScopeComponent::new(block_count, binding_count))
        .collect::<Vec<_>>();

    for (scope, entry) in &cfg.scope_entries {
        let blocks = reachable_blocks_dense(cfg, *entry, block_count);
        let mut exit_defs = DenseBitSet::new(binding_count);
        for block_index in blocks.iter_ones() {
            let block_id = BlockId(block_index as u32);
            if cfg
                .successors(block_id)
                .iter()
                .all(|(successor, _)| !blocks.contains(successor.index()))
            {
                exit_defs.union_with(&reaching_out[block_index]);
            }
        }
        components[scope.index()] = ExactScopeComponent { blocks, exit_defs };
    }

    components
}

fn build_initializing_name_ids(
    bindings: &[Binding],
    binding_data: &DenseBindingData,
) -> Vec<Option<NameId>> {
    bindings
        .iter()
        .enumerate()
        .map(|(binding_index, binding)| {
            binding_initializes_name(binding)
                .then_some(binding_data.binding_name_ids[binding_index])
        })
        .collect()
}

fn initialized_names_from_dense(
    reaching_definitions: &DenseBitSet,
    initializing_name_ids: &[Option<NameId>],
    name_count: usize,
) -> DenseBitSet {
    let mut initialized_names = DenseBitSet::new(name_count);
    for binding_index in reaching_definitions.iter_ones() {
        if let Some(name_id) = initializing_name_ids[binding_index] {
            initialized_names.insert(name_id.index());
        }
    }
    initialized_names
}

fn reachable_blocks_dense(
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
fn build_scope_read_plans(
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    synthetic_reads: &[SyntheticRead],
    reference_name_ids: &[NameId],
    synthetic_read_name_ids: &[NameId],
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
    name_count: usize,
) -> (Vec<ScopeReadPlan>, Vec<Vec<CallerReadSite>>) {
    let function_scopes = function_scopes_by_binding(scopes, bindings);
    let calls_by_scope = resolved_calls_by_scope(scopes, bindings, call_sites, &function_scopes);
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
            kind: ScopeReadEventKind::Direct(name_id),
        });
    }

    for (read_index, synthetic_read) in synthetic_reads.iter().enumerate() {
        let plan = &mut plans[synthetic_read.scope.index()];
        let name_id = synthetic_read_name_ids[read_index];
        plan.direct_reads.insert(name_id.index());
        plan.events.push(ScopeReadEvent {
            offset: synthetic_read.span.start.offset,
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

fn compute_interprocedural_read_sets(
    read_plans: &[ScopeReadPlan],
    callers_by_callee: &[Vec<CallerReadSite>],
    name_count: usize,
) -> InterproceduralReadSets {
    let mut transitive_reads = vec![DenseBitSet::new(name_count); read_plans.len()];
    loop {
        let mut changed = false;
        for (scope_index, plan) in read_plans.iter().enumerate() {
            let mut reads = plan.direct_reads.clone();
            for call in &plan.calls {
                reads.union_with(&transitive_reads[call.callee_scope.index()]);
            }
            if transitive_reads[scope_index] != reads {
                transitive_reads[scope_index] = reads;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let future_reads = build_future_read_summaries(read_plans, &transitive_reads, name_count);
    let mut escape_reads = vec![DenseBitSet::new(name_count); read_plans.len()];
    loop {
        let mut changed = false;
        for (scope_index, plan) in read_plans.iter().enumerate() {
            if !plan.is_function {
                continue;
            }

            let mut reads = DenseBitSet::new(name_count);
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
            if escape_reads[scope_index] != reads {
                escape_reads[scope_index] = reads;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    InterproceduralReadSets {
        transitive_reads,
        escape_reads,
        future_reads,
    }
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
                suffix_reads[event_index] = suffix_reads[event_index + 1].clone();
                match plan.events[event_index].kind {
                    ScopeReadEventKind::Direct(name_id) => {
                        suffix_reads[event_index].insert(name_id.index());
                    }
                    ScopeReadEventKind::Call(callee_scope) => {
                        suffix_reads[event_index]
                            .union_with(&transitive_reads[callee_scope.index()]);
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

fn mark_reaching_defs_for_names_used(
    used_bindings: &mut DenseBitSet,
    incoming: &DenseBitSet,
    binding_name_ids: &[NameId],
    used_names: &DenseBitSet,
) {
    for binding_index in incoming.iter_ones() {
        if used_names.contains(binding_name_ids[binding_index].index()) {
            used_bindings.insert(binding_index);
        }
    }
}

fn mark_reaching_candidate_bindings_used(
    used_bindings: &mut DenseBitSet,
    incoming: &DenseBitSet,
    candidates: &[BindingId],
) {
    for candidate in candidates {
        if incoming.contains(candidate.index()) {
            used_bindings.insert(candidate.index());
        }
    }
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
    bindings_by_name: &FxHashMap<Name, Vec<BindingId>>,
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

    let mut killed = FxHashSet::default();
    for name in overwritten_names {
        let Some(binding_ids) = bindings_by_name.get(&name) else {
            continue;
        };
        for binding_id in binding_ids {
            if !block.bindings.contains(binding_id) {
                killed.insert(*binding_id);
            }
        }
    }
    killed
}

fn binding_initializes_name(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::Declaration(_) | BindingKind::Nameref => binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED),
        BindingKind::FunctionDefinition | BindingKind::Imported => false,
        BindingKind::Assignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
    }
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
                    span: site.span,
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

        if scope_id != scope {
            if let Some(binding) = candidates.iter().rev().copied().find(|binding| {
                matches!(
                    bindings[binding.index()].kind,
                    BindingKind::FunctionDefinition
                )
            }) {
                return Some(binding);
            }
            continue;
        }

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
