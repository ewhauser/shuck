mod binding;
mod builder;
mod call_graph;
mod cfg;
mod contract;
mod dataflow;
mod declaration;
mod reference;
mod runtime;
mod scope;
mod source_closure;
mod source_ref;
mod zsh_options;

pub use binding::{Binding, BindingAttributes, BindingId, BindingKind};
pub use call_graph::{CallGraph, CallSite, OverwrittenFunction};
pub use cfg::{BasicBlock, BlockId, ControlFlowGraph, EdgeKind, FlowContext};
pub use contract::{
    ContractCertainty, FileContract, FunctionContract, ProvidedBinding, ProvidedBindingKind,
    SemanticBuildOptions,
};
pub use dataflow::{
    DeadCode, ReachingDefinitions, UninitializedCertainty, UninitializedReference,
    UnusedAssignment, UnusedReason,
};
pub use declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
pub use reference::{Reference, ReferenceId, ReferenceKind};
pub use scope::{FunctionScopeKind, Scope, ScopeId, ScopeKind};
pub use shuck_parser::{OptionValue, ShellProfile, ZshEmulationMode, ZshOptionState};
pub use source_ref::{SourceRef, SourceRefKind, SourceRefResolution};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, File, Name, Span};
use shuck_indexer::Indexer;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::builder::SemanticModelBuilder;
use crate::cfg::{RecordedProgram, build_control_flow_graph};
use crate::dataflow::{DataflowContext, DataflowResult, ExactVariableDataflow};
use crate::runtime::RuntimePrelude;
use crate::source_closure::ImportedBindingContractSite;
use crate::zsh_options::ZshOptionAnalysis;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntheticRead {
    pub(crate) scope: ScopeId,
    pub(crate) span: Span,
    pub(crate) name: Name,
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
    functions: &FxHashMap<Name, Vec<BindingId>>,
    call_sites: &FxHashMap<Name, Vec<CallSite>>,
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

#[derive(Debug)]
pub struct SemanticModel {
    shell_profile: ShellProfile,
    scopes: Vec<Scope>,
    scope_lookup: ScopeLookup,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    guarded_parameter_refs: FxHashSet<ReferenceId>,
    binding_index: FxHashMap<Name, Vec<BindingId>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, Vec<BindingId>>,
    call_sites: FxHashMap<Name, Vec<CallSite>>,
    call_graph: CallGraph,
    source_refs: Vec<SourceRef>,
    runtime: RuntimePrelude,
    declarations: Vec<Declaration>,
    indirect_targets_by_binding: FxHashMap<BindingId, Vec<BindingId>>,
    indirect_targets_by_reference: FxHashMap<ReferenceId, Vec<BindingId>>,
    synthetic_reads: Vec<SyntheticRead>,
    entry_bindings: Vec<BindingId>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    import_origins_by_binding: FxHashMap<BindingId, Vec<PathBuf>>,
    heuristic_unused_assignments: Vec<BindingId>,
    zsh_option_analysis: Option<ZshOptionAnalysis>,
}

#[derive(Debug)]
pub struct SemanticAnalysis<'model> {
    model: &'model SemanticModel,
    cfg: OnceLock<ControlFlowGraph>,
    exact_variable_dataflow: OnceLock<ExactVariableDataflow>,
    dataflow: OnceLock<DataflowResult>,
    unused_assignments: OnceLock<Vec<BindingId>>,
    uninitialized_references: OnceLock<Vec<UninitializedReference>>,
    dead_code: OnceLock<Vec<DeadCode>>,
    overwritten_functions: OnceLock<Vec<OverwrittenFunction>>,
}

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
        let indirect_targets_by_binding =
            build_indirect_targets_by_binding(&built.bindings, &built.indirect_target_hints);
        let indirect_targets_by_reference = build_indirect_targets_by_reference(
            &built.references,
            &built.resolved,
            &built.indirect_expansion_refs,
            &indirect_targets_by_binding,
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
            predefined_runtime_refs: built.predefined_runtime_refs,
            guarded_parameter_refs: built.guarded_parameter_refs,
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
            synthetic_reads: Vec::new(),
            entry_bindings: Vec::new(),
            flow_contexts: built.flow_contexts,
            recorded_program: built.recorded_program,
            command_bindings: built.command_bindings,
            command_references: built.command_references,
            import_origins_by_binding: FxHashMap::default(),
            heuristic_unused_assignments: built.heuristic_unused_assignments,
            zsh_option_analysis,
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

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn references(&self) -> &[Reference] {
        &self.references
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

    pub fn is_guarded_parameter_reference(&self, id: ReferenceId) -> bool {
        self.guarded_parameter_refs.contains(&id)
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
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn visible_binding(&self, name: &Name, at: Span) -> Option<&Binding> {
        let scope = self.scope_at(at.start.offset);
        for scope in self.ancestor_scopes(scope) {
            if let Some(bindings) = self.scopes[scope.index()].bindings.get(name) {
                for binding in bindings.iter().rev() {
                    let binding = &self.bindings[binding.index()];
                    if binding.span.start.offset <= at.start.offset {
                        return Some(binding);
                    }
                }
            }
        }
        None
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

    fn needs_precise_unused_assignments(&self) -> bool {
        if self.heuristic_unused_assignments.is_empty() {
            return false;
        }

        if !self.synthetic_reads.is_empty()
            || !self.entry_bindings.is_empty()
            || !self.indirect_targets_by_reference.is_empty()
        {
            return true;
        }

        let has_call_sites = !self.call_sites.is_empty();
        self.heuristic_unused_assignments.iter().any(|binding_id| {
            let binding = &self.bindings[binding_id.index()];
            self.runtime.is_always_used_binding(&binding.name)
                || self
                    .binding_index
                    .get(&binding.name)
                    .is_some_and(|binding_ids| binding_ids.len() > 1)
                || (has_call_sites
                    && matches!(
                        self.scopes[binding.scope.index()].kind,
                        ScopeKind::Function(_)
                    )
                    && !binding.attributes.contains(BindingAttributes::LOCAL))
        })
    }

    fn can_use_heuristic_unused_assignments_with_linear_cfg(&self, cfg: &ControlFlowGraph) -> bool {
        self.references.is_empty()
            && self.synthetic_reads.is_empty()
            && self.entry_bindings.is_empty()
            && self.indirect_targets_by_reference.is_empty()
            && self.call_sites.is_empty()
            && cfg.blocks().len() <= 1
            && !self.heuristic_unused_assignments.iter().any(|binding_id| {
                self.runtime
                    .is_always_used_binding(&self.bindings[binding_id.index()].name)
            })
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

    pub fn flow_context_at(&self, span: &Span) -> Option<&FlowContext> {
        self.flow_contexts
            .iter()
            .find_map(|(candidate, context)| (candidate == span).then_some(context))
            .or_else(|| {
                self.flow_contexts.iter().find_map(|(candidate, context)| {
                    (contains_span(*candidate, *span) || contains_span(*span, *candidate))
                        .then_some(context)
                })
            })
    }

    fn add_imported_binding(
        &mut self,
        provided: &ProvidedBinding,
        scope: ScopeId,
        span: Span,
        command_span: Option<Span>,
        origin_paths: Vec<PathBuf>,
    ) -> BindingId {
        let mut attributes = BindingAttributes::empty();
        if provided.certainty == ContractCertainty::Possible {
            attributes |= BindingAttributes::IMPORTED_POSSIBLE;
        }
        if provided.kind == ProvidedBindingKind::Function {
            attributes |= BindingAttributes::IMPORTED_FUNCTION;
        }

        let id = BindingId(self.bindings.len() as u32);
        self.bindings.push(Binding {
            id,
            name: provided.name.clone(),
            kind: BindingKind::Imported,
            scope,
            span,
            references: Vec::new(),
            attributes,
        });
        self.binding_index
            .entry(provided.name.clone())
            .or_default()
            .push(id);
        self.scopes[scope.index()]
            .bindings
            .entry(provided.name.clone())
            .or_default()
            .push(id);
        if provided.kind == ProvidedBindingKind::Function {
            self.functions
                .entry(provided.name.clone())
                .or_default()
                .push(id);
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
        {
            return;
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
            let id = self.add_imported_binding(binding, ScopeId(0), entry_span, None, origin_paths);
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

    pub(crate) fn apply_source_contracts(
        &mut self,
        synthetic_reads: Vec<SyntheticRead>,
        imported_bindings: Vec<ImportedBindingContractSite>,
        source_ref_resolutions: Vec<SourceRefResolution>,
        source_ref_explicitness: Vec<bool>,
    ) {
        if synthetic_reads.is_empty()
            && imported_bindings.is_empty()
            && source_ref_resolutions.is_empty()
            && source_ref_explicitness.is_empty()
        {
            return;
        }

        let mut merged_reads = self.synthetic_reads.clone();
        merged_reads.extend(synthetic_reads);
        self.set_synthetic_reads(dedup_synthetic_reads(merged_reads));

        if !source_ref_resolutions.is_empty() {
            debug_assert_eq!(source_ref_resolutions.len(), self.source_refs.len());
            for (source_ref, resolution) in self
                .source_refs
                .iter_mut()
                .zip(source_ref_resolutions.into_iter())
            {
                source_ref.resolution = resolution;
            }
        }
        if !source_ref_explicitness.is_empty() {
            debug_assert_eq!(source_ref_explicitness.len(), self.source_refs.len());
            for (source_ref, explicitly_provided) in self
                .source_refs
                .iter_mut()
                .zip(source_ref_explicitness.into_iter())
            {
                source_ref.explicitly_provided = explicitly_provided;
            }
        }

        for site in imported_bindings {
            self.add_imported_binding(
                &site.binding,
                site.scope,
                site.span,
                Some(site.span),
                site.origin_paths,
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
        self.functions.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn call_sites_for(&self, name: &Name) -> &[CallSite] {
        self.call_sites.get(name).map(Vec::as_slice).unwrap_or(&[])
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
            resolved: &self.resolved,
            call_sites: &self.call_sites,
            indirect_targets_by_reference: &self.indirect_targets_by_reference,
            synthetic_reads: &self.synthetic_reads,
            entry_bindings: &self.entry_bindings,
        }
    }
}

impl<'model> SemanticAnalysis<'model> {
    fn new(model: &'model SemanticModel) -> Self {
        Self {
            model,
            cfg: OnceLock::new(),
            exact_variable_dataflow: OnceLock::new(),
            dataflow: OnceLock::new(),
            unused_assignments: OnceLock::new(),
            uninitialized_references: OnceLock::new(),
            dead_code: OnceLock::new(),
            overwritten_functions: OnceLock::new(),
        }
    }

    pub fn cfg(&self) -> &ControlFlowGraph {
        self.cfg.get_or_init(|| {
            build_control_flow_graph(
                &self.model.recorded_program,
                &self.model.command_bindings,
                &self.model.command_references,
            )
        })
    }

    fn exact_variable_dataflow(&self) -> &ExactVariableDataflow {
        self.exact_variable_dataflow.get_or_init(|| {
            let cfg = self.cfg();
            let context = self.model.dataflow_context(cfg);
            dataflow::build_exact_variable_dataflow(&context)
        })
    }

    #[allow(dead_code)]
    fn dataflow(&self) -> &DataflowResult {
        self.dataflow.get_or_init(|| {
            let cfg = self.cfg();
            let context = self.model.dataflow_context(cfg);
            let exact = self.exact_variable_dataflow();
            dataflow::analyze(&context, exact)
        })
    }

    pub fn unused_assignments(&self) -> &[BindingId] {
        if !self.model.needs_precise_unused_assignments() {
            return &self.model.heuristic_unused_assignments;
        }

        self.unused_assignments
            .get_or_init(|| {
                let cfg = self.cfg();
                if self
                    .model
                    .can_use_heuristic_unused_assignments_with_linear_cfg(cfg)
                {
                    return self.model.heuristic_unused_assignments.clone();
                }
                let context = self.model.dataflow_context(cfg);
                let exact = self.exact_variable_dataflow();
                dataflow::analyze_unused_assignments(&context, exact)
            })
            .as_slice()
    }

    pub fn uninitialized_references(&self) -> &[UninitializedReference] {
        self.uninitialized_references
            .get_or_init(|| {
                let cfg = self.cfg();
                let context = self.model.dataflow_context(cfg);
                let exact = self.exact_variable_dataflow();
                dataflow::analyze_uninitialized_references(&context, exact)
            })
            .as_slice()
    }

    pub fn dead_code(&self) -> &[DeadCode] {
        self.dead_code
            .get_or_init(|| dataflow::analyze_dead_code(self.cfg()))
            .as_slice()
    }

    pub fn is_reachable(&self, span: &Span) -> bool {
        let cfg = self.cfg();
        cfg.block_ids_for_span(*span)
            .iter()
            .all(|block| !cfg.unreachable().contains(block))
    }

    pub fn overwritten_functions(&self) -> &[OverwrittenFunction] {
        self.overwritten_functions
            .get_or_init(|| self.compute_overwritten_functions())
            .as_slice()
    }

    pub(crate) fn summarize_scope_provided_bindings(&self, scope: ScopeId) -> Vec<ProvidedBinding> {
        dataflow::summarize_scope_provided_bindings(
            self.cfg(),
            &self.model.scopes,
            &self.model.bindings,
            &self.model.entry_bindings,
            scope,
        )
    }

    pub(crate) fn summarize_scope_provided_functions(
        &self,
        scope: ScopeId,
    ) -> Vec<ProvidedBinding> {
        dataflow::summarize_scope_provided_functions(
            self.cfg(),
            &self.model.scopes,
            &self.model.bindings,
            &self.model.entry_bindings,
            scope,
        )
    }

    fn compute_overwritten_functions(&self) -> Vec<OverwrittenFunction> {
        if self.model.functions.is_empty() {
            return Vec::new();
        }

        let cfg = self.cfg();
        let unreachable = cfg.unreachable().iter().copied().collect::<FxHashSet<_>>();
        let binding_blocks = build_binding_block_index(cfg.blocks(), self.model.bindings.len());
        let mut reachability = ReachabilityCache::new(cfg);
        let mut overwritten = Vec::new();

        for (name, bindings) in &self.model.functions {
            let mut bindings_by_scope = FxHashMap::<ScopeId, Vec<BindingId>>::default();
            for &binding in bindings {
                bindings_by_scope
                    .entry(self.model.binding(binding).scope)
                    .or_default()
                    .push(binding);
            }

            for scope_bindings in bindings_by_scope.values_mut() {
                scope_bindings
                    .sort_by_key(|binding| self.model.binding(*binding).span.start.offset);

                for pair in scope_bindings.windows(2) {
                    let first = pair[0];
                    let second = pair[1];
                    let Some(first_blocks) =
                        reachable_binding_blocks(first, &binding_blocks, &unreachable)
                    else {
                        continue;
                    };
                    let Some(second_blocks) =
                        reachable_binding_blocks(second, &binding_blocks, &unreachable)
                    else {
                        continue;
                    };

                    if !blocks_have_path(&first_blocks, &second_blocks, &mut reachability) {
                        continue;
                    }

                    let first_called = self.model.call_sites_for(name).iter().any(|site| {
                        self.model
                            .visible_binding(name, site.span)
                            .is_some_and(|binding| binding.id == first)
                            && {
                                let site_blocks = cfg
                                    .block_ids_for_span(site.span)
                                    .iter()
                                    .copied()
                                    .filter(|block| !unreachable.contains(block))
                                    .collect::<Vec<_>>();
                                !site_blocks.is_empty()
                                    && blocks_have_path(
                                        &first_blocks,
                                        &site_blocks,
                                        &mut reachability,
                                    )
                                    && blocks_have_path(
                                        &site_blocks,
                                        &second_blocks,
                                        &mut reachability,
                                    )
                            }
                    });

                    overwritten.push(OverwrittenFunction {
                        name: name.clone(),
                        first,
                        second,
                        first_called,
                    });
                }
            }
        }

        overwritten.sort_by_key(|overwritten| {
            (
                self.model.binding(overwritten.first).span.start.offset,
                self.model.binding(overwritten.second).span.start.offset,
            )
        });
        overwritten
    }
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
            analyzed_paths: None,
            shell_profile: None,
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
    let mut model = build_semantic_model_base(
        file,
        source,
        indexer,
        observer,
        options.source_path,
        options.shell_profile.clone(),
    );
    if let Some(contract) = options.file_entry_contract {
        model.apply_file_entry_contract(contract, file);
    }
    if let Some(source_path) = options.source_path {
        let (synthetic_reads, imported_bindings, source_ref_resolutions, source_ref_explicitness) =
            source_closure::collect_source_closure_contracts(
                &model,
                file,
                source,
                source_path,
                options.source_path_resolver,
                options.analyzed_paths,
            );
        model.apply_source_contracts(
            synthetic_reads,
            imported_bindings,
            source_ref_resolutions,
            source_ref_explicitness,
        );
    }
    model
}

pub(crate) fn build_semantic_model_base(
    file: &File,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
    shell_profile: Option<ShellProfile>,
) -> SemanticModel {
    let shell_profile = shell_profile.unwrap_or_else(|| infer_shell_profile(source, source_path));
    let built = SemanticModelBuilder::build(
        file,
        source,
        indexer,
        observer,
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
    if let Some(line) = source.lines().next().map(str::trim)
        && let Some(line) = line.strip_prefix("#!").map(str::trim)
    {
        let mut parts = line.split_whitespace();
        let first = parts.next();
        let interpreter = first
            .and_then(|first| {
                if Path::new(first).file_name()?.to_str()? == "env" {
                    parts.next()
                } else {
                    Path::new(first).file_name()?.to_str()
                }
            })
            .unwrap_or_default();
        return match interpreter.to_ascii_lowercase().as_str() {
            "sh" | "dash" | "ksh" | "posix" => shuck_parser::ShellDialect::Posix,
            "mksh" => shuck_parser::ShellDialect::Mksh,
            "zsh" => shuck_parser::ShellDialect::Zsh,
            _ => shuck_parser::ShellDialect::Bash,
        };
    }

    match path
        .and_then(|path| path.extension().and_then(|ext| ext.to_str()))
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("sh" | "dash" | "ksh") => shuck_parser::ShellDialect::Posix,
        Some("mksh") => shuck_parser::ShellDialect::Mksh,
        Some("zsh") => shuck_parser::ShellDialect::Zsh,
        _ => shuck_parser::ShellDialect::Bash,
    }
}

fn bash_runtime_vars_enabled(source: &str, path: Option<&Path>) -> bool {
    infer_bash_from_shebang(source).unwrap_or_else(|| {
        path.and_then(|path| path.extension().and_then(|ext| ext.to_str()))
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bash"))
    })
}

fn infer_bash_from_shebang(source: &str) -> Option<bool> {
    let first_line = source.lines().next()?.trim();
    let line = first_line.strip_prefix("#!")?.trim();

    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let interpreter = if Path::new(first).file_name()?.to_str()? == "env" {
        parts.next()?
    } else {
        Path::new(first).file_name()?.to_str()?
    };

    Some(interpreter.eq_ignore_ascii_case("bash"))
}

fn contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset <= span.end.offset
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

fn build_binding_block_index(blocks: &[BasicBlock], binding_count: usize) -> Vec<Vec<BlockId>> {
    let mut binding_blocks = vec![Vec::new(); binding_count];
    for block in blocks {
        for &binding in &block.bindings {
            binding_blocks[binding.index()].push(block.id);
        }
    }
    binding_blocks
}

fn reachable_binding_blocks(
    binding: BindingId,
    binding_blocks: &[Vec<BlockId>],
    unreachable: &FxHashSet<BlockId>,
) -> Option<Vec<BlockId>> {
    let blocks = binding_blocks
        .get(binding.index())
        .into_iter()
        .flat_map(|blocks| blocks.iter())
        .copied()
        .filter(|block| !unreachable.contains(block))
        .collect::<Vec<_>>();

    (!blocks.is_empty()).then_some(blocks)
}

fn blocks_have_path(
    starts: &[BlockId],
    ends: &[BlockId],
    reachability: &mut ReachabilityCache<'_>,
) -> bool {
    starts.iter().copied().any(|start| {
        ends.iter()
            .copied()
            .any(|end| reachability.reaches(start, end))
    })
}

struct ReachabilityCache<'a> {
    cfg: &'a ControlFlowGraph,
    cache: FxHashMap<BlockId, FxHashSet<BlockId>>,
}

impl<'a> ReachabilityCache<'a> {
    fn new(cfg: &'a ControlFlowGraph) -> Self {
        Self {
            cfg,
            cache: FxHashMap::default(),
        }
    }

    fn reaches(&mut self, start: BlockId, end: BlockId) -> bool {
        self.cache
            .entry(start)
            .or_insert_with(|| {
                let mut visited = FxHashSet::default();
                let mut stack = vec![start];

                while let Some(block) = stack.pop() {
                    if !visited.insert(block) {
                        continue;
                    }
                    for (successor, _) in self.cfg.successors(block) {
                        stack.push(*successor);
                    }
                }

                visited
            })
            .contains(&end)
    }
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
mod tests {
    use super::*;
    use crate::cfg::{RecordedCommandKind, build_control_flow_graph};
    use shuck_ast::{Command, CompoundCommand};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn model(source: &str) -> SemanticModel {
        model_with_dialect(source, ShellDialect::Bash)
    }

    fn model_with_dialect(source: &str, dialect: ShellDialect) -> SemanticModel {
        let output = Parser::with_dialect(source, dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        SemanticModel::build(&output.file, source, &indexer)
    }

    fn model_with_profile(source: &str, profile: ShellProfile) -> SemanticModel {
        let output = Parser::with_profile(source, profile.clone())
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                shell_profile: Some(profile),
                ..SemanticBuildOptions::default()
            },
        )
    }

    fn model_at_path_with_parse_dialect(path: &Path, dialect: ShellDialect) -> SemanticModel {
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::with_dialect(&source, dialect).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let mut observer = NoopTraversalObserver;
        build_with_observer_at_path_with_resolver(
            &output.file,
            &source,
            &indexer,
            &mut observer,
            Some(path),
            None,
        )
    }

    fn model_at_path(path: &Path) -> SemanticModel {
        model_at_path_with_resolver(path, None)
    }

    fn model_at_path_with_resolver(
        path: &Path,
        source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    ) -> SemanticModel {
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let mut observer = NoopTraversalObserver;
        build_with_observer_at_path_with_resolver(
            &output.file,
            &source,
            &indexer,
            &mut observer,
            Some(path),
            source_path_resolver,
        )
    }

    fn reportable_unused_names(model: &SemanticModel) -> Vec<Name> {
        let analysis = model.analysis();
        analysis
            .unused_assignments()
            .iter()
            .filter_map(|binding| {
                let binding = model.binding(*binding);
                matches!(
                    binding.kind,
                    BindingKind::Assignment
                        | BindingKind::ArrayAssignment
                        | BindingKind::LoopVariable
                        | BindingKind::ReadTarget
                        | BindingKind::MapfileTarget
                        | BindingKind::PrintfTarget
                        | BindingKind::GetoptsTarget
                        | BindingKind::ArithmeticAssignment
                )
                .then_some(binding.name.clone())
            })
            .collect()
    }

    fn assert_unused_assignment_parity(model: &SemanticModel) {
        let analysis = model.analysis();
        let precise = analysis.unused_assignments().to_vec();
        let exact = analysis.dataflow().unused_assignment_ids().to_vec();
        assert_eq!(precise, exact);
    }

    fn assert_uninitialized_reference_parity(model: &SemanticModel) {
        let analysis = model.analysis();
        let precise = analysis.uninitialized_references().to_vec();
        let exact = analysis.dataflow().uninitialized_references.clone();
        assert_eq!(precise, exact);
    }

    fn assert_dead_code_parity(model: &SemanticModel) {
        let analysis = model.analysis();
        let precise = analysis.dead_code().to_vec();
        let exact = analysis.dataflow().dead_code.clone();
        assert_eq!(precise, exact);
    }

    fn binding_names(model: &SemanticModel, ids: &[BindingId]) -> Vec<String> {
        ids.iter()
            .map(|binding_id| model.binding(*binding_id).name.to_string())
            .collect()
    }

    fn sorted_binding_names<I>(model: &SemanticModel, ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = BindingId>,
    {
        let mut names = ids
            .into_iter()
            .map(|binding_id| model.binding(binding_id).name.to_string())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    fn block_with_reference(cfg: &ControlFlowGraph, reference: ReferenceId) -> BlockId {
        cfg.blocks()
            .iter()
            .find(|block| block.references.contains(&reference))
            .map(|block| block.id)
            .expect("reference should be assigned to a CFG block")
    }

    fn unresolved_names(model: &SemanticModel) -> Vec<String> {
        model
            .unresolved_references()
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
            .collect()
    }

    fn uninitialized_names(model: &SemanticModel) -> Vec<String> {
        let analysis = model.analysis();
        let references = analysis
            .uninitialized_references()
            .iter()
            .map(|reference| reference.reference)
            .collect::<Vec<_>>();
        references
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
            .collect()
    }

    fn uninitialized_details(model: &SemanticModel) -> Vec<(String, UninitializedCertainty)> {
        let references = model.analysis().uninitialized_references().to_vec();
        references
            .iter()
            .map(|reference| {
                (
                    model.reference(reference.reference).name.to_string(),
                    reference.certainty,
                )
            })
            .collect()
    }

    fn assert_names_absent(names: &[&str], actual: &[String]) {
        for name in names {
            assert!(
                !actual.iter().any(|actual_name| actual_name == name),
                "did not expect `{name}` in {actual:?}"
            );
        }
    }

    fn assert_names_present(names: &[&str], actual: &[String]) {
        for name in names {
            assert!(
                actual.iter().any(|actual_name| actual_name == name),
                "expected `{name}` in {actual:?}"
            );
        }
    }

    fn arithmetic_read_count(model: &SemanticModel, name: &str) -> usize {
        model
            .references()
            .iter()
            .filter(|reference| {
                reference.kind == ReferenceKind::ArithmeticRead && reference.name == name
            })
            .count()
    }

    fn arithmetic_write_count(model: &SemanticModel, name: &str) -> usize {
        model
            .bindings()
            .iter()
            .filter(|binding| {
                binding.kind == BindingKind::ArithmeticAssignment && binding.name == name
            })
            .count()
    }

    fn assert_arithmetic_usage(
        model: &SemanticModel,
        name: &str,
        expected_reads: usize,
        expected_writes: usize,
    ) {
        assert_eq!(
            arithmetic_read_count(model, name),
            expected_reads,
            "unexpected arithmetic read count for {name}"
        );
        assert_eq!(
            arithmetic_write_count(model, name),
            expected_writes,
            "unexpected arithmetic write count for {name}"
        );
    }

    fn common_runtime_source(shebang: &str) -> String {
        format!(
            "{shebang}\nprintf '%s\\n' \"$IFS\" \"$USER\" \"$HOME\" \"$SHELL\" \"$PWD\" \"$TERM\" \"$PATH\" \"$CDPATH\" \"$LANG\" \"$LC_ALL\" \"$LC_TIME\" \"$SUDO_USER\" \"$DOAS_USER\"\n"
        )
    }

    fn bash_runtime_source(shebang: &str) -> String {
        format!(
            "{shebang}\nprintf '%s\\n' \"$LINENO\" \"$FUNCNAME\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_LINENO[0]}}\" \"$RANDOM\" \"${{BASH_REMATCH[0]}}\" \"$READLINE_LINE\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[0]}}\" \"$OSTYPE\" \"$HISTCONTROL\" \"$HISTSIZE\"\n"
        )
    }

    #[test]
    fn creates_file_and_function_scopes_and_resolves_local_shadowing() {
        let source = "VAR=global\nf() { local VAR=local; echo $VAR; }\n";
        let model = model(source);

        assert!(matches!(model.scope_kind(ScopeId(0)), ScopeKind::File));
        assert!(model.scopes().iter().any(|scope| {
            matches!(
                &scope.kind,
                ScopeKind::Function(function) if function.contains_name_str("f")
            )
        }));

        let local_binding = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "VAR"
                    && matches!(
                        binding.kind,
                        BindingKind::Declaration(DeclarationBuiltin::Local)
                    )
            })
            .unwrap();
        assert!(matches!(
            model.scope_kind(local_binding.scope),
            ScopeKind::Function(function) if function.contains_name_str("f")
        ));

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "VAR")
            .unwrap();
        let resolved = model.resolved_binding(reference.id).unwrap();
        assert_eq!(resolved.id, local_binding.id);
    }

    #[test]
    fn zsh_anonymous_functions_create_function_scoped_locals() {
        let source =
            "function { local scoped=1; echo \"$scoped\" \"$1\"; } arg\necho \"$scoped\"\n";
        let model = model_with_dialect(source, ShellDialect::Zsh);

        let local_binding = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "scoped"
                    && matches!(
                        binding.kind,
                        BindingKind::Declaration(DeclarationBuiltin::Local)
                    )
            })
            .unwrap();
        let ScopeKind::Function(function_scope) = model.scope_kind(local_binding.scope) else {
            panic!("expected local binding to live in a function scope");
        };
        assert!(function_scope.is_anonymous());

        let scoped_refs = model
            .references()
            .iter()
            .filter(|reference| {
                reference.kind == ReferenceKind::Expansion && reference.name == "scoped"
            })
            .collect::<Vec<_>>();
        assert_eq!(scoped_refs.len(), 2);

        let inner_ref = scoped_refs
            .iter()
            .find(|reference| reference.span.start.line == 1)
            .unwrap();
        let outer_ref = scoped_refs
            .iter()
            .find(|reference| reference.span.start.line == 2)
            .unwrap();

        assert_eq!(
            model.resolved_binding(inner_ref.id).unwrap().id,
            local_binding.id
        );
        assert!(model.resolved_binding(outer_ref.id).is_none());
    }

    #[test]
    fn zsh_multi_name_functions_bind_each_static_alias() {
        let source = "function music itunes() { local track=1; }\n";
        let model = model_with_dialect(source, ShellDialect::Zsh);

        let music_defs = model.function_definitions(&Name::from("music"));
        let itunes_defs = model.function_definitions(&Name::from("itunes"));
        assert_eq!(music_defs.len(), 1);
        assert_eq!(itunes_defs.len(), 1);
        assert_eq!(model.binding(music_defs[0]).span.slice(source), "music");
        assert_eq!(model.binding(itunes_defs[0]).span.slice(source), "itunes");

        let local_binding = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "track"
                    && matches!(
                        binding.kind,
                        BindingKind::Declaration(DeclarationBuiltin::Local)
                    )
            })
            .unwrap();
        let ScopeKind::Function(function_scope) = model.scope_kind(local_binding.scope) else {
            panic!("expected local binding to live in a function scope");
        };
        assert!(function_scope.contains_name_str("music"));
        assert!(function_scope.contains_name_str("itunes"));
        assert_eq!(function_scope.static_names().len(), 2);
    }

    #[test]
    fn zsh_multi_name_function_lookup_works_through_any_alias() {
        let source = "flag=1\nfunction music itunes() { echo \"$flag\"; }\nitunes\n";
        let model = model_with_dialect(source, ShellDialect::Zsh);

        assert_eq!(model.call_sites_for(&Name::from("itunes")).len(), 1);
        assert!(model.call_graph().reachable.contains(&Name::from("itunes")));
        assert!(
            !reportable_unused_names(&model)
                .into_iter()
                .any(|name| name == "flag")
        );
    }

    #[test]
    fn zsh_parameter_modifiers_still_register_references() {
        let model = model_with_dialect("print ${(m)foo}\n", ShellDialect::Zsh);
        let unresolved = unresolved_names(&model);

        assert_names_present(&["foo"], &unresolved);
    }

    #[test]
    fn zsh_parameter_operations_walk_operand_references_conservatively() {
        let model = model_with_dialect(
            "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}}\n",
            ShellDialect::Zsh,
        );
        let unresolved = unresolved_names(&model);

        assert_names_present(
            &[
                "foo",
                "needle",
                "pattern",
                "replacement",
                "offset",
                "length",
            ],
            &unresolved,
        );
    }

    #[test]
    fn zsh_for_loops_bind_all_targets() {
        let source = "\
for key value in a b c d; do
  print -r -- \"$key:$value\"
done
for 1 2 3; do
  print -r -- \"$1|$2|$3\"
done
";
        let model = model_with_dialect(source, ShellDialect::Zsh);

        let loop_bindings = model
            .bindings()
            .iter()
            .filter(|binding| binding.kind == BindingKind::LoopVariable)
            .map(|binding| {
                (
                    binding.name.to_string(),
                    binding.span.slice(source).to_string(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            loop_bindings,
            vec![
                ("key".to_owned(), "key".to_owned()),
                ("value".to_owned(), "value".to_owned()),
                ("1".to_owned(), "1".to_owned()),
                ("2".to_owned(), "2".to_owned()),
                ("3".to_owned(), "3".to_owned()),
            ]
        );

        for name in ["key", "value", "1", "2", "3"] {
            let reference = model
                .references()
                .iter()
                .find(|reference| {
                    reference.kind == ReferenceKind::Expansion && reference.name == name
                })
                .unwrap_or_else(|| panic!("expected expansion reference for {name}"));
            let binding = model
                .resolved_binding(reference.id)
                .unwrap_or_else(|| panic!("expected {name} to resolve to a loop binding"));
            assert_eq!(binding.kind, BindingKind::LoopVariable);
            assert_eq!(binding.name, name);
        }
    }

    #[test]
    fn isolates_subshell_bindings_from_parent_resolution() {
        let source = "VAR=outer\n( VAR=inner )\necho $VAR\n";
        let model = model(source);

        let reference = model.references().last().unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.span.slice(source), "VAR");
    }

    #[test]
    fn records_pipeline_segment_scopes() {
        let source = "a | b | c\n";
        let model = model(source);

        let pipeline_scopes = model
            .scopes()
            .iter()
            .filter(|scope| matches!(scope.kind, ScopeKind::Pipeline))
            .count();
        assert_eq!(pipeline_scopes, 3);
    }

    #[test]
    fn indexed_scope_lookup_matches_linear_scan_for_all_offsets() {
        let source = "\
outer() {
  local current=1
  (
    printf '%s\\n' \"$(
      printf '%s\\n' \"$current\" | tr a b
    )\"
  )
  inner() { echo \"$current\"; }
}
outer
";
        let model = model(source);

        for offset in 0..=source.len() {
            assert_eq!(
                model.scope_at(offset),
                linear_scope_at(model.scopes(), offset),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn recorded_program_preserves_logical_list_order_in_ranges() {
        let source = "a && b || c\n";
        let model = model(source);

        let file_commands = model
            .recorded_program
            .commands_in(model.recorded_program.file_commands());
        assert_eq!(file_commands.len(), 1);

        let command = model.recorded_program.command(file_commands[0]);
        let (first, rest) = match command.kind {
            RecordedCommandKind::List { first, rest } => (first, rest),
            other => panic!("expected list command, found {other:?}"),
        };

        assert!(
            model
                .recorded_program
                .command(first)
                .span
                .slice(source)
                .starts_with("a")
        );
        let rest = model.recorded_program.list_items(rest);
        assert_eq!(rest.len(), 2);
        assert!(
            model
                .recorded_program
                .command(rest[0].command)
                .span
                .slice(source)
                .starts_with("b")
        );
        assert!(
            model
                .recorded_program
                .command(rest[1].command)
                .span
                .slice(source)
                .starts_with("c")
        );
    }

    #[test]
    fn recorded_program_preserves_pipeline_segment_order_in_ranges() {
        let source = "a | b | c\n";
        let model = model(source);

        let file_commands = model
            .recorded_program
            .commands_in(model.recorded_program.file_commands());
        assert_eq!(file_commands.len(), 1);

        let command = model.recorded_program.command(file_commands[0]);
        let segments = match command.kind {
            RecordedCommandKind::Pipeline { segments } => {
                model.recorded_program.pipeline_segments(segments)
            }
            other => panic!("expected pipeline command, found {other:?}"),
        };

        assert_eq!(segments.len(), 3);
        assert!(
            model
                .recorded_program
                .command(segments[0].command)
                .span
                .slice(source)
                .starts_with("a")
        );
        assert!(
            model
                .recorded_program
                .command(segments[1].command)
                .span
                .slice(source)
                .starts_with("b")
        );
        assert!(
            model
                .recorded_program
                .command(segments[2].command)
                .span
                .slice(source)
                .starts_with("c")
        );
    }

    #[test]
    fn arithmetic_plain_assignment_is_write_only() {
        let model = model("(( i = 0 ))\n");
        assert_arithmetic_usage(&model, "i", 0, 1);
    }

    #[test]
    fn arithmetic_compound_assignment_is_read_write() {
        let model = model("(( i += 2 ))\n");
        assert_arithmetic_usage(&model, "i", 1, 1);
    }

    #[test]
    fn arithmetic_prefix_update_is_read_write() {
        let model = model("(( ++i ))\n");
        assert_arithmetic_usage(&model, "i", 1, 1);
    }

    #[test]
    fn arithmetic_postfix_update_is_read_write() {
        let model = model("(( i++ ))\n");
        assert_arithmetic_usage(&model, "i", 1, 1);
    }

    #[test]
    fn arithmetic_assignment_reads_index_expressions() {
        let model = model("(( a[i++] = 1 ))\n");
        assert_arithmetic_usage(&model, "a", 0, 1);
        assert_arithmetic_usage(&model, "i", 1, 1);
    }

    #[test]
    fn arithmetic_conditional_tracks_branch_reads_and_writes() {
        let model = model("(( x ? y++ : (z = 1) ))\n");
        assert_arithmetic_usage(&model, "x", 1, 0);
        assert_arithmetic_usage(&model, "y", 1, 1);
        assert_arithmetic_usage(&model, "z", 0, 1);
    }

    #[test]
    fn arithmetic_comma_walks_each_expression_in_order() {
        let model = model("(( x = 1, y += x, z ))\n");
        assert_arithmetic_usage(&model, "x", 1, 1);
        assert_arithmetic_usage(&model, "y", 1, 1);
        assert_arithmetic_usage(&model, "z", 1, 0);
    }

    #[test]
    fn arithmetic_shell_words_still_walk_nested_expansions() {
        let model = model("echo $(( $(printf '%s' \"$x\") + y ))\n");
        assert!(model.references().iter().any(|reference| {
            reference.kind == ReferenceKind::Expansion && reference.name == "x"
        }));
        assert_arithmetic_usage(&model, "y", 1, 0);
    }

    #[test]
    fn classifies_nameref_and_source_directives() {
        let source = "\
declare -n ref=target
# shellcheck source=lib.sh
source \"$x\"
";
        let model = model(source);

        let nameref = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "ref")
            .unwrap();
        assert!(matches!(nameref.kind, BindingKind::Nameref));
        assert!(nameref.attributes.contains(BindingAttributes::NAMEREF));

        assert_eq!(
            model.source_refs()[0].kind,
            SourceRefKind::Directive("lib.sh".to_string())
        );
        assert_eq!(
            model.source_refs()[0].resolution,
            SourceRefResolution::Unchecked
        );
    }

    #[test]
    fn source_directive_applies_across_contiguous_own_line_comments() {
        let source = "\
# shellcheck source=lib.sh
# shellcheck disable=SC2154
source \"$x\"
";
        let model = model(source);

        assert_eq!(
            model.source_refs()[0].kind,
            SourceRefKind::Directive("lib.sh".to_string())
        );
    }

    #[test]
    fn builds_transitive_call_graph_and_overwritten_functions() {
        let source = "\
f() { g; }
g() { echo hi; }
f
f() { echo again; }
";
        let model = model(source);

        assert!(model.call_graph().reachable.contains("f"));
        assert!(model.call_graph().reachable.contains("g"));
        assert_eq!(model.call_graph().overwritten.len(), 1);
        assert_eq!(model.call_graph().overwritten[0].name, "f");
    }

    #[test]
    fn precise_overwritten_functions_track_real_overwrites() {
        let source = "\
f() { echo hi; }
f() { echo again; }
";
        let model = model(source);
        let analysis = model.analysis();
        let overwritten = analysis.overwritten_functions();

        assert_eq!(overwritten.len(), 1);
        assert_eq!(overwritten[0].name, "f");
        assert!(!overwritten[0].first_called);
    }

    #[test]
    fn precise_overwritten_functions_preserve_calls_before_redefinition() {
        let source = "\
f() { echo hi; }
f
f() { echo again; }
";
        let model = model(source);
        let analysis = model.analysis();
        let overwritten = analysis.overwritten_functions();

        assert_eq!(overwritten.len(), 1);
        assert!(overwritten[0].first_called);
    }

    #[test]
    fn precise_overwritten_functions_ignore_mutually_exclusive_branches() {
        let source = "\
if cond; then
  helper() { return 0; }
else
  helper() { return 1; }
fi
helper
";
        let model = model(source);

        assert!(model.analysis().overwritten_functions().is_empty());
    }

    #[test]
    fn precise_overwritten_functions_do_not_merge_distinct_helper_scopes() {
        let source = "\
factory_one() {
  helper() { return 0; }
  helper
}
factory_two() {
  helper() { return 1; }
  helper
}
factory_one
factory_two
";
        let model = model(source);

        assert!(model.analysis().overwritten_functions().is_empty());
    }

    #[test]
    fn tracks_flow_context_for_conditions_and_loops() {
        let source = "\
if cmd; then
  echo ok
fi
for x in 1 2; do
  break
done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);

        let Command::Compound(CompoundCommand::If(if_command)) = &output.file.body[0].command
        else {
            panic!("expected if command");
        };
        let condition_span = match &if_command.condition[0].command {
            Command::Simple(command) => command.span,
            other => panic!("unexpected condition command: {other:?}"),
        };
        let condition_context = model.flow_context_at(&condition_span).unwrap();
        assert!(condition_context.exit_status_checked);

        let Command::Compound(CompoundCommand::For(for_command)) = &output.file.body[1].command
        else {
            panic!("expected for command");
        };
        let break_span = match &for_command.body[0].command {
            Command::Builtin(shuck_ast::BuiltinCommand::Break(command)) => command.span,
            other => panic!("unexpected loop body command: {other:?}"),
        };
        let break_context = model.flow_context_at(&break_span).unwrap();
        assert_eq!(break_context.loop_depth, 1);
    }

    #[test]
    fn detects_overwritten_assignments_and_possible_uninitialized_reads() {
        let overwritten_source = "VAR=x\nVAR=y\necho $VAR\n";
        let overwritten = model(overwritten_source);
        let overwritten_analysis = overwritten.analysis();
        let dataflow = overwritten_analysis.dataflow();
        assert_eq!(dataflow.unused_assignments.len(), 1);
        assert!(matches!(
            dataflow.unused_assignments[0].reason,
            UnusedReason::Overwritten { .. }
        ));

        let partial_source = "if cond; then VAR=x; fi\necho $VAR\n";
        let partial = model(partial_source);
        let partial_analysis = partial.analysis();
        let dataflow = partial_analysis.dataflow();
        assert_eq!(dataflow.uninitialized_references.len(), 1);
        assert_eq!(
            dataflow.uninitialized_references[0].certainty,
            UninitializedCertainty::Possible
        );
    }

    #[test]
    fn precise_uninitialized_references_match_dataflow_for_representative_cases() {
        let cases = [
            "echo $VAR\n",
            "if cond; then VAR=x; fi\necho $VAR\n",
            "f() { local VAR; echo \"$VAR\"; }\nf\n",
            "#!/bin/bash\nf() { local carrier; echo \"${!carrier}\"; }\nf\n",
            "printf '%s\\n' \"${VAR:-fallback}\" \"${MAYBE:+alt}\" \"${REQ:?missing}\" \"${INIT:=value}\"\nprintf '%s\\n' \"$INIT\"\n",
            "#!/bin/sh\nprintf '%s\\n' \"$HOME\"\n",
            "#!/bin/bash\nprintf '%s\\n' \"$RANDOM\"\n",
        ];

        for source in cases {
            let model = model(source);
            assert_uninitialized_reference_parity(&model);
        }
    }

    #[test]
    fn precise_dead_code_matches_dataflow_for_representative_cases() {
        let cases = [
            "exit 0\necho dead\n",
            "\
if true; then
  exit 0
else
  exit 1
fi
echo unreachable
",
            "\
f() {
  return 0
  echo dead
}
f
",
        ];

        for source in cases {
            let model = model(source);
            assert_dead_code_parity(&model);
        }
    }

    #[test]
    fn precompute_unused_assignments_skips_dataflow_for_linear_duplicate_assignments() {
        let model = model(
            "\
emoji[grinning]=1
emoji[smile]=2
",
        );
        let analysis = model.analysis();

        let precise = analysis.unused_assignments().to_vec();

        assert!(analysis.cfg.get().is_some());
        assert!(analysis.dataflow.get().is_none());
        assert_eq!(binding_names(&model, &precise), vec!["emoji", "emoji"]);

        let exact = analysis.dataflow().unused_assignment_ids().to_vec();
        assert_eq!(precise, exact);
    }

    #[test]
    fn heuristic_unused_assignment_path_skips_exact_variable_dataflow_bundle() {
        let model = model("unused=1\n");
        let analysis = model.analysis();

        let precise = analysis.unused_assignments().to_vec();

        assert!(analysis.exact_variable_dataflow.get().is_none());
        assert!(analysis.dataflow.get().is_none());
        assert_eq!(binding_names(&model, &precise), vec!["unused"]);
    }

    #[test]
    fn variable_dataflow_results_do_not_depend_on_query_order() {
        let source = "VAR=x\nVAR=y\necho $VAR\necho $UNDEF\n";
        let model = model(source);

        let unused_then_uninitialized = {
            let analysis = model.analysis();
            let unused = analysis.unused_assignments().to_vec();
            let uninitialized = analysis.uninitialized_references().to_vec();
            (unused, uninitialized)
        };
        let uninitialized_then_unused = {
            let analysis = model.analysis();
            let uninitialized = analysis.uninitialized_references().to_vec();
            let unused = analysis.unused_assignments().to_vec();
            (unused, uninitialized)
        };

        assert_eq!(unused_then_uninitialized.0, uninitialized_then_unused.0);
        assert_eq!(unused_then_uninitialized.1, uninitialized_then_unused.1);
    }

    #[test]
    fn shared_exact_variable_dataflow_is_reused_across_accessors() {
        let model = model("VAR=x\nVAR=y\necho $VAR\necho $UNDEF\n");
        let analysis = model.analysis();

        assert!(analysis.exact_variable_dataflow.get().is_none());

        let unused = analysis.unused_assignments().to_vec();
        let bundle_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;
        let uninitialized = analysis.uninitialized_references().to_vec();
        let reused_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;

        assert_eq!(bundle_ptr, reused_ptr);
        assert_eq!(binding_names(&model, &unused), vec!["VAR"]);
        assert_eq!(
            uninitialized
                .iter()
                .map(|reference| model.reference(reference.reference).name.to_string())
                .collect::<Vec<_>>(),
            vec!["UNDEF"]
        );
    }

    #[test]
    fn materialized_reaching_definitions_match_dense_exact_results() {
        let model = model("VAR=outer\nif cond; then VAR=inner; fi\necho $VAR\n");
        let analysis = model.analysis();
        let dataflow = analysis.dataflow();
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name.as_str() == "VAR")
            .expect("expected a VAR reference");
        let block_id = block_with_reference(analysis.cfg(), reference.id);

        assert_eq!(
            sorted_binding_names(
                &model,
                dataflow.reaching_definitions.reaching_in[&block_id]
                    .iter()
                    .copied()
            ),
            vec!["VAR", "VAR"]
        );
    }

    #[test]
    fn precise_unused_assignments_match_dataflow_for_representative_cases() {
        let cases = [
            "VAR=x\nVAR=y\necho $VAR\n",
            "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
",
            "\
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
",
            "\
check_status() {
  if [[ $is_wget ]]; then
    printf '%s\\n' ok
  else
    is_wget=1
    check_status
  fi
}
check_status
",
            "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
",
            "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
",
            "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
",
            "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
",
        ];

        for source in cases {
            let model = model(source);
            assert_unused_assignment_parity(&model);
        }
    }

    #[test]
    fn branch_assignments_reaching_a_later_read_are_both_used() {
        let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
";
        let model = model(source);
        let analysis = model.analysis();
        let dataflow = analysis.dataflow();

        assert!(dataflow.unused_assignments.is_empty());
    }

    #[test]
    fn mutually_exclusive_unused_branch_assignments_collapse_to_one_reported_id() {
        let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
";
        let model = model(source);
        let all_bindings = model.bindings_for(&Name::from("code_command")).to_vec();
        let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

        assert_eq!(model.analysis().dataflow().unused_assignments.len(), 2);
        assert_eq!(binding_ids, vec![all_bindings[1]]);
    }

    #[test]
    fn partially_used_branch_assignments_keep_each_dead_arm_reported() {
        let source = "\
if a; then
  VAR=1
elif b; then
  VAR=2
else
  VAR=3
  echo \"$VAR\"
fi
";
        let model = model(source);
        let all_bindings = model.bindings_for(&Name::from("VAR")).to_vec();
        let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

        assert_eq!(binding_ids, vec![all_bindings[0], all_bindings[1]]);
    }

    #[test]
    fn branch_join_defs_used_in_later_function_body_are_all_live() {
        let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
show_version() { ${code_command} --version; }
";
        let model = model(source);
        let unused_bindings = model
            .analysis()
            .dataflow()
            .unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect::<Vec<_>>();
        let unused_names = unused_bindings
            .into_iter()
            .map(|binding| model.binding(binding).name.to_string())
            .collect::<Vec<_>>();

        assert!(!unused_names.contains(&"code_command".to_string()));
    }

    #[test]
    fn elif_branch_join_defs_used_in_later_function_body_are_all_live() {
        let source = "\
if [ \"$arch\" = amd64 ]; then
  jq_arch=amd64
elif [ \"$arch\" = arm64 ]; then
  jq_arch=arm64
else
  jq_arch=unknown
fi
download() { echo \"$jq_arch\"; }
";
        let model = model(source);
        let unused_bindings = model
            .analysis()
            .dataflow()
            .unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect::<Vec<_>>();
        let unused_names = unused_bindings
            .into_iter()
            .map(|binding| model.binding(binding).name.to_string())
            .collect::<Vec<_>>();

        assert!(!unused_names.contains(&"jq_arch".to_string()));
    }

    #[test]
    fn case_branch_join_defs_used_in_later_function_body_are_all_live() {
        let source = "\
case \"$arch\" in
amd64 | x86_64)
  jq_arch=amd64
  core_arch=64
  ;;
arm64 | aarch64)
  jq_arch=arm64
  core_arch=arm64-v8a
  ;;
esac
download() {
  echo \"$jq_arch\"
  echo \"$core_arch\"
}
";
        let model = model(source);
        let unused_bindings = model
            .analysis()
            .dataflow()
            .unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect::<Vec<_>>();
        let unused_names = unused_bindings
            .into_iter()
            .map(|binding| model.binding(binding).name.to_string())
            .collect::<Vec<_>>();

        assert!(!unused_names.contains(&"jq_arch".to_string()));
        assert!(!unused_names.contains(&"core_arch".to_string()));
    }

    #[test]
    fn case_without_matching_arm_keeps_initializer_live() {
        let source = "\
value=''
case \"$kind\" in
  one)
    value=1
    ;;
  two)
    value=2
    ;;
esac
printf '%s\\n' \"$value\"
";
        let model = model_with_dialect(source, ShellDialect::Posix);
        let unused = reportable_unused_names(&model);

        assert!(
            !unused.contains(&Name::from("value")),
            "unused bindings: {:?}",
            unused
        );
    }

    #[test]
    fn case_with_catch_all_arm_overwrites_initializer() {
        let source = "\
value=''
case \"$kind\" in
  one)
    value=1
    ;;
  *)
    value=2
    ;;
