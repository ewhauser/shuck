//! Shared workspace index for cross-file function and variable editor features.
//!
//! The index projects each shell file into compact semantic function and
//! variable facts, resolves determinable `source` edges, and retains just
//! enough source metadata to turn byte spans back into LSP ranges. Open
//! buffers shadow disk content.
//!
//! The session caches the linked index and invalidates it whenever any input
//! changes. Underneath it, per-file analysis (source text, line index, and
//! projected facts) is cached separately under a closed-files epoch that only
//! moves when on-disk inputs may have changed, so a rebuild after a buffer
//! edit reanalyzes just the edited buffers and relinks `source` edges instead
//! of rediscovering and reparsing every closed file in the workspace.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
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
    CallFactSourceEdge, CallNodeKind, CrossFileCall, ExactFunctionRename, ExactFunctionRenameError,
    FileCallFacts, VisibleSourcedFunction, WorkspaceCallIndex, source_ref_candidate_paths,
};

use crate::PositionEncoding;
use crate::edit::DocumentVersion;
use crate::editor::analyze_editor_document;
use crate::session::{ClientOptions, RequestCancellationToken, WorkspaceSettingsSnapshot};
use crate::symbols::WorkspaceOpenDocument;
use crate::workspace_variables::{
    FileVariableFacts, WorkspaceVariableIndex, WorkspaceVariableTarget,
};

/// Total bytes of closed-file source one index build will load and retain.
/// Together with the per-file cap this bounds the memory a build can pin,
/// whatever `max_files` is set to.
const MAX_CLOSED_SOURCE_BYTES: usize = 128 * 1024 * 1024;

/// Watched-file batches larger than this drop every cached closed file rather
/// than evicting paths one by one. Batches this large usually come from VCS
/// operations that touch much of the tree anyway.
const MAX_TARGETED_FILE_EVENTS: usize = 256;

/// Immutable session state needed to build or query the cross-file symbol index.
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
    /// Closed-files epoch the context was captured under; cached per-file
    /// analysis is only reused and stored when it still matches.
    pub(crate) closed_epoch: u64,
    pub(crate) cancellation: RequestCancellationToken,
}

/// Session-lifetime cache of the built cross-file symbol index.
///
/// `epoch` moves on every input change, including open-buffer edits, and
/// guards the linked index. `closed_epoch` moves only when closed-file inputs
/// may have changed (watched files, configuration, workspace folders, closing
/// a buffer) and guards the per-file analysis reused across builds.
#[derive(Default)]
pub(crate) struct WorkspaceFunctionIndexCache {
    epoch: AtomicU64,
    closed_epoch: AtomicU64,
    built: Mutex<Option<(u64, Arc<WorkspaceFunctionIndex>)>>,
    analyses: Mutex<FileAnalysisCache>,
    /// Closed files read from disk by builds, for tests that assert reuse.
    #[cfg(test)]
    closed_reads: AtomicUsize,
}

