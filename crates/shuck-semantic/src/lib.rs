mod binding;
mod builder;
mod call_graph;
mod cfg;
mod dataflow;
mod declaration;
mod reference;
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
use std::path::Path;

use crate::builder::SemanticModelBuilder;
use crate::cfg::{RecordedProgram, build_control_flow_graph};
use crate::dataflow::DataflowResult;

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
    bash_runtime_vars_enabled: bool,
    declarations: Vec<Declaration>,
    indirect_target_hints: FxHashMap<BindingId, IndirectTargetHint>,
    indirect_expansion_refs: FxHashSet<ReferenceId>,
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
            bash_runtime_vars_enabled: built.bash_runtime_vars_enabled,
            declarations: built.declarations,
            indirect_target_hints: built.indirect_target_hints,
            indirect_expansion_refs: built.indirect_expansion_refs,
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

        if !self.synthetic_reads.is_empty() || !self.indirect_expansion_refs.is_empty() {
            return true;
        }

        let has_call_sites = !self.call_sites.is_empty();
        self.heuristic_unused_assignments.iter().any(|binding_id| {
            let binding = &self.bindings[binding_id.index()];
            binding.name == "IFS"
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
        self.bash_runtime_vars_enabled
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
                    &self.scopes,
                    &self.bindings,
                    &self.references,
                    &self.predefined_runtime_refs,
                    &self.resolved,
                    &self.call_sites,
                    &self.indirect_target_hints,
                    &self.indirect_expansion_refs,
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
                &self.scopes,
                &self.bindings,
                &self.references,
                &self.resolved,
                &self.call_sites,
                &self.indirect_target_hints,
                &self.indirect_expansion_refs,
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
    build_semantic_model(script, source, indexer, observer, None, false)
}

#[doc(hidden)]
pub fn build_with_observer_at_path(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
) -> SemanticModel {
    build_semantic_model(script, source, indexer, observer, source_path, true)
}

fn build_semantic_model(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
    source_path: Option<&Path>,
    include_source_closure: bool,
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
        let synthetic_reads =
            source_closure::collect_source_closure_reads(&model, script, source, source_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::{RecordedProgram, build_control_flow_graph};
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
        let source = fs::read_to_string(path).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let mut observer = NoopTraversalObserver;
        build_with_observer_at_path(&output.script, &source, &indexer, &mut observer, Some(path))
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
                        | BindingKind::AppendAssignment
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
    fn bash_runtime_vars_are_not_marked_uninitialized() {
        let source = "\
#!/bin/bash
printf '%s %s %s %s\\n' \"$LINENO\" \"$FUNCNAME\" \"${BASH_SOURCE[0]}\" \"${BASH_LINENO[0]}\"
";
        let mut model = model(source);

        let unresolved = model
            .unresolved_references()
            .iter()
            .map(|reference| model.reference(*reference).name.as_str())
            .collect::<Vec<_>>();
        assert!(!unresolved.contains(&"LINENO"));
        assert!(!unresolved.contains(&"FUNCNAME"));
        assert!(!unresolved.contains(&"BASH_SOURCE"));
        assert!(!unresolved.contains(&"BASH_LINENO"));

        let uninitialized_ids = model
            .precompute_uninitialized_references()
            .iter()
            .map(|reference| reference.reference)
            .collect::<Vec<_>>();
        let uninitialized = uninitialized_ids
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
            .collect::<Vec<_>>();
        assert!(!uninitialized.iter().any(|name| name == "LINENO"));
        assert!(!uninitialized.iter().any(|name| name == "FUNCNAME"));
        assert!(!uninitialized.iter().any(|name| name == "BASH_SOURCE"));
        assert!(!uninitialized.iter().any(|name| name == "BASH_LINENO"));
    }

    #[test]
    fn bash_runtime_vars_remain_unresolved_in_non_bash_scripts() {
        let source = "\
#!/bin/sh
printf '%s %s %s %s\\n' \"$LINENO\" \"$FUNCNAME\" \"${BASH_SOURCE[0]}\" \"${BASH_LINENO[0]}\"
";
        let mut model = model(source);

        let unresolved = model
            .unresolved_references()
            .iter()
            .map(|reference| model.reference(*reference).name.as_str())
            .collect::<Vec<_>>();
        assert!(unresolved.contains(&"LINENO"));
        assert!(unresolved.contains(&"FUNCNAME"));
        assert!(unresolved.contains(&"BASH_SOURCE"));
        assert!(unresolved.contains(&"BASH_LINENO"));

        let uninitialized_ids = model
            .precompute_uninitialized_references()
            .iter()
            .map(|reference| reference.reference)
            .collect::<Vec<_>>();
        let uninitialized = uninitialized_ids
            .iter()
            .map(|reference| model.reference(*reference).name.to_string())
            .collect::<Vec<_>>();
        assert!(uninitialized.iter().any(|name| name == "LINENO"));
        assert!(uninitialized.iter().any(|name| name == "FUNCNAME"));
        assert!(uninitialized.iter().any(|name| name == "BASH_SOURCE"));
        assert!(uninitialized.iter().any(|name| name == "BASH_LINENO"));
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
    fn recorded_program_cfg_and_dataflow_match_legacy_conversion() {
        let source = "\
f() { echo $X; }
if cond; then
  X=1
else
  X=2
fi
while cond; do
  X=3
  break
done
echo $X
( Y=1 )
echo $(printf '%s' \"$X\")
";
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.script, source, &indexer);

        let new_cfg = build_control_flow_graph(
            &model.recorded_program,
            &model.command_bindings,
            &model.command_references,
        );
        let legacy_program = RecordedProgram::from_script(&output.script, model.scopes());
        let legacy_cfg = build_control_flow_graph(
            &legacy_program,
            &model.command_bindings,
            &model.command_references,
        );

        assert_eq!(new_cfg.blocks(), legacy_cfg.blocks());
        assert_eq!(new_cfg.entry(), legacy_cfg.entry());
        assert_eq!(new_cfg.exits(), legacy_cfg.exits());
        assert_eq!(new_cfg.unreachable(), legacy_cfg.unreachable());

        for block in new_cfg.blocks() {
            assert_eq!(
                new_cfg.successors(block.id),
                legacy_cfg.successors(block.id),
                "successors differed for block {:?}",
                block.id
            );
            assert_eq!(
                new_cfg.predecessors(block.id),
                legacy_cfg.predecessors(block.id),
                "predecessors differed for block {:?}",
                block.id
            );
        }

        let new_dataflow = crate::dataflow::analyze(
            &new_cfg,
            &model.scopes,
            &model.bindings,
            &model.references,
            &model.predefined_runtime_refs,
            &model.resolved,
            &model.call_sites,
            &model.indirect_target_hints,
            &model.indirect_expansion_refs,
            &model.synthetic_reads,
        );
        let legacy_dataflow = crate::dataflow::analyze(
            &legacy_cfg,
            &model.scopes,
            &model.bindings,
            &model.references,
            &model.predefined_runtime_refs,
            &model.resolved,
            &model.call_sites,
            &model.indirect_target_hints,
            &model.indirect_expansion_refs,
            &model.synthetic_reads,
        );
        assert_eq!(new_dataflow, legacy_dataflow);
    }
}
