//! Variable dataflow analyses shared by semantic queries and linter rules.
//!
//! The analyses in this module answer questions that need control-flow-aware
//! shell reasoning rather than local syntax checks. For example, in:
//!
//! ```sh
//! value=old
//! if fetch_value; then
//!   value=new
//! fi
//! printf '%s\n' "$value"
//! ```
//!
//! the final expansion may see either assignment, so reaching definitions and
//! initialization state must be computed over the CFG instead of by scanning
//! bindings in source order. The public crate surface stays in this facade
//! module; the implementation is split by analysis concern below.

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_ast::Span;
use smallvec::SmallVec;

use crate::dense_bit_set::{DenseBitMatrix, DenseBitSet};
use crate::function_resolution::resolved_function_calls_with_callee_scope;
use crate::reachability::block_reaches_without;
use crate::runtime::RuntimePrelude;
use crate::{
    Binding, BindingAttributes, BindingId, BindingKind, BlockId, CallSite, ContractCertainty,
    ControlFlowGraph, EdgeKind, ProvidedBinding, ProvidedBindingKind, Reference, ReferenceId,
    ReferenceKind, Scope, ScopeId, ScopeKind, SpanKey, SyntheticRead, UnreachableCauseKind,
    UnusedAssignmentAnalysisOptions,
};
use std::sync::OnceLock;

/// Materialized reaching-definition sets for each CFG block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachingDefinitions {
    /// Definitions reaching the start of each block.
    pub reaching_in: FxHashMap<BlockId, FxHashSet<BindingId>>,
    /// Definitions reaching the end of each block.
    pub reaching_out: FxHashMap<BlockId, FxHashSet<BindingId>>,
}

/// One assignment that was proven unused together with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedAssignment {
    /// Unused binding.
    pub binding: BindingId,
    /// Why the assignment is considered unused.
    pub reason: UnusedReason,
}

/// Reason an assignment is considered unused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnusedReason {
    /// The value is overwritten before any read.
    Overwritten {
        /// Binding that overwrites the unused value.
        by: BindingId,
    },
    /// Control reaches the end of its relevant scope without a read.
    ScopeEnd,
}

/// One reference that may read an uninitialized binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninitializedReference {
    /// Reference that may be uninitialized.
    pub reference: ReferenceId,
    /// How certain the analysis is.
    pub certainty: UninitializedCertainty,
}

/// Confidence for an uninitialized-reference result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninitializedCertainty {
    /// Every feasible path leaves the reference uninitialized.
    Definite,
    /// At least one feasible path leaves the reference uninitialized.
    Possible,
}

/// One region of unreachable code and the syntax that makes it unreachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadCode {
    /// Spans classified as unreachable.
    pub unreachable: Vec<Span>,
    /// Span of the control-flow construct that causes the region to be unreachable.
    pub cause: Span,
    /// Broad category for the cause.
    pub cause_kind: UnreachableCauseKind,
}

#[cfg(test)]
/// Full dataflow result materialized for crate tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataflowResult {
    /// Unused-assignment diagnostics with reasons.
    pub unused_assignments: Vec<UnusedAssignment>,
    /// Uninitialized-reference diagnostics.
    pub uninitialized_references: Vec<UninitializedReference>,
    /// Unreachable-code diagnostics.
    pub dead_code: Vec<DeadCode>,
    pub(crate) unused_assignment_ids: Vec<BindingId>,
}

#[cfg(test)]
impl DataflowResult {
    /// Returns the binding ids reported as unused in test-only result materialization.
    pub fn unused_assignment_ids(&self) -> &[BindingId] {
        &self.unused_assignment_ids
    }
}

/// Borrowed semantic inputs needed by dataflow analyses.
///
/// This keeps the individual analysis modules narrow: they receive one view of
/// the semantic model plus the exact cached dataflow bundle, instead of each
/// module rebuilding indexes over bindings, references, calls, or CFG blocks.
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
    pub(crate) visible_function_call_bindings: &'a FxHashMap<SpanKey, BindingId>,
    pub(crate) function_body_scopes: &'a FxHashMap<BindingId, ScopeId>,
    pub(crate) indirect_targets_by_reference: &'a FxHashMap<ReferenceId, Vec<BindingId>>,
    pub(crate) array_like_indirect_expansion_refs: &'a FxHashSet<ReferenceId>,
    pub(crate) synthetic_reads: &'a [SyntheticRead],
    pub(crate) entry_bindings: &'a [BindingId],
}

mod dead_code;
mod dense;
mod exact;
mod scope_reads;
mod uninitialized;
mod unused;

use dead_code::build_dead_code;
#[cfg(test)]
pub(crate) use dense::materialize_reaching_definitions;
use dense::*;
pub(crate) use exact::ExactVariableDataflow;
use scope_reads::{
    ScopeReadPlan, binding_has_future_reads_before_local_shadow, build_scope_read_plans,
    compute_compatibility_read_sets, compute_transitive_read_sets, is_function_escape_candidate,
    next_shadowing_local_declarations, reachable_blocks_dense,
};
pub(crate) use scope_reads::{
    binding_initializes_name, function_binding_certainty, summarize_scope_provided_bindings,
    summarize_scope_provided_functions,
};
use uninitialized::analyze_uninitialized_references_exact;
use unused::analyze_unused_assignments_exact;

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
        forward_block_order: OnceLock::new(),
        backward_block_order: OnceLock::new(),
        reaching_definitions: OnceLock::new(),
        initialized_name_states: OnceLock::new(),
        c006_initialized_name_states: OnceLock::new(),
        scope_components: OnceLock::new(),
    }
}

#[cfg(test)]
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

    DataflowResult {
        unused_assignments: unused_assignments.unused_assignments,
        uninitialized_references,
        dead_code,
        unused_assignment_ids: unused_assignments.unused_assignment_ids,
    }
}

pub(crate) fn analyze_dead_code(cfg: &ControlFlowGraph) -> Vec<DeadCode> {
    build_dead_code(cfg)
}

#[cfg(test)]
mod tests;