esac
printf '%s\\n' \"$value\"
";
        let model = model_with_dialect(source, ShellDialect::Posix);
        let unused = reportable_unused_names(&model);
        let count = unused
            .iter()
            .filter(|name| name.as_str() == "value")
            .count();

        assert_eq!(count, 1, "unused bindings: {:?}", unused);
    }

    #[test]
    fn empty_case_catch_all_arm_keeps_following_code_reachable() {
        let source = "\
case \"$kind\" in
  *)
    ;;
esac
printf '%s\\n' ok
";
        let model = model_with_dialect(source, ShellDialect::Posix);

        assert!(
            model.analysis().dead_code().is_empty(),
            "dead code: {:?}",
            model.analysis().dead_code()
        );
    }

    #[test]
    fn catch_all_continue_case_arm_keeps_following_code_reachable() {
        let source = "\
case \"$kind\" in
  *)
    :
    ;;&
esac
printf '%s\\n' ok
";
        let model = model(source);

        assert!(
            model.analysis().dead_code().is_empty(),
            "dead code: {:?}",
            model.analysis().dead_code()
        );
    }

    #[test]
    fn function_global_assignments_read_later_by_caller_are_live() {
        let source = "\
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
";
        let model = model(source);
        let unused_bindings = model
            .analysis()
            .dataflow()
            .unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect::<Vec<_>>();
        let unused_names = unused_bindings
            .into_iter()
            .map(|binding| model.binding(binding).name.to_string())
            .collect::<Vec<_>>();

        assert!(!unused_names.contains(&"local_install".to_string()));
        assert!(!unused_names.contains(&"proxy".to_string()));
    }

    #[test]
    fn callee_subshell_reads_keep_caller_assignments_live() {
        let source = "\
#!/bin/bash
install_package() {
  (
    printf '%s\\n' \"$archive_format\" \"${configure[@]}\"
  )
}
install_readline() {
  archive_format='tar.gz'
  configure=( ./configure --disable-dependency-tracking )
  install_package
}
install_readline
";
        let model = model(source);
        let unused = reportable_unused_names(&model);

        assert!(
            !unused.contains(&Name::from("archive_format")),
            "unused: {:?}",
            unused
        );
        assert!(
            !unused.contains(&Name::from("configure")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn later_file_scope_helper_reads_keep_caller_local_assignment_live() {
        let source = "\
main() {
  local status=''
  helper
  printf '%s\\n' \"$status\"
}
helper() {
  status=ok
}
main
";
        let model = model(source);

        let unused = reportable_unused_names(&model);
        assert!(
            !unused.contains(&Name::from("status")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn later_file_scope_helper_appends_keep_caller_local_array_live() {
        let source = "\
#!/bin/bash
main() {
  local errors=()
  helper
  printf '%s\\n' \"${errors[@]}\"
}
helper() {
  errors+=(oops)
}
main
";
        let model = model(source);

        let unused = reportable_unused_names(&model);
        assert!(
            !unused.contains(&Name::from("errors")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn recursive_function_reads_keep_later_global_write_live() {
        let source = "\
check_status() {
  if [[ $is_wget ]]; then
    printf '%s\\n' ok
  else
    is_wget=1
    check_status
  fi
}
check_status
";
        let model = model(source);
        let unused_bindings = model
            .analysis()
            .dataflow()
            .unused_assignments
            .iter()
            .map(|unused| unused.binding)
            .collect::<Vec<_>>();
        let unused_names = unused_bindings
            .into_iter()
            .map(|binding| model.binding(binding).name.to_string())
            .collect::<Vec<_>>();

        assert!(!unused_names.contains(&"is_wget".to_string()));
    }

    #[test]
    fn name_only_export_consumes_existing_binding() {
        let source = "foo=1\nexport foo\n";
        let model = model(source);

        let foo_bindings = model
            .bindings()
            .iter()
            .filter(|binding| binding.name == "foo")
            .collect::<Vec<_>>();
        assert_eq!(foo_bindings.len(), 1);
        assert!(
            foo_bindings[0]
                .attributes
                .contains(BindingAttributes::EXPORTED)
        );

        let declaration_reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::DeclarationName && reference.name == "foo"
            })
            .unwrap();
        let resolved = model.resolved_binding(declaration_reference.id).unwrap();
        assert_eq!(resolved.id, foo_bindings[0].id);
    }

    #[test]
    fn name_only_local_creates_a_binding_for_later_reads() {
        let source = "f() { local VAR; echo \"$VAR\"; }\n";
        let model = model(source);

        let local_binding = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "VAR"
                    && matches!(
                        binding.kind,
                        BindingKind::Declaration(DeclarationBuiltin::Local)
                    )
            })
            .unwrap();
        assert!(
            !local_binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
        );

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "VAR")
            .unwrap();
        let resolved = model.resolved_binding(reference.id).unwrap();
        assert_eq!(resolved.id, local_binding.id);
        let reference_id = reference.id;
        let analysis = model.analysis();
        let uninitialized = analysis.uninitialized_references();
        assert_eq!(uninitialized.len(), 1);
        assert_eq!(uninitialized[0].reference, reference_id);
        assert_eq!(uninitialized[0].certainty, UninitializedCertainty::Definite);
    }

    #[test]
    fn special_command_targets_store_name_only_spans() {
        let source = "\
read -r read_target
mapfile mapfile_target
readarray readarray_target
printf -v printf_target '%s' value
getopts 'ab' getopts_target
";
        let model = model(source);

        let read_target = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "read_target" && matches!(binding.kind, BindingKind::ReadTarget)
            })
            .unwrap();
        assert_eq!(read_target.span.slice(source), "read_target");

        let mapfile_target = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "mapfile_target"
                    && matches!(binding.kind, BindingKind::MapfileTarget)
            })
            .unwrap();
        assert_eq!(mapfile_target.span.slice(source), "mapfile_target");

        let readarray_target = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "readarray_target"
                    && matches!(binding.kind, BindingKind::MapfileTarget)
            })
            .unwrap();
        assert_eq!(readarray_target.span.slice(source), "readarray_target");

        let printf_target = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "printf_target" && matches!(binding.kind, BindingKind::PrintfTarget)
            })
            .unwrap();
        assert_eq!(printf_target.span.slice(source), "printf_target");

        let getopts_target = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "getopts_target"
                    && matches!(binding.kind, BindingKind::GetoptsTarget)
            })
            .unwrap();
        assert_eq!(getopts_target.span.slice(source), "getopts_target");
    }

    #[test]
    fn read_header_bindings_consumed_in_loop_body_are_live() {
        let source = "\
printf '%s\n' 'service safe ok yes' | while read UNIT EXPOSURE PREDICATE HAPPY; do
  printf '%s %s %s %s\n' \"$UNIT\" \"$EXPOSURE\" \"$PREDICATE\" \"$HAPPY\"
done
";
        let model = model(source);
        let unused = reportable_unused_names(&model);

        for name in ["UNIT", "EXPOSURE", "PREDICATE", "HAPPY"] {
            assert!(
                !unused.contains(&Name::from(name)),
                "unused bindings: {:?}",
                unused
            );
        }
    }

    #[test]
    fn command_prefix_assignments_do_not_create_shell_bindings() {
        let source = "\
base_flags=1
CFLAGS=\"$base_flags\" make
echo \"$CFLAGS\"
";
        let model = model(source);

        assert!(
            model
                .bindings()
                .iter()
                .all(|binding| binding.name != "CFLAGS")
        );

        let cflags_reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::Expansion && reference.name == "CFLAGS"
            })
            .unwrap();
        assert!(model.resolved_binding(cflags_reference.id).is_none());
        assert!(model.unresolved_references().contains(&cflags_reference.id));
    }

    #[test]
    fn indirect_expansion_keeps_dynamic_target_arrays_live() {
        let source = "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"apache_args"));
        assert!(!unused.contains(&"nginx_args"));

        let carrier = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "args_var")
            .unwrap();
        let reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::IndirectExpansion && reference.name == "args_var"
            })
            .unwrap();

        let mut carrier_targets =
            binding_names(&model, model.indirect_targets_for_binding(carrier.id));
        carrier_targets.sort();
        carrier_targets.dedup();
        assert_eq!(carrier_targets, vec!["apache_args", "nginx_args"]);

        let mut reference_targets =
            binding_names(&model, model.indirect_targets_for_reference(reference.id));
        reference_targets.sort();
        reference_targets.dedup();
        assert_eq!(reference_targets, vec!["apache_args", "nginx_args"]);
    }

    #[test]
    fn append_assignments_contribute_to_later_array_expansion() {
        let source = "\
#!/bin/bash
arr=(--first)
arr+=(--second)
printf '%s\\n' \"${arr[@]}\"
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"arr"));
    }

    #[test]
    fn associative_compound_declaration_marks_binding_assoc_and_array() {
        let model = model("#!/bin/bash\ndeclare -A assoc=(one [foo]=bar [bar]+=baz)\n");

        let assoc = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "assoc")
            .expect("expected assoc binding");
        assert!(assoc.attributes.contains(BindingAttributes::ARRAY));
        assert!(assoc.attributes.contains(BindingAttributes::ASSOC));
    }

    #[test]
    fn read_implicitly_consumes_visible_ifs_binding() {
        let source = "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
";
        let model = model(source);
        model.analysis().dataflow();

        assert!(model.references().iter().any(|reference| {
            reference.name == "IFS" && matches!(reference.kind, ReferenceKind::ImplicitRead)
        }));

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"IFS"));
        assert!(unused.contains(&"unused"));
    }

    #[test]
    fn ifs_assignments_are_treated_as_implicitly_used() {
        let source = "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"IFS"));
        assert!(unused.contains(&"unused"));
    }

    #[test]
    fn shell_runtime_assignments_are_treated_as_implicitly_used() {
        let source = "\
#!/bin/sh
PATH=$PATH:/opt/custom
CDPATH=/tmp
LANG=C
LC_ALL=C
LC_TIME=C
unused=1
echo ok
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        for name in ["PATH", "CDPATH", "LANG", "LC_ALL", "LC_TIME"] {
            assert!(!unused.contains(&name), "unused bindings: {:?}", unused);
        }
        assert!(unused.contains(&"unused"));
    }

    #[test]
    fn bash_completion_runtime_vars_are_treated_as_live() {
        let source = "\
#!/bin/bash
_pyenv() {
  COMPREPLY=()
  local word=\"${COMP_WORDS[COMP_CWORD]}\"
  COMPREPLY=( $(compgen -W \"$(printf 'a b')\" -- \"$word\") )
}
complete -F _pyenv pyenv
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"COMPREPLY"));

        let uninitialized = uninitialized_names(&model);
        assert!(!uninitialized.contains(&"COMP_WORDS".to_string()));
        assert!(!uninitialized.contains(&"COMP_CWORD".to_string()));
    }

    #[test]
    fn exact_indirect_expansion_does_not_keep_unrelated_array_live() {
        let source = "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
";
        let model = model(source);
        model.analysis().dataflow();

        let unused = model
            .analysis()
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"apache_args"));
        assert!(unused.contains(&"unused_args"));

        let reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::IndirectExpansion && reference.name == "args_var"
            })
            .unwrap();
        let targets = binding_names(&model, model.indirect_targets_for_reference(reference.id));
        assert_eq!(targets, vec!["apache_args"]);
    }

    #[test]
    fn exact_indirect_target_resolution_tracks_underlying_binding() {
        let source = "\
#!/bin/bash
target=ok
name=target
printf '%s\\n' \"${!name}\"
";
        let model = model(source);

        let carrier = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "name")
            .unwrap();
        let reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::IndirectExpansion && reference.name == "name"
            })
            .unwrap();

        assert_eq!(
            binding_names(&model, model.indirect_targets_for_binding(carrier.id)),
            vec!["target"]
        );
        assert_eq!(
            binding_names(&model, model.indirect_targets_for_reference(reference.id)),
            vec!["target"]
        );
    }

    #[test]
    fn resolved_indirect_expansion_carrier_is_not_marked_uninitialized() {
        let source = "\
#!/bin/bash
f() {
  local carrier
  echo \"${!carrier}\"
}
f
";
        let model = model(source);
        assert!(uninitialized_names(&model).is_empty());
    }

    #[test]
    fn guarded_parameter_expansions_are_not_marked_uninitialized() {
        let source = "\
printf '%s\\n' \
  \"${missing_default:-fallback}\" \
  \"${missing_assign:=value}\" \
  \"${missing_replace:+alt}\" \
  \"${missing_error:?missing}\"
";
        let model = model(source);
        let unresolved = unresolved_names(&model);
        let uninitialized = uninitialized_names(&model);

        assert_names_present(
            &[
                "missing_default",
                "missing_assign",
                "missing_replace",
                "missing_error",
            ],
            &unresolved,
        );
        assert_names_absent(
            &[
                "missing_default",
                "missing_assign",
                "missing_replace",
                "missing_error",
            ],
            &uninitialized,
        );
    }

    #[test]
    fn guarded_parameter_operands_are_not_marked_uninitialized() {
        let source = "\
printf '%s\\n' \
  \"${missing_default:-$fallback_name}\" \
  \"${missing_assign:=${seed_name:-value}}\" \
  \"${missing_replace:+$replacement_name}\" \
  \"${missing_error:?$hint_name}\"
";
        let model = model(source);
        let unresolved = unresolved_names(&model);
        let uninitialized = uninitialized_names(&model);

        assert_names_present(
            &[
                "fallback_name",
                "seed_name",
                "replacement_name",
                "hint_name",
            ],
            &unresolved,
        );
        assert_names_absent(
            &[
                "fallback_name",
                "seed_name",
                "replacement_name",
                "hint_name",
            ],
            &uninitialized,
        );
    }

    #[test]
    fn assign_default_parameter_expansion_initializes_later_reads() {
        let source = "\
printf '%s\\n' \"${config_path:=/tmp/default}\"
printf '%s\\n' \"$config_path\" \"$still_missing\"
";
        let model = model(source);
        let uninitialized = uninitialized_names(&model);

        assert_names_absent(&["config_path"], &uninitialized);
        assert_names_present(&["still_missing"], &uninitialized);

        let binding = model
            .bindings()
            .iter()
            .find(|binding| {
                binding.name == "config_path"
                    && matches!(binding.kind, BindingKind::ParameterDefaultAssignment)
            })
            .unwrap();
        assert_eq!(binding.span.slice(source), "config_path");
    }

    #[test]
    fn default_parameter_operand_reads_are_tracked() {
        let source = "\
repo_root=$(pwd)
cache_dir=${1:-\"$repo_root/.cache\"}
printf '%s\\n' \"$cache_dir\"
";
        let model = model_with_dialect(source, ShellDialect::Posix);
        let unused = reportable_unused_names(&model);

        assert!(
            !unused.contains(&Name::from("repo_root")),
            "unused bindings: {:?}",
            unused
        );

        let reference = model
            .references()
            .iter()
            .find(|reference| {
                reference.kind == ReferenceKind::Expansion && reference.name == "repo_root"
            })
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.name, "repo_root");
    }

    #[test]
    fn detects_dead_code_after_exit() {
        let source = "exit 0\necho dead\n";
        let model = model(source);
        let analysis = model.analysis();
        let dead_code = analysis.dead_code();
        assert_eq!(dead_code.len(), 1);
        assert_eq!(
            dead_code[0].unreachable[0].slice(source).trim_end(),
            "echo dead"
        );
        assert_eq!(dead_code[0].cause.slice(source).trim_end(), "exit 0");
    }

    #[test]
    fn deferred_function_bodies_resolve_later_file_scope_bindings() {
        let source = "f() { echo $X; }\nX=1\nf\n";
        let model = model(source);

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.span.slice(source), "X");
    }

    #[test]
    fn deferred_non_brace_function_bodies_resolve_later_file_scope_bindings() {
        let source = "f() if true; then echo $X; fi\nX=1\nf\n";
        let model = model(source);

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.span.slice(source), "X");
    }

    #[test]
    fn top_level_reads_remain_source_order_sensitive() {
        let source = "echo $X\nX=1\n";
        let model = model(source);

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
            .unwrap();
        assert!(model.resolved_binding(reference.id).is_none());
        assert_eq!(model.unresolved_references(), &[reference.id]);
    }

    #[test]
    fn common_runtime_vars_are_not_marked_uninitialized_in_bash_and_sh_scripts() {
        let names = [
            "IFS",
            "USER",
            "HOME",
            "SHELL",
            "PWD",
            "TERM",
            "PATH",
            "CDPATH",
            "LANG",
            "LC_ALL",
            "LC_TIME",
            "SUDO_USER",
            "DOAS_USER",
        ];

        for shebang in ["#!/bin/bash", "#!/bin/sh"] {
            let source = common_runtime_source(shebang);
            let model = model(&source);
            let unresolved = unresolved_names(&model);
            let uninitialized = uninitialized_names(&model);

            assert_names_absent(&names, &unresolved);
            assert_names_absent(&names, &uninitialized);
        }
    }

    #[test]
    fn bash_runtime_vars_are_not_marked_uninitialized_in_bash_scripts() {
        let source = bash_runtime_source("#!/bin/bash");
        let model = model(&source);
        let names = [
            "LINENO",
            "FUNCNAME",
            "BASH_SOURCE",
            "BASH_LINENO",
            "RANDOM",
            "BASH_REMATCH",
            "READLINE_LINE",
            "BASH_VERSION",
            "BASH_VERSINFO",
            "OSTYPE",
            "HISTCONTROL",
            "HISTSIZE",
        ];

        let unresolved = unresolved_names(&model);
        let uninitialized = uninitialized_names(&model);

        assert_names_absent(&names, &unresolved);
        assert_names_absent(&names, &uninitialized);
    }

    #[test]
    fn bash_runtime_vars_remain_unresolved_in_non_bash_scripts() {
        let source = bash_runtime_source("#!/bin/sh");
        let model = model(&source);
        let names = [
            "LINENO",
            "FUNCNAME",
            "BASH_SOURCE",
            "BASH_LINENO",
            "RANDOM",
            "BASH_REMATCH",
            "READLINE_LINE",
            "BASH_VERSION",
            "BASH_VERSINFO",
            "OSTYPE",
            "HISTCONTROL",
            "HISTSIZE",
        ];

        let unresolved = unresolved_names(&model);
        let uninitialized = uninitialized_names(&model);

        assert_names_present(&names, &unresolved);
        assert_names_present(&names, &uninitialized);
    }

    #[test]
    fn deferred_nested_function_bodies_resolve_later_outer_bindings() {
        let source = "\
outer() {
  inner() { echo $X; }
  X=1
  inner
}
outer
";
        let model = model(source);

        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.span.slice(source).trim(), "X");
        assert!(matches!(
            model.scope_kind(binding.scope),
            ScopeKind::Function(function) if function.contains_name_str("outer")
        ));
    }

    #[test]
    fn top_level_assignment_read_by_later_function_call_is_live() {
        let source = "\
show() { echo \"$flag\"; }
flag=1
show
";
        let model = model(source);

        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn sourced_helper_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn bash_source_file_suffix_reads_keep_top_level_assignment_live_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[0]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn bash_source_double_zero_suffix_reads_keep_top_level_assignment_live_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[00]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn bash_source_spaced_zero_suffix_reads_keep_top_level_assignment_live_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[ 0 ]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn bash_source_nonzero_suffix_does_not_keep_top_level_assignment_live_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("loader.bash__dep.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"${BASH_SOURCE[1]}__dep.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            !model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert_eq!(unused, vec!["flag"]);
    }

    #[test]
    fn bash_source_dirname_reads_keep_top_level_assignment_live_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn executed_helper_reads_keep_loop_variable_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
for queryip in 127.0.0.1; do
  helper.sh
done
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' \"$queryip\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model
                .synthetic_reads
                .iter()
                .any(|read| read.name == "queryip"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(
            !unused.contains(&Name::from("queryip")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn executed_helper_without_read_does_not_keep_unrelated_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
unused=1
helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "printf '%s\\n' ok\n").unwrap();

        let model = model_at_path(&main);

        let unused = reportable_unused_names(&model);
        assert!(
            unused.contains(&Name::from("unused")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn loader_function_source_reads_keep_top_level_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
load() { . \"$ROOT/$1\"; }
flag=1
load helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn source_path_resolver_keeps_helper_reads_generic() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("resolved/helper.sh");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let without_resolver = model_at_path(&main);
        let unused_without_resolver = reportable_unused_names(&without_resolver);
        assert!(
            unused_without_resolver.contains(&Name::from("flag")),
            "unused without resolver: {:?}",
            unused_without_resolver
        );

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, candidate: &str| {
            if source_path == main_path.as_path() && candidate == "./helper.sh" {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let with_resolver = model_at_path_with_resolver(&main, Some(&resolver));
        assert!(
            with_resolver
                .synthetic_reads
                .iter()
                .any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            with_resolver.synthetic_reads
        );
        let unused_with_resolver = reportable_unused_names(&with_resolver);
        assert!(
            !unused_with_resolver.contains(&Name::from("flag")),
            "unused with resolver: {:?}",
            unused_with_resolver
        );
    }

    #[test]
    fn missing_literal_source_is_marked_unresolved() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(&main, "#!/bin/sh\n. ./missing.sh\n").unwrap();

        let model = model_at_path(&main);

        assert_eq!(model.source_refs().len(), 1);
        assert_eq!(
            model.source_refs()[0].resolution,
            SourceRefResolution::Unresolved
        );
    }

    #[test]
    fn resolved_literal_source_is_marked_resolved() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(&main, "#!/bin/sh\n. ./helper.sh\n").unwrap();
        fs::write(&helper, "echo helper\n").unwrap();

        let model = model_at_path(&main);

        assert_eq!(model.source_refs().len(), 1);
        assert_eq!(
            model.source_refs()[0].resolution,
            SourceRefResolution::Resolved
        );
    }

    #[test]
    fn source_path_resolver_can_use_single_variable_static_tails() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("tests/main.sh");
        let helper = temp.path().join("scripts/rvm");
        fs::create_dir_all(main.parent().unwrap()).unwrap();
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
source \"$rvm_path/scripts/rvm\"
",
        )
        .unwrap();
        fs::write(&helper, "echo \"$flag\"\n").unwrap();

        let without_resolver = model_at_path(&main);
        let unused_without_resolver = reportable_unused_names(&without_resolver);
        assert!(
            unused_without_resolver.contains(&Name::from("flag")),
            "unused without resolver: {:?}",
            unused_without_resolver
        );

        let main_path = main.clone();
        let helper_path = helper.clone();
        let resolver = move |source_path: &Path, candidate: &str| {
            if source_path == main_path.as_path() && candidate == "scripts/rvm" {
                vec![helper_path.clone()]
            } else {
                Vec::new()
            }
        };

        let with_resolver = model_at_path_with_resolver(&main, Some(&resolver));
        assert!(
            with_resolver
                .synthetic_reads
                .iter()
                .any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            with_resolver.synthetic_reads
        );
        let unused_with_resolver = reportable_unused_names(&with_resolver);
        assert!(
            !unused_with_resolver.contains(&Name::from("flag")),
            "unused with resolver: {:?}",
            unused_with_resolver
        );
    }

    #[test]
    fn sourced_helper_exports_definite_imported_binding() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(&helper, "flag=1\n").unwrap();

        let model = model_at_path(&main);

        let imported = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "flag" && binding.kind == BindingKind::Imported)
            .unwrap();
        assert!(
            !imported
                .attributes
                .contains(BindingAttributes::IMPORTED_POSSIBLE)
        );
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn sourced_helper_exports_possible_imported_binding() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
if cond; then
  flag=1
fi
",
        )
        .unwrap();

        let model = model_at_path(&main);
        let imported_is_possible = model
            .bindings()
            .iter()
            .find(|binding| binding.name == "flag" && binding.kind == BindingKind::Imported)
            .map(|binding| {
                binding
                    .attributes
                    .contains(BindingAttributes::IMPORTED_POSSIBLE)
            })
            .unwrap();
        let details = uninitialized_details(&model);
        assert!(imported_is_possible, "uninitialized: {:?}", details);
        assert_eq!(
            details,
            vec![("flag".to_owned(), UninitializedCertainty::Possible)]
        );
    }

    #[test]
    fn sourced_helper_function_reads_do_not_keep_assignments_live_until_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
        )
        .unwrap();

        let model = model_at_path(&main);
        let unused = reportable_unused_names(&model);
        assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
    }

    #[test]
    fn quoted_heredoc_body_does_not_report_uninitialized_reads() {
        let source = "\
build=\"$(command cat <<\\END
printf '%s\\n' \"$workdir\"
END
)\"
";
        let model = model(source);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn escaped_dollar_heredoc_body_does_not_report_uninitialized_reads() {
        let source = "\
#!/bin/sh
cat <<EOF
\\${devtype} \\${devnum}
EOF
";
        let model = model(source);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn escaped_dollar_word_does_not_report_uninitialized_reads() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"\\$workdir\"
";
        let model = model(source);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn escaped_parameter_expansion_with_nested_default_stays_inert() {
        let source = "\
#!/bin/sh
printf '%s\\n' \\${workdir:-$fallback}
";
        let model = model(source);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn unquoted_heredoc_body_reports_live_uninitialized_reads() {
        let source = "\
archname=archive
cat <<EOF > \"$archname\"
#!/bin/sh
ORIG_UMASK=`umask`
if test \"$KEEP_UMASK\" = n; then
    umask 077
fi

CRCsum=\"$CRCsum\"
archdirname=\"$archdirname\"
EOF
";
        let model = model(source);
        let details = uninitialized_details(&model);

        assert!(
            details.iter().any(|(name, certainty)| name == "CRCsum"
                && *certainty == UninitializedCertainty::Definite)
        );
        assert!(details.iter().any(|(name, certainty)| name == "archdirname"
            && *certainty == UninitializedCertainty::Definite));
    }

    #[test]
    fn quoted_heredoc_source_text_does_not_keep_assignments_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
outdir=/tmp
build=\"$(command cat <<\\END
. \\\"$outdir\\\"/build.info
END
)\"
",
        )
        .unwrap();

        let model = model_at_path(&main);
        let unused = reportable_unused_names(&model);
        assert!(
            unused.contains(&Name::from("outdir")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn posix_quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
        )
        .unwrap();

        let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn posix_second_quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
usage=\"$(command cat <<\\END
Usage
END
)\"

