use super::*;

/// Lazily materialized exact variable-flow indexes for one semantic model.
///
/// This is the shared cache point for dataflow-heavy semantic queries. A rule
/// such as unused assignment and a query such as reaching definitions can reuse
/// the same dense name table, binding/block indexes, and CFG orders without
/// constructing their own per-rule maps. That matters for shell snippets like:
///
/// ```sh
/// make_flag() { flag=1; }
/// make_flag
/// printf '%s\n' "$flag"
/// ```
///
/// where deciding whether `flag` is visible after the call requires the same
/// scope and call summaries that unused-assignment analysis also needs.
#[derive(Debug)]
pub(crate) struct ExactVariableDataflow {
    pub(super) names: NameTable,
    pub(super) binding_data: DenseBindingData,
    pub(super) binding_blocks: Vec<Option<BlockId>>,
    pub(super) reference_blocks: Vec<Option<BlockId>>,
    pub(super) unreachable_blocks: DenseBitSet,
    pub(super) forward_block_order: OnceLock<Box<[BlockId]>>,
    pub(super) backward_block_order: OnceLock<Box<[BlockId]>>,
    pub(super) reaching_definitions: OnceLock<DenseReachingDefinitions>,
    pub(super) initialized_name_states: OnceLock<DenseInitializedNameStates>,
    pub(super) c006_initialized_name_states: OnceLock<DenseInitializedNameStates>,
    pub(super) scope_components: OnceLock<Vec<ExactScopeComponent>>,
}

impl ExactVariableDataflow {
    pub(super) fn forward_block_order(&self, cfg: &ControlFlowGraph) -> &[BlockId] {
        self.forward_block_order
            .get_or_init(|| compute_reverse_postorder(cfg))
    }

    pub(super) fn backward_block_order(&self, cfg: &ControlFlowGraph) -> &[BlockId] {
        self.backward_block_order
            .get_or_init(|| compute_postorder(cfg))
    }

    pub(super) fn reaching_definitions<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a DenseReachingDefinitions {
        self.reaching_definitions.get_or_init(|| {
            compute_reaching_definitions_dense(
                context.cfg,
                context.bindings,
                &self.binding_data,
                context.entry_bindings,
                self.forward_block_order(context.cfg),
            )
        })
    }

    pub(super) fn initialized_name_states<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a DenseInitializedNameStates {
        self.initialized_name_states.get_or_init(|| {
            compute_initialized_name_states_dense(
                context.cfg,
                context.bindings,
                &self.binding_data,
                context.entry_bindings,
                self.forward_block_order(context.cfg),
            )
        })
    }

    pub(super) fn c006_initialized_name_states<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a DenseInitializedNameStates {
        self.c006_initialized_name_states.get_or_init(|| {
            let extra_initialized_names = context
                .parameter_guard_flow_refs
                .iter()
                .copied()
                .filter_map(|reference_id| {
                    let reference = &context.references[reference_id.index()];
                    let block = self.reference_blocks[reference_id.index()]?;
                    let name = self.names.get(&reference.name)?;
                    Some((block, name))
                })
                .collect::<Vec<_>>();
            compute_initialized_name_states_dense_with_extra_name_gens(
                context.cfg,
                context.bindings,
                &self.binding_data,
                context.entry_bindings,
                &extra_initialized_names,
                self.forward_block_order(context.cfg),
            )
        })
    }

    pub(super) fn scope_components<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a [ExactScopeComponent] {
        self.scope_components
            .get_or_init(|| {
                compute_scope_components_dense(
                    context.cfg,
                    context.scopes.len(),
                    context.cfg.blocks().len(),
                )
            })
            .as_slice()
    }

    pub(crate) fn reaching_bindings_for_reference(
        &self,
        context: &DataflowContext<'_>,
        reference: &Reference,
    ) -> Vec<BindingId> {
        let Some(block_id) = self.reference_blocks[reference.id.index()] else {
            return Vec::new();
        };
        if self.unreachable_blocks.contains(block_id.index()) {
            return Vec::new();
        }

        let Some(name_id) = self.names.get(&reference.name) else {
            return Vec::new();
        };
        let incoming = &self.reaching_definitions(context).reaching_in[block_id.index()];

        self.binding_data.bindings_for_name[name_id.index()]
            .iter_ones()
            .filter(|binding_index| incoming.contains(*binding_index))
            .map(|binding_index| BindingId(binding_index as u32))
            .collect()
    }

    pub(crate) fn binding_block(&self, binding_id: BindingId) -> Option<BlockId> {
        self.binding_blocks[binding_id.index()]
    }

    pub(crate) fn reference_block(&self, reference: &Reference) -> Option<BlockId> {
        self.reference_blocks[reference.id.index()]
    }
}
