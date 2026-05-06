use super::*;

/// Finds variable reads that can happen before a real initialization.
///
/// The analysis distinguishes definite and possible cases. For example,
/// `echo "$x"` is definite if no path initializes `x`, while:
///
/// ```sh
/// if ready; then
///   x=1
/// fi
/// echo "$x"
/// ```
///
/// is possible because the false branch reaches the read without assigning
/// `x`. Parameter guards and file-entry contracts are handled as semantic
/// inputs so this pass stays focused on CFG state.
pub(super) fn analyze_uninitialized_references_exact(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> Vec<UninitializedReference> {
    let initialized_name_states = exact.c006_initialized_name_states(context);
    let maybe_defined = &initialized_name_states.maybe_in;
    let definitely_defined = &initialized_name_states.definite_in;
    let guarded_parameter_ref_keys = guarded_parameter_reference_keys(context, exact);
    let parameter_guard_flow_index = ParameterGuardFlowIndex::new(context, exact);

    let mut uninitialized_references = Vec::new();
    for reference in context.references {
        let Some(name_id) = exact.names.get(&reference.name) else {
            continue;
        };
        if matches!(
            reference.kind,
            ReferenceKind::ImplicitRead
                | ReferenceKind::DeclarationName
                | ReferenceKind::ParameterPattern
                | ReferenceKind::ParameterSliceArithmetic
        ) || context.predefined_runtime_refs.contains(&reference.id)
            || context.guarded_parameter_refs.contains(&reference.id)
            || context
                .self_referential_assignment_refs
                .contains(&reference.id)
            || guarded_parameter_ref_keys.contains(&(name_id, SpanKey::new(reference.span)))
        {
            continue;
        }
        if matches!(reference.kind, ReferenceKind::IndirectExpansion)
            && (context.resolved.contains_key(&reference.id)
                || context
                    .indirect_targets_by_reference
                    .contains_key(&reference.id))
        {
            continue;
        }
        let Some(block_id) = exact.reference_blocks[reference.id.index()] else {
            continue;
        };
        // File-entry contracts describe ambient names supplied by the caller
        // environment, not assignments performed by this file, so a read that
        // resolves only to such an import remains uninitialized until we see a
        // real write in dataflow.
        if reference_resolves_to_file_entry_contract_variable(context, reference) {
            uninitialized_references.push(UninitializedReference {
                reference: reference.id,
                certainty: UninitializedCertainty::Definite,
            });
            continue;
        }
        let same_block_guard =
            parameter_guard_flow_index.precedes_reference(reference, block_id, name_id);
        let maybe = maybe_defined.contains(block_id.index(), name_id.index()) || same_block_guard;
        let definite =
            definitely_defined.contains(block_id.index(), name_id.index()) || same_block_guard;

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

fn guarded_parameter_reference_keys(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> FxHashSet<(NameId, SpanKey)> {
    context
        .guarded_parameter_refs
        .iter()
        .copied()
        .filter_map(|guard_id| {
            let guard = &context.references[guard_id.index()];
            let name = exact.names.get(&guard.name)?;
            Some((name, SpanKey::new(guard.span)))
        })
        .collect()
}

#[derive(Debug, Default)]
struct ParameterGuardFlowIndex {
    offsets_by_block_name: FxHashMap<(BlockId, NameId), Vec<usize>>,
}

impl ParameterGuardFlowIndex {
    fn new(context: &DataflowContext<'_>, exact: &ExactVariableDataflow) -> Self {
        let mut offsets_by_block_name = FxHashMap::<(BlockId, NameId), Vec<usize>>::default();
        for guard_id in context.parameter_guard_flow_refs.iter().copied() {
            let guard = &context.references[guard_id.index()];
            let Some(block) = exact.reference_blocks[guard_id.index()] else {
                continue;
            };
            let Some(name) = exact.names.get(&guard.name) else {
                continue;
            };
            offsets_by_block_name
                .entry((block, name))
                .or_default()
                .push(guard.span.start.offset);
        }
        for offsets in offsets_by_block_name.values_mut() {
            offsets.sort_unstable();
        }
        Self {
            offsets_by_block_name,
        }
    }

    fn precedes_reference(&self, reference: &Reference, block: BlockId, name: NameId) -> bool {
        self.offsets_by_block_name
            .get(&(block, name))
            .is_some_and(|offsets| {
                offsets.partition_point(|offset| *offset < reference.span.start.offset) > 0
            })
    }
}

fn reference_resolves_to_file_entry_contract_variable(
    context: &DataflowContext<'_>,
    reference: &Reference,
) -> bool {
    let Some(binding_id) = context.resolved.get(&reference.id).copied() else {
        return false;
    };
    let binding = &context.bindings[binding_id.index()];
    matches!(binding.kind, BindingKind::Imported)
        && !binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FUNCTION)
        && binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FILE_ENTRY)
        && !binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FILE_ENTRY_INITIALIZED)
}