build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
        )
        .unwrap();

        let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn quoted_heredoc_build_template_executed_later_stays_inert() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("build.info");
        fs::write(
            &main,
            "\
#!/bin/sh
usage=\"$(command cat <<\\END
Usage
END
)\"

build=\"$(command cat <<\\END
outdir=\"$(command pwd)\"
workdir=\"${TMPDIR:-/tmp}/gitstatus-build.tmp.$$\"\n\
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null 2>&1; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"

eval \"$build\"
",
        )
        .unwrap();
        fs::write(&helper, "libgit2_version=1.0\n").unwrap();

        let model = model_at_path(&main);
        let references = model.analysis().uninitialized_references().to_vec();
        let names = references
            .iter()
            .map(|reference| model.reference(reference.reference).name.clone())
            .collect::<Vec<_>>();
        assert!(
            !names.iter().any(|name| {
                matches!(
                    name.as_str(),
                    "formula" | "archflag" | "nopltflag" | "outdir"
                )
            }),
            "uninitialized names: {names:?}"
        );
    }

    #[test]
    fn escaped_dollar_heredoc_body_stays_inert_with_source_closure_enabled() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
cat <<EOF > ./postinst
if [ \"\\$1\" = \"configure\" ]; then
  for ver in 1 current; do
    for x in rewriteSystem rewriteURI; do
      xmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
    done
  done
fi
EOF
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn quoted_heredoc_case_arm_and_nested_same_name_heredoc_stay_inert() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
build=\"$(command cat <<\\END
case \"$gitstatus_kernel\" in
  linux)
    for formula in libiconv cmake git wget; do
      if command brew ls --version \"$formula\" >/dev/null; then
        command brew upgrade \"$formula\"
      else
        command brew install \"$formula\"
      fi
    done
  ;;
