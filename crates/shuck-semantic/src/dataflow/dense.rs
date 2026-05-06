use super::*;

/// Compact numeric name identifier used by the dense bitset matrices.
///
/// Shell variables are sparse strings, but the fixed-point solvers want small
/// integer columns. Interning lets `$PATH`, `$flag`, and `$1` become stable
/// matrix positions while preserving the original `Name` on bindings and
/// references for user-facing queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct NameId(pub(super) u32);

impl NameId {
    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct NameTable {
    pub(super) ids_by_name: FxHashMap<Name, NameId>,
}

impl NameTable {
    pub(super) fn intern(&mut self, name: &Name) -> NameId {
        if let Some(id) = self.ids_by_name.get(name).copied() {
            return id;
        }

        let id = NameId(self.ids_by_name.len() as u32);
        self.ids_by_name.insert(name.clone(), id);
        id
    }

    pub(super) fn get(&self, name: &Name) -> Option<NameId> {
        self.ids_by_name.get(name).copied()
    }

    pub(super) fn len(&self) -> usize {
        self.ids_by_name.len()
    }
}

#[derive(Debug, Clone)]
pub(super) struct DenseBindingData {
    pub(super) binding_name_ids: Vec<NameId>,
    pub(super) bindings_for_name: Vec<DenseBitSet>,
    pub(super) next_overwrite: Vec<Option<BindingId>>,
}

#[derive(Debug, Clone)]
pub(super) struct DenseReachingDefinitions {
    pub(super) reaching_in: Vec<DenseBitSet>,
    pub(super) reaching_out: Vec<DenseBitSet>,
}

#[cfg(test)]
pub(crate) fn materialize_reaching_definitions(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> ReachingDefinitions {
    materialize_dense_reaching_definitions(context.cfg, exact.reaching_definitions(context))
}

#[cfg(test)]
fn materialize_dense_reaching_definitions(
    cfg: &ControlFlowGraph,
    dense: &DenseReachingDefinitions,
) -> ReachingDefinitions {
    let mut reaching_in = FxHashMap::default();
    let mut reaching_out = FxHashMap::default();

    for block in cfg.blocks() {
        reaching_in.insert(
            block.id,
            dense.reaching_in[block.id.index()]
                .iter_ones()
                .map(|binding_index| BindingId(binding_index as u32))
                .collect::<FxHashSet<_>>(),
        );
        reaching_out.insert(
            block.id,
            dense.reaching_out[block.id.index()]
                .iter_ones()
                .map(|binding_index| BindingId(binding_index as u32))
                .collect::<FxHashSet<_>>(),
        );
    }

    ReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

#[derive(Debug, Clone)]
pub(super) struct DenseInitializedNameStates {
    /// Names that might be initialized before each block.
    ///
    /// In `if cond; then x=1; fi; echo "$x"`, `x` is in `maybe_in` at the
    /// `echo` block but not in `definite_in`.
    pub(super) maybe_in: DenseBitMatrix,
    pub(super) maybe_out: DenseBitMatrix,
    /// Names that are initialized on every path into or out of each block.
    ///
    /// In `x=1; echo "$x"`, `x` is definite at the `echo`; in a conditional
    /// assignment it is only possible unless all branches initialize it.
    pub(super) definite_in: DenseBitMatrix,
    pub(super) definite_out: DenseBitMatrix,
}

#[derive(Debug, Clone)]
pub(super) struct ExactScopeComponent {
    pub(super) blocks: DenseBitSet,
}

impl ExactScopeComponent {
    fn new(block_count: usize) -> Self {
        Self {
            blocks: DenseBitSet::new(block_count),
        }
    }
}

pub(super) fn build_name_table(
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

pub(super) fn build_dense_binding_data(
    bindings: &[Binding],
    scopes: &[Scope],
    names: &NameTable,
) -> DenseBindingData {
    build_dense_binding_data_for_scope_count(bindings, scopes.len(), names)
}

pub(super) fn build_dense_binding_data_for_scope_count(
    bindings: &[Binding],
    _scope_count: usize,
    names: &NameTable,
) -> DenseBindingData {
    let name_count = names.len();
    let binding_count = bindings.len();
    let mut binding_name_ids = Vec::with_capacity(binding_count);
    let mut bindings_for_name = (0..name_count)
        .map(|_| DenseBitSet::new(binding_count))
        .collect::<Vec<_>>();
    let mut bindings_by_name = vec![Vec::new(); name_count];

    for binding in bindings {
        let Some(name_id) = names.get(&binding.name) else {
            unreachable!("binding name interned");
        };
        binding_name_ids.push(name_id);
        bindings_for_name[name_id.index()].insert(binding.id.index());
        bindings_by_name[name_id.index()].push(binding.id);
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
        next_overwrite,
    }
}

pub(super) fn build_binding_block_index(
    cfg: &ControlFlowGraph,
    binding_count: usize,
) -> Vec<Option<BlockId>> {
    let mut blocks = vec![None; binding_count];
    for block in cfg.blocks() {
        for binding in &block.bindings {
            blocks[binding.index()] = Some(block.id);
        }
    }
    blocks
}

pub(super) fn build_reference_block_index(
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

pub(super) fn build_unreachable_block_set(cfg: &ControlFlowGraph) -> DenseBitSet {
    let mut unreachable = DenseBitSet::new(cfg.blocks().len());
    for block in cfg.unreachable() {
        unreachable.insert(block.index());
    }
    unreachable
}

pub(super) fn command_block_for_span(cfg: &ControlFlowGraph, span: Span) -> Option<BlockId> {
    cfg.block_ids_for_span(span).last().copied()
}

pub(super) fn compute_reverse_postorder(cfg: &ControlFlowGraph) -> Box<[BlockId]> {
    compute_block_order(cfg, BlockOrderKind::ReversePostorder)
}

pub(super) fn compute_postorder(cfg: &ControlFlowGraph) -> Box<[BlockId]> {
    compute_block_order(cfg, BlockOrderKind::Postorder)
}

#[derive(Clone, Copy)]
enum BlockOrderKind {
    ReversePostorder,
    Postorder,
}

fn compute_block_order(cfg: &ControlFlowGraph, kind: BlockOrderKind) -> Box<[BlockId]> {
    let block_count = cfg.blocks().len();
    let mut visited = DenseBitSet::new(block_count);
    let mut order: Vec<BlockId> = Vec::with_capacity(block_count);

    let mut sources: Vec<BlockId> = Vec::new();
    sources.push(cfg.entry());
    sources.extend(cfg.scope_entries.values().copied());
    for block in cfg.blocks() {
        if cfg.predecessors(block.id).is_empty() {
            sources.push(block.id);
        }
    }

    enum Frame {
        Enter(BlockId),
        Exit(BlockId),
    }
    let mut stack: Vec<Frame> = Vec::new();
    for source in sources {
        if visited.contains(source.index()) {
            continue;
        }
        stack.push(Frame::Enter(source));
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(block) => {
                    if visited.contains(block.index()) {
                        continue;
                    }
                    visited.insert(block.index());
                    stack.push(Frame::Exit(block));
                    for (successor, _) in cfg.successors(block) {
                        if !visited.contains(successor.index()) {
                            stack.push(Frame::Enter(*successor));
                        }
                    }
                }
                Frame::Exit(block) => order.push(block),
            }
        }
    }

    for block in cfg.blocks() {
        if !visited.contains(block.id.index()) {
            order.push(block.id);
        }
    }

    if matches!(kind, BlockOrderKind::ReversePostorder) {
        order.reverse();
    }
    order.into_boxed_slice()
}

fn run_forward_dataflow_worklist<F>(cfg: &ControlFlowGraph, rpo: &[BlockId], mut transfer: F)
where
    F: FnMut(BlockId) -> bool,
{
    let block_count = cfg.blocks().len();
    let mut dirty = DenseBitSet::new(block_count);
    for block in rpo {
        dirty.insert(block.index());
    }

    while !dirty.is_empty() {
        for &block in rpo {
            if !dirty.contains(block.index()) {
                continue;
            }
            dirty.remove(block.index());
            if transfer(block) {
                for (successor, _) in cfg.successors(block) {
                    dirty.insert(successor.index());
                }
            }
        }
    }
}

pub(super) fn run_backward_dataflow_worklist<F>(
    cfg: &ControlFlowGraph,
    postorder: &[BlockId],
    mut transfer: F,
) where
    F: FnMut(BlockId) -> bool,
{
    let block_count = cfg.blocks().len();
    let mut dirty = DenseBitSet::new(block_count);
    for block in postorder {
        dirty.insert(block.index());
    }

    while !dirty.is_empty() {
        for &block in postorder {
            if !dirty.contains(block.index()) {
                continue;
            }
            dirty.remove(block.index());
            if transfer(block) {
                for predecessor in cfg.predecessors(block) {
                    dirty.insert(predecessor.index());
                }
            }
        }
    }
}

pub(super) fn compute_reaching_definitions_dense(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
    entry_bindings: &[BindingId],
    forward_order: &[BlockId],
) -> DenseReachingDefinitions {
    let entry_blocks = entry_binding_root_blocks(cfg);
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
    let mut incoming = DenseBitSet::new(binding_count);
    let mut carried = DenseBitSet::new(binding_count);
    let mut outgoing = DenseBitSet::new(binding_count);

    run_forward_dataflow_worklist(cfg, forward_order, |block_id| {
        let block_index = block_id.index();
        incoming.clear();
        for predecessor in cfg.predecessors(block_id) {
            incoming.union_with(&reaching_out[predecessor.index()]);
        }
        if entry_blocks.contains(&block_id) {
            for binding in entry_bindings {
                incoming.insert(binding.index());
            }
        }

        carried.copy_from(&incoming);
        carried.subtract_with(&kill_sets[block_index]);
        outgoing.copy_from(&gen_sets[block_index]);
        outgoing.union_with(&carried);

        reaching_in[block_index].replace_if_changed(&incoming);
        reaching_out[block_index].replace_if_changed(&outgoing)
    });

    DenseReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

pub(super) fn compute_initialized_name_states_dense(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
    entry_bindings: &[BindingId],
    forward_order: &[BlockId],
) -> DenseInitializedNameStates {
    compute_initialized_name_states_dense_with_extra_name_gens(
        cfg,
        bindings,
        binding_data,
        entry_bindings,
        &[],
        forward_order,
    )
}

pub(super) fn compute_initialized_name_states_dense_with_extra_name_gens(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
    entry_bindings: &[BindingId],
    extra_initialized_names: &[(BlockId, NameId)],
    forward_order: &[BlockId],
) -> DenseInitializedNameStates {
    let entry_blocks = entry_binding_root_blocks(cfg);
    let block_count = cfg.blocks().len();
    let name_count = binding_data.bindings_for_name.len();
    let mut maybe_gen = DenseBitMatrix::zeros(block_count, name_count);
    let mut definite_gen = DenseBitMatrix::zeros(block_count, name_count);
    let mut overwritten_names = DenseBitMatrix::zeros(block_count, name_count);

    for block in cfg.blocks() {
        let block_index = block.id.index();
        for binding in &block.bindings {
            let name_id = binding_data.binding_name_ids[binding.index()];
            overwritten_names.insert(block_index, name_id.index());
            match binding_initializes_name(&bindings[binding.index()]) {
                Some(ContractCertainty::Definite) => {
                    maybe_gen.insert(block_index, name_id.index());
                    definite_gen.insert(block_index, name_id.index());
                }
                Some(ContractCertainty::Possible) => {
                    maybe_gen.insert(block_index, name_id.index());
                }
                None => {}
            }
        }
    }

    for (block, name) in extra_initialized_names {
        maybe_gen.insert(block.index(), name.index());
        definite_gen.insert(block.index(), name.index());
    }

    let mut entry_maybe = DenseBitSet::new(name_count);
    let mut entry_definite = DenseBitSet::new(name_count);
    for binding in entry_bindings {
        let name_id = binding_data.binding_name_ids[binding.index()];
        match binding_initializes_name(&bindings[binding.index()]) {
            Some(ContractCertainty::Definite) => {
                entry_maybe.insert(name_id.index());
                entry_definite.insert(name_id.index());
            }
            Some(ContractCertainty::Possible) => {
                entry_maybe.insert(name_id.index());
            }
            None => {}
        }
    }

    let mut all_names = DenseBitSet::new(name_count);
    for index in 0..name_count {
        all_names.insert(index);
    }

    let mut maybe_in = DenseBitMatrix::zeros(block_count, name_count);
    let mut maybe_out = DenseBitMatrix::zeros(block_count, name_count);
    let mut definite_in = DenseBitMatrix::zeros(block_count, name_count);
    definite_in.fill_all_rows_from_words(all_names.as_words());
    let mut definite_out = DenseBitMatrix::zeros(block_count, name_count);
    definite_out.fill_all_rows_from_words(all_names.as_words());
    let mut incoming_maybe = DenseBitSet::new(name_count);
    let mut incoming_definite = DenseBitSet::new(name_count);
    let mut outgoing_maybe = DenseBitSet::new(name_count);
    let mut outgoing_definite = DenseBitSet::new(name_count);

    run_forward_dataflow_worklist(cfg, forward_order, |block_id| {
        let block_index = block_id.index();

        incoming_maybe.clear();
        for predecessor in cfg.predecessors(block_id) {
            incoming_maybe.union_with_words(maybe_out.row(predecessor.index()));
        }
        if entry_blocks.contains(&block_id) {
            incoming_maybe.union_with(&entry_maybe);
        }

        let predecessors = cfg.predecessors(block_id);
        let uses_virtual_entry_boundary = entry_blocks.contains(&block_id)
            && predecessors.iter().all(|predecessor| {
                cfg.successors(*predecessor)
                    .iter()
                    .any(|(successor, kind)| *successor == block_id && *kind == EdgeKind::LoopBack)
            });
        if uses_virtual_entry_boundary {
            incoming_definite.copy_from(&entry_definite);
        } else if let Some(first_predecessor) = predecessors.first() {
            incoming_definite.copy_from_words(definite_out.row(first_predecessor.index()));
        } else {
            incoming_definite.clear();
        }
        for (predecessor_index, predecessor) in predecessors.iter().enumerate() {
            if !uses_virtual_entry_boundary && predecessor_index == 0 {
                continue;
            }
            incoming_definite.intersect_with_words(definite_out.row(predecessor.index()));
        }

        outgoing_maybe.copy_from(&incoming_maybe);
        outgoing_maybe.subtract_with_words(overwritten_names.row(block_index));
        outgoing_maybe.union_with_words(maybe_gen.row(block_index));

        outgoing_definite.copy_from(&incoming_definite);
        outgoing_definite.subtract_with_words(overwritten_names.row(block_index));
        outgoing_definite.union_with_words(definite_gen.row(block_index));

        maybe_in.replace_row_if_changed(block_index, incoming_maybe.as_words());
        definite_in.replace_row_if_changed(block_index, incoming_definite.as_words());
        let maybe_out_changed =
            maybe_out.replace_row_if_changed(block_index, outgoing_maybe.as_words());
        let definite_out_changed =
            definite_out.replace_row_if_changed(block_index, outgoing_definite.as_words());
        maybe_out_changed || definite_out_changed
    });

    DenseInitializedNameStates {
        maybe_in,
        maybe_out,
        definite_in,
        definite_out,
    }
}

fn entry_binding_root_blocks(cfg: &ControlFlowGraph) -> FxHashSet<BlockId> {
    cfg.scope_entries
        .values()
        .copied()
        .chain(
            cfg.blocks()
                .iter()
                .filter(|block| cfg.predecessors(block.id).is_empty())
                .map(|block| block.id),
        )
        .collect()
}

pub(super) fn compute_scope_components_dense(
    cfg: &ControlFlowGraph,
    scope_count: usize,
    block_count: usize,
) -> Vec<ExactScopeComponent> {
    let mut components = (0..scope_count)
        .map(|_| ExactScopeComponent::new(block_count))
        .collect::<Vec<_>>();

    for (scope, entry) in &cfg.scope_entries {
        let blocks = reachable_blocks_dense(cfg, *entry, block_count);
        components[scope.index()] = ExactScopeComponent { blocks };
    }

    components
}

pub(super) fn block_exits_component(
    cfg: &ControlFlowGraph,
    component_blocks: &DenseBitSet,
    block_id: BlockId,
) -> bool {
    let successors = cfg.successors(block_id);
    successors.is_empty()
        || successors
            .iter()
            .any(|(successor, _)| !component_blocks.contains(successor.index()))
}
