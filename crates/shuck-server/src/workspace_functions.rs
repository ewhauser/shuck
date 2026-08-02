//! Shared workspace index for cross-file function editor features.
//!
//! The index projects each shell file into compact semantic function facts,
//! resolves determinable `source` edges, and retains just enough source metadata
//! to turn byte spans back into LSP ranges. Open buffers shadow disk content.
//! The session caches one build and invalidates it whenever documents,
//! workspaces, watched files, or configuration change.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lsp_types as types;
use sha2::{Digest, Sha256};
use shuck_ast::Span;
use shuck_config::{
    ConfigArguments, apply_config_overrides, load_project_config, resolve_project_root_for_file,
};
use shuck_indexer::LineIndex;
use shuck_linter::ShellDialect;
use shuck_semantic::{
    CallFactSourceEdge, CallNodeKind, CrossFileCall, FileCallFacts, VisibleSourcedFunction,
    WorkspaceCallIndex, source_ref_candidate_paths,
};

use crate::PositionEncoding;
use crate::edit::DocumentVersion;
use crate::editor::analyze_editor_document;
use crate::session::{ClientOptions, RequestCancellationToken, WorkspaceSettingsSnapshot};
use crate::symbols::WorkspaceOpenDocument;

/// Immutable session state needed to build or query the workspace function index.
pub(crate) struct WorkspaceFunctionContext {
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) settings_workspace_roots: Vec<PathBuf>,
    pub(crate) workspace_settings: Vec<WorkspaceSettingsSnapshot>,
    pub(crate) global_options: ClientOptions,
    pub(crate) open_documents: Vec<WorkspaceOpenDocument>,
    pub(crate) encoding: PositionEncoding,
    /// Hard bound on indexed files. A build that reaches this limit is marked
    /// incomplete so mutation features can fail closed.
    pub(crate) max_files: usize,
    pub(crate) cache: Arc<WorkspaceFunctionIndexCache>,
    pub(crate) epoch: u64,
    pub(crate) cancellation: RequestCancellationToken,
}

/// Session-lifetime cache of the built workspace function index.
#[derive(Default)]
pub(crate) struct WorkspaceFunctionIndexCache {
    epoch: AtomicU64,
    built: Mutex<Option<(u64, Arc<WorkspaceFunctionIndex>)>>,
}

impl WorkspaceFunctionIndexCache {
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

    fn get(&self, epoch: u64) -> Option<Arc<WorkspaceFunctionIndex>> {
        let slot = self.built.lock().ok()?;
        slot.as_ref()
            .filter(|(built_epoch, _)| *built_epoch == epoch)
            .map(|(_, built)| built.clone())
    }

    fn store(&self, epoch: u64, built: Arc<WorkspaceFunctionIndex>) {
        if epoch != self.current_epoch() {
            return;
        }
        if let Ok(mut slot) = self.built.lock() {
            *slot = Some((epoch, built));
        }
    }
}

/// Returns the cached index for this context, building it on a miss.
///
/// Cancellation is checked between index-population steps. An aborted miss does
/// not populate the cache, and the request wrapper suppresses its response.
pub(crate) fn workspace_function_index(
    context: &WorkspaceFunctionContext,
) -> Option<Arc<WorkspaceFunctionIndex>> {
    if context.cancellation.is_cancelled() {
        return None;
    }
    if let Some(built) = context.cache.get(context.epoch) {
        return Some(built);
    }
    let built = Arc::new(WorkspaceFunctionIndex::build(context)?);
    context.cache.store(context.epoch, built.clone());
    Some(built)
}

/// Source snapshot retained for one indexed file.
pub(crate) struct IndexedWorkspaceFile {
    uri: types::Url,
    open_uri: Option<types::Url>,
    source: String,
    line_index: LineIndex,
    #[allow(dead_code)] // Used by the upcoming cross-file rename safety check.
    version: Option<DocumentVersion>,
    #[allow(dead_code)] // Used by the upcoming cross-file rename safety check.
    content_hash: [u8; 32],
}

impl IndexedWorkspaceFile {
    pub(crate) fn uri(&self) -> &types::Url {
        &self.uri
    }