esac
command cat >&2 <<-END
\tSUCCESS
\tEND
END
)\"
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn tab_stripped_escaped_dollar_heredoc_body_stays_inert() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in 1 current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
\t\t\tdone
\t\tdone
\tfi
\tEOF
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(
            model.analysis().uninitialized_references().is_empty(),
            "uninitialized: {:?}",
            model.analysis().uninitialized_references()
        );
    }

    #[test]
    fn posix_tab_stripped_escaped_dollar_heredoc_body_stays_inert() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
",
        )
        .unwrap();

        let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
        let references = model.analysis().uninitialized_references().to_vec();
        let names = references
            .iter()
            .map(|reference| model.reference(reference.reference).name.clone())
            .collect::<Vec<_>>();
        assert!(
            !names
                .iter()
                .any(|name| matches!(name.as_str(), "x" | "ver")),
            "uninitialized names: {names:?}"
        );
    }

    #[test]
    fn posix_docbook_wrapper_does_not_treat_escaped_placeholders_as_reads() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
termux_step_create_debscripts() {
\tcat <<- EOF > ./postinst
\t#!$TERMUX_PREFIX/bin/sh
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://cdn.docbook.org/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-${TERMUX_PKG_VERSION}-nons\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
}
",
        )
        .unwrap();

        let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
        let references = model.analysis().uninitialized_references().to_vec();
        let names = references
            .iter()
            .map(|reference| model.reference(reference.reference).name.clone())
            .collect::<Vec<_>>();
        assert!(
            !names
                .iter()
                .any(|name| matches!(name.as_str(), "x" | "ver")),
            "uninitialized names: {names:?}"
        );
    }

    #[test]
    fn sourced_helper_function_reads_keep_assignments_live_when_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
