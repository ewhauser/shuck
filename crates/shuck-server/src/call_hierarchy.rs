//! Cross-file call hierarchy (spec 025).
//!
//! `incomingCalls` / `outgoingCalls` answer across the whole workspace by
//! building a [`WorkspaceCallIndex`]: every workspace shell file (open buffer
//! preferred over disk) is parsed and projected into call facts, its
//! determinable source edges resolved, and the two directions answered as
//! traversals of the resulting call graph. `prepareCallHierarchy` stays a
//! single-file identity step; the item it returns (name + file URI + a
//! [`CallHierarchyData`] payload distinguishing top-level nodes) is enough for
//! the index queries to locate the node.
//!
//! The built index is cached on the session and invalidated whenever a
//! document, workspace folder, or configuration changes, so expanding a call
//! tree re-uses one build instead of re-analyzing the workspace per request.
//!
//! Call sites combine binding-accurate in-file definitions with positioned
//! source edges, so later sourced and local definitions override earlier ones
//! in shell execution order. Known limitation:
//! nodes are keyed by function *name* within a file, so two same-named
//! definitions in one file collapse onto the first.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lsp_types as types;
use shuck_ast::{Name, Span};
use shuck_config::{ConfigArguments, load_project_config, resolve_project_root_for_file};
use shuck_indexer::LineIndex;
use shuck_linter::ShellDialect;
use shuck_semantic::{
    CallFactSourceEdge, CallNodeKind, CrossFileCall, FileCallFacts, WorkspaceCallIndex,
    resolve_source_ref_targets,
};

use crate::PositionEncoding;
use crate::editor::analyze_editor_document;
use crate::editor_features::CallHierarchyData;
use crate::symbols::WorkspaceOpenDocument;

/// Resolves and memoizes `[lint] source-paths` per project root, so cross-file
/// hint resolution honors configured search roots the same way `shuck check`
/// does. Relative roots are resolved against the project root.
#[derive(Default)]
struct SourcePathsCache {
    by_project_root: HashMap<PathBuf, Vec<String>>,
}

impl SourcePathsCache {
    /// Returns the configured source-path roots for `path` and the project root
    /// that relative roots are resolved against.
    fn resolve(&mut self, path: &Path, workspace_roots: &[PathBuf]) -> (Vec<String>, PathBuf) {
        let fallback = workspace_roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
            .or_else(|| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let project_root = resolve_project_root_for_file(path, &fallback, true).unwrap_or(fallback);
        let roots = self
            .by_project_root
            .entry(project_root.clone())
            .or_insert_with(|| {
                load_project_config(&project_root, &ConfigArguments::default())
                    .map(|config| config.lint.source_paths.unwrap_or_default())
                    .unwrap_or_default()
            })
            .clone();
        (roots, project_root)
    }
}

pub(crate) type IncomingResponse = Option<Vec<types::CallHierarchyIncomingCall>>;
pub(crate) type OutgoingResponse = Option<Vec<types::CallHierarchyOutgoingCall>>;

/// Workspace state needed to build the call index for one request.
pub(crate) struct CallHierarchyContext {
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) open_documents: Vec<WorkspaceOpenDocument>,
    pub(crate) encoding: PositionEncoding,
    /// Upper bound on indexed files (`server.callHierarchy.maxFiles`): a
    /// runaway workspace degrades to a partial graph, not an unbounded scan.
    pub(crate) max_files: usize,
    pub(crate) cache: Arc<CallIndexCache>,
    pub(crate) epoch: u64,
}

/// Session-lifetime cache of the built workspace call index.
///
/// Every document, workspace, or configuration change bumps the epoch and
/// drops the cached build. Requests capture the epoch when they snapshot the
/// session; a build finished after a concurrent change carries a stale epoch
/// and is never served to later requests.
#[derive(Default)]
pub(crate) struct CallIndexCache {
    epoch: AtomicU64,
    built: Mutex<Option<(u64, Arc<BuiltIndex>)>>,
}

impl CallIndexCache {
    /// Drops any cached index and marks in-flight builds stale.
    pub(crate) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.built.lock() {
            *slot = None;
        }
    }

    pub(crate) fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn get(&self, epoch: u64) -> Option<Arc<BuiltIndex>> {
        let slot = self.built.lock().ok()?;
        slot.as_ref()
            .filter(|(built_epoch, _)| *built_epoch == epoch)
            .map(|(_, built)| built.clone())
    }

    fn store(&self, epoch: u64, built: Arc<BuiltIndex>) {
        // A build raced by an invalidation is tagged with a stale epoch; skip
        // storing it so it cannot displace a fresher build. (If it slips past
        // this check, `get`'s epoch comparison still refuses to serve it.)
        if epoch != self.current_epoch() {
            return;
        }
        if let Ok(mut slot) = self.built.lock() {
            *slot = Some((epoch, built));
        }
    }
}