    /// URI used by the editor for an open buffer, falling back to the
    /// canonical file URI for disk snapshots.
    pub(crate) fn editor_uri(&self) -> &types::Url {
        self.open_uri.as_ref().unwrap_or(&self.uri)
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// Open-document version captured by the request, or `None` for disk input.
    #[allow(dead_code)]
    pub(crate) fn version(&self) -> Option<DocumentVersion> {
        self.version
    }

    /// SHA-256 of the exact content used to build semantic facts and ranges.
    #[allow(dead_code)]
    pub(crate) fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }
}

/// Shared index queried by call hierarchy and future cross-file editor features.
pub(crate) struct WorkspaceFunctionIndex {
    graph: WorkspaceCallIndex,
    files: BTreeMap<PathBuf, IndexedWorkspaceFile>,
    encoding: PositionEncoding,
    #[allow(dead_code)] // Mutation features must reject partial workspace indexes.
    complete: bool,
}

impl WorkspaceFunctionIndex {
    fn build(context: &WorkspaceFunctionContext) -> Option<Self> {
        let mut graph = WorkspaceCallIndex::new();
        let mut files = BTreeMap::new();
        let mut complete = true;
        let max_files = context.max_files;
        let mut source_paths = SourcePathsCache::default();

        let mut open_docs = context
            .open_documents
            .iter()
            .filter_map(|open| {
                let path = canonical_path(&open.uri.to_file_path().ok()?);
                Some((path, open))
            })
            .collect::<Vec<_>>();
        open_docs.sort_by(|(left, _), (right, _)| left.cmp(right));
        let open_paths = open_docs
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>();

        for (path, open) in &open_docs {
            if context.cancellation.is_cancelled() {
                return None;
            }
            if graph.file_count() >= max_files {
                complete = false;
                tracing::warn!(
                    "workspace functions: open documents exceed the {max_files}-file limit; \
                     indexing only the first {max_files}"
                );
                break;
            }
            let resolution = source_paths.resolve(path, context);
            complete &= resolution.complete;
            insert_file(
                &mut graph,
                &mut files,
                WorkspaceFileInput {
                    path,
                    uri: open.uri.clone(),
                    source: open.document.contents(),
                    version: Some(open.document.version()),
                },
                &resolution,
                &open_paths,
            );
        }

        if context.cancellation.is_cancelled() {
            return None;
        }
        let remaining = max_files.saturating_sub(graph.file_count());
        let discovery = discover_closed_shell_files(
            &context.workspace_roots,
            &open_paths,
            remaining,
            &context.cancellation,
        )?;
        complete &= discovery.complete;
        for file in discovery.files {
            if context.cancellation.is_cancelled() {
                return None;
            }
            let Ok(source) = std::fs::read_to_string(&file) else {
                complete = false;
                continue;
            };
            let Ok(uri) = types::Url::from_file_path(&file) else {
                complete = false;
                continue;
            };
            let resolution = source_paths.resolve(&file, context);
            complete &= resolution.complete;
            insert_file(
                &mut graph,
                &mut files,
                WorkspaceFileInput {
                    path: &file,
                    uri,
                    source: &source,
                    version: None,
                },
                &resolution,
                &open_paths,
            );
        }

        'expand: loop {
            if context.cancellation.is_cancelled() {
                return None;
            }
            let missing = graph
                .files()
                .flat_map(|(_, facts)| facts.source_edges.iter().map(|edge| edge.path.clone()))
                .filter(|target| !graph.contains(target))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if missing.is_empty() {
                break;
            }
            for target in missing {
                if context.cancellation.is_cancelled() {
                    return None;
                }
                if graph.file_count() >= max_files {
                    complete = false;
                    tracing::warn!(
                        "workspace functions: source-edge targets exceed the {max_files}-file \
                         limit; cross-file results may be incomplete"
                    );
                    break 'expand;
                }
                let Some(open) = open_docs
                    .iter()
                    .find_map(|(path, open)| (path == &target).then_some(*open))
                else {
                    let Ok(source) = std::fs::read_to_string(&target) else {
                        complete = false;
                        graph.insert(target.clone(), FileCallFacts::default());
                        continue;
                    };
                    let Ok(uri) = types::Url::from_file_path(&target) else {
                        complete = false;
                        graph.insert(target.clone(), FileCallFacts::default());
                        continue;
                    };
                    let resolution = source_paths.resolve(&target, context);
                    complete &= resolution.complete;
                    insert_file(
                        &mut graph,
                        &mut files,
                        WorkspaceFileInput {
                            path: &target,
                            uri,
                            source: &source,
                            version: None,
                        },
                        &resolution,
                        &open_paths,
                    );
                    continue;
                };
                let resolution = source_paths.resolve(&target, context);
                complete &= resolution.complete;
                insert_file(
                    &mut graph,
                    &mut files,
                    WorkspaceFileInput {
                        path: &target,
                        uri: open.uri.clone(),
                        source: open.document.contents(),
                        version: Some(open.document.version()),
                    },
                    &resolution,
                    &open_paths,
                );
            }
        }