flag=1
. ./helper.sh
use_flag
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
        )
        .unwrap();

        let model = model_at_path(&main);
        let unused = reportable_unused_names(&model);
        assert!(
            !unused.contains(&Name::from("flag")),
            "unused: {:?}",
            unused
        );
    }

    #[test]
    fn sourced_helper_function_exports_definite_imported_binding_when_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
set_flag
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
set_flag() {
  flag=1
}
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn sourced_helper_function_exports_possible_imported_binding_when_called() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./helper.sh
set_flag
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(
            &helper,
            "\
set_flag() {
  if cond; then
    flag=1
  fi
}
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert_eq!(
            uninitialized_details(&model),
            vec![("flag".to_owned(), UninitializedCertainty::Possible)]
        );
    }

    #[test]
    fn layered_source_closure_imports_function_contracts_transitively() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let loader = temp.path().join("loader.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./loader.sh
set_flag
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(&loader, ". ./helper.sh\n").unwrap();
        fs::write(
            &helper,
            "\
set_flag() {
  flag=1
}
",
        )
        .unwrap();

        let model = model_at_path(&main);
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn executed_helper_does_not_import_bindings_back_to_the_caller() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
./helper.sh
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(&helper, "flag=1\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model
                .bindings()
                .iter()
                .all(|binding| !(binding.name == "flag" && binding.kind == BindingKind::Imported))
        );
        assert_eq!(
            uninitialized_details(&model),
            vec![("flag".to_owned(), UninitializedCertainty::Definite)]
        );
    }

    #[test]
    fn imported_bindings_do_not_resolve_reads_before_the_import_site() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
