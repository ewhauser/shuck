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

type SourceResolution = Option<(PathBuf, Span)>;

/// Identity of a call-graph node within a file: a named function, or the file's
/// top-level (module) body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallNodeKind {
    /// A named function defined in the file.
    Function(Name),
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

        let mut function_names_by_scope: FxHashMap<ScopeId, Vec<Name>> = FxHashMap::default();
        let mut definitions = Vec::new();
        for binding in model.function_definition_bindings() {
            definitions.push(CallFactDefinition {
                name: binding.name.clone(),
                def_span: binding_definition_span(binding),
                selection_span: binding.span,
            });
            if let Some(scope) = analysis.function_scope_for_binding(binding.id) {
                function_names_by_scope
                    .entry(scope)
                    .or_default()
                    .push(binding.name.clone());
            }
        }

        let mut call_sites = Vec::new();
        for site in model.all_call_sites() {
            let enclosing_functions = model
                .ancestor_scopes(site.scope)
                .find_map(|scope| function_names_by_scope.get(&scope));
            let local_definition_span = analysis
                .visible_function_binding_at_call(&site.callee, site.name_span)
                .map(|binding_id| binding_definition_span(model.binding(binding_id)));
            if let Some(enclosing_functions) = enclosing_functions {
                for enclosing in enclosing_functions {
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

        Self {
            definitions,
            call_sites,
            source_edges,
        }
    }

    /// Returns the definition of `name` in this file, if any (first wins).
    pub fn definition(&self, name: &Name) -> Option<&CallFactDefinition> {
        self.definitions.iter().find(|def| &def.name == name)
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

    /// Removes a file from the index.
    pub fn remove(&mut self, path: &Path) {
        self.files.remove(path);
    }

    /// Returns whether `path` is indexed.
    pub fn contains(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    /// Returns the facts for `path`, if indexed.
    pub fn facts(&self, path: &Path) -> Option<&FileCallFacts> {
        self.files.get(path)
    }

    /// Iterates all indexed files and their facts (unordered).
    pub fn files(&self) -> impl Iterator<Item = (&Path, &FileCallFacts)> {
        self.files
            .iter()
            .map(|(path, facts)| (path.as_path(), facts))
    }

    /// Number of indexed files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns whether the index holds no files.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
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
        choose_resolved_target(from_path, local, sourced)
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
                    .map(|path| (path, edge.span))
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
    ) -> Option<PathBuf> {
        if !stack.insert(path.to_path_buf()) {
            return None;
        }
        let Some(facts) = self.files.get(path) else {
            stack.remove(path);
            return None;
        };

        enum Event<'a> {
            Definition,
            Source(&'a CallFactSourceEdge),
        }

        let mut events = Vec::new();
        for definition in facts.definitions.iter().filter(|def| &def.name == callee) {
            events.push((definition.def_span.start.offset, Event::Definition));
        }
        for edge in &facts.source_edges {
            events.push((edge.span.start.offset, Event::Source(edge)));
        }
        events.sort_by_key(|(offset, _)| *offset);

        for (_, event) in events.into_iter().rev() {
            let resolved = match event {
                Event::Definition => Some(path.to_path_buf()),
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

        let mut order: Vec<(PathBuf, Name)> = Vec::new();
        let mut spans: FxHashMap<(PathBuf, Name), Vec<Span>> = FxHashMap::default();
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
            let Some(target_path) = target else {
                continue;
            };
            let key = (target_path, site.callee.clone());
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
            .map(|(target_path, name)| {
                let definition = self
                    .files
                    .get(&target_path)
                    .and_then(|facts| facts.definition(&name));
                let call_spans = spans
                    .remove(&(target_path.clone(), name.clone()))
                    .unwrap_or_default();
                CrossFileCall {
                    path: target_path,
                    def_span: definition.map(|def| def.def_span),
                    selection_span: definition.map(|def| def.selection_span),
                    node: CallNodeKind::Function(name),
                    call_spans,
                }
            })
            .collect()
    }

    /// Returns the callers of the function `name` defined in `target_path`,
    /// grouped by caller node. A caller is any file that transitively sources
    /// `target_path` and calls `name` without a nearer shadowing definition.
    pub fn incoming(&self, target_path: &Path, name: &Name) -> Vec<CrossFileCall> {
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
                        .as_deref()
                        == Some(target_path);
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
) -> Option<PathBuf> {
    match (local_definition, sourced) {
        (Some(definition), Some((path, source_span)))
            if source_span != Span::new() && source_span.start.offset > definition.start.offset =>
        {
            Some(path)
        }
        (Some(_), _) => Some(from_path.to_path_buf()),
        (None, Some((path, _))) => Some(path),
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

    fn name(text: &str) -> Name {
        Name::from(text)
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
        assert!(enclosings.contains(&("inner".to_owned(), CallNodeKind::Function(name("outer")))));
        assert!(enclosings.contains(&("inner".to_owned(), CallNodeKind::Function(name("nested")))));
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
        let incoming = index.incoming(Path::new("/w/a.sh"), &name("greet"));
        let mut callers: Vec<String> = incoming
            .iter()
            .map(|call| format!("{}:{:?}", call.path.to_string_lossy(), call.node))
            .collect();
        callers.sort();
        // b.sh's `run` (follow edge) and c.sh's top level (assume edge).
        assert_eq!(
            callers,
            vec![
                format!("/w/b.sh:{:?}", CallNodeKind::Function(name("run"))),
                format!("/w/c.sh:{:?}", CallNodeKind::TopLevel),
            ]
        );
    }

    #[test]
    fn outgoing_descends_into_followed_file() {
        let index = three_file_index();
        let outgoing = index.outgoing(Path::new("/w/b.sh"), &CallNodeKind::Function(name("run")));
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].path, PathBuf::from("/w/a.sh"));
        assert_eq!(outgoing[0].node, CallNodeKind::Function(name("greet")));
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
        index.insert(
            PathBuf::from("/w/b.sh"),
            facts("greet() { echo b; }\ngreet\n", &["/w/a.sh"]),
        );
        assert_eq!(
            index.resolve(Path::new("/w/b.sh"), &name("greet")),
            Some(PathBuf::from("/w/b.sh"))
        );
        // a.sh's greet therefore has no incoming call from b.sh.
        assert!(
            index
                .incoming(Path::new("/w/a.sh"), &name("greet"))
                .is_empty()
        );
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
        index.insert(PathBuf::from("/w/b.sh"), facts("greet\n", &[]));
        assert_eq!(index.resolve(Path::new("/w/b.sh"), &name("greet")), None);
        assert!(
            index
                .incoming(Path::new("/w/a.sh"), &name("greet"))
                .is_empty()
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
        index.insert(
            caller_path.clone(),
            FileCallFacts::project_with_source_edges(&caller, edges),
        );
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

        let incoming_a = index.incoming(&a_path, &name("greet"));
        assert_eq!(incoming_a.len(), 1);
        assert_eq!(incoming_a[0].call_spans.len(), 1);
        let incoming_c = index.incoming(&c_path, &name("greet"));
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
        index.insert(
            caller_path.clone(),
            FileCallFacts::project_with_source_edges(&caller, vec![edge]),
        );
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

        let outgoing = index.outgoing(&path, &CallNodeKind::Function(name("itunes")));
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].node, CallNodeKind::Function(name("helper")));

        let mut callers = index
            .incoming(&path, &name("helper"))
            .into_iter()
            .map(|call| call.node)
            .collect::<Vec<_>>();
        callers.sort_by_key(|node| format!("{node:?}"));
        assert_eq!(
            callers,
            [
                CallNodeKind::Function(name("itunes")),
                CallNodeKind::Function(name("music")),
            ]
        );
    }
}
