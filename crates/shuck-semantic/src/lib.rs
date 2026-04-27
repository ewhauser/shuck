#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! Semantic analysis for shell scripts parsed by Shuck.
//!
//! The semantic model tracks scopes, bindings, references, control flow, and selected dataflow
//! facts so higher-level crates can reason about shell behavior without re-traversing the AST.
#[allow(missing_docs)]
mod analysis;
#[allow(missing_docs)]
mod binding;
#[allow(missing_docs)]
mod builder;
#[allow(missing_docs)]
mod call_graph;
#[allow(missing_docs)]
mod cfg;
#[allow(missing_docs)]
mod contract;
#[allow(missing_docs)]
mod dataflow;
#[allow(missing_docs)]
mod declaration;
#[allow(missing_docs)]
mod reachability;
#[allow(missing_docs)]
mod reference;
#[allow(missing_docs)]
mod runtime;
#[allow(missing_docs)]
mod scope;
#[allow(missing_docs)]
mod source_closure;
#[allow(missing_docs)]
mod source_ref;
#[allow(missing_docs)]
mod uninitialized;
#[allow(missing_docs)]
mod unused;
#[allow(missing_docs)]
mod zsh_options;

/// Binding types and provenance metadata discovered during semantic analysis.
pub use binding::{
    AssignmentValueOrigin, Binding, BindingAttributes, BindingId, BindingKind, BindingOrigin,
    BuiltinBindingTargetKind, LoopValueOrigin,
};
/// Call-graph structures derived from the analyzed script.
pub use call_graph::{
    CallGraph, CallSite, OverwrittenFunction, UnreachedFunction, UnreachedFunctionReason,
};
/// Control-flow graph types and flow-context annotations.
pub use cfg::{BasicBlock, BlockId, ControlFlowGraph, EdgeKind, FlowContext, UnreachableCauseKind};
/// Contract and build-option types used when constructing semantic models.
pub use contract::{
    ContractCertainty, FileContract, FileEntryBindingInitialization, FileEntryContractCollector,
    FunctionContract, ProvidedBinding, ProvidedBindingKind, SemanticBuildOptions,
};
/// Dataflow results surfaced by the semantic analysis layer.
pub use dataflow::{
    DeadCode, ReachingDefinitions, UninitializedCertainty, UninitializedReference,
    UnusedAssignment, UnusedReason,
};
/// Declaration records discovered while building the semantic model.
pub use declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
/// Reference types and identifiers tracked by the semantic model.
pub use reference::{Reference, ReferenceId, ReferenceKind};
/// Scope types and identifiers tracked by the semantic model.
pub use scope::{FunctionScopeKind, Scope, ScopeId, ScopeKind};
/// Shell parser option types reused by the semantic analysis layer.
pub use shuck_parser::{OptionValue, ShellProfile, ZshEmulationMode, ZshOptionState};
/// Source-reference records and resolution state.
pub use source_ref::{SourceRef, SourceRefDiagnosticClass, SourceRefKind, SourceRefResolution};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, File, Name, Span};
use shuck_indexer::Indexer;
use smallvec::{Array, SmallVec};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::builder::SemanticModelBuilder;
use crate::cfg::RecordedProgram;
use crate::dataflow::{DataflowContext, DataflowResult, ExactVariableDataflow};
use crate::runtime::RuntimePrelude;
use crate::source_closure::ImportedBindingContractSite;
use crate::zsh_options::ZshOptionAnalysis;

const MAX_FUNCTIONS_FOR_TERMINATION_REACHABILITY: usize = 200;
const MAX_TERMINATION_REACHABILITY_WORK: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SpanKey {
    start: usize,
    end: usize,
}

impl SpanKey {
    pub(crate) fn new(span: Span) -> Self {
        Self {
            start: span.start.offset,
            end: span.end.offset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceDirectiveOverride {
    pub(crate) kind: SourceRefKind,
    pub(crate) own_line: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndirectTargetHint {
    Exact {
        name: Name,
        array_like: bool,
    },
    Pattern {
        prefix: String,
        suffix: String,
        array_like: bool,
    },
}

/// Synthetic read introduced by semantic modeling for later analysis passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticRead {
    pub(crate) scope: ScopeId,
    pub(crate) span: Span,
    pub(crate) name: Name,
}

/// Behavior flags for unused-assignment analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnusedAssignmentAnalysisOptions {
    /// Whether a resolved scalar indirect-expansion target like `${!name}` counts as a use
    /// of the target. ShellCheck-compatible analysis leaves this disabled. Array-like
    /// targets such as `name=arr[@]; ${!name}` stay live in both modes.
    pub treat_indirect_expansion_targets_as_used: bool,
    /// Whether assignments in statically unreachable blocks should still be eligible
    /// for unused-assignment reporting.
    pub report_unreachable_assignments: bool,
}

/// Behavior flags for unreached-function analysis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnreachedFunctionAnalysisOptions {
    /// Whether nested function definitions should be reported when their enclosing function scope
    /// is not reached by a direct call chain.
    pub report_unreached_nested_definitions: bool,
}

#[allow(missing_docs)]
impl SyntheticRead {
    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn name(&self) -> &Name {
        &self.name
    }
}

#[doc(hidden)]
pub trait TraversalObserver {
    fn enter_command(&mut self, _command: &Command, _scope: ScopeId, _flow: FlowContext) {}

    fn exit_command(&mut self, _command: &Command, _scope: ScopeId) {}

    fn record_binding(&mut self, _binding: &Binding) {}

    fn record_reference(&mut self, _reference: &Reference, _resolved: Option<&Binding>) {}
}

#[doc(hidden)]
pub struct NoopTraversalObserver;

impl TraversalObserver for NoopTraversalObserver {}

#[doc(hidden)]
pub trait SourcePathResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf>;
}

impl<F> SourcePathResolver for F
where
    F: Fn(&Path, &str) -> Vec<PathBuf> + Send + Sync,
{
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        self(source_path, candidate)
    }
}

fn dedup_synthetic_reads(reads: Vec<SyntheticRead>) -> Vec<SyntheticRead> {
    let mut seen = FxHashSet::default();
    let mut deduped = Vec::new();
    for read in reads {
        if seen.insert((read.scope, read.span.start.offset, read.name.clone())) {
            deduped.push(read);
        }
    }
    deduped
}