/// Returns the cached index for this context's epoch, building it on a miss.
fn cached_index(context: &CallHierarchyContext) -> Arc<BuiltIndex> {
    if let Some(built) = context.cache.get(context.epoch) {
        return built;
    }
    let built = Arc::new(BuiltIndex::build(context));
    context.cache.store(context.epoch, built.clone());
    built
}

/// Source text and line index for one indexed file, retained for span→range
/// mapping of cross-file results.
struct FileText {
    source: String,
    line_index: LineIndex,
}

/// The per-request index plus the text needed to render results.
struct BuiltIndex {
    index: WorkspaceCallIndex,
    texts: BTreeMap<PathBuf, FileText>,
    encoding: PositionEncoding,
}

pub(crate) fn incoming_calls(
    context: CallHierarchyContext,
    params: types::CallHierarchyIncomingCallsParams,
) -> crate::server::Result<IncomingResponse> {
    let Some((path, node)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let CallNodeKind::Function(name) = node else {
        // Nothing "calls" a script's top level in this model.
        return Ok(Some(Vec::new()));
    };
    let built = cached_index(&context);
    let calls = built
        .index
        .incoming(&path, &name)
        .into_iter()
        .filter_map(|call| {
            let from = built.item_for(&call)?;
            Some(types::CallHierarchyIncomingCall {
                from_ranges: built.ranges_in(&call.path, &call.call_spans),
                from,
            })
        })
        .collect();
    Ok(Some(calls))
}

pub(crate) fn outgoing_calls(
    context: CallHierarchyContext,
    params: types::CallHierarchyOutgoingCallsParams,
) -> crate::server::Result<OutgoingResponse> {
    let Some((path, node)) = item_identity(&params.item) else {
        return Ok(None);
    };
    let built = cached_index(&context);
    // Outgoing call-token spans live in the queried file.
    let calls = built
        .index
        .outgoing(&path, &node)
        .into_iter()
        .filter_map(|call| {
            let to = built.item_for(&call)?;
            Some(types::CallHierarchyOutgoingCall {
                from_ranges: built.ranges_in(&path, &call.call_spans),
                to,
            })
        })
        .collect();
    Ok(Some(calls))
}

/// The queried node's identity: its file path plus which node in that file.
///
/// The `data` payload stamped by `prepare` (and by [`BuiltIndex::item_for`])
/// distinguishes a script top-level MODULE node from a function; without it a
/// top-level item would be misread as a function named after the file's label.
/// Items from clients that drop `data` fall back to the LSP `kind`.
fn item_identity(item: &types::CallHierarchyItem) -> Option<(PathBuf, CallNodeKind)> {
    let path = canonical(&item.uri.to_file_path().ok()?);
    let data = item
        .data
        .clone()
        .and_then(|value| serde_json::from_value::<CallHierarchyData>(value).ok());
    let node = match data {
        Some(CallHierarchyData::TopLevel) => CallNodeKind::TopLevel,
        Some(CallHierarchyData::Function { .. }) => {
            CallNodeKind::Function(Name::from(item.name.as_str()))
        }
        None if item.kind == types::SymbolKind::MODULE => CallNodeKind::TopLevel,
        None => CallNodeKind::Function(Name::from(item.name.as_str())),
    };
    Some((path, node))
}

impl BuiltIndex {
    fn build(context: &CallHierarchyContext) -> Self {
        let mut index = WorkspaceCallIndex::new();
        let mut texts: BTreeMap<PathBuf, FileText> = BTreeMap::new();

        // `max_files` is a hard bound on the total index size. One budget is
        // shared by all three population phases — open buffers, closed-file
        // discovery, and source-edge expansion — and every insertion checks it.
        let max_files = context.max_files;
        let mut source_paths = SourcePathsCache::default();
        // Resolve every open path up front (even ones over budget) so
        // discovery below never re-reads an open buffer's stale on-disk
        // contents.
        let open_docs: Vec<(PathBuf, &WorkspaceOpenDocument)> = context
            .open_documents
            .iter()
            .filter_map(|open| {
                let path = canonical(&open.uri.to_file_path().ok()?);
                Some((path, open))
            })
            .collect();
        let open_paths: Vec<PathBuf> = open_docs.iter().map(|(path, _)| path.clone()).collect();
        for (path, open) in &open_docs {
            if index.file_count() >= max_files {
                tracing::warn!(
                    "call hierarchy: open documents exceed the {max_files}-file limit; \
                     indexing only the first {max_files}"
                );
                break;
            }
            let (roots, base) = source_paths.resolve(path, &context.workspace_roots);
            insert_file(
                &mut index,
                &mut texts,
                path,
                open.document.contents(),
                &roots,
                &base,
            );
        }

        // Discovery is capped to the budget the open buffers left over.
        let remaining = max_files.saturating_sub(index.file_count());
        for file in discover_closed_shell_files(&context.workspace_roots, &open_paths, remaining) {
            let Ok(source) = std::fs::read_to_string(&file) else {
                continue;
            };
            let (roots, base) = source_paths.resolve(&file, &context.workspace_roots);
            insert_file(&mut index, &mut texts, &file, &source, &roots, &base);
        }

        // Resolved source edges may point outside discovery (gitignored
        // vendored files, targets outside the workspace roots). Index those
        // files too — otherwise their definitions are invisible to the graph —
        // following edges of newly added files to a fixpoint, re-checking the
        // budget before every insertion.
        'expand: loop {
            let missing: Vec<PathBuf> = index
                .files()
                .flat_map(|(_, facts)| facts.source_edges.iter().map(|edge| edge.path.clone()))
                .filter(|target| !index.contains(target))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if missing.is_empty() {
                break;
            }
            for target in missing {
                if index.file_count() >= max_files {
                    tracing::warn!(
                        "call hierarchy: source-edge targets exceed the {max_files}-file \
                         limit; the call graph may be missing edges"
                    );
                    break 'expand;
                }
                let Ok(source) = std::fs::read_to_string(&target) else {
                    // Unreadable target: record an empty entry so the loop
                    // terminates instead of retrying it forever.
                    index.insert(target.clone(), FileCallFacts::default());
                    continue;
                };
                let (roots, base) = source_paths.resolve(&target, &context.workspace_roots);
                insert_file(&mut index, &mut texts, &target, &source, &roots, &base);
            }
        }

        Self {
            index,
            texts,
            encoding: context.encoding,
        }
    }

    /// Builds an LSP item for one end of a cross-file edge. Functions carry their
    /// file URI and definition ranges; a top-level caller becomes a MODULE node.
    fn item_for(&self, call: &CrossFileCall) -> Option<types::CallHierarchyItem> {
        let uri = types::Url::from_file_path(&call.path).ok()?;
        match &call.node {
            CallNodeKind::Function(name) => {
                let range = self.range_of(&call.path, call.def_span?)?;
                let selection_range = call
                    .selection_span
                    .and_then(|span| self.range_of(&call.path, span))
                    .unwrap_or(range);
                Some(crate::editor_features::call_hierarchy_function_item(
                    name.to_string(),
                    uri,
                    range,
                    selection_range,
                ))
            }
            CallNodeKind::TopLevel => {
                Some(crate::editor_features::call_hierarchy_top_level_item(uri))
            }
        }
    }

    fn ranges_in(&self, path: &Path, spans: &[Span]) -> Vec<types::Range> {
        spans
            .iter()
            .filter_map(|span| self.range_of(path, *span))
            .collect()
    }

    fn range_of(&self, path: &Path, span: Span) -> Option<types::Range> {
        let text = self.texts.get(path)?;
        Some(crate::edit::to_lsp_range(
            span.to_range(),
            &text.source,
            &text.line_index,
            self.encoding,
        ))
    }
}

