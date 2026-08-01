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

        Self {
            definitions,
            call_sites,
            source_edges,
            source_effects,
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

/// A workspace-wide index of per-file call facts.
#[derive(Debug, Default)]
pub struct WorkspaceCallIndex {
    files: FxHashMap<PathBuf, FileCallFacts>,
}

impl WorkspaceCallIndex {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces the facts for `path`.
    pub fn insert(&mut self, path: PathBuf, facts: FileCallFacts) {
        self.files.insert(path, facts);
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
        let facts = self.files.get(from_path)?;
        let site = facts
            .call_sites
            .iter()
            .find(|site| site.name_span == name_span)?;
        let mut stack = FxHashSet::default();
        stack.insert(from_path.to_path_buf());
        let resolution = match &site.enclosing {
            CallNodeKind::TopLevel => self.resolve_exact_events(
                from_path,
                &site.callee,
                ExactQuery::top_level(site.name_span.start.offset),
                &mut stack,
            ),
            CallNodeKind::Function(function) => {
                if self.called_source_function_may_mutate(from_path, function)
                    || facts.source_effects.iter().any(|effect| {
                        (effect.enclosing_function.is_none()
                            && effect.span.start.offset >= function.definition_start)
                            || (effect.enclosing_function.as_ref() == Some(function)
                                && effect.span.start.offset >= site.name_span.start.offset)
                    })
                {
                    return None;
                }
                self.resolve_exact_events(
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
                )
            }
        };
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

    fn called_source_function_may_mutate(
        &self,
        current_path: &Path,
        current_function: &CallFunctionId,
    ) -> bool {
        let called_names = self
            .files
            .values()
            .flat_map(|facts| facts.call_sites.iter().map(|site| &site.callee))
            .collect::<FxHashSet<_>>();
        let has_dynamic_dispatch = self
            .files
            .values()
            .any(|facts| facts.has_dynamic_command_dispatch);
        self.files.iter().any(|(path, facts)| {
            facts.source_effects.iter().any(|effect| {
                effect.enclosing_function.as_ref().is_some_and(|function| {
                    (path.as_path() != current_path || function != current_function)
                        && (has_dynamic_dispatch || called_names.contains(&function.name))
                })
            })
        })
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
            .map(|(source_ref, path)| CallFactSourceEdge {
                path: PathBuf::from(path),
                span: source_ref.span,
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