        Some(Self {
            graph,
            files,
            encoding: context.encoding,
            complete,
        })
    }

    pub(crate) fn resolve_call_site(
        &self,
        from_path: &Path,
        name_span: Span,
    ) -> Option<CrossFileCall> {
        self.graph.resolve_call_site(from_path, name_span)
    }

    pub(crate) fn resolve_call_site_exact(
        &self,
        from_path: &Path,
        name_span: Span,
    ) -> Option<CrossFileCall> {
        self.graph.resolve_call_site_exact(from_path, name_span)
    }

    pub(crate) fn visible_sourced_functions(
        &self,
        from_path: &Path,
        source_spans: &[Span],
    ) -> Vec<VisibleSourcedFunction> {
        self.graph
            .visible_sourced_functions_from_source_spans(from_path, source_spans)
    }

    pub(crate) fn incoming(
        &self,
        target_path: &Path,
        target_node: &CallNodeKind,
    ) -> Vec<CrossFileCall> {
        self.graph.incoming(target_path, target_node)
    }

    pub(crate) fn outgoing(
        &self,
        from_path: &Path,
        from_node: &CallNodeKind,
    ) -> Vec<CrossFileCall> {
        self.graph.outgoing(from_path, from_node)
    }

    pub(crate) fn file(&self, path: &Path) -> Option<&IndexedWorkspaceFile> {
        self.files.get(path)
    }

    pub(crate) fn range_of(&self, path: &Path, span: Span) -> Option<types::Range> {
        let file = self.file(path)?;
        Some(crate::edit::to_lsp_range(
            span.to_range(),
            file.source(),
            file.line_index(),
            self.encoding,
        ))
    }

    pub(crate) fn ranges_in(&self, path: &Path, spans: &[Span]) -> Vec<types::Range> {
        spans
            .iter()
            .filter_map(|span| self.range_of(path, *span))
            .collect()
    }

    /// Whether discovery and source-edge expansion completed within the file
    /// budget and without unreadable inputs.
    #[allow(dead_code)]
    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns whether a closed file still matches the content used to build
    /// this index. Open documents are versioned by the LSP session instead.
    #[allow(dead_code)]
    pub(crate) fn closed_file_is_current(&self, path: &Path) -> bool {
        let Some(file) = self.file(path) else {
            return false;
        };
        if file.version().is_some() {
            return true;
        }
        std::fs::read(path)
            .map(|contents| content_hash(&contents) == file.content_hash())
            .unwrap_or(false)
    }

    #[cfg(test)]
    fn file_count(&self) -> usize {
        self.graph.file_count()
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.graph.contains(path)
    }
}

