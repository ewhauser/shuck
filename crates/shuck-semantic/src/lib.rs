mod binding;
mod builder;
mod call_graph;
mod cfg;
mod dataflow;
mod declaration;
mod reference;
mod scope;
mod source_ref;

pub use binding::{Binding, BindingAttributes, BindingId, BindingKind};
pub use call_graph::{CallGraph, CallSite, OverwrittenFunction};
pub use cfg::{BasicBlock, BlockId, ControlFlowGraph, EdgeKind, FlowContext};
pub use dataflow::{
    DataflowResult, DeadCode, ReachingDefinitions, UninitializedCertainty, UninitializedReference,
    UnusedAssignment, UnusedReason,
};
pub use declaration::{Declaration, DeclarationBuiltin, DeclarationOperand};
pub use reference::{Reference, ReferenceId, ReferenceKind};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use source_ref::{SourceRef, SourceRefKind};

use rustc_hash::FxHashMap;
use shuck_ast::{Command, Name, Script, Span};
use shuck_indexer::Indexer;

use crate::builder::SemanticModelBuilder;
use crate::cfg::{RecordedProgram, build_control_flow_graph};

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
    binding_index: FxHashMap<Name, Vec<BindingId>>,
    resolved: FxHashMap<ReferenceId, BindingId>,
    unresolved: Vec<ReferenceId>,
    functions: FxHashMap<Name, Vec<BindingId>>,
    call_sites: FxHashMap<Name, Vec<CallSite>>,
    call_graph: CallGraph,
    source_refs: Vec<SourceRef>,
    declarations: Vec<Declaration>,
    flow_contexts: Vec<(Span, FlowContext)>,
    recorded_program: RecordedProgram,
    command_bindings: FxHashMap<SpanKey, Vec<BindingId>>,
    command_references: FxHashMap<SpanKey, Vec<ReferenceId>>,
    cfg: Option<ControlFlowGraph>,
    dataflow: Option<DataflowResult>,
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
            binding_index: built.binding_index,
            resolved: built.resolved,
            unresolved: built.unresolved,
            functions: built.functions,
            call_sites: built.call_sites,
            call_graph: built.call_graph,
            source_refs: built.source_refs,
            declarations: built.declarations,
            flow_contexts: built.flow_contexts,
            recorded_program: built.recorded_program,
            command_bindings: built.command_bindings,
            command_references: built.command_references,
            cfg: None,
            dataflow: None,
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
            .unwrap_or(&self.heuristic_unused_assignments)
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

    pub fn dataflow(&mut self) -> &DataflowResult {
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
                dataflow::analyze(cfg, &self.bindings, &self.references)
            };
            self.dataflow = Some(result);
        }
        self.dataflow.as_ref().unwrap()
    }

    pub fn is_reachable(&mut self, span: &Span) -> bool {
        let cfg = self.cfg();
        cfg.block_ids_for_span(*span)
            .iter()
            .all(|block| !cfg.unreachable().contains(block))
    }

    pub fn dead_code(&mut self) -> &[DeadCode] {
        &self.dataflow().dead_code
    }
}

#[doc(hidden)]
pub fn build_with_observer(
    script: &Script,
    source: &str,
    indexer: &Indexer,
    observer: &mut dyn TraversalObserver,
) -> SemanticModel {
    let built = SemanticModelBuilder::build(script, source, indexer, observer);
    SemanticModel::from_build_output(built)
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

    fn model(source: &str) -> SemanticModel {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        SemanticModel::build(&output.script, source, &indexer)
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
        assert_eq!(binding.span.slice(source), "VAR=outer");
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
    fn detects_dead_code_after_exit() {
        let source = "exit 0\necho dead\n";
        let mut model = model(source);
        let dead_code = model.dead_code();
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
        assert_eq!(binding.span.slice(source), "X=1");
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
        assert_eq!(binding.span.slice(source).trim(), "X=1");
        assert!(matches!(
            model.scope_kind(binding.scope),
            ScopeKind::Function(name) if name == "outer"
        ));
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

        let new_dataflow = crate::dataflow::analyze(&new_cfg, &model.bindings, &model.references);
        let legacy_dataflow =
            crate::dataflow::analyze(&legacy_cfg, &model.bindings, &model.references);
        assert_eq!(new_dataflow, legacy_dataflow);
    }
}
