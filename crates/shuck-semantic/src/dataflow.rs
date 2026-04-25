use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_ast::Span;
use smallvec::SmallVec;

use crate::runtime::RuntimePrelude;
use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BlockId, CallSite, ContractCertainty,
    ControlFlowGraph, EdgeKind, FunctionScopeKind, ProvidedBinding, ProvidedBindingKind, Reference,
    ReferenceId, ReferenceKind, Scope, ScopeId, ScopeKind, SpanKey, SyntheticRead,
    UnreachableCauseKind, UnusedAssignmentAnalysisOptions,
};
use std::sync::OnceLock;

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
    pub cause_kind: UnreachableCauseKind,
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unused_assignment_ids(&self) -> &[BindingId] {
        &self.unused_assignment_ids
    }
}

pub(crate) struct DataflowContext<'a> {
    pub(crate) cfg: &'a ControlFlowGraph,
    pub(crate) runtime: &'a RuntimePrelude,
    pub(crate) scopes: &'a [Scope],
    pub(crate) bindings: &'a [Binding],
    pub(crate) references: &'a [Reference],
    pub(crate) predefined_runtime_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) guarded_parameter_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) parameter_guard_flow_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) self_referential_assignment_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) resolved: &'a FxHashMap<ReferenceId, BindingId>,
    pub(crate) call_sites: &'a FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    pub(crate) indirect_targets_by_reference: &'a FxHashMap<ReferenceId, Vec<BindingId>>,
    pub(crate) array_like_indirect_expansion_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) synthetic_reads: &'a [SyntheticRead],
    pub(crate) entry_bindings: &'a [BindingId],
}

#[derive(Debug)]
pub(crate) struct ExactVariableDataflow {
    names: NameTable,
    binding_data: DenseBindingData,
    binding_blocks: Vec<Option<BlockId>>,
    reference_blocks: Vec<Option<BlockId>>,
    unreachable_blocks: DenseBitSet,
    reaching_definitions: OnceLock<DenseReachingDefinitions>,
    initialized_name_states: OnceLock<DenseInitializedNameStates>,
    c006_initialized_name_states: OnceLock<DenseInitializedNameStates>,
    scope_components: OnceLock<Vec<ExactScopeComponent>>,
}

impl ExactVariableDataflow {
    fn reaching_definitions<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a DenseReachingDefinitions {
        self.reaching_definitions.get_or_init(|| {
            compute_reaching_definitions_dense(
                context.cfg,
                context.bindings,
                &self.binding_data,
                context.entry_bindings,
            )
        })
    }