fn build_call_graph(
    scopes: &[Scope],
    all_bindings: &[Binding],
    functions: &FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    call_sites: &FxHashMap<Name, SmallVec<[CallSite; 2]>>,
) -> CallGraph {
    let mut reachable = FxHashSet::default();
    let mut worklist = call_sites
        .values()
        .flat_map(|sites| sites.iter())
        .filter(|site| !is_in_function_scope(scopes, site.scope))
        .map(|site| site.callee.clone())
        .collect::<Vec<_>>();

    while let Some(name) = worklist.pop() {
        if reachable.contains(name.as_str()) {
            continue;
        }
        for sites in call_sites.values() {
            for site in sites {
                if is_in_named_function_scope(scopes, site.scope, &name) {
                    worklist.push(site.callee.clone());
                }
            }
        }
        reachable.insert(name);
    }

    let uncalled = functions
        .iter()
        .filter(|(name, _)| !reachable.contains(*name))
        .flat_map(|(_, bindings)| bindings.iter().copied())
        .collect();

    let overwritten = functions
        .iter()
        .flat_map(|(name, function_bindings)| {
            function_bindings
                .windows(2)
                .map(move |pair| OverwrittenFunction {
                    name: name.clone(),
                    first: pair[0],
                    second: pair[1],
                    first_called: call_sites
                        .get(name)
                        .into_iter()
                        .flat_map(|sites| sites.iter())
                        .any(|site| {
                            let first = all_bindings[pair[0].index()].span.start.offset;
                            let second = all_bindings[pair[1].index()].span.start.offset;
                            site.span.start.offset > first && site.span.start.offset < second
                        }),
                })
        })
        .collect();

    CallGraph {
        reachable,
        uncalled,
        overwritten,
    }
}

fn is_in_function_scope(scopes: &[Scope], scope: ScopeId) -> bool {
    ancestor_scopes(scopes, scope)
        .any(|scope| matches!(scopes[scope.index()].kind, ScopeKind::Function(_)))
}

fn is_in_named_function_scope(scopes: &[Scope], scope: ScopeId, name: &Name) -> bool {
    ancestor_scopes(scopes, scope).any(|scope| {
        matches!(
            &scopes[scope.index()].kind,
            ScopeKind::Function(function) if function.contains_name(name)
        )
    })
}

fn assignment_like_binding(kind: BindingKind) -> bool {
    matches!(
        kind,
        BindingKind::Assignment
            | BindingKind::AppendAssignment
            | BindingKind::ArrayAssignment
            | BindingKind::ArithmeticAssignment
    )
}

fn binding_blocks_same_scope_assoc_lookup(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::LOCAL) || !assignment_like_binding(binding.kind)
}

fn previous_visible_binding_id_from_slice(
    all_bindings: &[Binding],
    bindings: &[BindingId],
    offset: usize,
    ignored_binding_span: Option<Span>,
) -> Option<BindingId> {
    let candidate_count = bindings
        .partition_point(|binding_id| all_bindings[binding_id.index()].span.start.offset <= offset);

    bindings[..candidate_count]
        .iter()
        .rev()
        .copied()
        .find(|binding_id| ignored_binding_span != Some(all_bindings[binding_id.index()].span))
}

trait BindingIdCollection {
    fn as_slice(&self) -> &[BindingId];
    fn insert_binding_id(&mut self, index: usize, id: BindingId);
}

impl BindingIdCollection for Vec<BindingId> {
    fn as_slice(&self) -> &[BindingId] {
        self
    }

    fn insert_binding_id(&mut self, index: usize, id: BindingId) {
        self.insert(index, id);
    }
}

impl<A> BindingIdCollection for SmallVec<A>
where
    A: Array<Item = BindingId>,
{
    fn as_slice(&self) -> &[BindingId] {
        self
    }

    fn insert_binding_id(&mut self, index: usize, id: BindingId) {
        self.insert(index, id);
    }
}

fn insert_binding_id_sorted(
    bindings: &mut impl BindingIdCollection,
    all_bindings: &[Binding],
    id: BindingId,
) {
    let target = &all_bindings[id.index()];
    let insertion = bindings.as_slice().partition_point(|candidate_id| {
        let candidate = &all_bindings[candidate_id.index()];
        candidate.span.start.offset < target.span.start.offset
            || (candidate.span.start.offset == target.span.start.offset
                && candidate.span.end.offset < target.span.end.offset)
            || (candidate.span.start.offset == target.span.start.offset
                && candidate.span.end.offset == target.span.end.offset
                && candidate.id.index() < target.id.index())
    });
    bindings.insert_binding_id(insertion, id);
}

#[derive(Debug)]
struct AssocLookupBindingIndex {
    blocking_bindings_by_scope: Vec<FxHashMap<Name, Box<[BindingId]>>>,
}

#[derive(Debug)]
struct ScopeProvidedBindingIndex {
    provided_bindings_by_scope: Vec<Box<[ProvidedBinding]>>,
    definite_provider_scopes_by_name: FxHashMap<Name, Box<[ScopeId]>>,
}

#[derive(Debug)]
struct ScopeLookup {
    children: Vec<Box<[ScopeId]>>,
}

impl ScopeLookup {
    fn new(scopes: &[Scope]) -> Self {
        let mut children = vec![Vec::new(); scopes.len()];

        for scope in scopes {
            if let Some(parent) = scope.parent {
                children[parent.index()].push(scope.id);
            }
        }

        for scope_ids in &mut children {
            scope_ids.sort_by_key(|scope_id| {
                let span = scopes[scope_id.index()].span;
                (span.start.offset, span.end.offset)
            });
        }

        Self {
            children: children.into_iter().map(Vec::into_boxed_slice).collect(),
        }
    }

    fn scope_at(&self, scopes: &[Scope], offset: usize) -> Option<ScopeId> {
        let root = scopes.first()?;
        if !contains_offset(root.span, offset) {
            return None;
        }

        let mut scope = root.id;
        while let Some(child) = self.child_scope_at(scopes, scope, offset) {
            scope = child;
        }

        Some(scope)
    }

