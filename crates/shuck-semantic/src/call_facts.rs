//! Workspace call-graph index for cross-file call hierarchy (spec 025).
//!
//! Each file contributes a compact [`FileCallFacts`] projection: the functions
//! it defines, the call sites it contains (tagged with their enclosing
//! function), and the resolved paths of its determinable source edges. A
//! [`WorkspaceCallIndex`] holds those projections keyed by path and answers
//! outgoing/incoming call-hierarchy queries as symmetric traversals of one
//! resolvable call graph.
//!
//! Path resolution (turning a `source`/hint operand into an on-disk path) lives
//! outside this module; callers supply already-resolved `source_edges`, so this
//! layer stays pure graph logic and is unit-testable without a filesystem.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};

use crate::editor::binding_definition_span;
use crate::{ScopeId, SemanticModel};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedDefinition {
    path: PathBuf,
    def_span: Span,
}

/// A definition exported through a source edge, paired with that edge's span
/// in the referring file for source-order precedence.
type SourceResolution = Option<(ResolvedDefinition, Span)>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExactResolution {
    Defined(ResolvedDefinition),
    Absent,
    Ambiguous,
}

#[derive(Clone, Copy)]
struct ExactQuery<'a> {
    top_level_cutoff: usize,
    local_definition: Option<Span>,
    include_top_level_definitions: bool,
    enclosing_function: Option<&'a CallFunctionId>,
    local_call_offset: Option<usize>,
}

impl ExactQuery<'_> {
    fn top_level(cutoff: usize) -> Self {
        Self {
            top_level_cutoff: cutoff,
            local_definition: None,
            include_top_level_definitions: true,
            enclosing_function: None,
            local_call_offset: None,
        }
    }
}

/// Stable identity of one function node within its file.
///
/// Function names alone are insufficient because shell permits later
/// definitions to replace earlier same-named definitions. The definition byte
/// range distinguishes those bindings while remaining serializable through an
/// LSP call-hierarchy item's opaque data payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallFunctionId {
    /// Function name.
    pub name: Name,
    /// Inclusive byte offset where the full definition starts.
    pub definition_start: usize,
    /// Exclusive byte offset where the full definition ends.
    pub definition_end: usize,
}

impl CallFunctionId {
    /// Creates an identity from a function name and its full definition span.
    pub fn new(name: Name, definition_span: Span) -> Self {
        Self {
            name,
            definition_start: definition_span.start.offset,
            definition_end: definition_span.end.offset,
        }
    }
}

/// Identity of a call-graph node within a file: an exact function definition,
/// or the file's top-level (module) body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallNodeKind {
    /// One named function definition in the file.
    Function(CallFunctionId),
    /// The file's top-level statements.
    TopLevel,
}

/// A function definition discovered in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFactDefinition {
    /// Function name.
    pub name: Name,
    /// Span covering the whole definition.
    pub def_span: Span,
    /// Span to select when navigating to the definition (the name token).
    pub selection_span: Span,
    /// Whether every path through the containing command installs it.
    pub unconditional: bool,
    /// Whether the definition executes in the persistent file scope.
    pub persistent_top_level: bool,
}

impl CallFactDefinition {
    /// Returns this definition's exact call-graph identity.
    pub fn identity(&self) -> CallFunctionId {
        CallFunctionId::new(self.name.clone(), self.def_span)
    }
}

/// A call site discovered in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFactSite {
    /// Callee name as written at the site.
    pub callee: Name,
    /// Span of the callee token.
    pub name_span: Span,
    /// Innermost enclosing function, or the file top level.
    pub enclosing: CallNodeKind,
    /// Definition span when the semantic model resolved this call to a function
    /// binding visible in this file. Retaining the span lets later source edges
    /// override that binding while preserving in-file definition order.
    pub local_definition_span: Option<Span>,
}

/// One statically resolved `source` edge, retaining its execution position in
/// the referring file so shadowing can follow shell source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFactSourceEdge {
    /// Resolved target path.
    pub path: PathBuf,
    /// Span of the `source` reference in the referring file.
    pub span: Span,
    /// Whether control flow may skip the source operation.
    pub conditional: bool,
    /// Whether completion may treat the source as an executed top-level edge.
    pub completion_visible: bool,
}

/// One function imported by a source edge and visible at a completion point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleSourcedFunction {
    /// Visible function name.
    pub name: Name,
    /// File containing the winning definition.
    pub path: PathBuf,
    /// Full definition span in `path`.
    pub def_span: Span,
    /// Name-token span in `path`.
    pub selection_span: Span,
    /// Source-edge span in the requesting file that imported this binding.
    pub import_span: Span,
}

/// A source operation that may affect function visibility, including
/// operations whose target could not be resolved statically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFactSourceEffect {
    /// Resolved target, or `None` when the operation is dynamic or unavailable.
    pub path: Option<PathBuf>,
    /// Span of the source operation in the referring file.
    pub span: Span,
    /// Whether control flow may skip the operation.
    pub conditional: bool,
    /// Innermost function containing the operation, if any.
    pub enclosing_function: Option<CallFunctionId>,
    /// Whether the operation's effects survive its containing execution scope.
    pub persistent: bool,
}

/// Call-relevant facts projected from one file, plus its resolved source edges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileCallFacts {
    /// Functions defined in this file.
    pub definitions: Vec<CallFactDefinition>,
    /// Call sites contained in this file.
    pub call_sites: Vec<CallFactSite>,
    /// Resolved on-disk paths of this file's determinable source edges (literal
    /// resolvable paths plus `source=` directive targets).
    pub source_edges: Vec<CallFactSourceEdge>,
    /// All source operations, including unresolved or conditional effects.
    pub source_effects: Vec<CallFactSourceEffect>,
    binding_mutators: Vec<CallFunctionId>,
    has_dynamic_command_dispatch: bool,
    analyzable: bool,
}

impl FileCallFacts {
    /// Projects call facts from a semantic model. `source_edges` are the resolved
    /// target paths of the file's determinable source edges, supplied by the
    /// caller (path resolution is not a semantic-layer concern).
    pub fn project(model: &SemanticModel, source_edges: Vec<PathBuf>) -> Self {
        Self::project_with_source_edges(
            model,
            source_edges
                .into_iter()
                .map(|path| CallFactSourceEdge {
                    path,
                    span: Span::new(),
                    conditional: false,
                    completion_visible: true,
                })
                .collect(),
        )
    }

    /// Projects call facts while preserving each source edge's position.
    pub fn project_with_source_edges(
        model: &SemanticModel,
        mut source_edges: Vec<CallFactSourceEdge>,
    ) -> Self {
        let analysis = model.analysis();
        source_edges.sort_by_key(|edge| edge.span.start.offset);

        let mut functions_by_scope: FxHashMap<ScopeId, Vec<(CallFunctionId, Span)>> =
            FxHashMap::default();
        let mut definitions = Vec::new();
        for binding in model.function_definition_bindings() {
            let definition = CallFactDefinition {
                name: binding.name.clone(),
                def_span: binding_definition_span(binding),
                selection_span: binding.span,
                unconditional: model.function_binding_is_unconditional(binding.id),
                persistent_top_level: model.enclosing_function_scope(binding.scope).is_none()
                    && model
                        .innermost_transient_scope_within_function(binding.scope)
                        .is_none(),
            };
            if let Some(scope) = analysis.function_scope_for_binding(binding.id) {
                functions_by_scope
                    .entry(scope)
                    .or_default()
                    .push((definition.identity(), definition.def_span));
            }
            definitions.push(definition);
        }

        let mut call_sites = Vec::new();
        for site in model.all_call_sites() {
            let enclosing_functions = model
                .ancestor_scopes(site.scope)
                .find_map(|scope| functions_by_scope.get(&scope));
            let local_definition_span = analysis
                .visible_function_binding_at_call(&site.callee, site.name_span)
                .map(|binding_id| binding_definition_span(model.binding(binding_id)))
                .or_else(|| {
                    enclosing_functions.and_then(|functions| {
                        functions
                            .iter()
                            .rev()
                            .find(|(function, _)| function.name == site.callee)
                            .map(|(_, definition_span)| *definition_span)
                    })
                });
            if let Some(enclosing_functions) = enclosing_functions {
                for (enclosing, _) in enclosing_functions {
                    call_sites.push(CallFactSite {
                        callee: site.callee.clone(),
                        name_span: site.name_span,
                        enclosing: CallNodeKind::Function(enclosing.clone()),
                        local_definition_span,
                    });
                }
            } else {
                call_sites.push(CallFactSite {
                    callee: site.callee.clone(),
                    name_span: site.name_span,
                    enclosing: CallNodeKind::TopLevel,
                    local_definition_span,
                });
            }
        }

        let mut source_effects = model
            .source_refs()
            .iter()
            .map(|source_ref| {
                let scope = model.scope_at(source_ref.span.start.offset);
                let enclosing_function = model
                    .ancestor_scopes(scope)
                    .find_map(|scope| functions_by_scope.get(&scope))
                    .and_then(|functions| functions.first())
                    .map(|(function, _)| function.clone());
                CallFactSourceEffect {
                    path: source_edges
                        .iter()
                        .find(|edge| edge.span == source_ref.span)
                        .map(|edge| edge.path.clone()),
                    span: source_ref.span,
                    conditional: source_ref.conditionally_executed,
                    enclosing_function,
                    persistent: model
                        .innermost_transient_scope_within_function(scope)
                        .is_none(),
                }
            })
            .collect::<Vec<_>>();
        for edge in &source_edges {
            if !source_effects.iter().any(|effect| effect.span == edge.span) {
                source_effects.push(CallFactSourceEffect {
                    path: Some(edge.path.clone()),
                    span: edge.span,
                    conditional: false,
                    enclosing_function: None,
                    persistent: true,
                });
            }
        }
        source_effects.sort_by_key(|effect| effect.span.start.offset);
        let has_dynamic_command_dispatch =
            model.recorded_program().commands().iter().any(|command| {
                command.command_info.is_some_and(|info| {
                    model
                        .recorded_program()
                        .command_info(info)
                        .dynamic_name_span
                        .is_some()
                })
            });
        let mut binding_mutators = FxHashSet::default();
        for binding in model.function_definition_bindings() {
            if model
                .innermost_transient_scope_within_function(binding.scope)
                .is_some()
            {
                continue;
            }
            if let Some(enclosing_functions) = model
                .ancestor_scopes(binding.scope)
                .find_map(|scope| functions_by_scope.get(&scope))
            {
                binding_mutators.extend(
                    enclosing_functions
                        .iter()
                        .map(|(function, _)| function.clone()),
                );
            }
        }

        Self {
            definitions,
            call_sites,
            source_edges,
            source_effects,
            binding_mutators: binding_mutators.into_iter().collect(),
            has_dynamic_command_dispatch,
            analyzable: true,
        }
    }

    /// Returns the exact function definition identified by `function`.
    pub fn definition(&self, function: &CallFunctionId) -> Option<&CallFactDefinition> {
        self.definitions.iter().find(|definition| {
            definition.name == function.name
                && definition.def_span.start.offset == function.definition_start
                && definition.def_span.end.offset == function.definition_end
        })
    }
}

/// One end of a cross-file call edge, with the call-token spans that realize it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossFileCall {
    /// File of the node at this end of the edge (the callee for outgoing, the
    /// caller for incoming).
    pub path: PathBuf,
    /// Which node in `path`.
    pub node: CallNodeKind,
    /// Definition span of the node's function; `None` for a top-level node.
    pub def_span: Option<Span>,
    /// Selection span of the node's function; `None` for a top-level node.
    pub selection_span: Option<Span>,
    /// Spans of the callee tokens that realize the edge.
    ///
    /// For an outgoing edge these live in the *queried* file; for an incoming
    /// edge they live in `path` (the caller's file).
    pub call_spans: Vec<Span>,
}

/// One call token proven to resolve to an exact workspace function binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactFunctionReference {
    /// File containing the call token.
    pub path: PathBuf,
    /// Span of the callee token in `path`.
    pub span: Span,
}

/// Exact references and all indexed files whose source relationship can
/// affect a cross-file function rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactFunctionRename {
    /// Call tokens proven to resolve to the selected function binding.
    pub references: Vec<ExactFunctionReference>,
    /// Source-connected files whose snapshots must still match the index.
    pub relevant_paths: Vec<PathBuf>,
}