#[derive(Clone)]
struct SourcePathResolution {
    roots: Vec<String>,
    project_root: PathBuf,
    complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SourcePathsCacheKey {
    project_root: PathBuf,
    workspace_root: Option<PathBuf>,
}

#[derive(Default)]
struct SourcePathsCache {
    by_project: HashMap<SourcePathsCacheKey, SourcePathResolution>,
}

impl SourcePathsCache {
    fn resolve(&mut self, path: &Path, context: &WorkspaceFunctionContext) -> SourcePathResolution {
        let workspace = workspace_settings_for_path(context, path);
        let fallback = context
            .settings_workspace_roots
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
            .or_else(|| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let (project_root, root_complete) =
            match resolve_project_root_for_file(path, &fallback, true) {
                Ok(project_root) => (project_root, true),
                Err(error) => {
                    tracing::warn!(
                        "workspace functions: failed to resolve the project root for {}: {error}",
                        path.display()
                    );
                    (fallback, false)
                }
            };
        let key = SourcePathsCacheKey {
            project_root: project_root.clone(),
            workspace_root: workspace.map(|workspace| workspace.root.clone()),
        };
        let mut resolution = self
            .by_project
            .entry(key)
            .or_insert_with(|| {
                let (mut config, config_complete) =
                    match load_project_config(&project_root, &ConfigArguments::default()) {
                        Ok(config) => (config, true),
                        Err(error) => {
                            tracing::warn!(
                                "workspace functions: failed to load config from {}: {error}",
                                project_root.display()
                            );
                            (Default::default(), false)
                        }
                    };
                apply_config_overrides(&mut config, context.global_options.to_config_overrides());
                if let Some(options) = workspace.and_then(|workspace| workspace.options.as_ref()) {
                    apply_config_overrides(&mut config, options.to_config_overrides());
                }
                SourcePathResolution {
                    roots: config.lint.source_paths.unwrap_or_default(),
                    project_root: project_root.clone(),
                    complete: root_complete && config_complete,
                }
            })
            .clone();
        resolution.complete &= root_complete;
        resolution
    }
}

fn workspace_settings_for_path<'a>(
    context: &'a WorkspaceFunctionContext,
    path: &Path,
) -> Option<&'a WorkspaceSettingsSnapshot> {
    context
        .workspace_settings
        .iter()
        .filter_map(|workspace| {
            [Some(&workspace.root), workspace.canonical_root.as_ref()]
                .into_iter()
                .flatten()
                .filter(|root| path.starts_with(root))
                .map(|root| root.components().count())
                .max()
                .map(|length| (workspace, length))
        })
        .max_by_key(|(_, length)| *length)
        .map(|(workspace, _)| workspace)
}

struct WorkspaceFileInput<'a> {
    path: &'a Path,
    uri: types::Url,
    source: &'a str,
    version: Option<DocumentVersion>,
}

fn insert_file(
    graph: &mut WorkspaceCallIndex,
    files: &mut BTreeMap<PathBuf, IndexedWorkspaceFile>,
    input: WorkspaceFileInput<'_>,
    source_paths: &SourcePathResolution,
    open_paths: &BTreeSet<PathBuf>,
) {
    let key = canonical_path(input.path);
    let open_uri = input.version.is_some().then(|| input.uri.clone());
    let uri = std::fs::canonicalize(input.path)
        .ok()
        .and_then(|path| types::Url::from_file_path(path).ok())
        .unwrap_or(input.uri);
    let shell = ShellDialect::infer(input.source, Some(input.path));
    let model = analyze_editor_document(input.source, Some(input.path), shell);
    let edges = model
        .source_refs()
        .iter()
        .filter_map(|source_ref| {
            let scope = model.scope_at(source_ref.span.start.offset());
            source_ref_candidate_paths(
                input.path,
                source_ref,
                &source_paths.roots,
                &source_paths.project_root,
            )
            .into_iter()
            .map(|candidate| canonical_path(&candidate))
            .find(|candidate| open_paths.contains(candidate) || candidate.is_file())
            .map(|target| CallFactSourceEdge {
                path: target,
                span: source_ref.span,
                conditional: source_ref.conditionally_executed,
                completion_visible: !source_ref.conditionally_executed
                    && model.enclosing_function_scope(scope).is_none()
                    && model
                        .innermost_transient_scope_within_function(scope)
                        .is_none(),
            })
        })
        .collect::<Vec<_>>();
    graph.insert(
        key.clone(),
        FileCallFacts::project_with_source_edges(&model, edges),
    );
    files.insert(
        key,
        IndexedWorkspaceFile {
            uri,
            open_uri,
            source: input.source.to_owned(),
            line_index: LineIndex::new(input.source),
            version: input.version,
            content_hash: content_hash(input.source.as_bytes()),
        },
    );
}

struct ClosedFileDiscovery {
    files: Vec<PathBuf>,
    complete: bool,
}