    fn child_scope_at(&self, scopes: &[Scope], parent: ScopeId, offset: usize) -> Option<ScopeId> {
        let children = self.children.get(parent.index())?;
        let cutoff = children
            .partition_point(|scope_id| scopes[scope_id.index()].span.start.offset <= offset);
        let mut best: Option<ScopeId> = None;
        let mut index = cutoff;

        while index > 0 {
            index -= 1;
            let scope_id = children[index];
            let span = scopes[scope_id.index()].span;
            if span.end.offset < offset {
                break;
            }
            if contains_offset(span, offset) {
                match best {
                    Some(current)
                        if scope_span_width(scopes[current.index()].span)
                            <= scope_span_width(span) => {}
                    _ => best = Some(scope_id),
                }
            }
        }

        best
    }
}

/// Semantic model constructed from a parsed shell file and source text.
#[derive(Debug)]
pub struct SemanticModel {
    shell_profile: ShellProfile,
    scopes: Vec<Scope>,
    scope_lookup: ScopeLookup,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    reference_index: FxHashMap<Name, SmallVec<[ReferenceId; 2]>>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    guarded_parameter_refs: FxHashSet<ReferenceId>,
    parameter_guard_flow_refs: FxHashSet<ReferenceId>,
    defaulting_parameter_operand_refs: FxHashSet<ReferenceId>,
    self_referential_assignment_refs: FxHashSet<ReferenceId>,
    binding_index: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, SmallVec<[BindingId; 2]>>,
    call_sites: FxHashMap<Name, SmallVec<[CallSite; 2]>>,
    call_graph: CallGraph,
    source_refs: Vec<SourceRef>,
    runtime: RuntimePrelude,
    declarations: Vec<Declaration>,
    indirect_targets_by_binding: FxHashMap<BindingId, Vec<BindingId>>,
    indirect_targets_by_reference: FxHashMap<ReferenceId, Vec<BindingId>>,
    array_like_indirect_expansion_refs: FxHashSet<ReferenceId>,
    synthetic_reads: Vec<SyntheticRead>,
    entry_bindings: Vec<BindingId>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, SmallVec<[BindingId; 2]>>,
    command_references: FxHashMap<SpanKey, SmallVec<[ReferenceId; 4]>>,
    cleared_variables: FxHashMap<(ScopeId, Name), SmallVec<[usize; 2]>>,
    import_origins_by_binding: FxHashMap<BindingId, Vec<PathBuf>>,
    heuristic_unused_assignments: Vec<BindingId>,
    zsh_option_analysis: Option<ZshOptionAnalysis>,
    assoc_lookup_binding_index: OnceLock<AssocLookupBindingIndex>,
    references_sorted_by_start: OnceLock<Vec<ReferenceId>>,
}

/// Lazy analysis view over a `SemanticModel`.
#[derive(Debug)]
pub struct SemanticAnalysis<'model> {
    model: &'model SemanticModel,
    cfg: OnceLock<ControlFlowGraph>,
    exact_variable_dataflow: OnceLock<ExactVariableDataflow>,
    dataflow: OnceLock<DataflowResult>,
    unused_assignments: OnceLock<Vec<BindingId>>,
    unused_assignments_shellcheck_compat: OnceLock<Vec<BindingId>>,
    uninitialized_references: OnceLock<Vec<UninitializedReference>>,
    uninitialized_reference_certainties: OnceLock<FxHashMap<SpanKey, UninitializedCertainty>>,
    dead_code: OnceLock<Vec<DeadCode>>,
    unreachable_blocks: OnceLock<FxHashSet<BlockId>>,
    overwritten_functions: OnceLock<Vec<OverwrittenFunction>>,
    unreached_functions: OnceLock<Vec<UnreachedFunction>>,
    unreached_functions_shellcheck_compat: OnceLock<Vec<UnreachedFunction>>,
    scope_provided_binding_index: OnceLock<ScopeProvidedBindingIndex>,
    unconditional_function_bindings: OnceLock<FxHashSet<BindingId>>,
    function_bindings_by_scope: OnceLock<FxHashMap<ScopeId, SmallVec<[BindingId; 2]>>>,
    visible_function_call_bindings: OnceLock<FxHashMap<SpanKey, BindingId>>,
}

#[allow(missing_docs)]
impl SemanticModel {
    pub fn build(file: &File, source: &str, indexer: &Indexer) -> Self {
        Self::build_with_options(file, source, indexer, SemanticBuildOptions::default())
    }

    pub fn build_with_options(
        file: &File,
        source: &str,
        indexer: &Indexer,
        options: SemanticBuildOptions<'_>,
    ) -> Self {
        let mut observer = NoopTraversalObserver;
        build_with_observer_with_options(file, source, indexer, &mut observer, options)
    }

    fn from_build_output(built: builder::BuildOutput) -> Self {
        let mut reference_index = built.reference_index;
        for reference_ids in reference_index.values_mut() {
            reference_ids.sort_by_key(|reference_id| {
                built.references[reference_id.index()].span.start.offset
            });
        }
        let indirect_targets_by_binding =
            build_indirect_targets_by_binding(&built.bindings, &built.indirect_target_hints);
        let indirect_targets_by_reference = build_indirect_targets_by_reference(
            &built.references,
            &built.resolved,
            &built.indirect_expansion_refs,
            &indirect_targets_by_binding,
        );
        let array_like_indirect_expansion_refs = build_array_like_indirect_expansion_refs(
            &built.references,
            &built.resolved,
            &built.indirect_expansion_refs,
            &built.indirect_target_hints,
        );
        let zsh_option_analysis = zsh_options::analyze(
            &built.shell_profile,
            &built.scopes,
            &built.bindings,
            &built.recorded_program,
        );
        let scope_lookup = ScopeLookup::new(&built.scopes);
        Self {
            shell_profile: built.shell_profile,
            scopes: built.scopes,
            scope_lookup,
            bindings: built.bindings,
            references: built.references,
            reference_index,
            predefined_runtime_refs: built.predefined_runtime_refs,
            guarded_parameter_refs: built.guarded_parameter_refs,
            parameter_guard_flow_refs: built.parameter_guard_flow_refs,
            defaulting_parameter_operand_refs: built.defaulting_parameter_operand_refs,
            self_referential_assignment_refs: built.self_referential_assignment_refs,
            binding_index: built.binding_index,
            resolved: built.resolved,
            unresolved: built.unresolved,
            functions: built.functions,
            call_sites: built.call_sites,
            call_graph: built.call_graph,
            source_refs: built.source_refs,
            runtime: built.runtime,
            declarations: built.declarations,
            indirect_targets_by_binding,
            indirect_targets_by_reference,
            array_like_indirect_expansion_refs,
            synthetic_reads: Vec::new(),
            entry_bindings: Vec::new(),
            flow_contexts: built.flow_contexts,
            recorded_program: built.recorded_program,
            command_bindings: built.command_bindings,
            command_references: built.command_references,
            cleared_variables: built.cleared_variables,
            import_origins_by_binding: FxHashMap::default(),
            heuristic_unused_assignments: built.heuristic_unused_assignments,
            zsh_option_analysis,
            assoc_lookup_binding_index: OnceLock::new(),
            references_sorted_by_start: OnceLock::new(),
        }
    }