impl WorkspaceFunctionIndexCache {
    /// Drops the linked index and marks in-flight builds stale. Cached
    /// closed-file analysis stays valid; use this when only open buffers
    /// changed.
    pub(crate) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.built.lock() {
            *slot = None;
        }
    }

    /// Drops the linked index and every cached per-file analysis.
    pub(crate) fn invalidate_closed_files(&self) {
        let mut analyses = self.lock_analyses();
        let closed_epoch = self.closed_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *analyses = FileAnalysisCache::new(closed_epoch);
        drop(analyses);
        self.invalidate();
    }

    /// Drops the linked index and the cached analysis of a document that was
    /// just closed, so the next build reads it from disk.
    pub(crate) fn invalidate_closed_document(&self, uri: &types::Url) {
        let Ok(path) = uri.to_file_path() else {
            self.invalidate();
            return;
        };
        self.evict_closed_paths(&[canonical_path(&path)], false);
        self.invalidate();
    }

    /// Applies watched-file events. Content changes evict just the affected
    /// files; creates, deletes, config edits, and very large batches can
    /// change discovery or how `source` paths resolve, so they drop every
    /// cached file.
    pub(crate) fn invalidate_file_events(
        &self,
        changes: &[types::FileEvent],
        config_changed: bool,
    ) {
        if config_changed
            || changes.len() > MAX_TARGETED_FILE_EVENTS
            || changes
                .iter()
                .any(|change| change.typ != types::FileChangeType::CHANGED)
        {
            self.invalidate_closed_files();
            return;
        }
        let paths = changes
            .iter()
            .filter_map(|change| change.uri.to_file_path().ok())
            .map(|path| canonical_path(&path))
            .collect::<Vec<_>>();
        if paths.len() != changes.len() {
            self.invalidate_closed_files();
            return;
        }
        self.evict_closed_paths(&paths, true);
        self.invalidate();
    }

    /// Evicts cached analysis for `paths` and moves the closed-files epoch so
    /// builds that may have read the old contents cannot store them back.
    /// With `rediscover_unknown`, a path outside the cached discovery (for
    /// example an ignore file, or a script that just gained a shebang) also
    /// drops the discovery snapshot.
    fn evict_closed_paths(&self, paths: &[PathBuf], rediscover_unknown: bool) {
        let mut analyses = self.lock_analyses();
        let previous = self.closed_epoch.fetch_add(1, Ordering::SeqCst);
        if analyses.closed_epoch != previous {
            *analyses = FileAnalysisCache::new(previous + 1);
            return;
        }
        analyses.closed_epoch = previous + 1;
        for path in paths {
            analyses.closed.remove(path);
            if rediscover_unknown
                && analyses
                    .discovery
                    .as_ref()
                    .is_some_and(|discovery| !discovery.files.contains(path))
            {
                analyses.discovery = None;
            }
        }
    }

    pub(crate) fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn current_closed_epoch(&self) -> u64 {
        self.closed_epoch.load(Ordering::SeqCst)
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

    /// Copies the cached per-file analysis for a build under `closed_epoch`.
    /// Entries are shared, so the copy is a map of reference-count bumps.
    fn analyses_for(&self, closed_epoch: u64) -> FileAnalysisCache {
        let analyses = self.lock_analyses();
        if analyses.closed_epoch == closed_epoch {
            analyses.clone()
        } else {
            FileAnalysisCache::new(closed_epoch)
        }
    }

    fn store_analyses(&self, analyses: FileAnalysisCache) {
        let mut slot = self.lock_analyses();
        if analyses.closed_epoch == self.current_closed_epoch() {
            *slot = analyses;
        }
    }

    /// Epoch bookkeeping must happen even if a build panicked while holding
    /// the lock, so recover the guard instead of skipping the update.
    fn lock_analyses(&self) -> std::sync::MutexGuard<'_, FileAnalysisCache> {
        self.analyses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn closed_reads(&self) -> usize {
        self.closed_reads.load(Ordering::SeqCst)
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

/// Builds a new index for a mutation request instead of trusting a cached
/// discovery snapshot.
///
/// Cross-file edits need to observe closed files that changed without a file
/// watcher notification, including files whose previous contents were not
/// connected to the selected function, so this rediscovers and rereads every
/// closed file. Epoch checks on both sides of the build reject concurrent
/// session invalidation.
pub(crate) fn fresh_workspace_function_index(
    context: &WorkspaceFunctionContext,
) -> Option<Arc<WorkspaceFunctionIndex>> {
    if context.cancellation.is_cancelled() || context.cache.current_epoch() != context.epoch {
        return None;
    }
    let built = Arc::new(WorkspaceFunctionIndex::build_with(
        context,
        FileAnalysisCache::new(context.closed_epoch),
    )?);
    if context.cancellation.is_cancelled() || context.cache.current_epoch() != context.epoch {
        return None;
    }
    context.cache.store(context.epoch, built.clone());
    Some(built)
}

/// Per-file analysis reused across index builds within one closed-files epoch.
#[derive(Clone, Default)]
struct FileAnalysisCache {
    closed_epoch: u64,
    discovery: Option<Arc<WorkspaceShellFiles>>,
    source_paths: SourcePathsCache,
    /// Disk snapshots, reused without rereading until evicted.
    closed: HashMap<PathBuf, Arc<AnalyzedWorkspaceFile>>,
    /// Open-buffer analysis, reused while the buffer text is unchanged.
    open: HashMap<PathBuf, Arc<AnalyzedWorkspaceFile>>,
}

impl FileAnalysisCache {
    fn new(closed_epoch: u64) -> Self {
        Self {
            closed_epoch,
            ..Self::default()
        }
    }

    /// Returns the workspace's discovered shell files, walking the workspace
    /// only when no matching snapshot is cached. `None` denotes cancellation.
    fn discovery(
        &mut self,
        context: &WorkspaceFunctionContext,
    ) -> Option<Arc<WorkspaceShellFiles>> {
        if let Some(discovery) = &self.discovery
            && discovery.roots == context.workspace_roots
            && discovery.max_files == context.max_files
        {
            return Some(discovery.clone());
        }
        let discovery = Arc::new(discover_workspace_shell_files(
            &context.workspace_roots,
            context.max_files,
            &context.cancellation,
        )?);
        self.discovery = Some(discovery.clone());
        Some(discovery)
    }

    fn open_file(
        &mut self,
        path: &Path,
        open: &WorkspaceOpenDocument,
        context: &WorkspaceFunctionContext,
    ) -> Arc<AnalyzedWorkspaceFile> {
        let contents = open.document.contents();
        if let Some(cached) = self.open.get(path)
            && cached.source.as_ref() == contents
        {
            return cached.clone();
        }
        let analyzed = Arc::new(AnalyzedWorkspaceFile::analyze(
            path,
            open.uri.clone(),
            contents.into(),
            self.source_paths.resolve(path, context),
        ));
        self.open.insert(path.to_path_buf(), analyzed.clone());
        analyzed
    }

    /// Returns a closed file's analysis, charging its source against the
    /// build's closed-source budget. `None` marks the build incomplete.
    fn closed_file(
        &mut self,
        path: &Path,
        closed_source_bytes: &mut usize,
        context: &WorkspaceFunctionContext,
    ) -> Option<Arc<AnalyzedWorkspaceFile>> {
        if let Some(cached) = self.closed.get(path) {
            let remaining = MAX_CLOSED_SOURCE_BYTES.saturating_sub(*closed_source_bytes);
            if cached.source.len() > remaining {
                tracing::debug!(
                    "workspace functions: skipping {} because it exceeds the source budget",
                    path.display()
                );
                return None;
            }
            *closed_source_bytes += cached.source.len();
            return Some(cached.clone());
        }
        #[cfg(test)]
        context.cache.closed_reads.fetch_add(1, Ordering::SeqCst);
        let source = read_closed_source(path, closed_source_bytes)?;
        let uri = types::Url::from_file_path(path).ok()?;
        let analyzed = Arc::new(AnalyzedWorkspaceFile::analyze(
            path,
            uri,
            source.into(),
            self.source_paths.resolve(path, context),
        ));
        self.closed.insert(path.to_path_buf(), analyzed.clone());
        Some(analyzed)
    }

    /// Keeps only the entries a build used, so the cache never retains more
    /// than one build's worth of files.
    fn retain_used(&mut self, files: &BTreeMap<PathBuf, IndexedWorkspaceFile>) {
        self.closed
            .retain(|path, _| files.get(path).is_some_and(|file| file.version.is_none()));
        self.open
            .retain(|path, _| files.get(path).is_some_and(|file| file.version.is_some()));
    }
}

/// Analysis of one file that does not depend on which other buffers are open.
struct AnalyzedWorkspaceFile {
    /// Canonical file URI.
    uri: types::Url,
    source: Arc<str>,
    line_index: LineIndex,
    content_hash: [u8; 32],
    /// Call facts projected without source edges; edges are linked per build.
    call_facts: FileCallFacts,
    /// Variable facts without source effects; effects are linked per build.
    variables: FileVariableFacts,
    source_refs: Vec<SourceRefTargets>,
    resolution: SourcePathResolution,
}

/// One `source` operation and the canonical paths it may name, in priority
/// order. Which candidate wins depends on the open buffers and the source
/// target scope, so the choice is made when the file is linked.
struct SourceRefTargets {
    span: Span,
    conditional: bool,
    completion_visible: bool,
    candidates: Vec<PathBuf>,
}

impl AnalyzedWorkspaceFile {
    fn analyze(
        path: &Path,
        fallback_uri: types::Url,
        source: Arc<str>,
        resolution: SourcePathResolution,
    ) -> Self {
        let uri = std::fs::canonicalize(path)
            .ok()
            .and_then(|path| types::Url::from_file_path(path).ok())
            .unwrap_or(fallback_uri);
        let shell = ShellDialect::infer(&source, Some(path));
        let model = analyze_editor_document(&source, Some(path), shell);
        let source_refs = model
            .source_refs()
            .iter()
            .map(|source_ref| {
                let scope = model.scope_at(source_ref.span.start.offset());
                let mut candidates = source_ref_candidate_paths(
                    path,
                    source_ref,
                    &resolution.roots,
                    &resolution.project_root,
                );
                if candidates.is_empty()
                    && let Some(candidate) = model.current_file_source_candidate(source_ref, path)
                {
                    candidates.push(candidate);
                }
                SourceRefTargets {
                    span: source_ref.span,
                    conditional: source_ref.conditionally_executed,
                    completion_visible: !source_ref.conditionally_executed
                        && model.enclosing_function_scope(scope).is_none()
                        && model
                            .innermost_transient_scope_within_function(scope)
                            .is_none(),
                    candidates: candidates
                        .into_iter()
                        .map(|candidate| canonical_path(&candidate))
                        .collect(),
                }
            })
            .collect();
        Self {
            uri,
            line_index: LineIndex::new(&source),
            content_hash: content_hash(source.as_bytes()),
            call_facts: FileCallFacts::project_with_source_edges(&model, Vec::new()),
            variables: FileVariableFacts::project(&model),
            source_refs,
            resolution,
            source,
        }
    }
}

/// Attaches resolved source edges to call facts that were projected without
/// any, matching what projecting with those edges would produce.
fn with_source_edges(
    mut facts: FileCallFacts,
    mut edges: Vec<CallFactSourceEdge>,
) -> FileCallFacts {
    edges.sort_by_key(|edge| edge.span.start.offset());
    for effect in &mut facts.source_effects {
        effect.path = edges
            .iter()
            .find(|edge| edge.span == effect.span)
            .map(|edge| edge.path.clone());
    }
    facts.source_edges = edges;
    facts
}

/// Source snapshot retained for one indexed file.
pub(crate) struct IndexedWorkspaceFile {
    analyzed: Arc<AnalyzedWorkspaceFile>,
    open_uri: Option<types::Url>,
    version: Option<DocumentVersion>,
}

impl IndexedWorkspaceFile {
    pub(crate) fn uri(&self) -> &types::Url {
        &self.analyzed.uri
    }

    /// URI used by the editor for an open buffer, falling back to the
    /// canonical file URI for disk snapshots.
    pub(crate) fn editor_uri(&self) -> &types::Url {
        self.open_uri.as_ref().unwrap_or(&self.analyzed.uri)
    }

    pub(crate) fn source(&self) -> &str {
        &self.analyzed.source
    }

    pub(crate) fn line_index(&self) -> &LineIndex {
        &self.analyzed.line_index
    }

    /// Open-document version captured by the request, or `None` for disk input.
    pub(crate) fn version(&self) -> Option<DocumentVersion> {
        self.version
    }

    /// SHA-256 of the exact content used to build semantic facts and ranges.
    pub(crate) fn content_hash(&self) -> [u8; 32] {
        self.analyzed.content_hash
    }
}

/// Shared index queried by cross-file editor features.
pub(crate) struct WorkspaceFunctionIndex {
    graph: WorkspaceCallIndex,
    variables: WorkspaceVariableIndex,
    files: BTreeMap<PathBuf, IndexedWorkspaceFile>,
    encoding: PositionEncoding,
    complete: bool,
}

/// Accumulates linked files while an index is built.
struct IndexLinker<'a> {
    graph: WorkspaceCallIndex,
    variables: WorkspaceVariableIndex,
    files: BTreeMap<PathBuf, IndexedWorkspaceFile>,
    open_paths: &'a BTreeSet<PathBuf>,
    source_scope: &'a SourceTargetScope,
}

impl IndexLinker<'_> {
    /// Resolves the file's source edges against the current open buffers and
    /// inserts it into the call graph, variable index, and file table.
    fn link(
        &mut self,
        path: &Path,
        analyzed: &Arc<AnalyzedWorkspaceFile>,
        open: Option<(&types::Url, DocumentVersion)>,
    ) {
        let edges = analyzed
            .source_refs
            .iter()
            .filter_map(|source_ref| {
                source_ref
                    .candidates
                    .iter()
                    .find(|candidate| {
                        self.open_paths.contains(*candidate)
                            || (candidate.is_file()
                                && self.source_scope.allows(candidate, &analyzed.resolution))
                    })
                    .map(|target| CallFactSourceEdge {
                        path: target.clone(),
                        span: source_ref.span,
                        conditional: source_ref.conditional,
                        completion_visible: source_ref.completion_visible,
                    })
            })
            .collect::<Vec<_>>();
        let call_facts = with_source_edges(analyzed.call_facts.clone(), edges);
        self.variables.insert_facts(
            path.to_path_buf(),
            analyzed
                .variables
                .with_source_effects(&call_facts.source_effects),
        );
        self.graph.insert(path.to_path_buf(), call_facts);
        self.files.insert(
            path.to_path_buf(),
            IndexedWorkspaceFile {
                analyzed: analyzed.clone(),
                open_uri: open.map(|(uri, _)| uri.clone()),
                version: open.map(|(_, version)| version),
            },
        );
    }
}