/// Why an exact cross-file function rename set could not be proven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactFunctionRenameError {
    /// A same-named call in the relevant source graph has ambiguous binding
    /// identity and therefore cannot be safely included or excluded.
    AmbiguousReference,
    /// A source operation in the relevant graph could not be resolved to a
    /// file represented by the workspace index.
    IncompleteSourceGraph,
}

type ExactReferenceIndex = FxHashMap<(PathBuf, CallNodeKind), Vec<ExactFunctionReference>>;

#[derive(Clone, Debug)]
struct ExactIncomingSource {
    parent_path: PathBuf,
    span: Span,
    conditional: bool,
    persistent: bool,
    enclosing_function: Option<CallFunctionId>,
}

#[derive(Debug)]
struct ExactWorkspaceContext {
    graph: SourceGraphIndex,
    incoming: FxHashMap<PathBuf, Vec<ExactIncomingSource>>,
    calls_by_component: Vec<FxHashSet<Name>>,
    dynamic_dispatch_by_component: Vec<bool>,
    mutator_functions_by_name: FxHashMap<Name, Vec<(usize, PathBuf, CallFunctionId, bool)>>,
    function_calls_by_name: FxHashMap<Name, Vec<(usize, Name)>>,
    may_mutate_cache: Mutex<FxHashMap<(usize, Name), bool>>,
    mutation_summary_cache: Mutex<FxHashMap<usize, MutationSummary>>,
}

#[derive(Clone, Debug)]
enum MutationSummary {
    None,
    Single(PathBuf, CallFunctionId, bool),
    Multiple,
}

impl MutationSummary {
    fn add(&mut self, path: &Path, function: &CallFunctionId, can_exclude_current: bool) {
        match self {
            Self::None => {
                *self = Self::Single(path.to_path_buf(), function.clone(), can_exclude_current);
            }
            Self::Single(existing_path, existing_function, existing_can_exclude)
                if existing_path.as_path() == path && existing_function == function =>
            {
                *existing_can_exclude &= can_exclude_current;
            }
            Self::Single(_, _, _) => *self = Self::Multiple,
            Self::Multiple => {}
        }
    }

    fn may_mutate_other_than(&self, path: &Path, function: &CallFunctionId) -> bool {
        match self {
            Self::None => false,
            Self::Single(candidate_path, candidate, can_exclude_current) => {
                candidate_path.as_path() != path || candidate != function || !can_exclude_current
            }
            Self::Multiple => true,
        }
    }
}

#[derive(Debug)]
struct SourceGraphIndex {
    paths: Vec<PathBuf>,
    nodes: FxHashMap<PathBuf, usize>,
    components: Vec<usize>,
    component_reachability: Vec<Vec<u64>>,
    component_edges: Vec<Vec<usize>>,
}

/// A workspace-wide index of per-file call facts.
#[derive(Debug, Default)]
pub struct WorkspaceCallIndex {
    files: FxHashMap<PathBuf, FileCallFacts>,
    exact_context: OnceLock<ExactWorkspaceContext>,
    exact_references: OnceLock<ExactReferenceIndex>,
}

impl SourceGraphIndex {
    fn build(
        files: &FxHashMap<PathBuf, FileCallFacts>,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<Self> {
        let mut paths = files.keys().cloned().collect::<Vec<_>>();
        paths.sort();
        let nodes = paths
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, path)| (path, index))
            .collect::<FxHashMap<_, _>>();
        let mut edges = vec![Vec::new(); paths.len()];
        let mut reverse_edges = vec![Vec::new(); paths.len()];
        for (from, path) in paths.iter().enumerate() {
            if is_cancelled() {
                return None;
            }
            for edge in &files.get(path)?.source_edges {
                let Some(&to) = nodes.get(&edge.path) else {
                    continue;
                };
                edges[from].push(to);
                reverse_edges[to].push(from);
            }
            edges[from].sort_unstable();
            edges[from].dedup();
        }

        let mut visited = vec![false; paths.len()];
        let mut order = Vec::with_capacity(paths.len());
        for start in 0..paths.len() {
            if visited[start] {
                continue;
            }
            visited[start] = true;
            let mut stack = vec![(start, 0usize)];
            while let Some((node, next_edge)) = stack.last_mut() {
                if is_cancelled() {
                    return None;
                }
                if *next_edge < edges[*node].len() {
                    let next = edges[*node][*next_edge];
                    *next_edge += 1;
                    if !visited[next] {
                        visited[next] = true;
                        stack.push((next, 0));
                    }
                } else {
                    let (node, _) = stack.pop()?;
                    order.push(node);
                }
            }
        }

        let mut components = vec![usize::MAX; paths.len()];
        let mut component_count = 0usize;
        for &start in order.iter().rev() {
            if components[start] != usize::MAX {
                continue;
            }
            components[start] = component_count;
            let mut stack = vec![start];
            while let Some(node) = stack.pop() {
                if is_cancelled() {
                    return None;
                }
                for &next in &reverse_edges[node] {
                    if components[next] == usize::MAX {
                        components[next] = component_count;
                        stack.push(next);
                    }
                }
            }
            component_count += 1;
        }

        let mut component_edges = vec![Vec::new(); component_count];
        for (from, outgoing) in edges.iter().enumerate() {
            for &to in outgoing {
                let from_component = components[from];
                let to_component = components[to];
                if from_component != to_component {
                    component_edges[from_component].push(to_component);
                }
            }
        }
        for outgoing in &mut component_edges {
            outgoing.sort_unstable();
            outgoing.dedup();
        }

        let mut component_postorder = Vec::with_capacity(component_count);
        let mut visited = vec![false; component_count];
        for start in 0..component_count {
            if visited[start] {
                continue;
            }
            visited[start] = true;
            let mut stack = vec![(start, 0usize)];
            while let Some((component, next_edge)) = stack.last_mut() {
                if is_cancelled() {
                    return None;
                }
                if *next_edge < component_edges[*component].len() {
                    let next = component_edges[*component][*next_edge];
                    *next_edge += 1;
                    if !visited[next] {
                        visited[next] = true;
                        stack.push((next, 0));
                    }
                } else {
                    let (component, _) = stack.pop()?;
                    component_postorder.push(component);
                }
            }
        }

        let word_count = component_count.div_ceil(64);
        let mut component_reachability = vec![vec![0u64; word_count]; component_count];
        for &component in &component_postorder {
            if is_cancelled() {
                return None;
            }
            set_bit(&mut component_reachability[component], component);
            for &successor in &component_edges[component] {
                let successor_reachability = component_reachability[successor].clone();
                for (word, value) in component_reachability[component]
                    .iter_mut()
                    .zip(successor_reachability)
                {
                    if is_cancelled() {
                        return None;
                    }
                    *word |= value;
                }
            }
        }

        Some(Self {
            paths,
            nodes,
            components,
            component_reachability,
            component_edges,
        })
    }

    fn component_for_path(&self, path: &Path) -> Option<usize> {
        self.nodes.get(path).map(|&node| self.components[node])
    }

    fn component_reaches(&self, from: usize, to: usize) -> bool {
        bit_is_set(&self.component_reachability[from], to)
    }

    fn source_environment_components(
        &self,
        target: usize,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<Vec<u64>> {
        let mut shared = vec![0; self.component_reachability[target].len()];
        for ancestor in 0..self.component_edges.len() {
            if is_cancelled() {
                return None;
            }
            if self.component_reaches(ancestor, target) {
                for (word, reachable) in shared
                    .iter_mut()
                    .zip(&self.component_reachability[ancestor])
                {
                    *word |= reachable;
                }
            }
        }
        Some(shared)
    }
}