printf '%s\\n' \"$flag\"
. ./helper.sh
",
        )
        .unwrap();
        fs::write(&helper, "flag=1\n").unwrap();

        let model = model_at_path(&main);
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == "flag")
            .unwrap();

        assert!(model.resolved_binding(reference.id).is_none());
        assert_eq!(
            uninitialized_details(&model),
            vec![("flag".to_owned(), UninitializedCertainty::Definite)]
        );
    }

    #[test]
    fn file_entry_contracts_seed_multiple_first_command_reads_as_imported_bindings() {
        let source = "printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$wrksrc\"\n";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                file_entry_contract: Some(FileContract {
                    required_reads: Vec::new(),
                    provided_bindings: vec![
                        ProvidedBinding::new(
                            Name::from("pkgname"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("pkgver"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("wrksrc"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                    ],
                    provided_functions: Vec::new(),
                }),
                ..SemanticBuildOptions::default()
            },
        );

        for name in ["pkgname", "pkgver", "wrksrc"] {
            let reference = model
                .references()
                .iter()
                .find(|reference| reference.name == name)
                .unwrap();
            let binding = model.resolved_binding(reference.id).unwrap();
            assert_eq!(binding.kind, BindingKind::Imported);
            assert_eq!(binding.name, name);
        }
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn file_entry_contracts_seed_deferred_function_body_reads_as_imported_bindings() {
        let source = "\
build() {
  printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$wrksrc\"
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                file_entry_contract: Some(FileContract {
                    required_reads: Vec::new(),
                    provided_bindings: vec![
                        ProvidedBinding::new(
                            Name::from("pkgname"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("pkgver"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("wrksrc"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                    ],
                    provided_functions: Vec::new(),
                }),
                ..SemanticBuildOptions::default()
            },
        );

        for name in ["pkgname", "pkgver", "wrksrc"] {
            let reference = model
                .references()
                .iter()
                .find(|reference| {
                    reference.name == name && reference.kind == ReferenceKind::Expansion
                })
                .unwrap();
            let binding = model.resolved_binding(reference.id).unwrap();
            assert_eq!(binding.kind, BindingKind::Imported);
            assert_eq!(binding.name, name);
        }
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn file_entry_contracts_seed_nested_function_regions_as_imported_bindings() {
        let source = "\
hook() {
  for f in ${pycompile_dirs}; do
    if [ \"${pkgname}\" = \"base-files\" ]; then
      echo \"python${pycompile_version}\"
    else
      printf '%s\\n' \"${pkgver}: ${f}\"
    fi
  done
}
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                file_entry_contract: Some(FileContract {
                    required_reads: Vec::new(),
                    provided_bindings: vec![
                        ProvidedBinding::new(
                            Name::from("pkgname"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("pkgver"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("pycompile_dirs"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                        ProvidedBinding::new(
                            Name::from("pycompile_version"),
                            ProvidedBindingKind::Variable,
                            ContractCertainty::Definite,
                        ),
                    ],
                    provided_functions: Vec::new(),
                }),
                ..SemanticBuildOptions::default()
            },
        );

        for name in ["pkgname", "pkgver", "pycompile_dirs", "pycompile_version"] {
            let reference = model
                .references()
                .iter()
                .find(|reference| reference.name == name)
                .unwrap();
            let binding = model.resolved_binding(reference.id).unwrap();
            assert_eq!(binding.kind, BindingKind::Imported);
            assert_eq!(binding.name, name);
        }
        assert!(model.analysis().uninitialized_references().is_empty());
    }

    #[test]
    fn cyclic_source_closure_does_not_invent_bindings() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let a = temp.path().join("a.sh");
        let b = temp.path().join("b.sh");
        fs::write(
            &main,
            "\
#!/bin/sh
. ./a.sh
printf '%s\\n' \"$flag\"
",
        )
        .unwrap();
        fs::write(&a, ". ./b.sh\n").unwrap();
        fs::write(&b, ". ./a.sh\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model
                .bindings()
                .iter()
                .all(|binding| !(binding.name == "flag" && binding.kind == BindingKind::Imported))
        );
        assert_eq!(
            uninitialized_details(&model),
            vec![("flag".to_owned(), UninitializedCertainty::Definite)]
        );
    }

    #[test]
    fn unsupported_bash_source_alias_fallback_does_not_keep_assignment_live() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
SELF=\"${BASH_SOURCE}\"
source \"$(dirname \"${SELF:-$0}\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            !model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
    }

    #[test]
    fn escaped_bash_source_template_does_not_import_helper() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source \"\\$(dirname \\\"${BASH_SOURCE[0]}\\\")/helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            !model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
    }

    #[test]
    fn shellcheck_source_directive_overrides_bash_source_template() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.bash");
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("alt-helper.bash");
        fs::write(
            &main,
            "\
#!/bin/bash
flag=1
source ./loader.bash
",
        )
        .unwrap();
        fs::write(
            &loader,
            "\
#!/bin/bash
# shellcheck source=alt-helper.bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/missing-helper.bash\"
",
        )
        .unwrap();
        fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

        let model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        assert_eq!(
            model.source_refs()[0].resolution,
            SourceRefResolution::Resolved
        );
        let unused = reportable_unused_names(&model);
        assert!(unused.is_empty(), "unused: {:?}", unused);
    }

    #[test]
    fn precise_unused_assignments_match_dataflow_for_source_closure_cases() {
        let temp = tempdir().unwrap();

        let sourced_main = temp.path().join("sourced-main.sh");
        let sourced_helper = temp.path().join("sourced-helper.sh");
        fs::write(
            &sourced_main,
            "\
#!/bin/sh
flag=1
. ./sourced-helper.sh
",
        )
        .unwrap();
        fs::write(&sourced_helper, "echo \"$flag\"\n").unwrap();

        let executed_main = temp.path().join("executed-main.sh");
        let executed_helper = temp.path().join("executed-helper.sh");
        fs::write(
            &executed_main,
            "\
#!/bin/sh
unused=1
executed-helper.sh
",
        )
        .unwrap();
        fs::write(&executed_helper, "printf '%s\\n' ok\n").unwrap();

        let sourced_model = model_at_path(&sourced_main);
        assert_unused_assignment_parity(&sourced_model);

        let executed_model = model_at_path(&executed_main);
        assert_unused_assignment_parity(&executed_model);
    }

    #[test]
    fn non_arithmetic_subscript_reads_are_recorded_in_conditionals_and_declarations() {
        let source = "\
#!/bin/bash
[[ -v assoc[\"$key\"] ]]
declare -A map=([\"$other\"]=1)
";
        let model = model(source);
        let unresolved = unresolved_names(&model);

        assert_names_present(&["key", "other"], &unresolved);

        let conditional_reference = model
            .references()
            .iter()
            .find(|reference| reference.name == "key")
            .expect("expected conditional subscript reference");
        assert_eq!(
            conditional_reference.kind,
            ReferenceKind::ConditionalOperand
        );

        let declaration_reference = model
            .references()
            .iter()
            .find(|reference| reference.name == "other")
            .expect("expected declaration subscript reference");
        assert_eq!(declaration_reference.kind, ReferenceKind::Expansion);
    }

    #[test]
    fn associative_subscript_literals_do_not_register_variable_reads() {
        let source = "\
#!/bin/bash
declare -A map
map[swift-cmark]=1
printf '%s\\n' \"${map[swift-cmark]}\" \"${map[$dynamic_key]}\"
";
        let model = model(source);
        let unresolved = unresolved_names(&model);

        assert_names_absent(&["swift", "cmark"], &unresolved);
        assert_names_present(&["dynamic_key"], &unresolved);
        assert!(
            model
                .bindings()
                .iter()
                .rev()
                .find(|binding| binding.name == "map")
                .is_some_and(|binding| binding.attributes.contains(BindingAttributes::ASSOC))
        );
    }

    #[test]
    fn escaped_parameter_replacement_patterns_do_not_register_variable_reads() {
        let source = "\
#!/bin/bash
d=lib
origin=/tmp
echo \"${d//\\$ORIGIN/$origin}\"
";
        let model = model(source);
        let unresolved = unresolved_names(&model);

        assert!(
            unresolved.is_empty(),
            "unexpected unresolved refs: {unresolved:?}"
        );
    }

    #[test]
    fn recorded_program_and_cfg_capture_non_arithmetic_var_ref_nested_regions() {
        let source = "\
[[ -v assoc[\"$(printf inner)\"] ]]
echo done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);

        let file_commands = model
            .recorded_program
            .commands_in(model.recorded_program.file_commands());
        assert_eq!(file_commands.len(), 2);
        let conditional = model.recorded_program.command(file_commands[0]);
        let nested_regions = model
            .recorded_program
            .nested_regions(conditional.nested_regions);
        assert_eq!(nested_regions.len(), 1);
        let nested_commands = model
            .recorded_program
            .commands_in(nested_regions[0].commands);
        assert_eq!(nested_commands.len(), 1);
        let nested = model.recorded_program.command(nested_commands[0]);
        assert_eq!(nested.span.slice(source), "printf inner");

        let cfg = build_control_flow_graph(
            &model.recorded_program,
            &model.command_bindings,
            &model.command_references,
        );

        assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
        assert!(!cfg.block_ids_for_span(nested.span).is_empty());
        assert!(
            cfg.blocks()
                .iter()
                .flat_map(|block| block.commands.iter())
                .any(|span| span.slice(source) == "printf inner")
        );
    }

    #[test]
    fn recorded_program_and_cfg_capture_arithmetic_var_ref_nested_regions() {
        let source = "\
[[ -v assoc[$(( $(printf inner) + 1 ))] ]]
echo done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);

        let file_commands = model
            .recorded_program
            .commands_in(model.recorded_program.file_commands());
        assert_eq!(file_commands.len(), 2);
        let conditional = model.recorded_program.command(file_commands[0]);
        let nested_regions = model
            .recorded_program
            .nested_regions(conditional.nested_regions);
        assert_eq!(nested_regions.len(), 1);
        let nested_commands = model
            .recorded_program
            .commands_in(nested_regions[0].commands);
        assert_eq!(nested_commands.len(), 1);
        let nested = model.recorded_program.command(nested_commands[0]);
        assert_eq!(nested.span.slice(source), "printf inner");

        let cfg = build_control_flow_graph(
            &model.recorded_program,
            &model.command_bindings,
            &model.command_references,
        );

        assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
        assert!(!cfg.block_ids_for_span(nested.span).is_empty());
        assert!(
            cfg.blocks()
                .iter()
                .flat_map(|block| block.commands.iter())
                .any(|span| span.slice(source) == "printf inner")
        );
    }

    #[test]
    fn zsh_option_analysis_exposes_native_defaults() {
        let source = "print $name\n";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.sh_word_split, OptionValue::Off);
        assert_eq!(options.glob, OptionValue::On);
        assert_eq!(options.short_loops, OptionValue::On);
    }

    #[test]
    fn zsh_option_analysis_tracks_setopt_updates_by_offset() {
        let source = "setopt no_glob\nprint *\n";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.glob, OptionValue::Off);
    }

    #[test]
    fn zsh_option_analysis_merges_conditionals_to_unknown_on_divergence() {
        let source = "if test \"$x\" = y; then\n  setopt no_glob\nfi\nprint *\n";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.glob, OptionValue::Unknown);
    }

    #[test]
    fn zsh_option_analysis_respects_local_options_in_functions() {
        let source = "\
fn() {
  setopt local_options no_glob
}
fn
print *
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.glob, OptionValue::On);
    }

    #[test]
    fn zsh_option_analysis_applies_top_level_local_options_to_function_leaks() {
        let source = "\
setopt localoptions
fn() {
  setopt no_glob
}
fn
print *
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.glob, OptionValue::On);
    }

    #[test]
    fn zsh_option_analysis_leaks_function_option_updates_by_default() {
        let source = "\
fn() {
  setopt sh_word_split
}
fn
print $name
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        assert!(
            model.scopes[0]
                .bindings
                .keys()
                .any(|name| name.as_str() == "fn"),
            "expected top-level function binding for `fn`"
        );
        assert!(
            model.recorded_program.function_body_scopes.len() == 1,
            "expected one recorded function body scope"
        );
        assert!(
            model
                .recorded_program
                .command_infos
                .values()
                .any(|info| info.static_callee.as_deref() == Some("fn")),
            "expected a static callee for the function call"
        );
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected zsh options");

        assert_eq!(options.sh_word_split, OptionValue::On);
    }

    #[test]
    fn zsh_option_analysis_falls_back_to_ancestor_state_in_uncalled_function_bodies() {
        let source = "\
fn() {
  print $name
}
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected inherited zsh options");

        assert_eq!(options.sh_word_split, OptionValue::Off);
        assert_eq!(options.glob, OptionValue::On);
    }

    #[test]
    fn zsh_option_analysis_merges_function_snapshots_from_multiple_call_contexts() {
        let source = "\
fn() {
  print $name
}
fn
setopt sh_word_split
fn
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected merged function zsh options");

        assert_eq!(options.sh_word_split, OptionValue::Unknown);
    }

    #[test]
    fn zsh_option_analysis_tracks_wrapped_option_builtins() {
        let source = "\
command setopt no_glob
builtin unsetopt short_loops
print *
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh option effects");

        assert_eq!(options.glob, OptionValue::Off);
        assert_eq!(options.short_loops, OptionValue::Off);
    }

    #[test]
    fn zsh_option_analysis_ignores_command_lookup_modes() {
        let source = "\
command -v setopt no_glob
print *
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh options");

        assert_eq!(options.glob, OptionValue::On);
    }
}
