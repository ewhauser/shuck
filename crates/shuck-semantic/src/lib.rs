mod binding;
mod builder;
mod call_graph;
mod cfg;
mod dataflow;
mod declaration;
mod reference;
mod runtime;
mod scope;
mod source_closure;
mod source_ref;

pub use binding::{Binding, BindingAttributes, BindingId, BindingKind};
pub use call_graph::{CallGraph, CallSite, OverwrittenFunction};
pub use cfg::{BasicBlock, BlockId, ControlFlowGraph, EdgeKind, FlowContext};
pub use dataflow::{
    DeadCode, ReachingDefinitions, UninitializedCertainty, UninitializedReference,
    UnusedAssignment, UnusedReason,
};
pub use declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
pub use reference::{Reference, ReferenceId, ReferenceKind};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use source_ref::{SourceRef, SourceRefKind};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, Name, Script, Span};
use shuck_indexer::Indexer;
use std::path::{Path, PathBuf};

use crate::builder::SemanticModelBuilder;
use crate::cfg::{RecordedProgram, build_control_flow_graph};
use crate::dataflow::DataflowResult;
use crate::runtime::RuntimePrelude;

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

#[derive(Debug, Clone)]
pub struct SemanticModel {
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    predefined_runtime_refs: FxHashSet<ReferenceId>,
    binding_index: FxHashMap<Name, Vec<BindingId>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, Vec<BindingId>>,
    call_sites: FxHashMap<Name, Vec<CallSite>>,
    call_graph: CallGraph,
    source_refs: Vec<SourceRef>,
    runtime: RuntimePrelude,
    declarations: Vec<Declaration>,
    indirect_targets_by_binding: Vec<Vec<BindingId>>,
    indirect_targets_by_reference: Vec<Vec<BindingId>>,
    synthetic_reads: Vec<SyntheticRead>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    cfg: Option<ControlFlowGraph>,
    dataflow: Option<DataflowResult>,
    precise_unused_assignments: Option<Vec<BindingId>>,
    precise_uninitialized_references: Option<Vec<UninitializedReference>>,
    precise_dead_code: Option<Vec<DeadCode>>,
    heuristic_unused_assignments: Vec<BindingId>,
}