impl ExactWorkspaceContext {
    fn build(
        files: &FxHashMap<PathBuf, FileCallFacts>,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<Self> {
        let graph = SourceGraphIndex::build(files, is_cancelled)?;
        let mut incoming: FxHashMap<PathBuf, Vec<ExactIncomingSource>> = FxHashMap::default();
        let mut function_calls_by_name: FxHashMap<Name, Vec<(usize, Name)>> = FxHashMap::default();
        let mut calls_by_component = vec![FxHashSet::default(); graph.component_edges.len()];
        let mut dynamic_dispatch_by_component = vec![false; graph.component_edges.len()];
        let mut mutators: FxHashMap<(PathBuf, CallFunctionId), (usize, bool)> =
            FxHashMap::default();
        for (node, path) in graph.paths.iter().enumerate() {
            if is_cancelled() {
                return None;
            }
            let facts = files.get(path)?;
            let component = graph.components[node];
            dynamic_dispatch_by_component[component] |= facts.has_dynamic_command_dispatch;
            for site in &facts.call_sites {
                calls_by_component[component].insert(site.callee.clone());
                if let CallNodeKind::Function(function) = &site.enclosing {
                    function_calls_by_name
                        .entry(function.name.clone())
                        .or_default()
                        .push((component, site.callee.clone()));
                }
            }
            for effect in &facts.source_effects {
                if let Some(function) = &effect.enclosing_function {
                    mutators
                        .entry((path.clone(), function.clone()))
                        .or_insert((component, true));
                }
                if let Some(target) = &effect.path {
                    incoming
                        .entry(target.clone())
                        .or_default()
                        .push(ExactIncomingSource {
                            parent_path: path.clone(),
                            span: effect.span,
                            conditional: effect.conditional,
                            persistent: effect.persistent,
                            enclosing_function: effect.enclosing_function.clone(),
                        });
                }
            }
            for function in &facts.binding_mutators {
                mutators
                    .entry((path.clone(), function.clone()))
                    .and_modify(|(_, can_exclude_current)| *can_exclude_current = false)
                    .or_insert((component, false));
            }
        }
        let mut mutator_functions_by_name: FxHashMap<
            Name,
            Vec<(usize, PathBuf, CallFunctionId, bool)>,
        > = FxHashMap::default();
        for ((path, function), (component, can_exclude_current)) in mutators {
            mutator_functions_by_name
                .entry(function.name.clone())
                .or_default()
                .push((component, path, function, can_exclude_current));
        }
        for edges in incoming.values_mut() {
            edges.sort_by(|left, right| {
                left.parent_path
                    .cmp(&right.parent_path)
                    .then_with(|| left.span.start.offset.cmp(&right.span.start.offset))
            });
        }

        Some(Self {
            graph,
            incoming,
            calls_by_component,
            dynamic_dispatch_by_component,
            mutator_functions_by_name,
            function_calls_by_name,
            may_mutate_cache: Mutex::new(FxHashMap::default()),
            mutation_summary_cache: Mutex::new(FxHashMap::default()),
        })
    }

    fn called_mutator_function_may_mutate(
        &self,
        current_path: &Path,
        current_function: &CallFunctionId,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<bool> {
        let Some(component) = self.graph.component_for_path(current_path) else {
            return Some(true);
        };
        if let Ok(cache) = self.mutation_summary_cache.lock()
            && let Some(summary) = cache.get(&component)
        {
            return Some(summary.may_mutate_other_than(current_path, current_function));
        }
        let mut summary = MutationSummary::None;
        let dynamic =
            self.dynamic_dispatch_by_component
                .iter()
                .enumerate()
                .any(|(candidate, &dynamic)| {
                    self.graph.component_reaches(component, candidate) && dynamic
                });
        if dynamic {
            for mutators in self.mutator_functions_by_name.values() {
                for (mutator_component, path, function, can_exclude_current) in mutators {
                    if is_cancelled() {
                        return None;
                    }
                    if self.graph.component_reaches(component, *mutator_component) {
                        summary.add(path, function, *can_exclude_current);
                    }
                }
            }
        } else {
            let mut called_names = FxHashSet::default();
            for (candidate, calls) in self.calls_by_component.iter().enumerate() {
                if is_cancelled() {
                    return None;
                }
                if self.graph.component_reaches(component, candidate) {
                    called_names.extend(calls.iter().cloned());
                }
            }
            for name in called_names {
                self.collect_mutators(component, &name, &mut summary, is_cancelled)?;
                if matches!(summary, MutationSummary::Multiple) {
                    break;
                }
            }
        }
        if let Ok(mut cache) = self.mutation_summary_cache.lock() {
            cache.insert(component, summary.clone());
        }
        Some(summary.may_mutate_other_than(current_path, current_function))
    }

    fn name_may_mutate(
        &self,
        from_path: &Path,
        name: &Name,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<bool> {
        let Some(component) = self.graph.component_for_path(from_path) else {
            return Some(true);
        };
        let key = (component, name.clone());
        if let Ok(cache) = self.may_mutate_cache.lock()
            && let Some(&cached) = cache.get(&key)
        {
            return Some(cached);
        }
        let mut summary = MutationSummary::None;
        self.collect_mutators(component, name, &mut summary, is_cancelled)?;
        let result = !matches!(summary, MutationSummary::None);
        if let Ok(mut cache) = self.may_mutate_cache.lock() {
            cache.insert(key, result);
        }
        Some(result)
    }

    fn collect_mutators(
        &self,
        component: usize,
        name: &Name,
        summary: &mut MutationSummary,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<()> {
        let mut pending = vec![name.clone()];
        let mut visited = FxHashSet::default();
        while let Some(name) = pending.pop() {
            if is_cancelled() {
                return None;
            }
            if !visited.insert(name.clone()) {
                continue;
            }
            for (mutator_component, path, function, can_exclude_current) in self
                .mutator_functions_by_name
                .get(&name)
                .into_iter()
                .flatten()
            {
                if self.graph.component_reaches(component, *mutator_component) {
                    summary.add(path, function, *can_exclude_current);
                }
            }
            if matches!(summary, MutationSummary::Multiple) {
                return Some(());
            }
            for (call_component, callee) in
                self.function_calls_by_name.get(&name).into_iter().flatten()
            {
                if self.graph.component_reaches(component, *call_component) {
                    pending.push(callee.clone());
                }
            }
        }
        Some(())
    }
}

fn set_bit(words: &mut [u64], bit: usize) {
    words[bit / 64] |= 1u64 << (bit % 64);
}

fn bit_is_set(words: &[u64], bit: usize) -> bool {
    words
        .get(bit / 64)
        .is_some_and(|word| word & (1u64 << (bit % 64)) != 0)
}

impl WorkspaceCallIndex {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces the facts for `path`.
    pub fn insert(&mut self, path: PathBuf, facts: FileCallFacts) {
        self.files.insert(path, facts);
        self.exact_context.take();
        self.exact_references.take();
    }

    /// Returns whether `path` is indexed.
    pub fn contains(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    /// Iterates all indexed files and their facts (unordered).
    pub fn files(&self) -> impl Iterator<Item = (&Path, &FileCallFacts)> {
        self.files
            .iter()
            .map(|(path, facts)| (path.as_path(), facts))
    }

    /// Returns the number of indexed files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Returns sourced functions that are unconditionally visible before
    /// `cutoff` in `from_path`, with source order and redefinitions applied.
    pub fn visible_sourced_functions(
        &self,
        from_path: &Path,
        cutoff: usize,
    ) -> Vec<VisibleSourcedFunction> {
        let Some(facts) = self.files.get(from_path) else {
            return Vec::new();
        };
        self.visible_sourced_functions_for_edges(
            from_path,
            facts.source_edges.iter().filter(|edge| {
                edge.completion_visible
                    && (edge.span == Span::new() || edge.span.start.offset < cutoff)
            }),
        )
    }

    /// Returns sourced functions imported by the exact source operations that
    /// semantic scope analysis proved visible at the completion point.
    pub fn visible_sourced_functions_from_source_spans(
        &self,
        from_path: &Path,
        source_spans: &[Span],
    ) -> Vec<VisibleSourcedFunction> {
        let Some(facts) = self.files.get(from_path) else {
            return Vec::new();
        };
        self.visible_sourced_functions_for_edges(
            from_path,
            facts
                .source_edges
                .iter()
                .filter(|edge| source_spans.contains(&edge.span)),
        )
    }

    fn visible_sourced_functions_for_edges<'a>(
        &self,
        from_path: &Path,
        edges: impl IntoIterator<Item = &'a CallFactSourceEdge>,
    ) -> Vec<VisibleSourcedFunction> {
        let mut visible = FxHashMap::<Name, (ResolvedDefinition, Span)>::default();
        for edge in edges {
            let mut stack = FxHashSet::default();
            stack.insert(from_path.to_path_buf());
            for (name, definition) in self.exported_functions(&edge.path, &mut stack) {
                visible.insert(name, (definition, edge.span));
            }
        }
        let mut functions = visible
            .into_iter()
            .filter_map(|(name, (definition, import_span))| {
                let selection_span = self
                    .files
                    .get(&definition.path)?
                    .definitions
                    .iter()
                    .find(|candidate| {
                        candidate.name == name && candidate.def_span == definition.def_span
                    })?
                    .selection_span;
                Some(VisibleSourcedFunction {
                    name,
                    path: definition.path,
                    def_span: definition.def_span,
                    selection_span,
                    import_span,
                })
            })
            .collect::<Vec<_>>();
        functions.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        functions
    }

    fn exported_functions(
        &self,
        path: &Path,
        stack: &mut FxHashSet<PathBuf>,
    ) -> FxHashMap<Name, ResolvedDefinition> {
        if !stack.insert(path.to_path_buf()) {
            return FxHashMap::default();
        }
        let Some(facts) = self.files.get(path) else {
            stack.remove(path);
            return FxHashMap::default();
        };

        enum Event<'a> {
            Definition(&'a CallFactDefinition),
            Source(&'a CallFactSourceEdge),
        }

        let mut events = facts
            .definitions
            .iter()
            .filter(|definition| definition.unconditional && definition.persistent_top_level)
            .map(|definition| {
                (
                    definition.def_span.start.offset,
                    Event::Definition(definition),
                )
            })
            .chain(
                facts
                    .source_edges
                    .iter()
                    .filter(|edge| edge.completion_visible)
                    .map(|edge| (edge.span.start.offset, Event::Source(edge))),
            )
            .collect::<Vec<_>>();
        events.sort_by_key(|(offset, _)| *offset);

        let mut exported = FxHashMap::default();
        for (_, event) in events {
            match event {
                Event::Definition(definition) => {
                    exported.insert(
                        definition.name.clone(),
                        ResolvedDefinition {
                            path: path.to_path_buf(),
                            def_span: definition.def_span,
                        },
                    );
                }
                Event::Source(edge) => {
                    exported.extend(self.exported_functions(&edge.path, stack));
                }
            }
        }
        stack.remove(path);
        exported
    }

    /// Resolves `callee`, as seen from `from_path`, to the file that defines it:
    /// the file's own definitions first, then its transitive source edges
    /// (nearest definition wins). Returns `None` for names with no reachable
    /// definition (builtins, external commands, unresolved dynamic sources).
    pub fn resolve(&self, from_path: &Path, callee: &Name) -> Option<PathBuf> {
        let facts = self.files.get(from_path)?;
        let local = facts
            .definitions
            .iter()
            .rev()
            .find(|definition| &definition.name == callee)
            .map(|definition| definition.def_span);
        let sourced = self.resolve_through_edges_before(from_path, callee, None);
        choose_resolved_target(from_path, local, sourced).map(|target| target.path)
    }

    /// Resolves the exact call token at `name_span` to its function node.
    ///
    /// Unlike [`Self::resolve`], this preserves the call site's execution
    /// position: top-level calls only see source edges that ran before them,
    /// while calls in deferred function bodies see the final sourced
    /// environment. File-local definitions participate in the same ordering.
    pub fn resolve_call_site(&self, from_path: &Path, name_span: Span) -> Option<CrossFileCall> {
        let facts = self.files.get(from_path)?;
        let site = facts
            .call_sites
            .iter()
            .find(|site| site.name_span == name_span)?;
        let cutoff = match site.enclosing {
            CallNodeKind::TopLevel => Some(site.name_span.start.offset),
            CallNodeKind::Function(_) => None,
        };
        let sourced = self.resolve_through_edges_before(from_path, &site.callee, cutoff);
        let target = choose_resolved_target(from_path, site.local_definition_span, sourced)?;
        let definition = self.files.get(&target.path).and_then(|facts| {
            facts.definitions.iter().find(|definition| {
                definition.name == site.callee && definition.def_span == target.def_span
            })
        });
        Some(CrossFileCall {
            path: target.path,
            node: CallNodeKind::Function(CallFunctionId::new(site.callee.clone(), target.def_span)),
            def_span: Some(target.def_span),
            selection_span: definition.map(|definition| definition.selection_span),
            call_spans: vec![site.name_span],
        })
    }

    /// Resolves a call only when source effects prove one exact definition.
    ///
    /// Unlike call hierarchy's best-effort graph, navigation must not guess
    /// across dynamic or conditional source operations. Calls in deferred
    /// function bodies also fail closed when a later source operation could
    /// run either before or after the function is invoked.
    pub fn resolve_call_site_exact(
        &self,
        from_path: &Path,
        name_span: Span,
    ) -> Option<CrossFileCall> {
        self.resolve_call_site_exact_cancellable(from_path, name_span, || false)
    }

    /// Resolves one exact call token while allowing workspace-context analysis
    /// to stop before publishing a partial answer.
    pub fn resolve_call_site_exact_cancellable(
        &self,
        from_path: &Path,
        name_span: Span,
        is_cancelled: impl Fn() -> bool,
    ) -> Option<CrossFileCall> {
        let context = self.exact_workspace_context(&is_cancelled)?;
        self.resolve_call_site_exact_with_context(from_path, name_span, context, &is_cancelled)
    }

    fn resolve_call_site_exact_with_context(
        &self,
        from_path: &Path,
        name_span: Span,
        context: &ExactWorkspaceContext,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<CrossFileCall> {
        let (site, resolution) = self.call_site_exact_resolution_with_context(
            from_path,
            name_span,
            context,
            is_cancelled,
        )?;
        let ExactResolution::Defined(target) = resolution else {
            return None;
        };
        let definition = self.files.get(&target.path).and_then(|facts| {
            facts.definitions.iter().find(|definition| {
                definition.name == site.callee && definition.def_span == target.def_span
            })
        });
        Some(CrossFileCall {
            path: target.path,
            node: CallNodeKind::Function(CallFunctionId::new(site.callee.clone(), target.def_span)),
            def_span: Some(target.def_span),
            selection_span: definition.map(|definition| definition.selection_span),
            call_spans: vec![site.name_span],
        })
    }

    fn call_site_exact_resolution_with_context<'a>(
        &'a self,
        from_path: &Path,
        name_span: Span,
        context: &ExactWorkspaceContext,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<(&'a CallFactSite, ExactResolution)> {
        let facts = self.files.get(from_path)?;
        let site = facts
            .call_sites
            .iter()
            .find(|site| site.name_span == name_span)?;
        if is_cancelled() {
            return None;
        }
        let mut stack = FxHashSet::default();
        stack.insert(from_path.to_path_buf());
        let resolution = match &site.enclosing {
            CallNodeKind::TopLevel => self.resolve_top_level_with_incoming(
                from_path,
                &site.callee,
                site.name_span.start.offset,
                context,
                &mut stack,
                is_cancelled,
            ),
            CallNodeKind::Function(function) => {
                let may_mutate = context
                    .called_mutator_function_may_mutate(from_path, function, is_cancelled)
                    .unwrap_or(true);
                if is_cancelled() {
                    return None;
                }
                if may_mutate
                    || self.top_level_prefix_may_invoke_source(
                        from_path,
                        function.definition_start,
                        context,
                        &mut FxHashSet::default(),
                        is_cancelled,
                    )
                    || facts.source_effects.iter().any(|effect| {
                        (effect.enclosing_function.is_none()
                            && effect.span.start.offset >= function.definition_start)
                            || (effect.enclosing_function.as_ref() == Some(function)
                                && effect.span.start.offset >= site.name_span.start.offset)
                    })
                {
                    return Some((site, ExactResolution::Ambiguous));
                }
                let resolution = self.resolve_exact_events(
                    from_path,
                    &site.callee,
                    ExactQuery {
                        top_level_cutoff: function.definition_start,
                        local_definition: site.local_definition_span,
                        include_top_level_definitions: false,
                        enclosing_function: Some(function),
                        local_call_offset: Some(site.name_span.start.offset),
                    },
                    &mut stack,
                );
                let resolution = if resolution == ExactResolution::Absent {
                    self.resolve_incoming_environment(
                        from_path,
                        &site.callee,
                        context,
                        &mut stack,
                        is_cancelled,
                    )
                } else {
                    resolution
                };
                if !self.deferred_incoming_contexts_are_stable(
                    from_path,
                    &site.callee,
                    context,
                    &mut FxHashSet::default(),
                    is_cancelled,
                ) {
                    if is_cancelled() {
                        return None;
                    }
                    return Some((site, ExactResolution::Ambiguous));
                }
                resolution
            }
        };
        (!is_cancelled()).then_some((site, resolution))
    }

    fn exact_workspace_context(
        &self,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<&ExactWorkspaceContext> {
        if self.exact_context.get().is_none() {
            let context = ExactWorkspaceContext::build(&self.files, is_cancelled)?;
            let _ = self.exact_context.set(context);
        }
        if is_cancelled() {
            return None;
        }
        self.exact_context.get()
    }

    fn resolve_top_level_with_incoming(
        &self,
        path: &Path,
        callee: &Name,
        cutoff: usize,
        context: &ExactWorkspaceContext,
        stack: &mut FxHashSet<PathBuf>,
        is_cancelled: &impl Fn() -> bool,
    ) -> ExactResolution {
        if is_cancelled() {
            return ExactResolution::Ambiguous;
        }
        if self.top_level_prefix_may_invoke_source(
            path,
            cutoff,
            context,
            &mut FxHashSet::default(),
            is_cancelled,
        ) {
            return ExactResolution::Ambiguous;
        }
        let resolution =
            self.resolve_exact_events(path, callee, ExactQuery::top_level(cutoff), stack);
        if resolution != ExactResolution::Absent {
            return resolution;
        }
        self.resolve_incoming_environment(path, callee, context, stack, is_cancelled)
    }

    fn top_level_prefix_may_invoke_source(
        &self,
        path: &Path,
        cutoff: usize,
        context: &ExactWorkspaceContext,
        visiting: &mut FxHashSet<PathBuf>,
        is_cancelled: &impl Fn() -> bool,
    ) -> bool {
        if is_cancelled() {
            return true;
        }
        if !visiting.insert(path.to_path_buf()) {
            return false;
        }
        let Some(facts) = self.files.get(path) else {
            visiting.remove(path);
            return true;
        };
        let may_invoke = facts.has_dynamic_command_dispatch
            || facts.call_sites.iter().any(|site| {
                matches!(site.enclosing, CallNodeKind::TopLevel)
                    && site.name_span.start.offset < cutoff
                    && context
                        .name_may_mutate(path, &site.callee, is_cancelled)
                        .unwrap_or(true)
            })
            || facts.source_effects.iter().any(|effect| {
                effect.enclosing_function.is_none()
                    && effect.persistent
                    && effect.span.start.offset < cutoff
                    && effect.path.as_deref().is_some_and(|target| {
                        self.top_level_prefix_may_invoke_source(
                            target,
                            usize::MAX,
                            context,
                            visiting,
                            is_cancelled,
                        )
                    })
            });
        visiting.remove(path);
        may_invoke
    }

    fn resolve_incoming_environment(
        &self,
        path: &Path,
        callee: &Name,
        context: &ExactWorkspaceContext,
        stack: &mut FxHashSet<PathBuf>,
        is_cancelled: &impl Fn() -> bool,
    ) -> ExactResolution {
        let Some(incoming) = context.incoming.get(path) else {
            return ExactResolution::Absent;
        };
        let mut inherited = None;
        for edge in incoming {
            if is_cancelled()
                || edge.conditional
                || !edge.persistent
                || edge.enclosing_function.is_some()
                || !stack.insert(edge.parent_path.clone())
            {
                return ExactResolution::Ambiguous;
            }
            let resolution = self.resolve_top_level_with_incoming(
                &edge.parent_path,
                callee,
                edge.span.start.offset,
                context,
                stack,
                is_cancelled,
            );
            stack.remove(&edge.parent_path);
            match (&inherited, resolution) {
                (_, ExactResolution::Ambiguous) => return ExactResolution::Ambiguous,
                (None, resolution) => inherited = Some(resolution),
                (Some(previous), resolution) if previous == &resolution => {}
                _ => return ExactResolution::Ambiguous,
            }
        }
        inherited.unwrap_or(ExactResolution::Absent)
    }

    fn deferred_incoming_contexts_are_stable(
        &self,
        path: &Path,
        callee: &Name,
        context: &ExactWorkspaceContext,
        visited: &mut FxHashSet<PathBuf>,
        is_cancelled: &impl Fn() -> bool,
    ) -> bool {
        if !visited.insert(path.to_path_buf()) {
            return false;
        }
        let stable = context.incoming.get(path).is_none_or(|incoming| {
            incoming.iter().all(|edge| {
                if is_cancelled()
                    || edge.conditional
                    || !edge.persistent
                    || edge.enclosing_function.is_some()
                {
                    return false;
                }
                let Some(parent) = self.files.get(&edge.parent_path) else {
                    return false;
                };
                if !parent.analyzable
                    || parent.has_dynamic_command_dispatch
                    || parent.definitions.iter().any(|definition| {
                        definition.name == *callee
                            && definition.def_span.start.offset > edge.span.start.offset
                    })
                    || parent
                        .source_effects
                        .iter()
                        .any(|effect| effect.span.start.offset > edge.span.start.offset)
                    || parent.call_sites.iter().any(|site| {
                        matches!(site.enclosing, CallNodeKind::TopLevel)
                            && site.name_span.start.offset > edge.span.start.offset
                            && context
                                .name_may_mutate(&edge.parent_path, &site.callee, is_cancelled)
                                .unwrap_or(true)
                    })
                {
                    return false;
                }
                self.deferred_incoming_contexts_are_stable(
                    &edge.parent_path,
                    callee,
                    context,
                    visited,
                    is_cancelled,
                )
            })
        });
        visited.remove(path);
        stable
    }

    /// Returns call tokens proven to resolve to `target_node` in `target_path`.
    ///
    /// The reverse index is built lazily and retained only after a complete,
    /// uncancelled pass. Ambiguous or unresolved calls are deliberately absent.
    pub fn exact_function_references(
        &self,
        target_path: &Path,
        target_node: &CallNodeKind,
        is_cancelled: impl Fn() -> bool,
    ) -> Option<&[ExactFunctionReference]> {
        if self.exact_references.get().is_none() {
            let built = self.build_exact_reference_index(&is_cancelled)?;
            let _ = self.exact_references.set(built);
        }
        if is_cancelled() {
            return None;
        }
        Some(
            self.exact_references
                .get()?
                .get(&(target_path.to_path_buf(), target_node.clone()))
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )
    }

    /// Builds the exact reference set required for a safe cross-file rename.
    ///
    /// In addition to returning proven references, this rejects a same-named
    /// call in the source-connected graph when its binding identity is
    /// ambiguous. The returned paths let callers verify that every snapshot
    /// which influenced the proof is still current before emitting edits.
    pub fn exact_function_rename(
        &self,
        target_path: &Path,
        target_node: &CallNodeKind,
        is_cancelled: impl Fn() -> bool,
    ) -> Option<Result<ExactFunctionRename, ExactFunctionRenameError>> {
        let CallNodeKind::Function(target_function) = target_node else {
            return Some(Err(ExactFunctionRenameError::AmbiguousReference));
        };
        let context = self.exact_workspace_context(&is_cancelled)?;
        let target_component = context.graph.component_for_path(target_path)?;
        let source_environment = context
            .graph
            .source_environment_components(target_component, &is_cancelled)?;
        let relevant_paths = context
            .graph
            .paths
            .iter()
            .enumerate()
            .filter(|(node, _)| bit_is_set(&source_environment, context.graph.components[*node]))
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        let mut references = Vec::new();
        for path in &relevant_paths {
            if is_cancelled() {
                return None;
            }
            let facts = self.files.get(path)?;
            if facts.source_effects.iter().any(|effect| {
                effect
                    .path
                    .as_ref()
                    .is_none_or(|target| !self.files.contains_key(target))
            }) {
                return Some(Err(ExactFunctionRenameError::IncompleteSourceGraph));
            }
            for site in facts
                .call_sites
                .iter()
                .filter(|site| site.callee == target_function.name)
            {
                let (_, resolution) = self.call_site_exact_resolution_with_context(
                    path,
                    site.name_span,
                    context,
                    &is_cancelled,
                )?;
                match resolution {
                    ExactResolution::Defined(target)
                        if target.path == target_path
                            && target.def_span.start.offset == target_function.definition_start
                            && target.def_span.end.offset == target_function.definition_end =>
                    {
                        references.push(ExactFunctionReference {
                            path: path.clone(),
                            span: site.name_span,
                        });
                    }
                    ExactResolution::Defined(_) | ExactResolution::Absent => {}
                    ExactResolution::Ambiguous => {
                        return Some(Err(ExactFunctionRenameError::AmbiguousReference));
                    }
                }
            }
        }
        references.sort_by(|left, right| {
            left.path.cmp(&right.path).then_with(|| {
                (left.span.start.offset, left.span.end.offset)
                    .cmp(&(right.span.start.offset, right.span.end.offset))
            })
        });
        references.dedup();
        Some(Ok(ExactFunctionRename {
            references,
            relevant_paths,
        }))
    }

    fn build_exact_reference_index(
        &self,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<ExactReferenceIndex> {
        let context = self.exact_workspace_context(is_cancelled)?;
        let mut paths = self.files.keys().cloned().collect::<Vec<_>>();
        paths.sort();
        let mut references = ExactReferenceIndex::default();
        for path in paths {
            if is_cancelled() {
                return None;
            }
            let mut call_spans = self
                .files
                .get(&path)?
                .call_sites
                .iter()
                .map(|site| site.name_span)
                .collect::<Vec<_>>();
            call_spans.sort_by_key(|span| (span.start.offset, span.end.offset));
            call_spans.dedup();
            for span in call_spans {
                if is_cancelled() {
                    return None;
                }
                let Some(target) =
                    self.resolve_call_site_exact_with_context(&path, span, context, is_cancelled)
                else {
                    continue;
                };
                references
                    .entry((target.path, target.node))
                    .or_default()
                    .push(ExactFunctionReference {
                        path: path.clone(),
                        span,
                    });
            }
        }
        Some(references)
    }

    fn resolve_exact_events(
        &self,
        path: &Path,
        callee: &Name,
        query: ExactQuery<'_>,
        stack: &mut FxHashSet<PathBuf>,
    ) -> ExactResolution {
        let Some(facts) = self.files.get(path) else {
            return ExactResolution::Ambiguous;
        };
        if !facts.analyzable
            || facts.source_effects.iter().any(|effect| {
                !effect.persistent
                    && ((effect.enclosing_function.is_none()
                        && effect.span.start.offset < query.top_level_cutoff)
                        || (effect.enclosing_function.as_ref() == query.enclosing_function
                            && query
                                .local_call_offset
                                .is_some_and(|offset| effect.span.start.offset < offset)))
            })
        {
            return ExactResolution::Ambiguous;
        }

        enum Event<'a> {
            Definition(&'a CallFactDefinition),
            Source(&'a CallFactSourceEffect),
        }

        let mut events = Vec::new();
        if query.include_top_level_definitions {
            for definition in facts.definitions.iter().filter(|definition| {
                definition.persistent_top_level
                    && &definition.name == callee
                    && definition.def_span.start.offset < query.top_level_cutoff
            }) {
                events.push((
                    definition.def_span.start.offset,
                    Event::Definition(definition),
                ));
            }
        } else if let Some(definition_span) = query.local_definition
            && let Some(definition) = facts
                .definitions
                .iter()
                .find(|definition| definition.def_span == definition_span)
        {
            events.push((
                definition.def_span.start.offset,
                Event::Definition(definition),
            ));
        }
        for effect in facts.source_effects.iter().filter(|effect| {
            effect.persistent
                && ((effect.enclosing_function.is_none()
                    && effect.span.start.offset < query.top_level_cutoff)
                    || (effect.enclosing_function.as_ref() == query.enclosing_function
                        && query
                            .local_call_offset
                            .is_some_and(|offset| effect.span.start.offset < offset)))
        }) {
            events.push((effect.span.start.offset, Event::Source(effect)));
        }
        events.sort_by_key(|(offset, _)| *offset);

        for (_, event) in events.into_iter().rev() {
            match event {
                Event::Definition(definition) => {
                    if !definition.unconditional {
                        return ExactResolution::Ambiguous;
                    }
                    return ExactResolution::Defined(ResolvedDefinition {
                        path: path.to_path_buf(),
                        def_span: definition.def_span,
                    });
                }
                Event::Source(effect) => {
                    let Some(target_path) = effect.path.as_deref() else {
                        return ExactResolution::Ambiguous;
                    };
                    let target = self.resolve_exported_exact(target_path, callee, stack);
                    if effect.conditional {
                        match target {
                            ExactResolution::Absent => continue,
                            ExactResolution::Defined(_) | ExactResolution::Ambiguous => {
                                return ExactResolution::Ambiguous;
                            }
                        }
                    }
                    match target {
                        ExactResolution::Defined(target) => {
                            return ExactResolution::Defined(target);
                        }
                        ExactResolution::Absent => continue,
                        ExactResolution::Ambiguous => return ExactResolution::Ambiguous,
                    }
                }
            }
        }
        ExactResolution::Absent
    }

    fn resolve_exported_exact(
        &self,
        path: &Path,
        callee: &Name,
        stack: &mut FxHashSet<PathBuf>,
    ) -> ExactResolution {
        if !stack.insert(path.to_path_buf()) {
            return ExactResolution::Ambiguous;
        }
        let resolved =
            self.resolve_exact_events(path, callee, ExactQuery::top_level(usize::MAX), stack);
        stack.remove(path);
        resolved
    }

    /// Resolves through source edges that execute before `cutoff`. A `None`
    /// cutoff represents the file's final sourced environment, which is also
    /// the conservative model for calls inside deferred function bodies.
    fn resolve_through_edges_before(
        &self,
        from_path: &Path,
        callee: &Name,
        cutoff: Option<usize>,
    ) -> SourceResolution {
        let facts = self.files.get(from_path)?;
        let mut stack = FxHashSet::default();
        stack.insert(from_path.to_path_buf());
        facts
            .source_edges
            .iter()
            .rev()
            .filter(|edge| {
                edge.span == Span::new()
                    || cutoff.is_none_or(|offset| edge.span.start.offset < offset)
            })
            .find_map(|edge| {
                self.resolve_exported(&edge.path, callee, &mut stack)
                    .map(|target| (target, edge.span))
            })
    }

    /// Resolves the final exported binding from one sourced file. Definitions
    /// and nested source edges are evaluated in reverse execution order, so the
    /// first successful event is the binding shell execution leaves visible.
    fn resolve_exported(
        &self,
        path: &Path,
        callee: &Name,
        stack: &mut FxHashSet<PathBuf>,
    ) -> Option<ResolvedDefinition> {
        if !stack.insert(path.to_path_buf()) {
            return None;
        }
        let Some(facts) = self.files.get(path) else {
            stack.remove(path);
            return None;
        };

        enum Event<'a> {
            Definition(&'a CallFactDefinition),
            Source(&'a CallFactSourceEdge),
        }

        let mut events = Vec::new();
        for definition in facts.definitions.iter().filter(|def| &def.name == callee) {
            events.push((
                definition.def_span.start.offset,
                Event::Definition(definition),
            ));
        }
        for edge in &facts.source_edges {
            events.push((edge.span.start.offset, Event::Source(edge)));
        }
        events.sort_by_key(|(offset, _)| *offset);

        for (_, event) in events.into_iter().rev() {
            let resolved = match event {
                Event::Definition(definition) => Some(ResolvedDefinition {
                    path: path.to_path_buf(),
                    def_span: definition.def_span,
                }),
                Event::Source(edge) => self.resolve_exported(&edge.path, callee, stack),
            };
            if resolved.is_some() {
                stack.remove(path);
                return resolved;
            }
        }
        stack.remove(path);
        None
    }

    /// Returns the functions that the node `from_kind` in `from_path` calls,
    /// grouped by callee. Callees that do not resolve to a defined function
    /// (builtins, external commands) are omitted.
    pub fn outgoing(&self, from_path: &Path, from_kind: &CallNodeKind) -> Vec<CrossFileCall> {
        let Some(facts) = self.files.get(from_path) else {
            return Vec::new();
        };

        let mut order: Vec<(PathBuf, CallFunctionId)> = Vec::new();
        let mut spans: FxHashMap<(PathBuf, CallFunctionId), Vec<Span>> = FxHashMap::default();
        // Resolution is per-callee, not per-site: memoize it so repeated calls
        // to the same helper do not re-run the source-edge search.
        let mut resolved: FxHashMap<(Name, Option<usize>), SourceResolution> = FxHashMap::default();
        for site in &facts.call_sites {
            if &site.enclosing != from_kind {
                continue;
            }
            let cutoff = match site.enclosing {
                CallNodeKind::TopLevel => Some(site.name_span.start.offset),
                CallNodeKind::Function(_) => None,
            };
            let sourced = resolved
                .entry((site.callee.clone(), cutoff))
                .or_insert_with(|| {
                    self.resolve_through_edges_before(from_path, &site.callee, cutoff)
                })
                .clone();
            let target = choose_resolved_target(from_path, site.local_definition_span, sourced);
            let Some(target) = target else {
                continue;
            };
            let target_path = target.path;
            let function = CallFunctionId::new(site.callee.clone(), target.def_span);
            let key = (target_path, function);
            spans
                .entry(key.clone())
                .or_insert_with(|| {
                    order.push(key);
                    Vec::new()
                })
                .push(site.name_span);
        }

        order
            .into_iter()
            .map(|(target_path, function)| {
                let definition = self
                    .files
                    .get(&target_path)
                    .and_then(|facts| facts.definition(&function));
                let call_spans = spans
                    .remove(&(target_path.clone(), function.clone()))
                    .unwrap_or_default();
                CrossFileCall {
                    path: target_path,
                    def_span: definition.map(|def| def.def_span),
                    selection_span: definition.map(|def| def.selection_span),
                    node: CallNodeKind::Function(function),
                    call_spans,
                }
            })
            .collect()
    }

    /// Returns callers of the exact function node in `target_path`, grouped by
    /// caller node. A caller is any file that transitively sources
    /// `target_path` and calls that binding without a nearer shadowing
    /// definition. Top-level nodes have no callers.
    pub fn incoming(&self, target_path: &Path, target_node: &CallNodeKind) -> Vec<CrossFileCall> {
        let CallNodeKind::Function(target_function) = target_node else {
            return Vec::new();
        };
        let name = &target_function.name;
        let mut order: Vec<(PathBuf, CallNodeKind)> = Vec::new();
        let mut spans: FxHashMap<(PathBuf, CallNodeKind), Vec<Span>> = FxHashMap::default();

        let mut caller_paths: Vec<&PathBuf> = self.files.keys().collect();
        caller_paths.sort();
        for caller_path in caller_paths {
            let facts = &self.files[caller_path];
            // Edge resolution is independent of the site, so compute it at
            // most once per caller file rather than per call site.
            let mut edge_resolutions: FxHashMap<Option<usize>, SourceResolution> =
                FxHashMap::default();
            for site in &facts.call_sites {
                if &site.callee != name {
                    continue;
                }
                let cutoff = match site.enclosing {
                    CallNodeKind::TopLevel => Some(site.name_span.start.offset),
                    CallNodeKind::Function(_) => None,
                };
                let sourced = edge_resolutions
                    .entry(cutoff)
                    .or_insert_with(|| self.resolve_through_edges_before(caller_path, name, cutoff))
                    .clone();
                let resolves =
                    choose_resolved_target(caller_path, site.local_definition_span, sourced)
                        .is_some_and(|target| {
                            target.path.as_path() == target_path
                                && target.def_span.start.offset == target_function.definition_start
                                && target.def_span.end.offset == target_function.definition_end
                        });
                if !resolves {
                    continue;
                }
                let key = (caller_path.clone(), site.enclosing.clone());
                spans
                    .entry(key.clone())
                    .or_insert_with(|| {
                        order.push(key);
                        Vec::new()
                    })
                    .push(site.name_span);
            }
        }

        order
            .into_iter()
            .map(|(caller_path, node)| {
                let definition = match &node {
                    CallNodeKind::Function(function) => self
                        .files
                        .get(&caller_path)
                        .and_then(|facts| facts.definition(function)),
                    CallNodeKind::TopLevel => None,
                };
                let call_spans = spans
                    .remove(&(caller_path.clone(), node.clone()))
                    .unwrap_or_default();
                CrossFileCall {
                    path: caller_path,
                    node,
                    def_span: definition.map(|def| def.def_span),
                    selection_span: definition.map(|def| def.selection_span),
                    call_spans,
                }
            })
            .collect()
    }
}

fn choose_resolved_target(
    from_path: &Path,
    local_definition: Option<Span>,
    sourced: SourceResolution,
) -> Option<ResolvedDefinition> {
    match (local_definition, sourced) {
        (Some(definition), Some((target, source_span)))
            if source_span != Span::new() && source_span.start.offset > definition.start.offset =>
        {
            Some(target)
        }
        (Some(def_span), _) => Some(ResolvedDefinition {
            path: from_path.to_path_buf(),
            def_span,
        }),
        (None, Some((target, _))) => Some(target),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::*;

    fn facts(source: &str, edges: &[&str]) -> FileCallFacts {
        let output = Parser::with_dialect(source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);
        FileCallFacts::project(&model, edges.iter().map(PathBuf::from).collect())
    }

    fn facts_with_positioned_sources(source: &str, paths: &[&str]) -> FileCallFacts {
        let output = Parser::with_dialect(source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);
        let edges = model
            .source_refs()
            .iter()
            .zip(paths)
            .map(|(source_ref, path)| {
                let scope = model.scope_at(source_ref.span.start.offset);
                CallFactSourceEdge {
                    path: PathBuf::from(path),
                    span: source_ref.span,
                    conditional: source_ref.conditionally_executed,
                    completion_visible: !source_ref.conditionally_executed
                        && model.enclosing_function_scope(scope).is_none()
                        && model
                            .innermost_transient_scope_within_function(scope)
                            .is_none(),
                }
            })
            .collect();
        FileCallFacts::project_with_source_edges(&model, edges)
    }
    fn name(text: &str) -> Name {
        Name::from(text)
    }

    fn function_node(
        index: &WorkspaceCallIndex,
        path: &str,
        function_name: &str,
        occurrence: usize,
    ) -> CallNodeKind {
        let facts = index
            .files()
            .find_map(|(candidate, facts)| (candidate == Path::new(path)).then_some(facts))
            .expect("file should be indexed");
        let definition = facts
            .definitions
            .iter()
            .filter(|definition| definition.name == name(function_name))
            .nth(occurrence)
            .expect("function definition should be indexed");
        CallNodeKind::Function(definition.identity())
    }

    fn function_name(node: &CallNodeKind) -> Option<&str> {
        match node {
            CallNodeKind::Function(function) => Some(function.name.as_str()),
            CallNodeKind::TopLevel => None,
        }
    }

    #[test]
    fn projects_definitions_call_sites_and_enclosing() {
        let facts = facts(
            "inner() { :; }\nouter() {\n  inner\n  nested() { inner; }\n}\nouter\n",
            &[],
        );
        let def_names: Vec<_> = facts
            .definitions
            .iter()
            .map(|def| def.name.to_string())
            .collect();
        assert!(def_names.contains(&"inner".to_owned()));
        assert!(def_names.contains(&"outer".to_owned()));

        // `outer` calls `inner`; the `inner` in `nested` is enclosed by `nested`;
        // the trailing `outer` call is top level.
        let enclosings: Vec<_> = facts
            .call_sites
            .iter()
            .map(|site| (site.callee.to_string(), site.enclosing.clone()))
            .collect();
        assert!(enclosings.iter().any(|(callee, enclosing)| {
            callee == "inner" && function_name(enclosing) == Some("outer")
        }));
        assert!(enclosings.iter().any(|(callee, enclosing)| {
            callee == "inner" && function_name(enclosing) == Some("nested")
        }));
        assert!(enclosings.contains(&("outer".to_owned(), CallNodeKind::TopLevel)));
    }

    fn three_file_index() -> WorkspaceCallIndex {
        // a.sh defines greet; b.sh follows a and calls greet; c.sh assumes a and
        // calls greet. Edges are supplied pre-resolved (as the server would).
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            PathBuf::from("/w/a.sh"),
            facts("greet() { echo hi; }\n", &[]),
        );
        index.insert(
            PathBuf::from("/w/b.sh"),
            facts("run() {\n  greet\n}\nrun\n", &["/w/a.sh"]),
        );
        index.insert(PathBuf::from("/w/c.sh"), facts("greet\n", &["/w/a.sh"]));
        index
    }

    #[test]
    fn resolve_finds_definition_through_source_edge() {
        let index = three_file_index();
        assert_eq!(
            index.resolve(Path::new("/w/b.sh"), &name("greet")),
            Some(PathBuf::from("/w/a.sh"))
        );
        // A builtin/external name resolves nowhere.
        assert_eq!(index.resolve(Path::new("/w/b.sh"), &name("echo")), None);
    }

    #[test]
    fn incoming_collects_callers_across_files_including_assume_and_top_level() {
        let index = three_file_index();
        let greet = function_node(&index, "/w/a.sh", "greet", 0);
        let incoming = index.incoming(Path::new("/w/a.sh"), &greet);
        let mut callers: Vec<String> = incoming
            .iter()
            .map(|call| {
                format!(
                    "{}:{}",
                    call.path.to_string_lossy(),
                    function_name(&call.node).unwrap_or("top-level")
                )
            })
            .collect();
        callers.sort();
        // b.sh's `run` (follow edge) and c.sh's top level (assume edge).
        assert_eq!(
            callers,
            vec!["/w/b.sh:run".to_owned(), "/w/c.sh:top-level".to_owned(),]
        );
    }

    #[test]
    fn outgoing_descends_into_followed_file() {
        let index = three_file_index();
        let run = function_node(&index, "/w/b.sh", "run", 0);
        let outgoing = index.outgoing(Path::new("/w/b.sh"), &run);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].path, PathBuf::from("/w/a.sh"));
        assert_eq!(function_name(&outgoing[0].node), Some("greet"));
        assert_eq!(outgoing[0].call_spans.len(), 1);
        assert!(outgoing[0].def_span.is_some());
    }

    #[test]
    fn local_definition_shadows_cross_file_one() {
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            PathBuf::from("/w/a.sh"),
            facts("greet() { echo a; }\n", &[]),
        );
        // b defines its own greet, so its call resolves locally, not to a.sh.
        let caller_path = PathBuf::from("/w/b.sh");
        let caller_facts = facts("greet() { echo b; }\ngreet\n", &["/w/a.sh"]);
        let call_span = caller_facts
            .call_sites
            .iter()
            .find(|site| site.callee == name("greet"))
            .expect("greet call should be projected")
            .name_span;
        index.insert(caller_path.clone(), caller_facts);
        assert_eq!(
            index.resolve(&caller_path, &name("greet")),
            Some(caller_path.clone())
        );
        assert_eq!(
            index
                .resolve_call_site(&caller_path, call_span)
                .map(|call| call.path),
            Some(caller_path)
        );
        // a.sh's greet therefore has no incoming call from b.sh.
        let sourced_greet = function_node(&index, "/w/a.sh", "greet", 0);
        assert!(
            index
                .incoming(Path::new("/w/a.sh"), &sourced_greet)
                .is_empty()
        );
    }

    #[test]
    fn same_named_definitions_keep_distinct_recursive_edges() {
        let source = "left() { :; }\nright() { :; }\nworker() { left; worker; }\nworker\nworker() { right; worker; }\nworker\n";
        let path = PathBuf::from("/w/script.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(path.clone(), facts(source, &[]));

        let first_worker = function_node(&index, "/w/script.sh", "worker", 0);
        let second_worker = function_node(&index, "/w/script.sh", "worker", 1);

        let first_outgoing = index.outgoing(&path, &first_worker);
        assert_eq!(
            first_outgoing
                .iter()
                .filter_map(|call| function_name(&call.node))
                .collect::<Vec<_>>(),
            ["left", "worker"]
        );
        assert!(first_outgoing.iter().any(|call| call.node == first_worker));
        assert!(!first_outgoing.iter().any(|call| call.node == second_worker));

        let second_outgoing = index.outgoing(&path, &second_worker);
        assert_eq!(
            second_outgoing
                .iter()
                .filter_map(|call| function_name(&call.node))
                .collect::<Vec<_>>(),
            ["right", "worker"]
        );
        assert!(
            second_outgoing
                .iter()
                .any(|call| call.node == second_worker)
        );
        assert!(!second_outgoing.iter().any(|call| call.node == first_worker));

        let first_incoming = index.incoming(&path, &first_worker);
        assert_eq!(first_incoming.len(), 2, "recursive and top-level callers");
        assert_eq!(
            first_incoming
                .iter()
                .map(|call| call.call_spans.len())
                .sum::<usize>(),
            2
        );
        let second_incoming = index.incoming(&path, &second_worker);
        assert_eq!(second_incoming.len(), 2, "recursive and top-level callers");
        assert_eq!(
            second_incoming
                .iter()
                .map(|call| call.call_spans.len())
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn sourced_redefinitions_report_incoming_calls_only_for_final_binding() {
        let target_path = PathBuf::from("/w/lib.sh");
        let caller_path = PathBuf::from("/w/main.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            target_path.clone(),
            facts("greet() { echo first; }\ngreet() { echo final; }\n", &[]),
        );
        index.insert(caller_path, facts("greet\n", &["/w/lib.sh"]));

        let first = function_node(&index, "/w/lib.sh", "greet", 0);
        let final_binding = function_node(&index, "/w/lib.sh", "greet", 1);
        assert!(index.incoming(&target_path, &first).is_empty());
        let incoming = index.incoming(&target_path, &final_binding);
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].call_spans.len(), 1);
    }

    #[test]
    fn replaced_file_does_not_resolve_a_stale_function_identity_by_name() {
        let path = PathBuf::from("/w/script.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            path.clone(),
            facts("worker() { helper; }\nhelper() { :; }\nworker\n", &[]),
        );
        let stale_worker = function_node(&index, "/w/script.sh", "worker", 0);

        index.insert(
            path.clone(),
            facts(
                "# moved\nworker() { helper; }\nhelper() { :; }\nworker\n",
                &[],
            ),
        );

        assert!(index.outgoing(&path, &stale_worker).is_empty());
        assert!(index.incoming(&path, &stale_worker).is_empty());
    }

    #[test]
    fn unresolved_dynamic_source_yields_no_cross_file_edge() {
        // b sources a computed path with no hint: no edge is supplied, so greet
        // stays unresolved and a.sh sees no caller.
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            PathBuf::from("/w/a.sh"),
            facts("greet() { echo hi; }\n", &[]),
        );
        let caller_path = PathBuf::from("/w/b.sh");
        let caller_facts = facts("greet\n", &[]);
        let call_span = caller_facts.call_sites[0].name_span;
        index.insert(caller_path.clone(), caller_facts);
        assert_eq!(index.resolve(&caller_path, &name("greet")), None);
        assert!(index.resolve_call_site(&caller_path, call_span).is_none());
        let greet = function_node(&index, "/w/a.sh", "greet", 0);
        assert!(index.incoming(Path::new("/w/a.sh"), &greet).is_empty());
    }

    #[test]
    fn exact_resolution_rejects_dynamic_source_that_may_replace_a_local_function() {
        let path = PathBuf::from("/w/main.sh");
        let caller = facts("foo() { :; }\nsource \"$plugin\"\nfoo\n", &[]);
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(path.clone(), caller);

        assert!(index.resolve_call_site(&path, call_span).is_some());
        assert!(index.resolve_call_site_exact(&path, call_span).is_none());
    }

    #[test]
    fn exact_resolution_rejects_conditional_source_and_conditional_export() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/lib.sh");
        let caller = facts_with_positioned_sources(
            "foo() { :; }\nif enabled; then source lib.sh; fi\nfoo\n",
            &["/w/lib.sh"],
        );
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(target_path, facts("foo() { echo sourced; }\n", &[]));
        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );

        let caller = facts_with_positioned_sources("source lib.sh\nfoo\n", &["/w/lib.sh"]);
        let call_span = caller.call_sites[0].name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(
            PathBuf::from("/w/lib.sh"),
            facts("if enabled; then foo() { :; }; fi\n", &[]),
        );
        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn exact_resolution_fails_closed_for_later_sources_after_deferred_definition() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/lib.sh");
        let caller =
            facts_with_positioned_sources("run() { foo; }\nrun\nsource lib.sh\n", &["/w/lib.sh"]);
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));

        assert_eq!(
            index
                .resolve_call_site(&caller_path, call_span)
                .map(|call| call.path),
            Some(target_path)
        );
        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn exact_resolution_accepts_source_before_deferred_function_definition() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/lib.sh");
        let caller =
            facts_with_positioned_sources("source lib.sh\nrun() { foo; }\nrun\n", &["/w/lib.sh"]);
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));

        assert_eq!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .map(|call| call.path),
            Some(target_path)
        );
    }