    fn initialized_name_states<'a>(
        &'a self,
        context: &DataflowContext<'_>,
    ) -> &'a DenseInitializedNameStates {
        self.initialized_name_states.get_or_init(|| {
            compute_initialized_name_states_dense(
                context.cfg,
                context.bindings,
                &self.binding_data,
                context.entry_bindings,
            )
        })
    }

    fn c006_initialized_name_states<'a>(
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
            )
        })
    }

    fn scope_components<'a>(&'a self, context: &DataflowContext<'_>) -> &'a [ExactScopeComponent] {
        self.scope_components
            .get_or_init(|| {
                let reaching_definitions = self.reaching_definitions(context);
                compute_scope_components_dense(
                    context.cfg,
                    context.scopes.len(),
                    context.cfg.blocks().len(),
                    context.bindings.len(),
                    &reaching_definitions.reaching_out,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnusedAssignmentsResult {
    unused_assignments: Vec<UnusedAssignment>,
    unused_assignment_ids: Vec<BindingId>,
}

pub(crate) fn analyze_uninitialized_references(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> Vec<UninitializedReference> {
    analyze_uninitialized_references_exact(context, exact)
}

pub(crate) fn analyze_unused_assignments(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> Vec<BindingId> {
    analyze_unused_assignments_with_options(
        context,
        exact,
        UnusedAssignmentAnalysisOptions::default(),
    )
}

pub(crate) fn analyze_unused_assignments_with_options(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    options: UnusedAssignmentAnalysisOptions,
) -> Vec<BindingId> {
    analyze_unused_assignments_exact(context, exact, options)
        .unused_assignments
        .into_iter()
        .map(|unused| unused.binding)
        .collect()
}

pub(crate) fn build_exact_variable_dataflow(
    context: &DataflowContext<'_>,
) -> ExactVariableDataflow {
    let names = build_name_table(
        context.bindings,
        context.references,
        context.synthetic_reads,
    );
    let binding_data = build_dense_binding_data(context.bindings, context.scopes, &names);
    let binding_blocks = build_binding_block_index(context.cfg, context.bindings.len());
    let reference_blocks = build_reference_block_index(context.cfg, context.references.len());
    let unreachable_blocks = build_unreachable_block_set(context.cfg);

    ExactVariableDataflow {
        names,
        binding_data,
        binding_blocks,
        reference_blocks,
        unreachable_blocks,
        reaching_definitions: OnceLock::new(),
        initialized_name_states: OnceLock::new(),
        c006_initialized_name_states: OnceLock::new(),
        scope_components: OnceLock::new(),
    }
}

pub(crate) fn analyze(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> DataflowResult {
    let unused_assignments = analyze_unused_assignments_exact(
        context,
        exact,
        UnusedAssignmentAnalysisOptions::default(),
    );
    let uninitialized_references = analyze_uninitialized_references_exact(context, exact);
    let dead_code = build_dead_code(context.cfg);
    let reaching_definitions = exact.reaching_definitions(context);

    DataflowResult {
        reaching_definitions: materialize_reaching_definitions(context.cfg, reaching_definitions),
        unused_assignments: unused_assignments.unused_assignments,
        uninitialized_references,
        dead_code,
        unused_assignment_ids: unused_assignments.unused_assignment_ids,
    }
}

pub(crate) fn analyze_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
    build_dead_code(cfg)
}

fn analyze_uninitialized_references_exact(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
) -> Vec<UninitializedReference> {
    let initialized_name_states = exact.c006_initialized_name_states(context);
    let maybe_defined = &initialized_name_states.maybe_in;
    let definitely_defined = &initialized_name_states.definite_in;
    let guarded_parameter_ref_keys = guarded_parameter_reference_keys(context);

    let mut uninitialized_references = Vec::new();
    for reference in context.references {
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
            || guarded_parameter_ref_keys
                .contains(&(reference.name.clone(), SpanKey::new(reference.span)))
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
        let Some(name_id) = exact.names.get(&reference.name) else {
            continue;
        };
        let same_block_guard = parameter_guard_flow_precedes_reference_in_same_block(
            context, exact, reference, block_id,
        );
        let maybe = maybe_defined[block_id.index()].contains(name_id.index()) || same_block_guard;
        let definite =
            definitely_defined[block_id.index()].contains(name_id.index()) || same_block_guard;

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

fn guarded_parameter_reference_keys(context: &DataflowContext<'_>) -> FxHashSet<(Name, SpanKey)> {
    context
        .guarded_parameter_refs
        .iter()
        .copied()
        .map(|guard_id| {
            let guard = &context.references[guard_id.index()];
            (guard.name.clone(), SpanKey::new(guard.span))
        })
        .collect()
}

fn parameter_guard_flow_precedes_reference_in_same_block(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    reference: &Reference,
    block_id: BlockId,
) -> bool {
    context
        .parameter_guard_flow_refs
        .iter()
        .copied()
        .any(|guard_id| {
            guard_id != reference.id
                && exact.reference_blocks[guard_id.index()] == Some(block_id)
                && {
                    let guard = &context.references[guard_id.index()];
                    guard.name == reference.name
                        && guard.span.start.offset < reference.span.start.offset
                }
        })
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

fn build_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
    let mut dead_code_by_cause: FxHashMap<
        (usize, usize, UnreachableCauseKind),
        (crate::cfg::UnreachableCause, Vec<Span>),
    > = FxHashMap::default();
    for block_id in cfg.unreachable() {
        let block = cfg.block(*block_id);
        if block.commands.is_empty() {
            continue;
        }
        let cause =
            cfg.unreachable_cause(*block_id)
                .unwrap_or_else(|| crate::cfg::UnreachableCause {
                    span: block.commands[0],
                    kind: UnreachableCauseKind::ShellTerminator,
                });
        dead_code_by_cause
            .entry((cause.span.start.offset, cause.span.end.offset, cause.kind))
            .or_insert_with(|| (cause, Vec::new()))
            .1
            .extend(block.commands.iter().copied());
    }
    let mut dead_code = dead_code_by_cause
        .into_iter()
        .map(|(_, (cause, unreachable))| DeadCode {
            unreachable: outermost_unreachable_spans(unreachable),
            cause: cause.span,
            cause_kind: cause.kind,
        })
        .collect::<Vec<_>>();
    dead_code.sort_by_key(|dead| (dead.cause.start.offset, dead.cause.end.offset));
    dead_code
}

fn outermost_unreachable_spans(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by(|left, right| {
        left.start
            .offset
            .cmp(&right.start.offset)
            .then_with(|| right.end.offset.cmp(&left.end.offset))
    });

    let mut outermost = Vec::new();
    for span in spans {
        if outermost
            .iter()
            .any(|outer| span_contained_by(span, *outer))
        {
            continue;
        }
        if outermost.contains(&span) {
            continue;
        }
        outermost.push(span);
    }
    outermost
}

fn span_contained_by(inner: Span, outer: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
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

fn analyze_unused_assignments_exact(
    context: &DataflowContext<'_>,
    exact: &ExactVariableDataflow,
    options: UnusedAssignmentAnalysisOptions,
) -> UnusedAssignmentsResult {
    let reaching_definitions = exact.reaching_definitions(context);
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
    let scope_components = exact.scope_components(context);
    let interprocedural_reads = if context.call_sites.is_empty() {
        None
    } else {
        let (read_plans, callers_by_callee) = build_scope_read_plans(
            context.cfg,
            context.scopes,
            context.bindings,
            context.references,
            context.synthetic_reads,
            &exact.reference_blocks,
            &reference_name_ids,
            &synthetic_read_name_ids,
            context.call_sites,
            exact.names.len(),
        );
        let interprocedural = compute_interprocedural_read_sets(
            &read_plans,
            &callers_by_callee,
            context.scopes,
            exact.names.len(),
        );
        Some((read_plans, interprocedural))
    };

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

    for (reference_index, reference) in context.references.iter().enumerate() {
        let Some(block_id) = exact.reference_blocks[reference_index] else {
            continue;
        };
        if exact.unreachable_blocks.contains(block_id.index())
            && !options.report_unreachable_assignments
        {
            continue;
        }

        let incoming = &reaching_definitions.reaching_in[block_id.index()];
        let name_id = reference_name_ids[reference_index];
        let resolved_binding_id = context.resolved.get(&reference.id).copied();

        if let Some(resolved_binding_id) = resolved_binding_id
            && resolved_binding_shadows_name_without_initializing(Some(
                &context.bindings[resolved_binding_id.index()],
            ))
        {
            mark_reaching_defs_used_except(
                &mut used_bindings,
                incoming,
                &exact.binding_data.bindings_for_name[name_id.index()],
                resolved_binding_id,
            );
        } else {
            used_bindings.or_intersection_with(
                incoming,
                &exact.binding_data.bindings_for_name[name_id.index()],
            );
        }

        let Some(resolved_binding_id) = resolved_binding_id else {
            continue;
        };
        let resolved_binding = &context.bindings[resolved_binding_id.index()];
        let component = &scope_components[resolved_binding.scope.index()];
        if !component.blocks.contains(block_id.index()) {
            used_bindings.or_intersection3_with(
                &component.exit_defs,
                &exact.binding_data.bindings_for_name[name_id.index()],
                &exact.binding_data.bindings_in_scope[resolved_binding.scope.index()],
            );
        }

        if (options.treat_indirect_expansion_targets_as_used
            || context
                .array_like_indirect_expansion_refs
                .contains(&reference.id))
            && let Some(candidates) = context.indirect_targets_by_reference.get(&reference.id)
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

    for (read_index, synthetic_read) in context.synthetic_reads.iter().enumerate() {
        let Some(block_id) = command_block_for_span(context.cfg, synthetic_read.span) else {
            continue;
        };
        if exact.unreachable_blocks.contains(block_id.index())
            && !options.report_unreachable_assignments
        {
            continue;
        }
        used_bindings.or_intersection_with(
            &reaching_definitions.reaching_in[block_id.index()],
            &exact.binding_data.bindings_for_name[synthetic_read_name_ids[read_index].index()],
        );
    }

    if let Some((read_plans, interprocedural)) = &interprocedural_reads {
        for plan in read_plans {
            for call in &plan.calls {
                let Some(block_id) = command_block_for_span(context.cfg, call.span) else {
                    continue;
                };
                if exact.unreachable_blocks.contains(block_id.index())
                    && !options.report_unreachable_assignments
                {
                    continue;
                }
                mark_reaching_defs_for_names_used(
                    &mut used_bindings,
                    &reaching_definitions.reaching_in[block_id.index()],
                    &exact.binding_data.binding_name_ids,
                    &interprocedural.transitive_reads[call.callee_scope.index()],
                );
            }
        }

        for binding in context.bindings {
            if is_function_escape_candidate(binding, context.scopes)
                && binding_has_future_reads_before_local_shadow(
                    binding,
                    exact.binding_data.binding_name_ids[binding.id.index()],
                    context.bindings,
                    context.cfg,
                    &exact.binding_blocks,
                    read_plans,
                    &interprocedural.transitive_reads,
                    &interprocedural.future_reads,
                    &interprocedural.escape_reads,
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

    fn clear(&mut self) {
        self.words.fill(0);
    }

    fn copy_from(&mut self, other: &Self) {
        debug_assert_eq!(self.words.len(), other.words.len());
        self.words.copy_from_slice(&other.words);
    }

    fn replace_if_changed(&mut self, other: &Self) -> bool {
        debug_assert_eq!(self.words.len(), other.words.len());
        if self.words == other.words {
            false
        } else {
            self.copy_from(other);
            true
        }
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

fn materialize_reaching_definitions(
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
struct DenseInitializedNameStates {
    maybe_in: Vec<DenseBitSet>,
    maybe_out: Vec<DenseBitSet>,
    definite_in: Vec<DenseBitSet>,
    definite_out: Vec<DenseBitSet>,
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
    block: Option<BlockId>,
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
        let Some(name_id) = names.get(&binding.name) else {
            unreachable!("binding name interned");
        };
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
    entry_bindings: &[BindingId],
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
    let mut changed = true;
    while changed {
        changed = false;
        for block in cfg.blocks() {
            let block_index = block.id.index();
            incoming.clear();
            for predecessor in cfg.predecessors(block.id) {
                incoming.union_with(&reaching_out[predecessor.index()]);
            }
            if entry_blocks.contains(&block.id) {
                for binding in entry_bindings {
                    incoming.insert(binding.index());
                }
            }

            carried.copy_from(&incoming);
            carried.subtract_with(&kill_sets[block_index]);
            outgoing.copy_from(&gen_sets[block_index]);
            outgoing.union_with(&carried);

            if reaching_in[block_index].replace_if_changed(&incoming) {
                changed = true;
            }
            if reaching_out[block_index].replace_if_changed(&outgoing) {
                changed = true;
            }
        }
    }

    DenseReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

fn compute_initialized_name_states_dense(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
    entry_bindings: &[BindingId],
) -> DenseInitializedNameStates {
    compute_initialized_name_states_dense_with_extra_name_gens(
        cfg,
        bindings,
        binding_data,
        entry_bindings,
        &[],
    )
}

fn compute_initialized_name_states_dense_with_extra_name_gens(
    cfg: &ControlFlowGraph,
    bindings: &[Binding],
    binding_data: &DenseBindingData,
    entry_bindings: &[BindingId],
    extra_initialized_names: &[(BlockId, NameId)],
) -> DenseInitializedNameStates {
    let entry_blocks = entry_binding_root_blocks(cfg);
    let block_count = cfg.blocks().len();
    let name_count = binding_data.bindings_for_name.len();
    let mut maybe_gen = vec![DenseBitSet::new(name_count); block_count];
    let mut definite_gen = vec![DenseBitSet::new(name_count); block_count];
    let mut overwritten_names = vec![DenseBitSet::new(name_count); block_count];

    for block in cfg.blocks() {
        let block_index = block.id.index();
        for binding in &block.bindings {
            let name_id = binding_data.binding_name_ids[binding.index()];
            overwritten_names[block_index].insert(name_id.index());
            match binding_initializes_name(&bindings[binding.index()]) {
                Some(ContractCertainty::Definite) => {
                    maybe_gen[block_index].insert(name_id.index());
                    definite_gen[block_index].insert(name_id.index());
                }
                Some(ContractCertainty::Possible) => {
                    maybe_gen[block_index].insert(name_id.index());
                }
                None => {}
            }
        }
    }

    for (block, name) in extra_initialized_names {
        maybe_gen[block.index()].insert(name.index());
        definite_gen[block.index()].insert(name.index());
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

    let mut maybe_in = vec![DenseBitSet::new(name_count); block_count];
    let mut maybe_out = vec![DenseBitSet::new(name_count); block_count];
    let mut definite_in = vec![all_names.clone(); block_count];
    let mut definite_out = vec![all_names; block_count];
    let mut incoming_maybe = DenseBitSet::new(name_count);
    let mut incoming_definite = DenseBitSet::new(name_count);
    let mut outgoing_maybe = DenseBitSet::new(name_count);
    let mut outgoing_definite = DenseBitSet::new(name_count);
    let mut changed = true;
    while changed {
        changed = false;
        for block in cfg.blocks() {
            let block_index = block.id.index();

            incoming_maybe.clear();
            for predecessor in cfg.predecessors(block.id) {
                incoming_maybe.union_with(&maybe_out[predecessor.index()]);
            }
            if entry_blocks.contains(&block.id) {
                incoming_maybe.union_with(&entry_maybe);
            }

            let predecessors = cfg.predecessors(block.id);
            let uses_virtual_entry_boundary = entry_blocks.contains(&block.id)
                && predecessors.iter().all(|predecessor| {
                    cfg.successors(*predecessor)
                        .iter()
                        .any(|(successor, kind)| {
                            *successor == block.id && *kind == EdgeKind::LoopBack
                        })
                });
            if uses_virtual_entry_boundary {
                incoming_definite.copy_from(&entry_definite);
            } else if let Some(first_predecessor) = predecessors.first() {
                incoming_definite.copy_from(&definite_out[first_predecessor.index()]);
            } else {
                incoming_definite.clear();
            }
            for (predecessor_index, predecessor) in predecessors.iter().enumerate() {
                if !uses_virtual_entry_boundary && predecessor_index == 0 {
                    continue;
                }
                incoming_definite.intersect_with(&definite_out[predecessor.index()]);
            }

            outgoing_maybe.copy_from(&incoming_maybe);
            outgoing_maybe.subtract_with(&overwritten_names[block_index]);
            outgoing_maybe.union_with(&maybe_gen[block_index]);

            outgoing_definite.copy_from(&incoming_definite);
            outgoing_definite.subtract_with(&overwritten_names[block_index]);
            outgoing_definite.union_with(&definite_gen[block_index]);

            if maybe_in[block_index].replace_if_changed(&incoming_maybe) {
                changed = true;
            }
            if maybe_out[block_index].replace_if_changed(&outgoing_maybe) {
                changed = true;
            }
            if definite_in[block_index].replace_if_changed(&incoming_definite) {
                changed = true;
            }
            if definite_out[block_index].replace_if_changed(&outgoing_definite) {
                changed = true;
            }
        }
    }

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
        if let Some(scope_exits) = cfg.scope_exits(*scope) {
            for exit in scope_exits {
                exit_defs.union_with(&reaching_out[exit.index()]);
            }
        } else {
            for block_index in blocks.iter_ones() {
                let block_id = BlockId(block_index as u32);
                if block_exits_component(cfg, &blocks, block_id) {
                    exit_defs.union_with(&reaching_out[block_index]);
                }
            }
        }
        components[scope.index()] = ExactScopeComponent { blocks, exit_defs };
    }

    components
}

fn block_exits_component(
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
        let maybe_names = &initialized_states.maybe_out[exit_block.index()];
        let definite_names = &initialized_states.definite_out[exit_block.index()];

        for name in &eligible_names {
            let Some(name_id) = exact.names.get(name) else {
                continue;
            };
            if maybe_names.contains(name_id.index()) {
                *maybe_counts.entry(name.clone()).or_default() += 1;
            }
            if definite_names.contains(name_id.index()) {
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
    cfg: &ControlFlowGraph,
    scopes: &[Scope],
    bindings: &[Binding],
    references: &[Reference],
    synthetic_reads: &[SyntheticRead],
    reference_blocks: &[Option<BlockId>],
    reference_name_ids: &[NameId],
    synthetic_read_name_ids: &[NameId],
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
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

fn compute_interprocedural_read_sets(
    read_plans: &[ScopeReadPlan],
    callers_by_callee: &[Vec<CallerReadSite>],
    scopes: &[Scope],
    name_count: usize,
) -> InterproceduralReadSets {
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

    let future_reads = build_future_read_summaries(read_plans, &transitive_reads, name_count);
    let mut escape_reads = vec![DenseBitSet::new(name_count); read_plans.len()];
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

    InterproceduralReadSets {
        transitive_reads,
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
fn binding_has_future_reads_before_local_shadow(
    binding: &Binding,
    name_id: NameId,
    bindings: &[Binding],
    cfg: &ControlFlowGraph,
    binding_blocks: &[Option<BlockId>],
    read_plans: &[ScopeReadPlan],
    transitive_reads: &[DenseBitSet],
    future_reads: &[ScopeFutureReads],
    escape_reads: &[DenseBitSet],
) -> bool {
    let shadow = next_shadowing_local_declaration(binding, bindings);
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

fn next_shadowing_local_declaration<'a>(
    binding: &Binding,
    bindings: &'a [Binding],
) -> Option<&'a Binding> {
    bindings
        .iter()
        .skip(binding.id.index() + 1)
        .find(|candidate| {
            candidate.name == binding.name
                && candidate.scope == binding.scope
                && candidate.span.start.offset > binding.span.start.offset
                && matches!(candidate.kind, BindingKind::Declaration(_))
                && candidate.attributes.contains(BindingAttributes::LOCAL)
        })
}

fn future_reads_contain_after_until(
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

fn block_reaches_without(
    cfg: &ControlFlowGraph,
    start: BlockId,
    end: BlockId,
    avoided: BlockId,
) -> bool {
    if start == avoided {
        return false;
    }

    let mut visited = DenseBitSet::new(cfg.blocks().len());
    let mut stack = vec![start];

    while let Some(block) = stack.pop() {
        if block == avoided || visited.contains(block.index()) {
            continue;
        }
        visited.insert(block.index());
        if block == end {
            return true;
        }
        for (successor, _) in cfg.successors(block) {
            stack.push(*successor);
        }
    }

    false
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

fn mark_reaching_defs_used_except(
    used_bindings: &mut DenseBitSet,
    incoming: &DenseBitSet,
    candidates: &DenseBitSet,
    excluded: BindingId,
) {
    for binding_index in incoming.iter_ones() {
        if binding_index != excluded.index() && candidates.contains(binding_index) {
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

fn binding_initializes_name(binding: &Binding) -> Option<ContractCertainty> {
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
        if let ScopeKind::Function(FunctionScopeKind::Named(names)) = &scope.kind
            && let Some(parent) = scope.parent
        {
            for name in names {
                scopes_by_parent_and_name
                    .entry((parent, name.clone()))
                    .or_default()
                    .push(scope.id);
            }
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
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_reads_contain_after_until_ignores_backwards_intervals() {
        let plan = ScopeReadPlan {
            direct_reads: DenseBitSet::new(1),
            calls: Vec::new(),
            events: vec![ScopeReadEvent {
                offset: 0,
                block: None,
                kind: ScopeReadEventKind::Direct(NameId(0)),
            }],
            is_function: false,
        };
        let transitive_reads = vec![DenseBitSet::new(1)];

        assert!(!future_reads_contain_after_until(
            ScopeId(0),
            10,
            5,
            NameId(0),
            &[plan],
            &transitive_reads,
        ));
    }
}