impl SemanticModel {
    pub fn build(script: &Script, source: &str, indexer: &Indexer) -> Self {
        let mut observer = NoopTraversalObserver;
        build_with_observer(script, source, indexer, &mut observer)
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
        Self {
            scopes: built.scopes,
            bindings: built.bindings,
            references: built.references,
            predefined_runtime_refs: built.predefined_runtime_refs,
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
            flow_contexts: built.flow_contexts,
            recorded_program: built.recorded_program,
            command_bindings: built.command_bindings,
            command_references: built.command_references,
            cfg: None,
            dataflow: None,
            precise_unused_assignments: None,
            precise_uninitialized_references: None,
            precise_dead_code: None,
            heuristic_unused_assignments: built.heuristic_unused_assignments,
        }
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

    pub fn indirect_targets_for_binding(&self, id: BindingId) -> &[BindingId] {
        self.indirect_targets_by_binding
            .get(id.index())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn indirect_targets_for_reference(&self, id: ReferenceId) -> &[BindingId] {
        self.indirect_targets_by_reference
            .get(id.index())
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

    pub fn unused_assignments(&self) -> &[BindingId] {
        self.dataflow
            .as_ref()
            .map(DataflowResult::unused_assignment_ids)
            .or(self.precise_unused_assignments.as_deref())
            .unwrap_or(&self.heuristic_unused_assignments)
    }

    pub fn uninitialized_references(&self) -> &[UninitializedReference] {
        self.dataflow
            .as_ref()
            .map(|dataflow| dataflow.uninitialized_references.as_slice())
            .or(self.precise_uninitialized_references.as_deref())
            .unwrap_or(&[])
    }

    fn needs_precise_unused_assignments(&self) -> bool {
        if self.heuristic_unused_assignments.is_empty() {
            return false;
        }

        if !self.synthetic_reads.is_empty()
            || self
                .indirect_targets_by_reference
                .iter()
                .any(|targets| !targets.is_empty())
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

    pub fn unresolved_references(&self) -> &[ReferenceId] {
        &self.unresolved
    }

    pub fn scope_at(&self, offset: usize) -> ScopeId {
        self.scopes
            .iter()
            .filter(|scope| contains_offset(scope.span, offset))
            .min_by_key(|scope| scope.span.end.offset - scope.span.start.offset)
            .map(|scope| scope.id)
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

    pub(crate) fn bash_runtime_vars_enabled(&self) -> bool {
        self.runtime.bash_enabled()
    }

    pub fn cfg(&mut self) -> &ControlFlowGraph {
        if self.cfg.is_none() {
            self.cfg = Some(build_control_flow_graph(
                &self.recorded_program,
                &self.command_bindings,
                &self.command_references,
            ));
        }
        self.cfg.as_ref().unwrap()
    }

    #[allow(dead_code)]
    fn dataflow(&mut self) -> &DataflowResult {
        if self.dataflow.is_none() {
            if self.cfg.is_none() {
                self.cfg = Some(build_control_flow_graph(
                    &self.recorded_program,
                    &self.command_bindings,
                    &self.command_references,
                ));
            }
            let result = {
                let cfg = self.cfg.as_ref().unwrap();
                dataflow::analyze(
                    cfg,
                    &self.runtime,
                    &self.scopes,
                    &self.bindings,
                    &self.references,
                    &self.predefined_runtime_refs,
                    &self.resolved,
                    &self.call_sites,
                    &self.indirect_targets_by_reference,
                    &self.synthetic_reads,
                )
            };
            self.dataflow = Some(result);
        }
        self.dataflow.as_ref().unwrap()
    }

    pub fn precompute_unused_assignments(&mut self) -> &[BindingId] {
        if self.precise_unused_assignments.is_none() {
            if !self.needs_precise_unused_assignments() {
                self.precise_unused_assignments = Some(self.heuristic_unused_assignments.clone());
                return self.precise_unused_assignments.as_deref().unwrap();
            }
            if self.cfg.is_none() {
                self.cfg = Some(build_control_flow_graph(
                    &self.recorded_program,
                    &self.command_bindings,
                    &self.command_references,
                ));
            }
            let cfg = self.cfg.as_ref().unwrap();
            self.precise_unused_assignments = Some(dataflow::analyze_unused_assignments(
                cfg,
                &self.runtime,
                &self.scopes,
                &self.bindings,
                &self.references,
                &self.resolved,
                &self.call_sites,
                &self.indirect_targets_by_reference,
                &self.synthetic_reads,
            ));
        }
        self.precise_unused_assignments.as_deref().unwrap()
    }

    pub fn precompute_uninitialized_references(&mut self) -> &[UninitializedReference] {
        if self.precise_uninitialized_references.is_none() {
            if self.cfg.is_none() {
                self.cfg = Some(build_control_flow_graph(
                    &self.recorded_program,
                    &self.command_bindings,
                    &self.command_references,
                ));
            }
            let cfg = self.cfg.as_ref().unwrap();
            self.precise_uninitialized_references =
                Some(dataflow::analyze_uninitialized_references(
                    cfg,
                    &self.bindings,
                    &self.references,
                    &self.predefined_runtime_refs,
                    &self.resolved,
                    &self.indirect_targets_by_reference,
                ));
        }
        self.precise_uninitialized_references.as_deref().unwrap()
    }

    pub fn precompute_dead_code(&mut self) -> &[DeadCode] {
        if self.precise_dead_code.is_none() {
            if self.cfg.is_none() {
                self.cfg = Some(build_control_flow_graph(
                    &self.recorded_program,
                    &self.command_bindings,
                    &self.command_references,
                ));
            }
            let cfg = self.cfg.as_ref().unwrap();
            self.precise_dead_code = Some(dataflow::analyze_dead_code(cfg));
        }
        self.precise_dead_code.as_deref().unwrap()
    }

    pub(crate) fn set_synthetic_reads(&mut self, synthetic_reads: Vec<SyntheticRead>) {
        self.synthetic_reads = synthetic_reads;
        self.dataflow = None;
        self.precise_unused_assignments = None;
        self.precise_uninitialized_references = None;
        self.precise_dead_code = None;
    }

    pub fn is_reachable(&mut self, span: &Span) -> bool {
        let cfg = self.cfg();
        cfg.block_ids_for_span(*span)
            .iter()
            .all(|block| !cfg.unreachable().contains(block))
    }

    pub fn dead_code(&self) -> &[DeadCode] {
        self.dataflow
            .as_ref()
            .map(|dataflow| dataflow.dead_code.as_slice())
            .or(self.precise_dead_code.as_deref())
            .unwrap_or(&[])
    }
}

#[doc(hidden)]
pub fn build_with_observer(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
) -> SemanticModel {
    build_semantic_model(script, source, indexer, observer, None, false, None)
}

#[doc(hidden)]
pub fn build_with_observer_at_path(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
) -> SemanticModel {
    build_with_observer_at_path_with_resolver(script, source, indexer, observer, source_path, None)
}

#[doc(hidden)]
pub fn build_with_observer_at_path_with_resolver(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> SemanticModel {
    build_semantic_model(
        script,
        source,
        indexer,
        observer,
        source_path,
        true,
        source_path_resolver,
    )
}

fn build_semantic_model(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
    include_source_closure: bool,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> SemanticModel {
    let built = SemanticModelBuilder::build(
        script,
        source,
        indexer,
        observer,
        bash_runtime_vars_enabled(source, source_path),
    );
    let mut model = SemanticModel::from_build_output(built);
    if include_source_closure && let Some(source_path) = source_path {
        let synthetic_reads = source_closure::collect_source_closure_reads(
            &model,
            script,
            source,
            source_path,
            source_path_resolver,
        );
        model.set_synthetic_reads(synthetic_reads);
    }
    model
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

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn build_indirect_targets_by_binding(
    bindings: &[Binding],
    indirect_target_hints: &FxHashMap<BindingId, IndirectTargetHint>,
) -> Vec<Vec<BindingId>> {
    let mut targets_by_binding = vec![Vec::new(); bindings.len()];
    for (binding_id, hint) in indirect_target_hints {
        let targets = bindings
            .iter()
            .filter(|binding| indirect_target_matches(hint, binding))
            .map(|binding| binding.id)
            .collect::<Vec<_>>();
        targets_by_binding[binding_id.index()] = targets;
    }
    targets_by_binding
}

fn build_indirect_targets_by_reference(
    references: &[Reference],
    resolved: &FxHashMap<ReferenceId, BindingId>,
    indirect_expansion_refs: &FxHashSet<ReferenceId>,
    indirect_targets_by_binding: &[Vec<BindingId>],
) -> Vec<Vec<BindingId>> {
    let mut targets_by_reference = vec![Vec::new(); references.len()];
    for reference in references {
        if !indirect_expansion_refs.contains(&reference.id) {
            continue;
        }
        let Some(binding_id) = resolved.get(&reference.id).copied() else {
            continue;
        };
        targets_by_reference[reference.id.index()] =
            indirect_targets_by_binding[binding_id.index()].clone();
    }
    targets_by_reference
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
    use crate::cfg::build_control_flow_graph;
    use shuck_ast::{Command, CompoundCommand};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn model(source: &str) -> SemanticModel {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        SemanticModel::build(&output.script, source, &indexer)
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
            &output.script,
            &source,
            &indexer,
            &mut observer,
            Some(path),
            source_path_resolver,
        )
    }

    fn reportable_unused_names(model: &mut SemanticModel) -> Vec<Name> {
        let _ = model.precompute_unused_assignments();
        model
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

    fn assert_unused_assignment_parity(model: &mut SemanticModel) {
        let precise = model.precompute_unused_assignments().to_vec();
        let exact = model.dataflow().unused_assignment_ids().to_vec();
        assert_eq!(precise, exact);
    }

    fn assert_uninitialized_reference_parity(model: &mut SemanticModel) {
        let precise = model.precompute_uninitialized_references().to_vec();
        let exact = model.dataflow().uninitialized_references.clone();
        assert_eq!(precise, exact);
    }

    fn assert_dead_code_parity(model: &mut SemanticModel) {
        let precise = model.precompute_dead_code().to_vec();
        let exact = model.dataflow().dead_code.clone();
        assert_eq!(precise, exact);
    }

    fn binding_names(model: &SemanticModel, ids: &[BindingId]) -> Vec<String> {
        ids.iter()
            .map(|binding_id| model.binding(*binding_id).name.to_string())
            .collect()
    }

    fn unresolved_names(model: &SemanticModel) -> Vec<String> {
        model
            .unresolved_references()
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
            .collect()
    }

    fn uninitialized_names(model: &mut SemanticModel) -> Vec<String> {
        let references = model
            .precompute_uninitialized_references()
            .iter()
            .map(|reference| reference.reference)
            .collect::<Vec<_>>();
        references
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
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
            "{shebang}\nprintf '%s\\n' \"$IFS\" \"$USER\" \"$HOME\" \"$SHELL\" \"$PWD\" \"$TERM\" \"$LANG\" \"$SUDO_USER\" \"$DOAS_USER\"\n"
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
        assert!(
            model
                .scopes()
                .iter()
                .any(|scope| matches!(&scope.kind, ScopeKind::Function(name) if name == "f"))
        );

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
            ScopeKind::Function(name) if name == "f"
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
        let model = SemanticModel::build(&output.script, source, &indexer);

        let Command::Compound(CompoundCommand::If(if_command), _) = &output.script.commands[0]
        else {
            panic!("expected if command");
        };
        let condition_span = match &if_command.condition[0] {
            Command::Simple(command) => command.span,
            other => panic!("unexpected condition command: {other:?}"),
        };
        let condition_context = model.flow_context_at(&condition_span).unwrap();
        assert!(condition_context.exit_status_checked);

        let Command::Compound(CompoundCommand::For(for_command), _) = &output.script.commands[1]
        else {
            panic!("expected for command");
        };
        let break_span = match &for_command.body[0] {
            Command::Builtin(shuck_ast::BuiltinCommand::Break(command)) => command.span,
            other => panic!("unexpected loop body command: {other:?}"),
        };
        let break_context = model.flow_context_at(&break_span).unwrap();
        assert_eq!(break_context.loop_depth, 1);
    }

    #[test]
    fn detects_overwritten_assignments_and_possible_uninitialized_reads() {
        let overwritten_source = "VAR=x\nVAR=y\necho $VAR\n";
        let mut overwritten = model(overwritten_source);
        let dataflow = overwritten.dataflow();
        assert_eq!(dataflow.unused_assignments.len(), 1);
        assert!(matches!(
            dataflow.unused_assignments[0].reason,
            UnusedReason::Overwritten { .. }
        ));

        let partial_source = "if cond; then VAR=x; fi\necho $VAR\n";
        let mut partial = model(partial_source);
        let dataflow = partial.dataflow();
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
            "#!/bin/sh\nprintf '%s\\n' \"$HOME\"\n",
            "#!/bin/bash\nprintf '%s\\n' \"$RANDOM\"\n",
        ];

        for source in cases {
            let mut model = model(source);
            assert_uninitialized_reference_parity(&mut model);
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
            let mut model = model(source);
            assert_dead_code_parity(&mut model);
        }
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
            let mut model = model(source);
            assert_unused_assignment_parity(&mut model);
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
        let mut model = model(source);
        let dataflow = model.dataflow();

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
        let mut model = model(source);
        let all_bindings = model.bindings_for(&Name::from("code_command")).to_vec();
        let binding_ids = model.dataflow().unused_assignment_ids().to_vec();

        assert_eq!(model.dataflow().unused_assignments.len(), 2);
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
        let mut model = model(source);
        let all_bindings = model.bindings_for(&Name::from("VAR")).to_vec();
        let binding_ids = model.dataflow().unused_assignment_ids().to_vec();

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
        let mut model = model(source);
        let unused_bindings = model
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
        let mut model = model(source);
        let unused_bindings = model
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
        let mut model = model(source);
        let unused_bindings = model
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
        let mut model = model(source);
        let unused_bindings = model
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
        let mut model = model(source);

        let unused = reportable_unused_names(&mut model);
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
        let mut model = model(source);

        let unused = reportable_unused_names(&mut model);
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
        let mut model = model(source);
        let unused_bindings = model
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
        let mut model = model(source);

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

        let uninitialized = model.precompute_uninitialized_references();
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
        let mut model = model(source);
        let unused = reportable_unused_names(&mut model);

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
        let mut model = model(source);
        model.dataflow();

        let unused = model
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
        let mut model = model(source);
        model.dataflow();

        let unused = model
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
        let mut model = model(source);
        model.dataflow();

        assert!(model.references().iter().any(|reference| {
            reference.name == "IFS" && matches!(reference.kind, ReferenceKind::ImplicitRead)
        }));

        let unused = model
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
        let mut model = model(source);
        model.dataflow();

        let unused = model
            .unused_assignments()
            .iter()
            .map(|binding| model.binding(*binding).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unused.contains(&"IFS"));
        assert!(unused.contains(&"unused"));
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
        let mut model = model(source);
        model.dataflow();

        let unused = model
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
        let mut model = model(source);
        assert!(uninitialized_names(&mut model).is_empty());
    }

    #[test]
    fn detects_dead_code_after_exit() {
        let source = "exit 0\necho dead\n";
        let mut model = model(source);
        let dead_code = model.precompute_dead_code();
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
            "LANG",
            "SUDO_USER",
            "DOAS_USER",
        ];

        for shebang in ["#!/bin/bash", "#!/bin/sh"] {
            let source = common_runtime_source(shebang);
            let mut model = model(&source);
            let unresolved = unresolved_names(&model);
            let uninitialized = uninitialized_names(&mut model);

            assert_names_absent(&names, &unresolved);
            assert_names_absent(&names, &uninitialized);
        }
    }

    #[test]
    fn bash_runtime_vars_are_not_marked_uninitialized_in_bash_scripts() {
        let source = bash_runtime_source("#!/bin/bash");
        let mut model = model(&source);
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
        let uninitialized = uninitialized_names(&mut model);

        assert_names_absent(&names, &unresolved);
        assert_names_absent(&names, &uninitialized);
    }

    #[test]
    fn bash_runtime_vars_remain_unresolved_in_non_bash_scripts() {
        let source = bash_runtime_source("#!/bin/sh");
        let mut model = model(&source);
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
        let uninitialized = uninitialized_names(&mut model);

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
            ScopeKind::Function(name) if name == "outer"
        ));
    }

    #[test]
    fn top_level_assignment_read_by_later_function_call_is_live() {
        let source = "\
show() { echo \"$flag\"; }
flag=1
show
";
        let mut model = model(source);

        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            !model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model
                .synthetic_reads
                .iter()
                .any(|read| read.name == "queryip"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut without_resolver = model_at_path(&main);
        let unused_without_resolver = reportable_unused_names(&mut without_resolver);
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

        let mut with_resolver = model_at_path_with_resolver(&main, Some(&resolver));
        assert!(
            with_resolver
                .synthetic_reads
                .iter()
                .any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            with_resolver.synthetic_reads
        );
        let unused_with_resolver = reportable_unused_names(&mut with_resolver);
        assert!(
            !unused_with_resolver.contains(&Name::from("flag")),
            "unused with resolver: {:?}",
            unused_with_resolver
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

        let mut model = model_at_path(&main);

        assert!(
            !model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut model = model_at_path(&main);

        assert!(
            model.synthetic_reads.iter().any(|read| read.name == "flag"),
            "synthetic reads: {:?}",
            model.synthetic_reads
        );
        let unused = reportable_unused_names(&mut model);
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

        let mut sourced_model = model_at_path(&sourced_main);
        assert_unused_assignment_parity(&mut sourced_model);

        let mut executed_model = model_at_path(&executed_main);
        assert_unused_assignment_parity(&mut executed_model);
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
        assert_eq!(conditional_reference.kind, ReferenceKind::ConditionalOperand);

        let declaration_reference = model
            .references()
            .iter()
            .find(|reference| reference.name == "other")
            .expect("expected declaration subscript reference");
        assert_eq!(declaration_reference.kind, ReferenceKind::Expansion);
    }

    #[test]
    fn recorded_program_and_cfg_capture_non_arithmetic_var_ref_nested_regions() {
        let source = "\
[[ -v assoc[\"$(printf inner)\"] ]]
echo done
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.script, source, &indexer);

        assert_eq!(model.recorded_program.file_commands.len(), 2);
        let conditional = &model.recorded_program.file_commands[0];
        assert_eq!(conditional.nested_regions.len(), 1);
        assert_eq!(conditional.nested_regions[0].commands.len(), 1);
        let nested = &conditional.nested_regions[0].commands[0];
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
        let model = SemanticModel::build(&output.script, source, &indexer);

        assert_eq!(model.recorded_program.file_commands.len(), 2);
        let conditional = &model.recorded_program.file_commands[0];
        assert_eq!(conditional.nested_regions.len(), 1);
        assert_eq!(conditional.nested_regions[0].commands.len(), 1);
        let nested = &conditional.nested_regions[0].commands[0];
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
}