fn discover_closed_shell_files(
    roots: &[PathBuf],
    open_paths: &BTreeSet<PathBuf>,
    max_files: usize,
    cancellation: &RequestCancellationToken,
) -> Option<ClosedFileDiscovery> {
    use shuck_discover::{DiscoveryOptions, FileKind, discover_files};

    let mut files = BTreeSet::new();
    let mut complete = true;
    for root in roots {
        if cancellation.is_cancelled() {
            return None;
        }
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
                complete = false;
                tracing::warn!(
                    "workspace functions: failed to discover files in {}: {error}",
                    root.display()
                );
                continue;
            }
        };
        for file in discovered {
            if cancellation.is_cancelled() {
                return None;
            }
            if file.kind != FileKind::Shell {
                continue;
            }
            let path = canonical_path(&file.absolute_path);
            if open_paths.contains(&path) {
                continue;
            }
            files.insert(path);
        }
    }
    if files.len() > max_files {
        complete = false;
        tracing::warn!(
            "workspace functions: workspace has {} closed shell files; indexing only {max_files}",
            files.len()
        );
    }
    Some(ClosedFileDiscovery {
        files: files.into_iter().take(max_files).collect(),
        complete,
    })
}

pub(crate) fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let normalized = normalize_path(path);
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    while let Some(name) = ancestor.file_name() {
        suffix.push(name.to_owned());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if let Ok(mut canonical) = std::fs::canonicalize(parent) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        ancestor = parent;
    }
    normalized
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn content_hash(contents: &[u8]) -> [u8; 32] {
    Sha256::digest(contents).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextDocument;
    use shuck_config::LintConfig;

    fn context_for(workspace: &Path, max_files: usize) -> WorkspaceFunctionContext {
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
        WorkspaceFunctionContext {
            workspace_roots: vec![workspace.to_path_buf()],
            settings_workspace_roots: vec![workspace.to_path_buf()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: vec![open_doc, open_b],
            encoding: PositionEncoding::UTF16,
            max_files,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        }
    }

    fn populate_workspace(workspace: &Path) {
        std::fs::write(workspace.join("open_a.sh"), "stale() { :; }\n").unwrap();
        std::fs::write(workspace.join("open_b.sh"), "stale() { :; }\n").unwrap();
        for index in 0..4 {
            std::fs::write(
                workspace.join(format!("closed_{index}.sh")),
                "closed() { :; }\n",
            )
            .unwrap();
        }
        std::fs::create_dir_all(workspace.join("vendored")).unwrap();
        std::fs::write(workspace.join(".gitignore"), "vendored/\n").unwrap();
        std::fs::write(workspace.join("vendored/edge.sh"), "edge_fn() { :; }\n").unwrap();
    }

    #[test]
    fn max_files_is_a_hard_bound_and_marks_partial_builds() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);

        for max_files in [1, 3, 5] {
            let context = context_for(&workspace, max_files);
            let built = WorkspaceFunctionIndex::build(&context).unwrap();
            assert!(built.file_count() <= max_files);
            assert!(!built.is_complete());
        }
    }

    #[test]
    fn generous_limit_indexes_open_discovered_and_edge_targets() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);

        let built = WorkspaceFunctionIndex::build(&context_for(&workspace, 100)).unwrap();
        assert_eq!(built.file_count(), 7);
        assert!(built.contains(&workspace.join("vendored/edge.sh")));
        assert!(built.is_complete());
    }

    #[test]
    fn open_unsaved_source_target_participates_without_disk_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        let caller_path = workspace.join("caller.sh");
        let target_path = workspace.join("new_target.sh");
        let caller_source = "source new_target.sh\nfrom_buffer\n";
        let caller = WorkspaceOpenDocument {
            uri: types::Url::from_file_path(&caller_path).unwrap(),
            document: Arc::new(
                TextDocument::new(caller_source.to_owned(), 1).with_language_id("shellscript"),
            ),
        };
        let target = WorkspaceOpenDocument {
            uri: types::Url::from_file_path(&target_path).unwrap(),
            document: Arc::new(
                TextDocument::new("from_buffer() { :; }\n".to_owned(), 2)
                    .with_language_id("shellscript"),
            ),
        };
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![workspace.clone()],
            settings_workspace_roots: vec![workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: vec![caller, target],
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        let caller_facts = built
            .graph
            .files()
            .find_map(|(path, facts)| (path == caller_path).then_some(facts))
            .unwrap();
        assert_eq!(caller_facts.source_edges.len(), 1);
        assert_eq!(caller_facts.source_edges[0].path, target_path);
        let target_facts = built
            .graph
            .files()
            .find_map(|(path, facts)| (path == target_path).then_some(facts))
            .unwrap();
        assert_eq!(target_facts.definitions.len(), 1);
        let call = caller_facts
            .call_sites
            .iter()
            .find(|call| call.callee.as_str() == "from_buffer")
            .unwrap();
        let call_span = call.name_span;
        let resolved = built.resolve_call_site(&caller_path, call_span).unwrap();
        assert_eq!(resolved.path, target_path);
    }

    #[cfg(unix)]
    #[test]
    fn open_unsaved_source_target_resolves_through_symlinked_workspace_root() {
        use std::os::unix::fs::symlink;

        let tempdir = tempfile::tempdir().unwrap();
        let real_workspace = tempdir.path().join("real");
        let linked_workspace = tempdir.path().join("linked");
        std::fs::create_dir(&real_workspace).unwrap();
        let real_workspace = std::fs::canonicalize(real_workspace).unwrap();
        symlink(&real_workspace, &linked_workspace).unwrap();

        let caller_path = linked_workspace.join("caller.sh");
        let target_path = linked_workspace.join("new_target.sh");
        std::fs::write(real_workspace.join("caller.sh"), "stale\n").unwrap();
        let caller = WorkspaceOpenDocument {
            uri: types::Url::from_file_path(&caller_path).unwrap(),
            document: Arc::new(
                TextDocument::new("source new_target.sh\nfrom_buffer\n".to_owned(), 1)
                    .with_language_id("shellscript"),
            ),
        };
        let target_uri = types::Url::from_file_path(&target_path).unwrap();
        let target = WorkspaceOpenDocument {
            uri: target_uri.clone(),
            document: Arc::new(
                TextDocument::new("from_buffer() { :; }\n".to_owned(), 2)
                    .with_language_id("shellscript"),
            ),
        };
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![linked_workspace.clone()],
            settings_workspace_roots: vec![linked_workspace.clone(), real_workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: vec![caller, target],
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        let canonical_caller = real_workspace.join("caller.sh");
        let canonical_target = real_workspace.join("new_target.sh");
        let facts = built
            .graph
            .files()
            .find_map(|(path, facts)| (path == canonical_caller).then_some(facts))
            .unwrap();
        let call = facts
            .call_sites
            .iter()
            .find(|call| call.callee.as_str() == "from_buffer")
            .unwrap();
        assert_eq!(
            built
                .resolve_call_site(&canonical_caller, call.name_span)
                .unwrap()
                .path,
            canonical_target
        );
        assert_eq!(built.file(&canonical_target).unwrap().uri(), &target_uri);
    }

    #[cfg(unix)]
    #[test]
    fn existing_open_target_retains_the_editors_symlink_uri() {
        use std::os::unix::fs::symlink;

        let tempdir = tempfile::tempdir().unwrap();
        let real_workspace = tempdir.path().join("real");
        let linked_workspace = tempdir.path().join("linked");
        std::fs::create_dir(&real_workspace).unwrap();
        let real_workspace = std::fs::canonicalize(real_workspace).unwrap();
        std::fs::write(
            real_workspace.join("caller.sh"),
            "source target.sh\ntarget\n",
        )
        .unwrap();
        std::fs::write(real_workspace.join("target.sh"), "stale() { :; }\n").unwrap();
        symlink(&real_workspace, &linked_workspace).unwrap();

        let caller_uri = types::Url::from_file_path(linked_workspace.join("caller.sh")).unwrap();
        let target_uri = types::Url::from_file_path(linked_workspace.join("target.sh")).unwrap();
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![linked_workspace.clone()],
            settings_workspace_roots: vec![linked_workspace, real_workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: vec![
                WorkspaceOpenDocument {
                    uri: caller_uri,
                    document: Arc::new(
                        TextDocument::new("source target.sh\ntarget\n".to_owned(), 1)
                            .with_language_id("shellscript"),
                    ),
                },
                WorkspaceOpenDocument {
                    uri: target_uri.clone(),
                    document: Arc::new(
                        TextDocument::new("target() { :; }\n".to_owned(), 2)
                            .with_language_id("shellscript"),
                    ),
                },
            ],
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        let file = built.file(&real_workspace.join("target.sh")).unwrap();
        assert_ne!(file.uri(), &target_uri);
        assert_eq!(file.editor_uri(), &target_uri);
    }

    #[test]
    fn closed_file_freshness_uses_indexed_content_hash() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        let file = workspace.join("closed.sh");
        std::fs::write(&file, "one() { :; }\n").unwrap();
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![workspace.clone()],
            settings_workspace_roots: vec![workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: Vec::new(),
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };
        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        assert!(built.closed_file_is_current(&file));
        std::fs::write(&file, "two() { :; }\n").unwrap();
        assert!(!built.closed_file_is_current(&file));
    }

    #[test]
    fn workspace_source_paths_layer_over_global_client_options() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        std::fs::create_dir(workspace.join("scripts")).unwrap();
        std::fs::create_dir(workspace.join("lib")).unwrap();
        let caller = workspace.join("scripts/main.sh");
        let target = workspace.join("lib/util.sh");
        std::fs::write(&caller, "source util.sh\nfrom_root\n").unwrap();
        std::fs::write(&target, "from_root() { :; }\n").unwrap();

        let global_options = ClientOptions {
            lint: Some(LintConfig {
                source_paths: Some(vec!["missing".to_owned()]),
                ..LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let workspace_options = ClientOptions {
            lint: Some(LintConfig {
                source_paths: Some(vec!["lib".to_owned()]),
                ..LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![workspace.clone()],
            settings_workspace_roots: vec![workspace.clone()],
            workspace_settings: vec![WorkspaceSettingsSnapshot {
                root: workspace.clone(),
                canonical_root: Some(workspace.clone()),
                options: Some(workspace_options),
            }],
            global_options,
            open_documents: Vec::new(),
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        let facts = built
            .graph
            .files()
            .find_map(|(path, facts)| (path == caller).then_some(facts))
            .unwrap();
        let call = facts
            .call_sites
            .iter()
            .find(|call| call.callee.as_str() == "from_root")
            .unwrap();
        assert_eq!(
            built
                .resolve_call_site(&caller, call.name_span)
                .unwrap()
                .path,
            target
        );
    }

    #[test]
    fn malformed_project_config_marks_the_index_incomplete() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        std::fs::write(workspace.join(".shuck.toml"), "[lint\n").unwrap();
        std::fs::write(workspace.join("main.sh"), "main() { :; }\n").unwrap();
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![workspace.clone()],
            settings_workspace_roots: vec![workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: Vec::new(),
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let resolution = SourcePathsCache::default().resolve(&workspace.join("main.sh"), &context);
        assert!(!resolution.complete);
        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        assert!(!built.is_complete());
    }

    #[cfg(unix)]
    #[test]
    fn project_root_resolution_failure_marks_source_paths_incomplete() {
        use std::os::unix::fs::symlink;

        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        std::fs::write(workspace.join(".shuck.toml"), "").unwrap();
        let loop_path = workspace.join("loop");
        symlink("loop", &loop_path).unwrap();
        let context = WorkspaceFunctionContext {
            workspace_roots: vec![workspace.clone()],
            settings_workspace_roots: vec![workspace.clone()],
            workspace_settings: Vec::new(),
            global_options: ClientOptions::default(),
            open_documents: Vec::new(),
            encoding: PositionEncoding::UTF16,
            max_files: 100,
            cache: Arc::new(WorkspaceFunctionIndexCache::default()),
            epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };

        let mut source_paths = SourcePathsCache::default();
        assert!(
            source_paths
                .resolve(&workspace.join("main.sh"), &context)
                .complete
        );
        let resolution = source_paths.resolve(&loop_path.join("main.sh"), &context);
        assert!(!resolution.complete);
    }

    #[test]
    fn cancelled_build_does_not_scan_or_populate_the_cache() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);
        let context = context_for(&workspace, 100);
        context.cancellation.cancel();

        assert!(workspace_function_index(&context).is_none());
        assert!(context.cache.get(context.epoch).is_none());
    }
}