    pub fn analysis(&self) -> SemanticAnalysis<'_> {
        SemanticAnalysis::new(self)
    }

    pub fn shell_profile(&self) -> &ShellProfile {
        &self.shell_profile
    }

    pub fn zsh_options_at(&self, offset: usize) -> Option<&ZshOptionState> {
        self.zsh_option_analysis
            .as_ref()
            .and_then(|analysis| analysis.options_at(&self.scopes, offset))
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// Yield every reference whose span is fully contained within `outer`.
    ///
    /// Backed by a lazily-built index sorted by reference start offset, so a
    /// per-span query costs `O(log n + matches)` rather than scanning every
    /// reference in the file.
    pub fn references_in_span(&self, outer: Span) -> ReferencesInSpan<'_> {
        let sorted = self
            .references_sorted_by_start
            .get_or_init(|| build_references_sorted_by_start(&self.references));
        let lower = sorted.partition_point(|id| {
            self.references[id.index()].span.start.offset < outer.start.offset
        });
        ReferencesInSpan {
            references: &self.references,
            ids: sorted[lower..].iter(),
            end: outer.end.offset,
        }
    }

    pub fn binding(&self, id: BindingId) -> &Binding {
        &self.bindings[id.index()]
    }

    pub fn reference(&self, id: ReferenceId) -> &Reference {
        &self.references[id.index()]
    }

    pub fn resolved_binding(&self, id: ReferenceId) -> Option<&Binding> {
        self.resolved
            .get(&id)
            .map(|binding| &self.bindings[binding.index()])
    }

    pub fn reference_is_predefined_runtime_array(&self, id: ReferenceId) -> bool {
        self.predefined_runtime_refs.contains(&id)
            && self
                .references
                .get(id.index())
                .is_some_and(|reference| self.runtime.is_preinitialized_array(&reference.name))
    }

    pub fn is_guarded_parameter_reference(&self, id: ReferenceId) -> bool {
        self.guarded_parameter_refs.contains(&id)
    }

    pub fn is_defaulting_parameter_operand_reference(&self, id: ReferenceId) -> bool {
        self.defaulting_parameter_operand_refs.contains(&id)
    }

    pub fn indirect_targets_for_binding(&self, id: BindingId) -> &[BindingId] {
        self.indirect_targets_by_binding
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn indirect_targets_for_reference(&self, id: ReferenceId) -> &[BindingId] {
        self.indirect_targets_by_reference
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn bindings_for(&self, name: &Name) -> &[BindingId] {
        self.binding_index
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    pub fn visible_binding(&self, name: &Name, at: Span) -> Option<&Binding> {
        self.previous_visible_binding(name, at, None)
    }

    #[doc(hidden)]
    pub fn binding_visible_at(&self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.binding(binding_id);
        binding.span.start.offset <= at.start.offset
            && self
                .ancestor_scopes(self.scope_at(at.start.offset))
                .any(|scope| scope == binding.scope)
    }

    #[doc(hidden)]
    pub fn binding_cleared_before(&self, binding_id: BindingId, at: Span) -> bool {
        let binding = self.binding(binding_id);
        self.cleared_variables
            .get(&(binding.scope, binding.name.clone()))
            .is_some_and(|cleared_offsets| {
                cleared_offsets.iter().any(|cleared_offset| {
                    *cleared_offset > binding.span.start.offset && *cleared_offset < at.start.offset
                })
            })
    }

    #[doc(hidden)]
    pub fn binding_and_reference_share_command(
        &self,
        binding_id: BindingId,
        reference_id: ReferenceId,
    ) -> bool {
        self.command_bindings.iter().any(|(command, bindings)| {
            bindings.contains(&binding_id)
                && self
                    .command_references
                    .get(command)
                    .is_some_and(|references| references.contains(&reference_id))
        })
    }

    #[doc(hidden)]
    pub fn previous_visible_binding(
        &self,
        name: &Name,
        at: Span,
        ignored_binding_span: Option<Span>,
    ) -> Option<&Binding> {
        let scope = self.scope_at(at.start.offset);
        self.previous_visible_binding_id_in_scope_chain(
            name,
            scope,
            at.start.offset,
            ignored_binding_span,
        )
        .map(|binding_id| self.binding(binding_id))
    }

    #[doc(hidden)]
    pub fn visible_binding_for_assoc_lookup(
        &self,
        name: &Name,
        current_scope: ScopeId,
        at: Span,
    ) -> Option<&Binding> {
        if let Some(binding_id) =
            self.previous_assoc_lookup_binding_id_in_scope(current_scope, name, at.start.offset)
        {
            return Some(self.binding(binding_id));
        }

        self.ancestor_scopes(current_scope)
            .skip(1)
            .find_map(|scope| {
                self.previous_visible_binding_id_in_scope(scope, name, at.start.offset, None)
            })
            .map(|binding_id| self.binding(binding_id))
    }

    pub fn defined_anywhere(&self, name: &Name) -> bool {
        self.binding_index.contains_key(name)
    }

    pub fn defined_in_any_function(&self, name: &Name) -> bool {
        self.binding_index.get(name).is_some_and(|bindings| {
            bindings.iter().any(|binding| {
                matches!(
                    self.scopes[self.bindings[binding.index()].scope.index()].kind,
                    ScopeKind::Function(_)
                )
            })
        })
    }

    pub fn is_runtime_consumed_binding(&self, binding_id: BindingId) -> bool {
        self.bindings
            .get(binding_id.index())
            .is_some_and(|binding| self.runtime.is_always_used_binding(&binding.name))
    }

    pub fn required_before(&self, name: &Name, scope: ScopeId, offset: usize) -> bool {
        self.references.iter().any(|reference| {
            reference.scope == scope
                && &reference.name == name
                && matches!(reference.kind, ReferenceKind::RequiredRead)
                && reference.span.start.offset < offset
        })
    }

    pub fn maybe_defined_outside(&self, name: &Name, scope: ScopeId) -> bool {
        self.ancestor_scopes(scope)
            .skip(1)
            .any(|scope| self.scopes[scope.index()].bindings.contains_key(name))
    }

    pub fn unresolved_references(&self) -> &[ReferenceId] {
        &self.unresolved
    }

    pub fn scope_at(&self, offset: usize) -> ScopeId {
        self.scope_lookup
            .scope_at(&self.scopes, offset)
            .unwrap_or(ScopeId(0))
    }

    pub fn scope_kind(&self, scope: ScopeId) -> &ScopeKind {
        &self.scopes[scope.index()].kind
    }

    pub fn ancestor_scopes(&self, scope: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        std::iter::successors(Some(scope), move |scope| self.scopes[scope.index()].parent)
    }

    fn previous_visible_binding_id_in_scope_chain(
        &self,
        name: &Name,
        scope: ScopeId,
        offset: usize,
        ignored_binding_span: Option<Span>,
    ) -> Option<BindingId> {
        self.ancestor_scopes(scope).find_map(|scope_id| {
            self.previous_visible_binding_id_in_scope(scope_id, name, offset, ignored_binding_span)
        })
    }

    fn previous_visible_binding_id_in_scope(
        &self,
        scope: ScopeId,
        name: &Name,
        offset: usize,
        ignored_binding_span: Option<Span>,
    ) -> Option<BindingId> {
        let bindings = self.scopes[scope.index()].bindings.get(name)?;
        previous_visible_binding_id_from_slice(
            &self.bindings,
            bindings,
            offset,
            ignored_binding_span,
        )
    }

    fn previous_assoc_lookup_binding_id_in_scope(
        &self,
        scope: ScopeId,
        name: &Name,
        offset: usize,
    ) -> Option<BindingId> {
        let bindings = self
            .assoc_lookup_binding_index()
            .blocking_bindings_by_scope
            .get(scope.index())
            .and_then(|bindings_by_name| bindings_by_name.get(name))?;
        previous_visible_binding_id_from_slice(&self.bindings, bindings, offset, None)
    }

    fn assoc_lookup_binding_index(&self) -> &AssocLookupBindingIndex {
        self.assoc_lookup_binding_index.get_or_init(|| {
            let blocking_bindings_by_scope = self
                .scopes
                .iter()
                .map(|scope| {
                    let mut bindings_by_name = FxHashMap::default();
                    for (name, bindings) in &scope.bindings {
                        let filtered = bindings
                            .iter()
                            .copied()
                            .filter(|binding_id| {
                                binding_blocks_same_scope_assoc_lookup(
                                    &self.bindings[binding_id.index()],
                                )
                            })
                            .collect::<Vec<_>>();
                        if !filtered.is_empty() {
                            bindings_by_name.insert(name.clone(), filtered.into_boxed_slice());
                        }
                    }
                    bindings_by_name
                })
                .collect();

            AssocLookupBindingIndex {
                blocking_bindings_by_scope,
            }
        })
    }

    pub fn flow_context_at(&self, span: &Span) -> Option<&FlowContext> {
        self.flow_contexts
            .iter()
            .rfind(|(candidate, _)| candidate == span)
            .map(|(_, context)| context)
            .or_else(|| {
                self.flow_contexts
                    .iter()
                    .enumerate()
                    .filter(|(_, (candidate, _))| {
                        contains_span(*candidate, *span) || contains_span(*span, *candidate)
                    })
                    .min_by_key(|(index, (candidate, _))| {
                        (
                            candidate.end.offset.saturating_sub(candidate.start.offset),
                            std::cmp::Reverse(*index),
                        )
                    })
                    .map(|(_, (_, context))| context)
            })
    }

    fn add_imported_binding(
        &mut self,
        provided: &ProvidedBinding,
        scope: ScopeId,
        span: Span,
        command_span: Option<Span>,
        origin_paths: Vec<PathBuf>,
        file_entry_contract: bool,
    ) -> BindingId {
        let mut attributes = BindingAttributes::empty();
        if provided.certainty == ContractCertainty::Possible {
            attributes |= BindingAttributes::IMPORTED_POSSIBLE;
        }
        if provided.kind == ProvidedBindingKind::Function {
            attributes |= BindingAttributes::IMPORTED_FUNCTION;
        }
        if file_entry_contract {
            attributes |= BindingAttributes::IMPORTED_FILE_ENTRY;
            if provided.file_entry_initialization == FileEntryBindingInitialization::Initialized {
                attributes |= BindingAttributes::IMPORTED_FILE_ENTRY_INITIALIZED;
            }
        }

        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: provided.name.clone(),
            kind: BindingKind::Imported,
            origin: BindingOrigin::Imported {
                definition_span: span,
            },
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        insert_binding_id_sorted(
            self.binding_index.entry(provided.name.clone()).or_default(),
            &self.bindings,
            id,
        );
        insert_binding_id_sorted(
            self.scopes[scope.index()]
                .bindings
                .entry(provided.name.clone())
                .or_default(),
            &self.bindings,
            id,
        );
        if provided.kind == ProvidedBindingKind::Function {
            insert_binding_id_sorted(
                self.functions.entry(provided.name.clone()).or_default(),
                &self.bindings,
                id,
            );
        }
        if let Some(command_span) = command_span {
            self.command_bindings
                .entry(SpanKey::new(command_span))
                .or_default()
                .push(id);
        }
        if !origin_paths.is_empty() {
            self.import_origins_by_binding.insert(id, origin_paths);
        }
        id
    }

    pub(crate) fn apply_file_entry_contract(&mut self, contract: FileContract, file: &File) {
        if contract.required_reads.is_empty()
            && contract.provided_bindings.is_empty()
            && contract.provided_functions.is_empty()
            && !contract.externally_consumed_bindings
        {
            return;
        }

        if contract.externally_consumed_bindings {
            self.mark_file_entry_consumed_bindings();
        }

        let mut synthetic_reads = self.synthetic_reads.clone();
        for name in contract.required_reads {
            synthetic_reads.push(SyntheticRead {
                scope: ScopeId(0),
                span: file.span,
                name,
            });
        }

        let entry_span = Span::from_positions(file.span.start, file.span.start);
        let mut entry_bindings = self.entry_bindings.clone();
        let function_origin_paths = contract
            .provided_functions
            .iter()
            .map(|function| (function.name.clone(), function.origin_paths.clone()))
            .collect::<FxHashMap<_, _>>();
        let mut provided_bindings = contract.provided_bindings;
        for function in contract.provided_functions {
            if !provided_bindings.iter().any(|binding| {
                binding.kind == ProvidedBindingKind::Function && binding.name == function.name
            }) {
                provided_bindings.push(ProvidedBinding::new(
                    function.name,
                    ProvidedBindingKind::Function,
                    ContractCertainty::Definite,
                ));
            }
        }
        for binding in &provided_bindings {
            let origin_paths = function_origin_paths
                .get(&binding.name)
                .cloned()
                .unwrap_or_default();
            let id = self.add_imported_binding(
                binding,
                ScopeId(0),
                entry_span,
                None,
                origin_paths,
                true,
            );
            entry_bindings.push(id);
        }

        self.set_synthetic_reads(dedup_synthetic_reads(synthetic_reads));
        self.set_entry_bindings(entry_bindings);
        self.resolve_unresolved_references();
        self.call_graph = build_call_graph(
            &self.scopes,
            &self.bindings,
            &self.functions,
            &self.call_sites,
        );
    }

    fn mark_file_entry_consumed_bindings(&mut self) {
        for binding in &mut self.bindings {
            if file_entry_contract_can_consume_binding(binding) {
                binding.attributes |= BindingAttributes::EXTERNALLY_CONSUMED;
            }
        }
        self.heuristic_unused_assignments.retain(|binding_id| {
            !self.bindings[binding_id.index()]
                .attributes
                .contains(BindingAttributes::EXTERNALLY_CONSUMED)
        });
    }

    pub(crate) fn apply_source_contracts(
        &mut self,
        synthetic_reads: Vec<SyntheticRead>,
        imported_bindings: Vec<ImportedBindingContractSite>,
        source_ref_resolutions: Vec<SourceRefResolution>,
        source_ref_explicitness: Vec<bool>,
        source_ref_diagnostic_classes: Vec<SourceRefDiagnosticClass>,
    ) {
        if synthetic_reads.is_empty()
            && imported_bindings.is_empty()
            && source_ref_resolutions.is_empty()
            && source_ref_explicitness.is_empty()
            && source_ref_diagnostic_classes.is_empty()
        {
            return;
        }

        let mut merged_reads = self.synthetic_reads.clone();
        merged_reads.extend(synthetic_reads);
        self.set_synthetic_reads(dedup_synthetic_reads(merged_reads));

        if !source_ref_resolutions.is_empty() {
            debug_assert_eq!(source_ref_resolutions.len(), self.source_refs.len());
            for (source_ref, resolution) in self.source_refs.iter_mut().zip(source_ref_resolutions)
            {
                source_ref.resolution = resolution;
            }
        }
        if !source_ref_explicitness.is_empty() {
            debug_assert_eq!(source_ref_explicitness.len(), self.source_refs.len());
            for (source_ref, explicitly_provided) in
                self.source_refs.iter_mut().zip(source_ref_explicitness)
            {
                source_ref.explicitly_provided = explicitly_provided;
            }
        }
        if !source_ref_diagnostic_classes.is_empty() {
            debug_assert_eq!(source_ref_diagnostic_classes.len(), self.source_refs.len());
            for (source_ref, diagnostic_class) in self
                .source_refs
                .iter_mut()
                .zip(source_ref_diagnostic_classes)
            {
                source_ref.diagnostic_class = diagnostic_class;
            }
        }

        for site in imported_bindings {
            self.add_imported_binding(
                &site.binding,
                site.scope,
                site.span,
                Some(site.span),
                site.origin_paths,
                false,
            );
        }
        self.resolve_unresolved_references();
        self.call_graph = build_call_graph(
            &self.scopes,
            &self.bindings,
            &self.functions,
            &self.call_sites,
        );
    }

    fn resolve_unresolved_references(&mut self) {
        let unresolved = std::mem::take(&mut self.unresolved);
        for reference_id in unresolved {
            let reference = &self.references[reference_id.index()];
            let resolved =
                self.resolve_binding_at(&reference.name, reference.scope, reference.span);
            if let Some(binding_id) = resolved {
                self.resolved.insert(reference_id, binding_id);
                self.bindings[binding_id.index()]
                    .references
                    .push(reference_id);
            } else {
                self.unresolved.push(reference_id);
            }
        }
    }

    fn resolve_binding_at(&self, name: &Name, scope: ScopeId, span: Span) -> Option<BindingId> {
        for scope in self.ancestor_scopes(scope) {
            let Some(bindings) = self.scopes[scope.index()].bindings.get(name) else {
                continue;
            };

            for binding in bindings.iter().rev().copied() {
                if self.bindings[binding.index()].span.start.offset <= span.start.offset {
                    return Some(binding);
                }
            }
        }
        None
    }

    pub fn function_definitions(&self, name: &Name) -> &[BindingId] {
        self.functions
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    pub fn call_sites_for(&self, name: &Name) -> &[CallSite] {
        self.call_sites
            .get(name)
            .map(SmallVec::as_slice)
            .unwrap_or(&[])
    }

    pub fn call_graph(&self) -> &CallGraph {
        &self.call_graph
    }

    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn synthetic_reads(&self) -> &[SyntheticRead] {
        &self.synthetic_reads
    }

    pub fn import_origins_for_binding(&self, id: BindingId) -> &[PathBuf] {
        self.import_origins_by_binding
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn recorded_program(&self) -> &RecordedProgram {
        &self.recorded_program
    }

    pub(crate) fn set_synthetic_reads(&mut self, synthetic_reads: Vec<SyntheticRead>) {
        self.synthetic_reads = synthetic_reads;
    }

    fn set_entry_bindings(&mut self, entry_bindings: Vec<BindingId>) {
        self.entry_bindings = entry_bindings;
    }

    fn dataflow_context<'a>(&'a self, cfg: &'a ControlFlowGraph) -> DataflowContext<'a> {
        DataflowContext {
            cfg,
            runtime: &self.runtime,
            scopes: &self.scopes,
            bindings: &self.bindings,
            references: &self.references,
            predefined_runtime_refs: &self.predefined_runtime_refs,
            guarded_parameter_refs: &self.guarded_parameter_refs,
            parameter_guard_flow_refs: &self.parameter_guard_flow_refs,
            self_referential_assignment_refs: &self.self_referential_assignment_refs,
            resolved: &self.resolved,
            call_sites: &self.call_sites,
            indirect_targets_by_reference: &self.indirect_targets_by_reference,
            array_like_indirect_expansion_refs: &self.array_like_indirect_expansion_refs,
            synthetic_reads: &self.synthetic_reads,
            entry_bindings: &self.entry_bindings,
        }
    }
}

fn file_entry_contract_can_consume_binding(binding: &Binding) -> bool {
    if binding.attributes.contains(BindingAttributes::LOCAL) {
        return false;
    }

    matches!(
        binding.kind,
        BindingKind::Assignment
            | BindingKind::ArrayAssignment
            | BindingKind::AppendAssignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::LoopVariable
            | BindingKind::ReadTarget
            | BindingKind::MapfileTarget
            | BindingKind::PrintfTarget
            | BindingKind::GetoptsTarget
            | BindingKind::ArithmeticAssignment
            | BindingKind::Declaration(_)
    )
}

#[doc(hidden)]
pub fn build_with_observer(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
) -> SemanticModel {
    build_with_observer_with_options(
        file,
        source,
        indexer,
        observer,
        SemanticBuildOptions::default(),
    )
}

#[doc(hidden)]
pub fn build_with_observer_with_options(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    options: SemanticBuildOptions<'_>,
) -> SemanticModel {
    build_semantic_model(file, source, indexer, observer, options)
}

#[doc(hidden)]
pub fn build_with_observer_at_path(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
) -> SemanticModel {
    build_with_observer_at_path_with_resolver(file, source, indexer, observer, source_path, None)
}

#[doc(hidden)]
pub fn build_with_observer_at_path_with_resolver(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> SemanticModel {
    build_semantic_model(
        file,
        source,
        indexer,
        observer,
        SemanticBuildOptions {
            source_path,
            source_path_resolver,
            file_entry_contract: None,
            file_entry_contract_collector: None,
            analyzed_paths: None,
            shell_profile: None,
            resolve_source_closure: true,
        },
    )
}

fn build_semantic_model(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    options: SemanticBuildOptions<'_>,
) -> SemanticModel {
    let SemanticBuildOptions {
        source_path,
        source_path_resolver,
        file_entry_contract,
        mut file_entry_contract_collector,
        analyzed_paths,
        shell_profile,
        resolve_source_closure,
    } = options;
    let mut model = build_semantic_model_base(
        file,
        source,
        indexer,
        observer,
        source_path,
        shell_profile.clone(),
        file_entry_contract_collector
            .as_mut()
            .map(|collector| &mut **collector as &mut dyn FileEntryContractCollector),
    );
    if let Some(contract) = file_entry_contract {
        model.apply_file_entry_contract(contract, file);
    }
    if let Some(contract) = file_entry_contract_collector
        .as_ref()
        .and_then(|collector| collector.finish())
    {
        model.apply_file_entry_contract(contract, file);
    }
    if let Some(source_path) = source_path {
        let (
            synthetic_reads,
            imported_bindings,
            source_ref_resolutions,
            source_ref_explicitness,
            source_ref_diagnostic_classes,
        ) = if resolve_source_closure {
            source_closure::collect_source_closure_contracts(
                &model,
                file,
                source,
                source_path,
                source_path_resolver,
                analyzed_paths,
            )
        } else {
            let (source_ref_resolutions, source_ref_explicitness, source_ref_diagnostic_classes) =
                source_closure::collect_source_ref_metadata(
                    &model,
                    source_path,
                    source_path_resolver,
                    analyzed_paths,
                );
            (
                Vec::new(),
                Vec::new(),
                source_ref_resolutions,
                source_ref_explicitness,
                source_ref_diagnostic_classes,
            )
        };
        model.apply_source_contracts(
            synthetic_reads,
            imported_bindings,
            source_ref_resolutions,
            source_ref_explicitness,
            source_ref_diagnostic_classes,
        );
    }
    model
}

pub(crate) fn build_semantic_model_base<'a>(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &'a mut dyn TraversalObserver,
    source_path: Option<&Path>,
    shell_profile: Option<ShellProfile>,
    file_entry_contract_collector: Option<&'a mut dyn FileEntryContractCollector>,
) -> SemanticModel {
    let shell_profile = shell_profile.unwrap_or_else(|| infer_shell_profile(source, source_path));
    let built = SemanticModelBuilder::build(
        file,
        source,
        indexer,
        observer,
        file_entry_contract_collector,
        bash_runtime_vars_enabled(source, source_path),
        shell_profile,
    );
    SemanticModel::from_build_output(built)
}

fn infer_shell_profile(source: &str, path: Option<&Path>) -> ShellProfile {
    let dialect = infer_parse_dialect_from_source(source, path);
    ShellProfile::native(dialect)
}

fn infer_parse_dialect_from_source(
    source: &str,
    path: Option<&Path>,
) -> shuck_parser::ShellDialect {
    if let Some(interpreter) = shebang_interpreter(source) {
        return parse_dialect_from_name(interpreter).unwrap_or(shuck_parser::ShellDialect::Bash);
    }

    infer_parse_dialect_from_path(path).unwrap_or(shuck_parser::ShellDialect::Bash)
}

pub(crate) fn infer_explicit_parse_dialect_from_source(
    source: &str,
    path: Option<&Path>,
) -> Option<shuck_parser::ShellDialect> {
    if let Some(interpreter) = shebang_interpreter(source)
        && let Some(dialect) = parse_dialect_from_name(interpreter)
    {
        return Some(dialect);
    }

    infer_parse_dialect_from_path(path)
}

fn shebang_interpreter(source: &str) -> Option<&str> {
    shuck_parser::shebang::interpreter_name(source.lines().next()?)
}

fn infer_parse_dialect_from_path(path: Option<&Path>) -> Option<shuck_parser::ShellDialect> {
    match path
        .and_then(|path| path.extension().and_then(|ext| ext.to_str()))
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("sh" | "dash" | "ksh") => Some(shuck_parser::ShellDialect::Posix),
        Some("mksh") => Some(shuck_parser::ShellDialect::Mksh),
        Some("bash") => Some(shuck_parser::ShellDialect::Bash),
        Some("zsh") => Some(shuck_parser::ShellDialect::Zsh),
        _ => None,
    }
}

fn parse_dialect_from_name(name: &str) -> Option<shuck_parser::ShellDialect> {
    match name.to_ascii_lowercase().as_str() {
        "sh" | "dash" | "ksh" | "posix" => Some(shuck_parser::ShellDialect::Posix),
        "mksh" => Some(shuck_parser::ShellDialect::Mksh),
        "bash" => Some(shuck_parser::ShellDialect::Bash),
        "zsh" => Some(shuck_parser::ShellDialect::Zsh),
        _ => None,
    }
}

fn bash_runtime_vars_enabled(source: &str, path: Option<&Path>) -> bool {
    infer_bash_from_shebang(source).unwrap_or_else(|| {
        path.and_then(|path| path.extension().and_then(|ext| ext.to_str()))
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bash"))
    })
}

fn infer_bash_from_shebang(source: &str) -> Option<bool> {
    shebang_interpreter(source).map(|interpreter| interpreter.eq_ignore_ascii_case("bash"))
}

fn contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset <= span.end.offset
}

fn build_references_sorted_by_start(references: &[Reference]) -> Vec<ReferenceId> {
    let mut ids: Vec<ReferenceId> = (0..references.len() as u32).map(ReferenceId).collect();
    ids.sort_by_key(|id| references[id.index()].span.start.offset);
    ids
}

/// Iterator returned by [`SemanticModel::references_in_span`].
///
/// Walks the references sorted index forward from the first candidate and
/// stops as soon as a reference starts past the outer span's end.
#[derive(Debug, Clone)]
pub struct ReferencesInSpan<'a> {
    references: &'a [Reference],
    ids: std::slice::Iter<'a, ReferenceId>,
    end: usize,
}

impl<'a> Iterator for ReferencesInSpan<'a> {
    type Item = &'a Reference;

    fn next(&mut self) -> Option<&'a Reference> {
        loop {
            let id = self.ids.next()?;
            let reference = &self.references[id.index()];
            if reference.span.start.offset > self.end {
                return None;
            }
            if reference.span.end.offset <= self.end {
                return Some(reference);
            }
        }
    }
}