    #[test]
    fn exact_resolution_scopes_function_local_sources_to_the_containing_function() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/lib.sh");
        let caller = facts_with_positioned_sources(
            "unused() { source other.sh; }\nrun() { source lib.sh; foo; }\n",
            &["/w/other.sh", "/w/lib.sh"],
        );
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(PathBuf::from("/w/other.sh"), facts(":\n", &[]));
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));

        assert_eq!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .map(|call| call.path),
            Some(target_path)
        );
    }

    #[test]
    fn exact_resolution_rejects_later_source_in_a_repeated_function_call() {
        let caller_path = PathBuf::from("/w/main.sh");
        let caller = facts_with_positioned_sources(
            "foo() { :; }\nrun() { foo; source lib.sh; }\nrun\nrun\n",
            &["/w/lib.sh"],
        );
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(
            PathBuf::from("/w/lib.sh"),
            facts("foo() { echo sourced; }\n", &[]),
        );

        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn exact_resolution_rejects_an_invoked_source_bearing_function() {
        let caller_path = PathBuf::from("/w/main.sh");
        let caller = facts_with_positioned_sources(
            "foo() { :; }\nload() { source lib.sh; }\nrun() { foo; }\nrun\nload\nrun\n",
            &["/w/lib.sh"],
        );
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(
            PathBuf::from("/w/lib.sh"),
            facts("foo() { echo sourced; }\n", &[]),
        );

        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn exact_resolution_rejects_a_called_sourced_function_that_can_mutate_bindings() {
        let caller_path = PathBuf::from("/w/main.sh");
        let library_path = PathBuf::from("/w/lib.sh");
        let caller = facts_with_positioned_sources(
            "source lib.sh\nfoo() { :; }\nrun() { foo; }\nrun\nload\nrun\n",
            &["/w/lib.sh"],
        );
        let library =
            facts_with_positioned_sources("load() { source override.sh; }\n", &["/w/override.sh"]);
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(library_path, library);
        index.insert(
            PathBuf::from("/w/override.sh"),
            facts("foo() { echo sourced; }\n", &[]),
        );

        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn exact_resolution_rejects_dynamic_dispatch_to_a_source_bearing_function() {
        let caller_path = PathBuf::from("/w/main.sh");
        let caller = facts_with_positioned_sources(
            "foo() { :; }\nload() { source lib.sh; }\nrun() { foo; }\nrun\nname=load\n\"$name\"\nrun\n",
            &["/w/lib.sh"],
        );
        let call_span = caller
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(
            PathBuf::from("/w/lib.sh"),
            facts("foo() { echo sourced; }\n", &[]),
        );

        assert!(
            index
                .resolve_call_site_exact(&caller_path, call_span)
                .is_none()
        );
    }

    #[test]
    fn sourced_completion_applies_cursor_order_and_duplicate_shadowing() {
        let caller_path = PathBuf::from("/w/main.sh");
        let a_path = PathBuf::from("/w/a.sh");
        let b_path = PathBuf::from("/w/b.sh");
        let source = "source a.sh\nmarker\nsource b.sh\nend\n";
        let caller = facts_with_positioned_sources(source, &["/w/a.sh", "/w/b.sh"]);
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(
            a_path.clone(),
            facts("dup() { :; }\nonly_a() { :; }\n", &[]),
        );
        index.insert(b_path.clone(), facts("dup() { :; }\n", &[]));

        let before_first = index.visible_sourced_functions(&caller_path, 0);
        assert!(before_first.is_empty());

        let between = index.visible_sourced_functions(&caller_path, source.find("marker").unwrap());
        assert_eq!(between.len(), 2);
        assert!(between.iter().all(|function| function.path == a_path));

        let after = index.visible_sourced_functions(&caller_path, source.len());
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .find(|function| function.name == name("dup"))
                .map(|function| &function.path),
            Some(&b_path)
        );
        assert_eq!(
            after
                .iter()
                .find(|function| function.name == name("only_a"))
                .map(|function| &function.path),
            Some(&a_path)
        );
    }

    #[test]
    fn sourced_completion_skips_conditional_edges_and_handles_cycles() {
        let caller_path = PathBuf::from("/w/main.sh");
        let a_path = PathBuf::from("/w/a.sh");
        let b_path = PathBuf::from("/w/b.sh");
        let conditional_path = PathBuf::from("/w/conditional.sh");
        let caller = facts_with_positioned_sources(
            "if enabled; then source conditional.sh; fi\nsource a.sh\n",
            &["/w/conditional.sh", "/w/a.sh"],
        );
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(conditional_path, facts("conditional_only() { :; }\n", &[]));
        index.insert(
            a_path.clone(),
            facts_with_positioned_sources("source b.sh\na_only() { :; }\n", &["/w/b.sh"]),
        );
        index.insert(
            b_path,
            facts_with_positioned_sources("source a.sh\nb_only() { :; }\n", &["/w/a.sh"]),
        );

        let visible = index.visible_sourced_functions(&caller_path, usize::MAX);
        let names = visible
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["a_only", "b_only"]);
    }

    #[test]
    fn sourced_completion_skips_unexecuted_function_and_subshell_sources() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/a.sh");
        let caller = facts_with_positioned_sources(
            "load() { source a.sh; }\n( source a.sh )\nmarker\n",
            &["/w/a.sh", "/w/a.sh"],
        );
        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller);
        index.insert(target_path, facts("not_visible() { :; }\n", &[]));

        assert!(
            index
                .visible_sourced_functions(&caller_path, usize::MAX)
                .is_empty()
        );
    }

    #[test]
    fn exact_reference_index_preserves_binding_identity_and_source_order() {
        let a_path = PathBuf::from("/w/a.sh");
        let b_path = PathBuf::from("/w/b.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let local_path = PathBuf::from("/w/local.sh");
        let caller_source = "source a.sh\nfoo\nsource b.sh\nfoo\n";
        let caller = facts_with_positioned_sources(caller_source, &["/w/a.sh", "/w/b.sh"]);
        let caller_spans = caller
            .call_sites
            .iter()
            .filter(|site| site.callee == name("foo"))
            .map(|site| site.name_span)
            .collect::<Vec<_>>();

        let mut index = WorkspaceCallIndex::new();
        index.insert(a_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(b_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(caller_path.clone(), caller);
        index.insert(
            local_path,
            facts_with_positioned_sources(
                "source a.sh\nfoo() { echo local; }\nfoo\n",
                &["/w/a.sh"],
            ),
        );

        let a_node = function_node(&index, "/w/a.sh", "foo", 0);
        let a_references = index
            .exact_function_references(&a_path, &a_node, || false)
            .expect("reference build should complete");
        assert_eq!(
            a_references,
            &[ExactFunctionReference {
                path: caller_path.clone(),
                span: caller_spans[0],
            }]
        );

        let b_node = function_node(&index, "/w/b.sh", "foo", 0);
        let b_references = index
            .exact_function_references(&b_path, &b_node, || false)
            .expect("cached reference lookup should complete");
        assert_eq!(
            b_references,
            &[ExactFunctionReference {
                path: caller_path,
                span: caller_spans[1],
            }]
        );
    }

    #[test]
    fn exact_function_rename_rejects_ambiguous_calls_in_the_source_graph() {
        let target_path = PathBuf::from("/w/a.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(
            caller_path,
            facts_with_positioned_sources("if true; then source a.sh; fi\nfoo\n", &["/w/a.sh"]),
        );

        let target = function_node(&index, "/w/a.sh", "foo", 0);
        assert_eq!(
            index.exact_function_rename(&target_path, &target, || false),
            Some(Err(ExactFunctionRenameError::AmbiguousReference))
        );
    }

    #[test]
    fn exact_function_rename_rejects_an_unresolved_source_edge() {
        let target_path = PathBuf::from("/w/a.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            target_path.clone(),
            facts("foo() { :; }\nfoo\nsource \"$dynamic\"\n", &[]),
        );

        let target = function_node(&index, "/w/a.sh", "foo", 0);
        assert_eq!(
            index.exact_function_rename(&target_path, &target, || false),
            Some(Err(ExactFunctionRenameError::IncompleteSourceGraph))
        );
    }

    #[test]
    fn exact_function_rename_rejects_an_unindexed_source_target() {
        let target_path = PathBuf::from("/w/a.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            target_path.clone(),
            facts_with_positioned_sources(
                "foo() { :; }\nfoo\nsource missing.sh\n",
                &["/w/missing.sh"],
            ),
        );

        let target = function_node(&index, "/w/a.sh", "foo", 0);
        assert_eq!(
            index.exact_function_rename(&target_path, &target, || false),
            Some(Err(ExactFunctionRenameError::IncompleteSourceGraph))
        );
    }

    #[test]
    fn exact_function_rename_ignores_disconnected_ambiguous_calls() {
        let target_path = PathBuf::from("/w/main.sh");
        let unrelated_path = PathBuf::from("/w/unrelated.sh");
        let main = facts("foo() { :; }\nfoo\n", &[]);
        let call_span = main.call_sites[0].name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(target_path.clone(), main);
        index.insert(unrelated_path, facts("source \"$dynamic\"\nfoo\n", &[]));

        let target = function_node(&index, "/w/main.sh", "foo", 0);
        assert_eq!(
            index
                .exact_function_rename(&target_path, &target, || false)
                .expect("rename analysis should complete")
                .expect("disconnected ambiguity should not block rename"),
            ExactFunctionRename {
                references: vec![ExactFunctionReference {
                    path: target_path.clone(),
                    span: call_span,
                }],
                relevant_paths: vec![target_path],
            }
        );
    }

    #[test]
    fn exact_function_rename_does_not_connect_files_through_a_shared_child() {
        let target_path = PathBuf::from("/w/target.sh");
        let unrelated_path = PathBuf::from("/w/unrelated.sh");
        let common_path = PathBuf::from("/w/common.sh");
        let target = facts_with_positioned_sources(
            "source common.sh\nfoo() { :; }\nfoo\n",
            &["/w/common.sh"],
        );
        let target_call = target
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .unwrap()
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(target_path.clone(), target);
        index.insert(
            unrelated_path,
            facts_with_positioned_sources(
                "source common.sh\nsource \"$dynamic\"\nfoo\n",
                &["/w/common.sh"],
            ),
        );
        index.insert(common_path.clone(), facts(":\n", &[]));

        let target = function_node(&index, "/w/target.sh", "foo", 0);
        assert_eq!(
            index
                .exact_function_rename(&target_path, &target, || false)
                .expect("rename analysis should complete")
                .expect("a shared child must not import the target into its other parent"),
            ExactFunctionRename {
                references: vec![ExactFunctionReference {
                    path: target_path.clone(),
                    span: target_call,
                }],
                relevant_paths: vec![common_path, target_path],
            }
        );
    }

    #[test]
    fn exact_reference_index_inherits_bindings_at_source_sites() {
        let main_path = PathBuf::from("/w/main.sh");
        let child_path = PathBuf::from("/w/child.sh");
        let child = facts("foo\n", &[]);
        let child_call = child.call_sites[0].name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            main_path.clone(),
            facts_with_positioned_sources("foo() { :; }\nsource child.sh\n", &["/w/child.sh"]),
        );
        index.insert(child_path.clone(), child);

        let target = function_node(&index, "/w/main.sh", "foo", 0);
        assert_eq!(
            index
                .exact_function_references(&main_path, &target, || false)
                .expect("inherited source environment should resolve"),
            &[ExactFunctionReference {
                path: child_path,
                span: child_call,
            }]
        );
    }

    #[test]
    fn exact_reference_index_omits_deferred_calls_with_incoming_overrides() {
        let base_path = PathBuf::from("/w/base.sh");
        let lib_path = PathBuf::from("/w/lib.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(base_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(
            lib_path,
            facts_with_positioned_sources("source base.sh\nrun() { foo; }\n", &["/w/base.sh"]),
        );
        index.insert(
            caller_path,
            facts_with_positioned_sources(
                "source lib.sh\nfoo() { echo override; }\nrun\nsource base.sh\n",
                &["/w/lib.sh", "/w/base.sh"],
            ),
        );

        let target = function_node(&index, "/w/base.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&base_path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn exact_reference_index_rejects_top_level_source_function_side_effects() {
        let base_path = PathBuf::from("/w/base.sh");
        let override_path = PathBuf::from("/w/override.sh");
        let main_path = PathBuf::from("/w/main.sh");
        let child_path = PathBuf::from("/w/child.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(base_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(override_path, facts("foo() { echo override; }\n", &[]));
        index.insert(
            main_path,
            facts_with_positioned_sources(
                "source base.sh\nload() { source override.sh; }\nload\nfoo\nsource child.sh\n",
                &["/w/base.sh", "/w/override.sh", "/w/child.sh"],
            ),
        );
        index.insert(child_path, facts("foo\n", &[]));

        let target = function_node(&index, "/w/base.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&base_path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn invoked_nested_function_definitions_are_binding_mutators() {
        let path = PathBuf::from("/w/main.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            path.clone(),
            facts(
                "foo() { echo base; }\ninstall() { foo() { echo override; }; }\ninstall\nfoo\n",
                &[],
            ),
        );

        let target = function_node(&index, "/w/main.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn repeated_current_binding_mutator_does_not_get_excluded() {
        let path = PathBuf::from("/w/main.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            path.clone(),
            facts(
                "foo() { echo base; }\nrun() { foo; foo() { echo override; }; }\nrun\nrun\n",
                &[],
            ),
        );

        let target = function_node(&index, "/w/main.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn conditional_sources_still_propagate_possible_loader_side_effects() {
        let base_path = PathBuf::from("/w/base.sh");
        let override_path = PathBuf::from("/w/override.sh");
        let child_path = PathBuf::from("/w/child.sh");
        let main_path = PathBuf::from("/w/main.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(base_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(override_path, facts("foo() { echo override; }\n", &[]));
        index.insert(
            child_path,
            facts_with_positioned_sources(
                "load() { source override.sh; }\nload\n",
                &["/w/override.sh"],
            ),
        );
        index.insert(
            main_path,
            facts_with_positioned_sources(
                "source base.sh\nif enabled; then source child.sh; fi\nfoo\n",
                &["/w/base.sh", "/w/child.sh"],
            ),
        );

        let target = function_node(&index, "/w/base.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&base_path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn exact_reference_index_rejects_transitive_source_function_wrappers() {
        let base_path = PathBuf::from("/w/base.sh");
        let lib_path = PathBuf::from("/w/lib.sh");
        let override_path = PathBuf::from("/w/override.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(base_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(
            lib_path,
            facts_with_positioned_sources("source base.sh\nrun() { foo; }\n", &["/w/base.sh"]),
        );
        index.insert(override_path, facts("foo() { echo override; }\n", &[]));
        index.insert(
            caller_path,
            facts_with_positioned_sources(
                "source lib.sh\nload() { source override.sh; }\nouter() { load; }\nouter\nrun\n",
                &["/w/lib.sh", "/w/override.sh"],
            ),
        );

        let target = function_node(&index, "/w/base.sh", "foo", 0);
        assert!(
            index
                .exact_function_references(&base_path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn source_wrapper_cycles_do_not_cache_query_order_dependent_negatives() {
        let path = PathBuf::from("/w/main.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            path.clone(),
            facts_with_positioned_sources(
                "a() { b; load; }\nb() { a; }\nload() { source plugin.sh; }\n",
                &["/w/plugin.sh"],
            ),
        );
        index.insert(PathBuf::from("/w/plugin.sh"), facts(":\n", &[]));
        let context = ExactWorkspaceContext::build(&index.files, &|| false)
            .expect("exact context should build");

        assert_eq!(
            context.name_may_mutate(&path, &name("a"), &|| false),
            Some(true)
        );
        assert_eq!(
            context.name_may_mutate(&path, &name("b"), &|| false),
            Some(true)
        );
    }

    #[test]
    fn deep_mutator_wrapper_walk_is_iterative_and_cancellable() {
        let path = PathBuf::from("/w/main.sh");
        let depth = 4_096usize;
        let mut call_sites = Vec::with_capacity(depth - 1);
        for index in 0..depth - 1 {
            call_sites.push(CallFactSite {
                callee: name(&format!("wrapper_{}", index + 1)),
                name_span: Span::new(),
                enclosing: CallNodeKind::Function(CallFunctionId::new(
                    name(&format!("wrapper_{index}")),
                    Span::new(),
                )),
                local_definition_span: None,
            });
        }
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            path.clone(),
            FileCallFacts {
                definitions: Vec::new(),
                call_sites,
                source_edges: Vec::new(),
                source_effects: Vec::new(),
                binding_mutators: vec![CallFunctionId::new(
                    name(&format!("wrapper_{}", depth - 1)),
                    Span::new(),
                )],
                has_dynamic_command_dispatch: false,
                analyzable: true,
            },
        );
        let context = ExactWorkspaceContext::build(&index.files, &|| false)
            .expect("exact context should build");
        let checks = std::cell::Cell::new(0usize);
        assert_eq!(
            context.name_may_mutate(&path, &name("wrapper_0"), &|| {
                checks.set(checks.get() + 1);
                checks.get() > 16
            }),
            None
        );
        assert_eq!(
            context.name_may_mutate(&path, &name("wrapper_0"), &|| false),
            Some(true)
        );
    }

    #[test]
    fn unrelated_source_functions_do_not_suppress_exact_references() {
        let main_path = PathBuf::from("/w/main.sh");
        let unrelated_path = PathBuf::from("/w/unrelated.sh");
        let plugin_path = PathBuf::from("/w/plugin.sh");
        let main = facts("foo() { :; }\nrun() { foo; }\nrun\n", &[]);
        let inner_call = main
            .call_sites
            .iter()
            .find(|site| site.callee == name("foo"))
            .expect("inner foo call")
            .name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(main_path.clone(), main);
        index.insert(
            unrelated_path,
            facts_with_positioned_sources(
                "load() { source plugin.sh; }\nload\n",
                &["/w/plugin.sh"],
            ),
        );
        index.insert(plugin_path, facts(":\n", &[]));

        let target = function_node(&index, "/w/main.sh", "foo", 0);
        assert_eq!(
            index
                .exact_function_references(&main_path, &target, || false)
                .expect("unrelated source closure should not interfere"),
            &[ExactFunctionReference {
                path: main_path,
                span: inner_call,
            }]
        );
    }

    #[test]
    fn exact_reference_index_cancels_without_caching_partial_results() {
        let target_path = PathBuf::from("/w/a.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(
            caller_path.clone(),
            facts_with_positioned_sources("source a.sh\nfoo\n", &["/w/a.sh"]),
        );
        let target = function_node(&index, "/w/a.sh", "foo", 0);
        let checks = std::cell::Cell::new(0usize);
        assert!(
            index
                .exact_function_references(&target_path, &target, || {
                    checks.set(checks.get() + 1);
                    checks.get() > 1
                })
                .is_none()
        );

        let references = index
            .exact_function_references(&target_path, &target, || false)
            .expect("retry should build a complete reverse index");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].path, caller_path);
        assert!(
            index
                .exact_function_rename(&target_path, &target, || true)
                .is_none()
        );
        let rename = index
            .exact_function_rename(&target_path, &target, || false)
            .expect("rename retry should complete")
            .expect("rename retry should remain exact");
        assert_eq!(rename.references, references);
    }

    #[test]
    fn exact_call_resolution_cancels_without_caching_partial_context() {
        let path = PathBuf::from("/w/main.sh");
        let facts = facts("foo() { :; }\nfoo\n", &[]);
        let call_span = facts.call_sites[0].name_span;
        let mut index = WorkspaceCallIndex::new();
        index.insert(path.clone(), facts);

        assert!(
            index
                .resolve_call_site_exact_cancellable(&path, call_span, || true)
                .is_none()
        );
        assert!(index.exact_context.get().is_none());
        assert!(
            index
                .resolve_call_site_exact_cancellable(&path, call_span, || false)
                .is_some()
        );
    }

    #[test]
    fn replacing_file_facts_invalidates_exact_reference_index() {
        let target_path = PathBuf::from("/w/a.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(target_path.clone(), facts("foo() { :; }\n", &[]));
        index.insert(
            caller_path.clone(),
            facts_with_positioned_sources("source a.sh\nfoo\n", &["/w/a.sh"]),
        );
        let target = function_node(&index, "/w/a.sh", "foo", 0);
        assert_eq!(
            index
                .exact_function_references(&target_path, &target, || false)
                .map(<[_]>::len),
            Some(1)
        );

        index.insert(caller_path, facts("unrelated\n", &[]));
        assert!(
            index
                .exact_function_references(&target_path, &target, || false)
                .is_some_and(<[_]>::is_empty)
        );
    }

    #[test]
    fn exact_reference_index_terminates_source_cycles() {
        let target_path = PathBuf::from("/w/a.sh");
        let cycle_path = PathBuf::from("/w/cycle.sh");
        let caller_path = PathBuf::from("/w/caller.sh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(
            target_path.clone(),
            facts_with_positioned_sources("source cycle.sh\nfoo() { :; }\n", &["/w/cycle.sh"]),
        );
        index.insert(
            cycle_path,
            facts_with_positioned_sources("source a.sh\n", &["/w/a.sh"]),
        );
        index.insert(
            caller_path.clone(),
            facts_with_positioned_sources("source a.sh\nfoo\n", &["/w/a.sh"]),
        );
        let target = function_node(&index, "/w/a.sh", "foo", 0);

        let references = index
            .exact_function_references(&target_path, &target, || false)
            .expect("cyclic graph should terminate");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].path, caller_path);
    }

    #[test]
    fn source_order_controls_cross_file_resolution_at_each_top_level_call() {
        let caller_source = "greet\nsource a.sh\ngreet\nsource c.sh\ngreet\n";
        let output = Parser::with_dialect(caller_source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(caller_source, &output);
        let caller = SemanticModel::build(&output.file, caller_source, &indexer);
        let edges = caller
            .source_refs()
            .iter()
            .zip(["/w/a.sh", "/w/c.sh"])
            .map(|(source_ref, path)| CallFactSourceEdge {
                path: PathBuf::from(path),
                span: source_ref.span,
                conditional: source_ref.conditionally_executed,
                completion_visible: !source_ref.conditionally_executed,
            })
            .collect();

        let caller_path = PathBuf::from("/w/main.sh");
        let a_path = PathBuf::from("/w/a.sh");
        let c_path = PathBuf::from("/w/c.sh");
        let mut index = WorkspaceCallIndex::new();
        let caller_facts = FileCallFacts::project_with_source_edges(&caller, edges);
        let call_spans = caller_facts
            .call_sites
            .iter()
            .filter(|site| site.callee == name("greet"))
            .map(|site| site.name_span)
            .collect::<Vec<_>>();
        index.insert(caller_path.clone(), caller_facts);
        index.insert(a_path.clone(), facts("greet() { echo a; }\n", &[]));
        index.insert(c_path.clone(), facts("greet() { echo c; }\n", &[]));

        // Before the first source there is no edge. Between the sources the
        // call lands in a.sh; after the second source its definition wins.
        let outgoing = index.outgoing(&caller_path, &CallNodeKind::TopLevel);
        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].path, a_path);
        assert_eq!(outgoing[0].call_spans.len(), 1);
        assert_eq!(outgoing[1].path, c_path);
        assert_eq!(outgoing[1].call_spans.len(), 1);
        assert_eq!(
            index.resolve(&caller_path, &name("greet")),
            Some(c_path.clone()),
            "the later sourced definition is the final visible binding"
        );
        assert_eq!(call_spans.len(), 3);
        assert!(
            index
                .resolve_call_site(&caller_path, call_spans[0])
                .is_none()
        );
        assert_eq!(
            index
                .resolve_call_site(&caller_path, call_spans[1])
                .map(|call| call.path),
            Some(a_path.clone())
        );
        assert_eq!(
            index
                .resolve_call_site(&caller_path, call_spans[2])
                .map(|call| call.path),
            Some(c_path.clone())
        );

        let greet_a = function_node(&index, "/w/a.sh", "greet", 0);
        let incoming_a = index.incoming(&a_path, &greet_a);
        assert_eq!(incoming_a.len(), 1);
        assert_eq!(incoming_a[0].call_spans.len(), 1);
        let greet_c = function_node(&index, "/w/c.sh", "greet", 0);
        let incoming_c = index.incoming(&c_path, &greet_c);
        assert_eq!(incoming_c.len(), 1);
        assert_eq!(incoming_c[0].call_spans.len(), 1);
    }

    #[test]
    fn later_source_and_local_definitions_override_each_other_in_order() {
        let caller_source =
            "greet() { echo local; }\nsource a.sh\ngreet\ngreet() { echo final; }\ngreet\n";
        let output = Parser::with_dialect(caller_source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(caller_source, &output);
        let caller = SemanticModel::build(&output.file, caller_source, &indexer);
        let edge = CallFactSourceEdge {
            path: PathBuf::from("/w/a.sh"),
            span: caller.source_refs()[0].span,
            conditional: false,
            completion_visible: true,
        };

        let caller_path = PathBuf::from("/w/main.sh");
        let a_path = PathBuf::from("/w/a.sh");
        let mut index = WorkspaceCallIndex::new();
        let caller_facts = FileCallFacts::project_with_source_edges(&caller, vec![edge]);
        let call_spans = caller_facts
            .call_sites
            .iter()
            .filter(|site| site.callee == name("greet"))
            .map(|site| site.name_span)
            .collect::<Vec<_>>();
        let final_definition_span = caller_facts
            .definitions
            .iter()
            .filter(|definition| definition.name == name("greet"))
            .max_by_key(|definition| definition.def_span.start.offset)
            .expect("final greet definition should be projected")
            .def_span;
        index.insert(caller_path.clone(), caller_facts);
        index.insert(a_path.clone(), facts("greet() { echo sourced; }\n", &[]));

        let outgoing = index.outgoing(&caller_path, &CallNodeKind::TopLevel);
        assert_eq!(outgoing.len(), 2);
        assert_eq!(
            outgoing[0].path, a_path,
            "source overrides the first local definition"
        );
        assert_eq!(outgoing[0].call_spans.len(), 1);
        assert_eq!(
            outgoing[1].path, caller_path,
            "the final local definition overrides the earlier source"
        );
        assert_eq!(outgoing[1].call_spans.len(), 1);
        assert_eq!(call_spans.len(), 2);
        assert_eq!(
            index
                .resolve_call_site(&caller_path, call_spans[0])
                .map(|call| call.path),
            Some(a_path)
        );
        let final_call = index
            .resolve_call_site(&caller_path, call_spans[1])
            .expect("final call should resolve locally");
        assert_eq!(final_call.path, caller_path);
        assert_eq!(final_call.def_span, Some(final_definition_span));
    }

    #[test]
    fn call_site_resolution_preserves_latest_sourced_definition_span() {
        let caller_path = PathBuf::from("/w/main.sh");
        let target_path = PathBuf::from("/w/a.sh");
        let caller_facts = facts("greet\n", &["/w/a.sh"]);
        let call_span = caller_facts.call_sites[0].name_span;
        let target_facts = facts("greet() { echo first; }\ngreet() { echo final; }\n", &[]);
        let final_definition = target_facts
            .definitions
            .iter()
            .max_by_key(|definition| definition.def_span.start.offset)
            .expect("final sourced definition should be projected")
            .clone();

        let mut index = WorkspaceCallIndex::new();
        index.insert(caller_path.clone(), caller_facts);
        index.insert(target_path.clone(), target_facts);

        let resolved = index
            .resolve_call_site(&caller_path, call_span)
            .expect("sourced call should resolve");
        assert_eq!(resolved.path, target_path);
        assert_eq!(resolved.def_span, Some(final_definition.def_span));
        assert_eq!(
            resolved.selection_span,
            Some(final_definition.selection_span)
        );
    }

    #[test]
    fn workspace_index_preserves_zsh_multi_name_function_bodies() {
        let source = "function music itunes() { helper; }\nhelper() { :; }\nitunes\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);
        let path = PathBuf::from("/w/script.zsh");
        let mut index = WorkspaceCallIndex::new();
        index.insert(path.clone(), FileCallFacts::project(&model, Vec::new()));

        let itunes = function_node(&index, "/w/script.zsh", "itunes", 0);
        let outgoing = index.outgoing(&path, &itunes);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(function_name(&outgoing[0].node), Some("helper"));

        let helper = function_node(&index, "/w/script.zsh", "helper", 0);
        let mut callers = index
            .incoming(&path, &helper)
            .into_iter()
            .filter_map(|call| function_name(&call.node).map(str::to_owned))
            .collect::<Vec<_>>();
        callers.sort();
        assert_eq!(callers, ["itunes", "music"]);
    }
}