impl WorkspaceFunctionIndex {
    fn build(context: &WorkspaceFunctionContext) -> Option<Self> {
        Self::build_with(context, context.cache.analyses_for(context.closed_epoch))
    }

    fn build_with(
        context: &WorkspaceFunctionContext,
        mut analyses: FileAnalysisCache,
    ) -> Option<Self> {
        let mut complete = true;
        let max_files = context.max_files;

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
        let source_scope = SourceTargetScope::new(context, &open_paths);
        let mut closed_source_bytes = 0usize;
        let mut linker = IndexLinker {
            graph: WorkspaceCallIndex::new(),
            variables: WorkspaceVariableIndex::default(),
            files: BTreeMap::new(),
            open_paths: &open_paths,
            source_scope: &source_scope,
        };

        for (path, open) in &open_docs {
            if context.cancellation.is_cancelled() {
                return None;
            }
            if linker.graph.file_count() >= max_files {
                complete = false;
                tracing::warn!(
                    "workspace functions: open documents exceed the {max_files}-file limit; \
                     indexing only the first {max_files}"
                );
                break;
            }
            let analyzed = analyses.open_file(path, open, context);
            complete &= analyzed.resolution.complete;
            linker.link(path, &analyzed, Some((&open.uri, open.document.version())));
        }

        if context.cancellation.is_cancelled() {
            return None;
        }
        let discovery = analyses.discovery(context)?;
        complete &= discovery.complete;
        let remaining = max_files.saturating_sub(linker.graph.file_count());
        let closed_files = discovery
            .files
            .iter()
            .filter(|path| !open_paths.contains(*path))
            .collect::<Vec<_>>();
        if closed_files.len() > remaining {
            complete = false;
            tracing::warn!(
                "workspace functions: workspace has more than {remaining} closed shell files; \
                 indexing only {remaining}"
            );
        }
        for file in closed_files.into_iter().take(remaining) {
            if context.cancellation.is_cancelled() {
                return None;
            }
            let Some(analyzed) = analyses.closed_file(file, &mut closed_source_bytes, context)
            else {
                complete = false;
                continue;
            };
            complete &= analyzed.resolution.complete;
            linker.link(file, &analyzed, None);
        }

        'expand: loop {
            if context.cancellation.is_cancelled() {
                return None;
            }
            let missing = linker
                .graph
                .files()
                .flat_map(|(_, facts)| facts.source_edges.iter().map(|edge| edge.path.clone()))
                .filter(|target| !linker.graph.contains(target))
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
                if linker.graph.file_count() >= max_files {
                    complete = false;
                    tracing::warn!(
                        "workspace functions: source-edge targets exceed the {max_files}-file \
                         limit; cross-file results may be incomplete"
                    );
                    break 'expand;
                }
                if let Some(open) = open_docs
                    .iter()
                    .find_map(|(path, open)| (path == &target).then_some(*open))
                {
                    let analyzed = analyses.open_file(&target, open, context);
                    complete &= analyzed.resolution.complete;
                    linker.link(
                        &target,
                        &analyzed,
                        Some((&open.uri, open.document.version())),
                    );
                    continue;
                }
                let Some(analyzed) =
                    analyses.closed_file(&target, &mut closed_source_bytes, context)
                else {
                    complete = false;
                    linker
                        .graph
                        .insert(target.clone(), FileCallFacts::default());
                    continue;
                };
                complete &= analyzed.resolution.complete;
                linker.link(&target, &analyzed, None);
            }
        }

        analyses.retain_used(&linker.files);
        context.cache.store_analyses(analyses);

        Some(Self {
            graph: linker.graph,
            variables: linker.variables,
            files: linker.files,
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
        cancellation: &RequestCancellationToken,
    ) -> Option<CrossFileCall> {
        self.graph
            .resolve_call_site_exact_cancellable(from_path, name_span, || {
                cancellation.is_cancelled()
            })
    }

    pub(crate) fn exact_function_reference_locations(
        &self,
        target_path: &Path,
        target_node: &CallNodeKind,
        cancellation: &RequestCancellationToken,
    ) -> Option<Vec<types::Location>> {
        if !self.complete || cancellation.is_cancelled() {
            return None;
        }
        let references = self
            .graph
            .exact_function_references(target_path, target_node, || cancellation.is_cancelled())?;
        let mut locations = Vec::with_capacity(references.len());
        for reference in references {
            if cancellation.is_cancelled() {
                return None;
            }
            let file = self.file(&reference.path)?;
            locations.push(types::Location {
                uri: file.editor_uri().clone(),
                range: crate::edit::to_lsp_range(
                    reference.span.to_range(),
                    file.source(),
                    file.line_index(),
                    self.encoding,
                ),
            });
        }
        Some(locations)
    }

    pub(crate) fn variable_definition_locations(
        &self,
        from_path: &Path,
        target: &WorkspaceVariableTarget,
        cancellation: &RequestCancellationToken,
    ) -> Option<Vec<types::Location>> {
        if !self.complete || cancellation.is_cancelled() {
            return None;
        }
        let occurrences = self
            .variables
            .definitions(from_path, target, &|| cancellation.is_cancelled())?;
        self.variable_locations(occurrences, cancellation)
    }

    pub(crate) fn variable_reference_locations(
        &self,
        from_path: &Path,
        target: &WorkspaceVariableTarget,
        include_declaration: bool,
        cancellation: &RequestCancellationToken,
    ) -> Option<Vec<types::Location>> {
        if !self.complete || cancellation.is_cancelled() {
            return None;
        }
        let occurrences =
            self.variables
                .references(from_path, target, include_declaration, &|| {
                    cancellation.is_cancelled()
                })?;
        self.variable_locations(occurrences, cancellation)
    }

    fn variable_locations(
        &self,
        occurrences: Vec<crate::workspace_variables::WorkspaceVariableOccurrence>,
        cancellation: &RequestCancellationToken,
    ) -> Option<Vec<types::Location>> {
        let mut locations = Vec::with_capacity(occurrences.len());
        for occurrence in occurrences {
            if cancellation.is_cancelled() {
                return None;
            }
            let file = self.file(&occurrence.path)?;
            locations.push(types::Location {
                uri: file.editor_uri().clone(),
                range: crate::edit::to_lsp_range(
                    occurrence.span.to_range(),
                    file.source(),
                    file.line_index(),
                    self.encoding,
                ),
            });
        }
        Some(locations)
    }

    pub(crate) fn exact_function_rename(
        &self,
        target_path: &Path,
        target_node: &CallNodeKind,
        cancellation: &RequestCancellationToken,
    ) -> Option<Result<ExactFunctionRename, ExactFunctionRenameError>> {
        if cancellation.is_cancelled() {
            return None;
        }
        self.graph
            .exact_function_rename(target_path, target_node, || cancellation.is_cancelled())
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

    pub(crate) fn source_target(
        &self,
        from_path: &Path,
        source_span: Span,
    ) -> Option<(&Path, &types::Url)> {
        let target = self.graph.source_target(from_path, source_span)?;
        self.file(target).map(|file| (target, file.editor_uri()))
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

    pub(crate) fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    pub(crate) fn ranges_in(&self, path: &Path, spans: &[Span]) -> Vec<types::Range> {
        spans
            .iter()
            .filter_map(|span| self.range_of(path, *span))
            .collect()
    }

    /// Whether discovery and source-edge expansion completed within the file
    /// budget and without unreadable inputs.
    pub(crate) fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns whether a closed file still matches the content used to build
    /// this index. Open documents are versioned by the LSP session instead.
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

    /// Verifies every closed snapshot retained by this index, including files
    /// which were disconnected from the selected binding when the index was
    /// built. The outer `None` denotes cancellation.
    pub(crate) fn validate_closed_files(
        &self,
        cancellation: &RequestCancellationToken,
    ) -> Option<Result<(), PathBuf>> {
        for (path, file) in &self.files {
            if cancellation.is_cancelled() {
                return None;
            }
            if file.version().is_none() && !self.closed_file_is_current(path) {
                return Some(Err(path.clone()));
            }
        }
        Some(Ok(()))
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
    /// Canonical project root and `source-path` roots that closed-file
    /// `source` edges may follow into.
    scope_roots: Vec<PathBuf>,
    complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SourcePathsCacheKey {
    project_root: PathBuf,
    workspace_root: Option<PathBuf>,
}

#[derive(Clone, Default)]
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
                let roots = config.lint.source_paths.unwrap_or_default();
                let scope_roots = std::iter::once(canonical_path(&project_root))
                    .chain(roots.iter().map(|root| {
                        let root = Path::new(root);
                        if root.is_absolute() {
                            canonical_path(root)
                        } else {
                            canonical_path(&project_root.join(root))
                        }
                    }))
                    .collect();
                SourcePathResolution {
                    roots,
                    project_root: project_root.clone(),
                    scope_roots,
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

/// Read a closed file for the index, charging it against the build's total
/// source budget. Returns `None` for unreadable, oversized, or over-budget
/// files, which leave the index incomplete.
fn read_closed_source(path: &Path, closed_source_bytes: &mut usize) -> Option<String> {
    let remaining = MAX_CLOSED_SOURCE_BYTES.saturating_sub(*closed_source_bytes);
    let max_bytes = crate::workspace::MAX_WORKSPACE_FILE_BYTES.min(remaining);
    match crate::workspace::read_source_bounded(path, max_bytes) {
        Ok(Some(source)) => {
            *closed_source_bytes += source.len();
            Some(source)
        }
        Ok(None) => {
            tracing::debug!(
                "workspace functions: skipping {} because it exceeds the source budget",
                path.display()
            );
            None
        }
        Err(error) => {
            tracing::debug!(
                "workspace functions: failed to read {}: {error}",
                path.display()
            );
            None
        }
    }
}

/// Directories a `source` edge may follow a closed file into.
///
/// Edges are confined to the workspace, the directories of open buffers, and
/// each file's project root and configured `source-path` roots, so a script
/// naming an arbitrary absolute path cannot make the server load files from
/// elsewhere on disk.
struct SourceTargetScope {
    roots: Vec<PathBuf>,
}

impl SourceTargetScope {
    fn new(context: &WorkspaceFunctionContext, open_paths: &BTreeSet<PathBuf>) -> Self {
        let mut roots = context
            .workspace_roots
            .iter()
            .chain(&context.settings_workspace_roots)
            .map(|root| canonical_path(root))
            .chain(
                open_paths
                    .iter()
                    .filter_map(|path| path.parent().map(Path::to_path_buf)),
            )
            .collect::<Vec<_>>();
        roots.sort();
        roots.dedup();
        Self { roots }
    }

    fn allows(&self, target: &Path, source_paths: &SourcePathResolution) -> bool {
        self.roots
            .iter()
            .chain(&source_paths.scope_roots)
            .any(|root| target.starts_with(root))
    }
}

/// Shell files discovered under the workspace roots, open buffers included.
struct WorkspaceShellFiles {
    roots: Vec<PathBuf>,
    max_files: usize,
    files: BTreeSet<PathBuf>,
    complete: bool,
}

/// Walks the workspace roots for shell files. Open buffers are not excluded,
/// so the snapshot stays valid as buffers open and close; each build filters
/// them out. Walks stop after `max_files + 1` files, enough to detect that a
/// build would be truncated however many of them are open.
fn discover_workspace_shell_files(
    roots: &[PathBuf],
    max_files: usize,
    cancellation: &RequestCancellationToken,
) -> Option<WorkspaceShellFiles> {
    use shuck_discover::{DiscoveryOptions, FileKind, discover_files_with_status};

    let mut files = BTreeSet::new();
    let mut complete = true;
    for root in roots {
        if cancellation.is_cancelled() {
            return None;
        }
        let limit = max_files.saturating_add(1).saturating_sub(files.len());
        let discovered = match discover_files_with_status(
            std::slice::from_ref(root),
            root,
            &DiscoveryOptions {
                respect_gitignore: true,
                parallel: true,
                use_config_roots: true,
                max_files: Some(limit),
                cancellation: Some(cancellation.discovery_token()),
                include_embedded: false,
                ..DiscoveryOptions::default()
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                complete = false;
                tracing::warn!(
                    "workspace functions: failed to discover files in {}: {error}",
                    root.display()
                );
                continue;
            }
        };
        if cancellation.is_cancelled() {
            return None;
        }
        complete &= discovered.complete;
        for file in discovered.files {
            if file.kind != FileKind::Shell {
                continue;
            }
            files.insert(canonical_path(&file.absolute_path));
        }
        if files.len() > max_files {
            break;
        }
    }
    Some(WorkspaceShellFiles {
        roots: roots.to_vec(),
        max_files,
        files,
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
            closed_epoch: 0,
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

    fn context_with_open_source(workspace: &Path, source: String) -> WorkspaceFunctionContext {
        let mut context = context_for(workspace, 100);
        context.open_documents = vec![WorkspaceOpenDocument {
            uri: types::Url::from_file_path(workspace.join("main.sh")).unwrap(),
            document: Arc::new(TextDocument::new(source, 1).with_language_id("shellscript")),
        }];
        context
    }

    #[test]
    fn source_edges_are_not_followed_outside_the_workspace() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(workspace_dir.path()).unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = std::fs::canonicalize(outside_dir.path()).unwrap();
        let outside_target = outside.join("secret.sh");
        std::fs::write(&outside_target, "outside_fn() { :; }\n").unwrap();
        let inside_target = workspace.join("lib.sh");
        std::fs::write(&inside_target, "inside_fn() { :; }\n").unwrap();

        let context = context_with_open_source(
            &workspace,
            format!(
                "source {}\nsource {}\noutside_fn\ninside_fn\n",
                outside_target.display(),
                inside_target.display()
            ),
        );
        let built = WorkspaceFunctionIndex::build(&context).unwrap();

        assert!(built.contains(&inside_target));
        assert!(!built.contains(&outside_target));
        assert!(built.file(&outside_target).is_none());
    }

    #[test]
    fn oversized_closed_files_are_skipped_and_mark_partial_builds() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(workspace_dir.path()).unwrap();
        let large = workspace.join("large.sh");
        let mut source = "big_fn() { :; }\n".to_owned();
        source.push_str(&"#".repeat(crate::workspace::MAX_WORKSPACE_FILE_BYTES));
        std::fs::write(&large, source).unwrap();
        std::fs::write(workspace.join("small.sh"), "small_fn() { :; }\n").unwrap();

        let context = context_with_open_source(&workspace, "small_fn\n".to_owned());
        let built = WorkspaceFunctionIndex::build(&context).unwrap();

        assert!(built.contains(&workspace.join("small.sh")));
        assert!(built.file(&large).is_none());
        assert!(!built.is_complete());
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
    fn incomplete_indexes_do_not_return_variable_definitions() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);

        let context = context_for(&workspace, 1);
        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        assert!(!built.is_complete());

        let path = workspace.join("open_a.sh");
        let model = analyze_editor_document("SHARED=1\n", Some(&path), ShellDialect::Bash);
        let symbol = model
            .editor_query()
            .target_at_offset(1)
            .expect("the assignment should be an editor target");
        let target = crate::workspace_variables::variable_target(&model, &symbol)
            .expect("the assignment should be a file variable");

        assert!(
            built
                .variable_definition_locations(&path, &target, &context.cancellation)
                .is_none()
        );
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
            closed_epoch: 0,
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

    #[test]
    fn shellcheck_source_path_comment_links_script_dir_source() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        let caller_path = workspace.join("caller.sh");
        let target_path = workspace.join("helper.sh");
        std::fs::write(&target_path, "helper() { :; }\n").unwrap();
        let source = "#!/bin/sh\n# shellcheck source-path=SCRIPTDIR\n. \"$(dirname \"$0\")/helper.sh\"\nhelper\n";
        let mut context = context_for(&workspace, 100);
        context.open_documents = vec![WorkspaceOpenDocument {
            uri: types::Url::from_file_path(&caller_path).unwrap(),
            document: Arc::new(
                TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
            ),
        }];

        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        let caller_facts = built
            .graph
            .files()
            .find_map(|(path, facts)| (path == caller_path).then_some(facts))
            .unwrap();
        assert_eq!(caller_facts.source_edges.len(), 1);
        assert_eq!(caller_facts.source_edges[0].path, target_path);
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
            closed_epoch: 0,
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
            closed_epoch: 0,
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
            closed_epoch: 0,
            cancellation: RequestCancellationToken::default(),
        };
        let built = WorkspaceFunctionIndex::build(&context).unwrap();
        assert!(built.closed_file_is_current(&file));
        assert_eq!(
            built.validate_closed_files(&context.cancellation),
            Some(Ok(()))
        );
        std::fs::write(&file, "two() { :; }\n").unwrap();
        assert!(!built.closed_file_is_current(&file));
        assert_eq!(
            built.validate_closed_files(&context.cancellation),
            Some(Err(file))
        );
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
            closed_epoch: 0,
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
            closed_epoch: 0,
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
            closed_epoch: 0,
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

    fn open_document(path: &Path, source: &str, version: i32) -> WorkspaceOpenDocument {
        WorkspaceOpenDocument {
            uri: types::Url::from_file_path(path).unwrap(),
            document: Arc::new(
                TextDocument::new(source.to_owned(), version).with_language_id("shellscript"),
            ),
        }
    }

    /// Captures a context the way the session does, against a shared cache.
    fn context_with_cache(
        workspace: &Path,
        cache: &Arc<WorkspaceFunctionIndexCache>,
        open_documents: Vec<WorkspaceOpenDocument>,
    ) -> WorkspaceFunctionContext {
        let mut context = context_for(workspace, 100);
        context.open_documents = open_documents;
        context.cache = cache.clone();
        context.epoch = cache.current_epoch();
        context.closed_epoch = cache.current_closed_epoch();
        context
    }

    fn changed(path: &Path) -> types::FileEvent {
        types::FileEvent {
            uri: types::Url::from_file_path(path).unwrap(),
            typ: types::FileChangeType::CHANGED,
        }
    }

    #[test]
    fn buffer_edits_reuse_cached_closed_file_analysis() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        populate_workspace(&workspace);
        let cache = Arc::new(WorkspaceFunctionIndexCache::default());
        let open_a = context_for(&workspace, 100).open_documents.remove(0);
        let open_b_path = workspace.join("open_b.sh");

        let first = workspace_function_index(&context_with_cache(
            &workspace,
            &cache,
            vec![
                open_a.clone(),
                open_document(&open_b_path, "b() { :; }\n", 1),
            ],
        ))
        .unwrap();
        // Four discovered closed files plus the ignored `source` target.
        assert_eq!(cache.closed_reads(), 5);
        assert_eq!(first.file_count(), 7);

        cache.invalidate();
        let edited = workspace_function_index(&context_with_cache(
            &workspace,
            &cache,
            vec![
                open_a.clone(),
                open_document(&open_b_path, "b() { :; }\nadded() { :; }\n", 2),
            ],
        ))
        .unwrap();
        assert!(!Arc::ptr_eq(&first, &edited));
        assert_eq!(
            cache.closed_reads(),
            5,
            "a buffer edit must not reread closed files"
        );
        assert_eq!(edited.file_count(), 7);
        assert!(edited.is_complete());
        assert!(edited.contains(&workspace.join("vendored/edge.sh")));
        let open_b = edited.file(&open_b_path).unwrap();
        assert!(open_b.source().contains("added"));
        assert_eq!(open_b.version(), Some(2));

        let closed = workspace.join("closed_1.sh");
        std::fs::write(&closed, "rewritten() { :; }\n").unwrap();
        cache.invalidate_file_events(&[changed(&closed)], false);
        let after_save = workspace_function_index(&context_with_cache(
            &workspace,
            &cache,
            vec![
                open_a.clone(),
                open_document(&open_b_path, "b() { :; }\nadded() { :; }\n", 2),
            ],
        ))
        .unwrap();
        assert_eq!(cache.closed_reads(), 6, "only the changed file is reread");
        assert!(
            after_save
                .file(&closed)
                .unwrap()
                .source()
                .contains("rewritten")
        );
        assert!(after_save.closed_file_is_current(&closed));

        cache.invalidate_closed_files();
        workspace_function_index(&context_with_cache(
            &workspace,
            &cache,
            vec![open_a, open_document(&open_b_path, "b() { :; }\n", 3)],
        ))
        .unwrap();
        assert_eq!(cache.closed_reads(), 11);
    }

    #[test]
    fn created_files_are_discovered_after_watched_file_events() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        std::fs::write(workspace.join("one.sh"), "one() { :; }\n").unwrap();
        let cache = Arc::new(WorkspaceFunctionIndexCache::default());
        let first =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert_eq!(first.file_count(), 1);

        let created = workspace.join("two.sh");
        std::fs::write(&created, "two() { :; }\n").unwrap();
        cache.invalidate_file_events(
            &[types::FileEvent {
                uri: types::Url::from_file_path(&created).unwrap(),
                typ: types::FileChangeType::CREATED,
            }],
            false,
        );
        let second =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert!(second.contains(&created));

        // A content change that turns an extensionless file into a script
        // changes discovery even though the file was not indexed before.
        let extensionless = workspace.join("tool");
        std::fs::write(&extensionless, "plain text\n").unwrap();
        cache.invalidate_closed_files();
        let third =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert!(!third.contains(&extensionless));
        std::fs::write(&extensionless, "#!/bin/sh\ntool() { :; }\n").unwrap();
        cache.invalidate_file_events(&[changed(&extensionless)], false);
        let fourth =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert!(fourth.contains(&extensionless));
    }

    #[test]
    fn opening_and_closing_buffers_relinks_cached_closed_sources() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        let caller = workspace.join("caller.sh");
        let target = workspace.join("new_target.sh");
        std::fs::write(&caller, "source new_target.sh\nfrom_buffer\n").unwrap();
        let cache = Arc::new(WorkspaceFunctionIndexCache::default());
        let caller_edges = |index: &WorkspaceFunctionIndex| {
            index
                .graph
                .files()
                .find_map(|(path, facts)| (path == caller).then(|| facts.source_edges.clone()))
                .unwrap()
        };

        let unopened =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert!(caller_edges(&unopened).is_empty());
        assert_eq!(cache.closed_reads(), 1);

        cache.invalidate();
        let opened = workspace_function_index(&context_with_cache(
            &workspace,
            &cache,
            vec![open_document(&target, "from_buffer() { :; }\n", 1)],
        ))
        .unwrap();
        assert_eq!(cache.closed_reads(), 1);
        let edges = caller_edges(&opened);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].path, target);
        let call = opened
            .graph
            .files()
            .find_map(|(path, facts)| (path == caller).then_some(facts))
            .unwrap()
            .call_sites
            .iter()
            .find(|call| call.callee.as_str() == "from_buffer")
            .unwrap()
            .name_span;
        assert_eq!(
            opened.resolve_call_site(&caller, call).unwrap().path,
            target
        );

        cache.invalidate_closed_document(&types::Url::from_file_path(&target).unwrap());
        let closed =
            workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert_eq!(cache.closed_reads(), 1);
        assert!(caller_edges(&closed).is_empty());
        assert!(!closed.contains(&target));
    }

    #[test]
    fn builds_from_a_stale_closed_epoch_neither_reuse_nor_store_analysis() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        std::fs::write(workspace.join("one.sh"), "one() { :; }\n").unwrap();
        let cache = Arc::new(WorkspaceFunctionIndexCache::default());
        workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert_eq!(cache.closed_reads(), 1);

        let stale = context_with_cache(&workspace, &cache, Vec::new());
        cache.invalidate_closed_files();
        WorkspaceFunctionIndex::build(&stale).unwrap();
        assert_eq!(cache.closed_reads(), 2);

        workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert_eq!(cache.closed_reads(), 3);
        workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();
        assert_eq!(cache.closed_reads(), 3);
    }

    #[test]
    fn fresh_indexes_reread_closed_files() {
        let tempdir = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(tempdir.path()).unwrap();
        let file = workspace.join("one.sh");
        std::fs::write(&file, "one() { :; }\n").unwrap();
        let cache = Arc::new(WorkspaceFunctionIndexCache::default());
        workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new())).unwrap();

        // No watched-file event: a mutation request must still see the edit.
        std::fs::write(&file, "two() { :; }\n").unwrap();
        cache.invalidate();
        let fresh =
            fresh_workspace_function_index(&context_with_cache(&workspace, &cache, Vec::new()))
                .unwrap();
        assert_eq!(cache.closed_reads(), 2);
        assert!(fresh.file(&file).unwrap().source().contains("two"));
    }

    #[test]
    fn relinked_source_edges_match_direct_projection() {
        let path = Path::new("/workspace/main.sh");
        let source = "source a.sh\nif x; then . b.sh; fi\nf() { source c.sh; }\nsource \"$dyn\"\n";
        let model = analyze_editor_document(source, Some(path), ShellDialect::Bash);
        let refs = model.source_refs();
        assert!(refs.len() >= 3);
        // Resolve every other source operation, out of order, to cover both
        // linked and unresolved effects.
        let edges = refs
            .iter()
            .enumerate()
            .rev()
            .filter(|(index, _)| index % 2 == 0)
            .map(|(index, source_ref)| CallFactSourceEdge {
                path: PathBuf::from(format!("/workspace/target_{index}.sh")),
                span: source_ref.span,
                conditional: source_ref.conditionally_executed,
                completion_visible: index == 0,
            })
            .collect::<Vec<_>>();

        let direct = FileCallFacts::project_with_source_edges(&model, edges.clone());
        let relinked = with_source_edges(
            FileCallFacts::project_with_source_edges(&model, Vec::new()),
            edges,
        );
        assert_eq!(relinked, direct);
    }
}