fn scope_span_width(span: Span) -> usize {
    span.end.offset.saturating_sub(span.start.offset)
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn ancestor_scopes(scopes: &[Scope], scope: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
    std::iter::successors(Some(scope), move |scope| scopes[scope.index()].parent)
}

#[cfg(test)]
fn linear_scope_at(scopes: &[Scope], offset: usize) -> ScopeId {
    scopes
        .iter()
        .filter(|scope| contains_offset(scope.span, offset))
        .min_by_key(|scope| scope_span_width(scope.span))
        .map(|scope| scope.id)
        .unwrap_or(ScopeId(0))
}

fn build_indirect_targets_by_binding(
    bindings: &[Binding],
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
) -> FxHashMap<BindingId, Vec<BindingId>> {
    let mut targets_by_binding = FxHashMap::default();
    for (binding_id, hint) in indirect_target_hints {
        let targets: Vec<_> = bindings
            .iter()
            .filter(|binding| indirect_target_matches(hint, binding))
            .map(|binding| binding.id)
            .collect();
        if !targets.is_empty() {
            targets_by_binding.insert(*binding_id, targets);
        }
    }
    targets_by_binding
}

fn build_indirect_targets_by_reference(
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
    indirect_targets_by_binding: &FxHashMap<BindingId, Vec<BindingId>>,
) -> FxHashMap<ReferenceId, Vec<BindingId>> {
    let mut targets_by_reference = FxHashMap::default();
    for reference in references {
        if !indirect_expansion_refs.contains(&reference.id) {
            continue;
        }
        let Some(binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        if let Some(targets) = indirect_targets_by_binding.get(&binding_id) {
            targets_by_reference.insert(reference.id, targets.clone());
        }
    }
    targets_by_reference
}

fn build_array_like_indirect_expansion_refs(
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
) -> FxHashSet<ReferenceId> {
    let mut array_like_refs = FxHashSet::default();
    for reference in references {
        if !indirect_expansion_refs.contains(&reference.id) {
            continue;
        }
        let Some(binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        let Some(hint) = indirect_target_hints.get(&binding_id) else {
            continue;
        };
        let array_like = match hint {
            IndirectTargetHint::Exact { array_like, .. }
            | IndirectTargetHint::Pattern { array_like, .. } => *array_like,
        };
        if array_like {
            array_like_refs.insert(reference.id);
        }
    }
    array_like_refs
}

fn indirect_target_matches(hint: &IndirectTargetHint, binding: &Binding) -> bool {
    match hint {
        IndirectTargetHint::Exact { name, array_like } => {
            binding.name == *name && (!array_like || binding::is_array_like_binding(binding))
        }
        IndirectTargetHint::Pattern {
            prefix,
            suffix,
            array_like,
        } => {
            let name = binding.name.as_str();
            name.starts_with(prefix)
                && name.ends_with(suffix)
                && (!array_like || binding::is_array_like_binding(binding))
        }
    }
}

#[cfg(test)]
mod tests;