fn insert_file(
    index: &mut WorkspaceCallIndex,
    texts: &mut BTreeMap<PathBuf, FileText>,
    path: &Path,
    source: &str,
    source_path_roots: &[String],
    source_path_base: &Path,
) {
    let key = canonical(path);
    let shell = ShellDialect::infer(source, Some(path));
    let model = analyze_editor_document(source, Some(path), shell);
    // Resolve each determinable source edge against the file's own directory
    // first, then the configured `[lint] source-paths` roots.
    let edges = model
        .source_refs()
        .iter()
        .filter_map(|source_ref| {
            resolve_source_ref_targets(path, source_ref, source_path_roots, source_path_base).map(
                |target| CallFactSourceEdge {
                    path: canonical(&target),
                    span: source_ref.span,
                },
            )
        })
        .collect::<Vec<_>>();
    index.insert(
        key.clone(),
        FileCallFacts::project_with_source_edges(&model, edges),
    );
    texts.insert(
        key,
        FileText {
            source: source.to_owned(),
            line_index: LineIndex::new(source),
        },
    );
}

fn discover_closed_shell_files(
    roots: &[PathBuf],
    open_paths: &[PathBuf],
    max_files: usize,
) -> Vec<PathBuf> {
    use shuck_discover::{DiscoveryOptions, FileKind, discover_files};

    let mut files: BTreeMap<PathBuf, ()> = BTreeMap::new();
    for root in roots {
        let discovered = match discover_files(
            std::slice::from_ref(root),
            root,
            &DiscoveryOptions {
                respect_gitignore: true,
                parallel: true,
                use_config_roots: true,
                ..DiscoveryOptions::default()
            },
        ) {
            Ok(files) => files,
            Err(error) => {
                tracing::warn!(
                    "call hierarchy: failed to discover files in {}: {error}",
                    root.display()
                );
                continue;
            }
        };
        for file in discovered {
            if file.kind != FileKind::Shell {
                continue;
            }
            let path = canonical(&file.absolute_path);
            if open_paths.contains(&path) {
                continue;
            }
            files.entry(path).or_default();
        }
    }
    if files.len() > max_files {
        tracing::warn!(
            "call hierarchy: workspace has {} shell files; indexing only {max_files}",
            files.len()
        );
    }
    files.into_keys().take(max_files).collect()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::edit::TextDocument;

    use super::*;

    /// Builds a workspace exercising all three index-population phases: open
    /// buffers, closed-file discovery, and source-edge expansion to a target
    /// outside the discovered set.
    fn context_for(workspace: &Path, max_files: usize) -> CallHierarchyContext {
        let open_path = workspace.join("open_a.sh");
        let open_doc = WorkspaceOpenDocument {
            uri: types::Url::from_file_path(&open_path).unwrap(),
            document: Arc::new(
                TextDocument::new(
                    "# shuck: source=vendored/edge.sh\nsource \"$DIR/edge.sh\"\nedge_fn\n"
                        .to_owned(),
                    1,
                )
                .with_language_id("shellscript"),
            ),
        };
        let open_b = WorkspaceOpenDocument {
            uri: types::Url::from_file_path(workspace.join("open_b.sh")).unwrap(),
            document: Arc::new(
                TextDocument::new("b() { :; }\n".to_owned(), 1).with_language_id("shellscript"),
            ),
        };
        CallHierarchyContext {
            workspace_roots: vec![workspace.to_path_buf()],
            open_documents: vec![open_doc, open_b],
            encoding: PositionEncoding::UTF16,
            max_files,
            cache: Arc::new(CallIndexCache::default()),
            epoch: 0,
        }
    }

    fn populate_workspace(workspace: &Path) {
        // Open buffers (also present on disk with different content).
        std::fs::write(workspace.join("open_a.sh"), "stale() { :; }\n").unwrap();
        std::fs::write(workspace.join("open_b.sh"), "stale() { :; }\n").unwrap();
        // Closed files picked up by discovery.
        for index in 0..4 {
            std::fs::write(
                workspace.join(format!("closed_{index}.sh")),
                "closed() { :; }\n",
            )
            .unwrap();
        }
        // Edge target hidden from discovery by gitignore, reachable only
        // through open_a.sh's source directive.
        std::fs::create_dir_all(workspace.join("vendored")).unwrap();
        std::fs::write(workspace.join(".gitignore"), "vendored/\n").unwrap();
        std::fs::write(workspace.join("vendored/edge.sh"), "edge_fn() { :; }\n").unwrap();
    }

    #[test]
    fn max_files_is_a_hard_bound_across_all_population_phases() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);

        // 2 open + 4 discovered + 1 edge target = 7 candidates; the limit
        // must cap the total, not any single phase.
        for max_files in [1, 3, 5] {
            let built = BuiltIndex::build(&context_for(&workspace, max_files));
            assert!(
                built.index.file_count() <= max_files,
                "index size {} exceeds max_files {max_files}",
                built.index.file_count()
            );
        }
    }

    #[test]
    fn generous_max_files_indexes_open_discovered_and_edge_targets() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);

        let built = BuiltIndex::build(&context_for(&workspace, 100));
        assert_eq!(
            built.index.file_count(),
            7,
            "2 open buffers + 4 discovered + 1 edge target"
        );
        // The edge target joined the graph through the source directive, so
        // the open buffer's call resolves into it.
        assert!(built.index.contains(&workspace.join("vendored/edge.sh")));
    }
}
