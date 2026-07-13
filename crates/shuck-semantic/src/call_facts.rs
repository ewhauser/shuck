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

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};

use crate::editor::binding_definition_span;
use crate::{ScopeId, SemanticModel};

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
    /// Whether the semantic model resolved this call to a function binding
    /// visible in this file (honoring definition order and shadowing). Sites
    /// that do not resolve locally are candidates for cross-file resolution
    /// through source edges; sites that do must not be re-resolved by name.
    pub locally_resolved: bool,
}

/// Call-relevant facts projected from one file, plus its resolved source edges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileCallFacts {
    /// Functions defined in this file.
    pub definitions: Vec<CallFactDefinition>,
    /// Call sites contained in this file.
    pub call_sites: Vec<CallFactSite>,
    /// Resolved on-disk paths of this file's determinable source edges (literal
    /// resolvable paths plus `assume-source` / `follow-source` targets).
    pub source_edges: Vec<PathBuf>,
}

impl FileCallFacts {
    /// Projects call facts from a semantic model. `source_edges` are the resolved
    /// target paths of the file's determinable source edges, supplied by the
    /// caller (path resolution is not a semantic-layer concern).
    pub fn project(model: &SemanticModel, source_edges: Vec<PathBuf>) -> Self {
        let analysis = model.analysis();

        let mut function_name_by_scope: FxHashMap<ScopeId, Name> = FxHashMap::default();
        let mut definitions = Vec::new();
        for binding in model.function_definition_bindings() {
            definitions.push(CallFactDefinition {
                name: binding.name.clone(),
                def_span: binding_definition_span(binding),
                selection_span: binding.span,
            });
            if let Some(scope) = analysis.function_scope_for_binding(binding.id) {
                function_name_by_scope
                    .entry(scope)
                    .or_insert_with(|| binding.name.clone());
            }
        }

        let mut call_sites = Vec::new();
        for site in model.all_call_sites() {
            let enclosing = model
                .ancestor_scopes(site.scope)
                .find_map(|scope| function_name_by_scope.get(&scope).cloned())
                .map(CallNodeKind::Function)
                .unwrap_or(CallNodeKind::TopLevel);
            let locally_resolved = analysis
                .visible_function_binding_at_call(&site.callee, site.name_span)
                .is_some();
            call_sites.push(CallFactSite {
                callee: site.callee.clone(),
                name_span: site.name_span,
                enclosing,
                locally_resolved,
            });
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
        if let Some(facts) = self.files.get(from_path)
            && facts.definition(callee).is_some()
        {
            return Some(from_path.to_path_buf());
        }

        self.resolve_through_edges(from_path, callee)
    }

    /// Resolves `callee` through `from_path`'s transitive source edges only,
    /// skipping the file's own definitions. Used for call sites the semantic
    /// model did not bind locally: a same-named local definition that is not
    /// visible at the site (defined later, or shadowed) must not capture it.
    fn resolve_through_edges(&self, from_path: &Path, callee: &Name) -> Option<PathBuf> {
        let mut visited: FxHashSet<PathBuf> = FxHashSet::default();
        visited.insert(from_path.to_path_buf());
        let mut queue: VecDeque<PathBuf> = self
            .files
            .get(from_path)
            .map(|facts| facts.source_edges.iter().cloned().collect())
            .unwrap_or_default();

        while let Some(path) = queue.pop_front() {
            if !visited.insert(path.clone()) {
                continue;
            }
            let Some(facts) = self.files.get(&path) else {
                continue;
            };
            if facts.definition(callee).is_some() {
                return Some(path);
            }
            for edge in &facts.source_edges {
                if !visited.contains(edge) {
                    queue.push_back(edge.clone());
                }
            }
        }
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
        let mut resolved: FxHashMap<Name, Option<PathBuf>> = FxHashMap::default();
        for site in &facts.call_sites {
            if &site.enclosing != from_kind {
                continue;
            }
            let target = if site.locally_resolved {
                // The semantic model already bound this call in-file.
                Some(from_path.to_path_buf())
            } else {
                resolved
                    .entry(site.callee.clone())
                    .or_insert_with(|| self.resolve_through_edges(from_path, &site.callee))
                    .clone()
            };
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
            let mut edges_resolve: Option<bool> = None;
            for site in &facts.call_sites {
                if &site.callee != name {
                    continue;
                }
                let resolves = if site.locally_resolved {
                    // A locally bound call belongs to this edge only when the
                    // queried function lives in the caller's own file.
                    caller_path.as_path() == target_path
                } else {
                    caller_path.as_path() != target_path
                        && *edges_resolve.get_or_insert_with(|| {
                            self.resolve_through_edges(caller_path, name).as_deref()
                                == Some(target_path)
                        })
                };
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
}
