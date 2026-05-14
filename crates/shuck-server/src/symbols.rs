use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use lsp_types as types;
use sha2::{Digest, Sha256};
use shuck_discover::{DiscoveredFile, DiscoveryOptions, FileKind, discover_files};
use shuck_indexer::LineIndex;
use shuck_linter::ShellDialect;
use shuck_parser::parser::Parser;
use shuck_semantic::{EditorDocumentSymbol, EditorSymbolKind, SemanticBuildOptions, SemanticModel};

use crate::PositionEncoding;
use crate::TextDocument;
use crate::edit::DocumentVersion;
use crate::session::{
    Client, ClientOptions, DocumentSnapshot, ShuckSettings, WorkspaceSettingsSnapshot,
    WorkspaceSymbolFeatureOptions,
};

pub(crate) type DocumentSymbolResponse = Option<types::DocumentSymbolResponse>;
pub(crate) type WorkspaceSymbolResponse = Option<types::WorkspaceSymbolResponse>;

#[derive(Clone)]
pub(crate) struct WorkspaceSymbolContext {
    pub(crate) index: Arc<WorkspaceSymbolIndex>,
    pub(crate) options: WorkspaceSymbolFeatureOptions,
    pub(crate) global_options: ClientOptions,
    pub(crate) workspace_settings: Vec<WorkspaceSettingsSnapshot>,
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) settings_workspace_roots: Vec<PathBuf>,
    pub(crate) open_documents: Vec<WorkspaceOpenDocument>,
    pub(crate) encoding: PositionEncoding,
}

#[derive(Clone)]
pub(crate) struct WorkspaceOpenDocument {
    uri: types::Url,
    document: Arc<TextDocument>,
}

pub(crate) struct WorkspaceSymbolIndex {
    cache: Mutex<WorkspaceSymbolCache>,
    rebuild_finished: Condvar,
}

#[derive(Debug)]
struct WorkspaceSymbolCache {
    summaries: BTreeMap<String, WorkspaceSymbolSummary>,
    partial: bool,
    dirty: bool,
    rebuilding: bool,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceSymbolSummary {
    uri: types::Url,
    version: Option<DocumentVersion>,
    content_hash: [u8; 32],
    symbols: Vec<WorkspaceSymbolEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceSymbolEntry {
    name: String,
    kind: types::SymbolKind,
    container_name: Option<String>,
    range: types::Range,
    selection_range: types::Range,
}

#[derive(Clone)]
struct WorkspaceSymbolCandidate {
    uri: types::Url,
    symbol: WorkspaceSymbolEntry,
}

struct ScoredWorkspaceSymbolCandidate {
    score: SymbolQueryScore,
    name_folded: String,
    candidate: WorkspaceSymbolCandidate,
}

struct WorkspaceSymbolRebuildGuard<'a> {
    index: &'a WorkspaceSymbolIndex,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolQueryScore {
    rank: u8,
    penalty: usize,
}

impl Default for WorkspaceSymbolCache {
    fn default() -> Self {
        Self {
            summaries: BTreeMap::new(),
            partial: false,
            dirty: true,
            rebuilding: false,
            generation: 0,
        }
    }
}

impl Default for WorkspaceSymbolIndex {
    fn default() -> Self {
        Self {
            cache: Mutex::new(WorkspaceSymbolCache::default()),
            rebuild_finished: Condvar::new(),
        }
    }
}

impl WorkspaceOpenDocument {
    pub(crate) fn new(uri: types::Url, document: Arc<TextDocument>) -> Self {
        Self { uri, document }
    }
}

impl WorkspaceSymbolIndex {
    pub(crate) fn invalidate_all(&self) {
        let mut cache = lock_or_recover(&self.cache);
        cache.summaries.clear();
        cache.partial = false;
        cache.dirty = true;
        cache.generation = cache.generation.wrapping_add(1);
        drop(cache);
        self.rebuild_finished.notify_all();
    }

    pub(crate) fn invalidate_file_events(&self, changes: &[types::FileEvent]) {
        let changed_uris = changes
            .iter()
            .map(|change| change.uri.as_str().to_owned())
            .collect::<Vec<_>>();
        if changed_uris.is_empty() {
            return;
        }

        let mut cache = lock_or_recover(&self.cache);
        for uri in changed_uris {
            cache.summaries.remove(&uri);
        }
        cache.dirty = true;
        cache.generation = cache.generation.wrapping_add(1);
        drop(cache);
        self.rebuild_finished.notify_all();
    }

    pub(crate) fn invalidate_uri(&self, uri: &types::Url) {
        self.invalidate_uri_key(uri.as_str());
    }

    fn invalidate_uri_key(&self, uri: &str) {
        let mut cache = lock_or_recover(&self.cache);
        cache.summaries.remove(uri);
        cache.dirty = true;
        cache.generation = cache.generation.wrapping_add(1);
        drop(cache);
        self.rebuild_finished.notify_all();
    }

    fn closed_summaries(
        &self,
        context: &WorkspaceSymbolContext,
        client: &Client,
    ) -> crate::server::Result<(Vec<WorkspaceSymbolSummary>, bool)> {
        loop {
            let generation = {
                let mut cache = lock_or_recover(&self.cache);
                while cache.dirty && cache.rebuilding {
                    cache = wait_or_recover(&self.rebuild_finished, cache);
                }

                if !cache.dirty {
                    return Ok(clone_cached_summaries(&cache));
                }

                cache.rebuilding = true;
                cache.generation
            };

            let mut rebuild_guard = WorkspaceSymbolRebuildGuard::new(self);
            let rebuilt = rebuild_closed_workspace_symbols(context)?;
            let mut cache = lock_or_recover(&self.cache);

            cache.rebuilding = false;

            if cache.generation != generation {
                drop(cache);
                self.rebuild_finished.notify_all();
                rebuild_guard.disarm();
                continue;
            }

            cache.summaries = rebuilt
                .summaries
                .into_iter()
                .map(|summary| (summary.uri.as_str().to_owned(), summary))
                .collect();
            cache.partial = rebuilt.partial;
            cache.dirty = false;
            let partial = cache.partial;
            let summaries = clone_cached_summaries(&cache);
            drop(cache);
            self.rebuild_finished.notify_all();
            rebuild_guard.disarm();

            if partial {
                let _ = client.log_message(
                    format!(
                        "Shuck workspace symbol index is partial; indexed the first {} closed shell files",
                        context.options.max_files
                    ),
                    types::MessageType::INFO,
                );
            }

            return Ok(summaries);
        }
    }
}

impl<'a> WorkspaceSymbolRebuildGuard<'a> {
    fn new(index: &'a WorkspaceSymbolIndex) -> Self {
        Self {
            index,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for WorkspaceSymbolRebuildGuard<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let mut cache = lock_or_recover(&self.index.cache);
        cache.rebuilding = false;
        drop(cache);
        self.index.rebuild_finished.notify_all();
    }
}

pub(crate) fn document_symbols(
    snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentSymbolParams,
) -> crate::server::Result<DocumentSymbolResponse> {
    let Some(shell) = crate::lint::document_shell(&snapshot) else {
        return Ok(None);
    };

    let query = snapshot.query();
    let source = query.document().contents();
    let editor_symbols = editor_document_symbols(source, query.file_path().as_deref(), shell);
    let render = SymbolRenderContext {
        uri: query.file_url(),
        source,
        line_index: query.document().index(),
        encoding: snapshot.encoding(),
    };

    if snapshot
        .resolved_client_capabilities()
        .hierarchical_document_symbols
    {
        let symbols = editor_symbols
            .iter()
            .map(|symbol| to_lsp_document_symbol(symbol, &render))
            .collect();
        Ok(Some(types::DocumentSymbolResponse::Nested(symbols)))
    } else {
        let symbols = editor_symbols
            .iter()
            .flat_map(|symbol| to_lsp_symbol_information(symbol, &render, None))
            .collect();
        Ok(Some(types::DocumentSymbolResponse::Flat(symbols)))
    }
}

pub(crate) fn workspace_symbols(
    context: WorkspaceSymbolContext,
    client: &Client,
    params: types::WorkspaceSymbolParams,
) -> crate::server::Result<WorkspaceSymbolResponse> {
    if !workspace_symbols_have_enabled_scope(&context) {
        return Ok(Some(types::WorkspaceSymbolResponse::Nested(Vec::new())));
    }

    let mut summaries = context
        .open_documents
        .iter()
        .filter(|document| workspace_symbol_options_for_uri(&context, &document.uri).enabled)
        .filter_map(|document| workspace_symbol_summary_from_open_document(&context, document))
        .collect::<Vec<_>>();
    let open_uris = context
        .open_documents
        .iter()
        .filter(|document| workspace_symbol_options_for_uri(&context, &document.uri).enabled)
        .flat_map(|document| workspace_uri_keys(&document.uri))
        .collect::<std::collections::BTreeSet<_>>();
    let (closed_summaries, _partial) = context.index.closed_summaries(&context, client)?;
    summaries.extend(
        closed_summaries
            .into_iter()
            .filter(|summary| !open_uris.contains(summary.uri.as_str())),
    );

    let symbols = workspace_symbol_candidates(&summaries, &params.query)
        .into_iter()
        .map(to_lsp_workspace_symbol)
        .collect();
    Ok(Some(types::WorkspaceSymbolResponse::Nested(symbols)))
}

#[allow(deprecated)]
struct SymbolRenderContext<'a> {
    uri: &'a types::Url,
    source: &'a str,
    line_index: &'a LineIndex,
    encoding: PositionEncoding,
}

#[allow(deprecated)]
fn to_lsp_document_symbol(
    symbol: &EditorDocumentSymbol,
    render: &SymbolRenderContext<'_>,
) -> types::DocumentSymbol {
    let children = (!symbol.children.is_empty()).then(|| {
        symbol
            .children
            .iter()
            .map(|child| to_lsp_document_symbol(child, render))
            .collect()
    });

    types::DocumentSymbol {
        name: symbol.name.to_string(),
        detail: None,
        kind: to_lsp_symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        range: crate::edit::to_lsp_range(
            symbol.range.to_range(),
            render.source,
            render.line_index,
            render.encoding,
        ),
        selection_range: crate::edit::to_lsp_range(
            symbol.selection_span.to_range(),
            render.source,
            render.line_index,
            render.encoding,
        ),
        children,
    }
}

#[allow(deprecated)]
fn to_lsp_symbol_information(
    symbol: &EditorDocumentSymbol,
    render: &SymbolRenderContext<'_>,
    container_name: Option<&str>,
) -> Vec<types::SymbolInformation> {
    let mut symbols = vec![types::SymbolInformation {
        name: symbol.name.to_string(),
        kind: to_lsp_symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        location: types::Location::new(
            render.uri.clone(),
            crate::edit::to_lsp_range(
                symbol.selection_span.to_range(),
                render.source,
                render.line_index,
                render.encoding,
            ),
        ),
        container_name: container_name.map(str::to_owned),
    }];

    symbols.extend(
        symbol
            .children
            .iter()
            .flat_map(|child| to_lsp_symbol_information(child, render, Some(symbol.name.as_str()))),
    );
    symbols
}

fn to_lsp_symbol_kind(kind: EditorSymbolKind) -> types::SymbolKind {
    match kind {
        EditorSymbolKind::Function => types::SymbolKind::FUNCTION,
        EditorSymbolKind::Array | EditorSymbolKind::AssociativeArray => types::SymbolKind::ARRAY,
        EditorSymbolKind::Variable
        | EditorSymbolKind::Declaration
        | EditorSymbolKind::RuntimeName => types::SymbolKind::VARIABLE,
    }
}

fn editor_document_symbols(
    source: &str,
    path: Option<&Path>,
    shell: ShellDialect,
) -> Vec<EditorDocumentSymbol> {
    let shell_profile = shell.shell_profile();
    let parse_result = Parser::with_profile(source, shell_profile.clone()).parse();
    let indexer = shuck_indexer::Indexer::new(source, &parse_result);
    let semantic = SemanticModel::build_with_options(
        &parse_result.file,
        source,
        &indexer,
        SemanticBuildOptions {
            source_path: path,
            shell_profile: Some(shell_profile),
            resolve_source_closure: false,
            ..SemanticBuildOptions::default()
        },
    );
    semantic.editor_query().document_symbols()
}

fn workspace_symbol_summary_from_open_document(
    context: &WorkspaceSymbolContext,
    open_document: &WorkspaceOpenDocument,
) -> Option<WorkspaceSymbolSummary> {
    let path = open_document.uri.to_file_path().ok();
    let settings = shuck_settings_for_document(context, path.as_deref());
    let source = open_document.document.contents();
    let shell = crate::lint::infer_document_shell_from_parts(
        &settings,
        open_document.document.language_id(),
        source,
        path.as_deref(),
    )?;
    let content_hash = content_hash(source);
    let symbols = editor_document_symbols(source, path.as_deref(), shell);
    let render = SymbolRenderContext {
        uri: &open_document.uri,
        source,
        line_index: open_document.document.index(),
        encoding: context.encoding,
    };
    Some(workspace_symbol_summary_from_editor_symbols(
        open_document.uri.clone(),
        Some(open_document.document.version()),
        content_hash,
        &symbols,
        &render,
    ))
}

fn workspace_symbol_summary_from_source(
    uri: types::Url,
    source: &str,
    path: &Path,
    shell: ShellDialect,
    encoding: PositionEncoding,
) -> WorkspaceSymbolSummary {
    let line_index = LineIndex::new(source);
    let symbols = editor_document_symbols(source, Some(path), shell);
    let render = SymbolRenderContext {
        uri: &uri,
        source,
        line_index: &line_index,
        encoding,
    };
    workspace_symbol_summary_from_editor_symbols(
        uri.clone(),
        None,
        content_hash(source),
        &symbols,
        &render,
    )
}

fn workspace_symbol_summary_from_editor_symbols(
    uri: types::Url,
    version: Option<DocumentVersion>,
    content_hash: [u8; 32],
    symbols: &[EditorDocumentSymbol],
    render: &SymbolRenderContext<'_>,
) -> WorkspaceSymbolSummary {
    let mut entries = Vec::new();
    flatten_workspace_symbols(symbols, render, None, &mut entries);
    WorkspaceSymbolSummary {
        uri,
        version,
        content_hash,
        symbols: entries,
    }
}

fn flatten_workspace_symbols(
    symbols: &[EditorDocumentSymbol],
    render: &SymbolRenderContext<'_>,
    container_name: Option<&str>,
    entries: &mut Vec<WorkspaceSymbolEntry>,
) {
    for symbol in symbols {
        let range = crate::edit::to_lsp_range(
            symbol.range.to_range(),
            render.source,
            render.line_index,
            render.encoding,
        );
        let selection_range = crate::edit::to_lsp_range(
            symbol.selection_span.to_range(),
            render.source,
            render.line_index,
            render.encoding,
        );
        entries.push(WorkspaceSymbolEntry {
            name: symbol.name.to_string(),
            kind: to_lsp_symbol_kind(symbol.kind),
            container_name: container_name.map(str::to_owned),
            range,
            selection_range,
        });
        flatten_workspace_symbols(
            &symbol.children,
            render,
            Some(symbol.name.as_str()),
            entries,
        );
    }
}

struct ClosedWorkspaceSymbolBuild {
    summaries: Vec<WorkspaceSymbolSummary>,
    partial: bool,
}

fn rebuild_closed_workspace_symbols(
    context: &WorkspaceSymbolContext,
) -> crate::server::Result<ClosedWorkspaceSymbolBuild> {
    if context.workspace_roots.is_empty() {
        return Ok(ClosedWorkspaceSymbolBuild {
            summaries: Vec::new(),
            partial: false,
        });
    }

    let mut files = BTreeMap::new();
    let mut partial = false;
    let open_paths = open_document_path_keys(context);
    for root in &context.workspace_roots {
        let options = workspace_symbol_options_for_path(context, Some(root));
        if !options.enabled {
            continue;
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
                tracing::warn!(
                    "Failed to discover workspace symbol files in {}: {error}",
                    root.display()
                );
                partial = true;
                continue;
            }
        };
        let mut root_files = discovered
            .into_iter()
            .filter(|file| file.kind == FileKind::Shell)
            .filter(|file| workspace_path_owned_by_root(context, &file.absolute_path, root))
            .filter(|file| !open_paths.contains(&file.absolute_path))
            .collect::<Vec<_>>();
        partial |= root_files.len() > options.max_files;
        root_files.truncate(options.max_files);
        for file in root_files {
            files.entry(file.absolute_path.clone()).or_insert(file);
        }
    }

    partial |= files.len() > context.options.max_files;
    let summaries = files
        .values()
        .take(context.options.max_files)
        .filter_map(|file| closed_workspace_symbol_summary(context, file))
        .collect();

    Ok(ClosedWorkspaceSymbolBuild { summaries, partial })
}

fn closed_workspace_symbol_summary(
    context: &WorkspaceSymbolContext,
    file: &DiscoveredFile,
) -> Option<WorkspaceSymbolSummary> {
    let source = match std::fs::read_to_string(&file.absolute_path) {
        Ok(source) => source,
        Err(error) => {
            tracing::warn!(
                "Failed to read workspace symbol file {}: {error}",
                file.absolute_path.display()
            );
            return None;
        }
    };
    let uri = match types::Url::from_file_path(&file.absolute_path) {
        Ok(uri) => uri,
        Err(()) => {
            tracing::warn!(
                "Failed to convert workspace symbol path to URI: {}",
                file.absolute_path.display()
            );
            return None;
        }
    };
    let settings = shuck_settings_for_document(context, Some(&file.absolute_path));
    let shell = crate::lint::infer_document_shell_from_parts(
        &settings,
        None,
        &source,
        Some(&file.absolute_path),
    )?;
    Some(workspace_symbol_summary_from_source(
        uri,
        &source,
        &file.absolute_path,
        shell,
        context.encoding,
    ))
}

fn shuck_settings_for_document(
    context: &WorkspaceSymbolContext,
    path: Option<&Path>,
) -> ShuckSettings {
    let workspace_options = path.and_then(|path| {
        context
            .workspace_settings
            .iter()
            .filter_map(|workspace| {
                workspace_root_match_len(path, workspace).map(|len| (workspace, len))
            })
            .max_by_key(|(_, len)| *len)
            .and_then(|(workspace, _)| workspace.options.as_ref())
    });
    if let Some(workspace_options) = workspace_options {
        return ShuckSettings::resolve(
            path,
            &context.settings_workspace_roots,
            &[&context.global_options, workspace_options],
        );
    }

    ShuckSettings::resolve(
        path,
        &context.settings_workspace_roots,
        &[&context.global_options],
    )
}

fn workspace_root_match_len(path: &Path, workspace: &WorkspaceSettingsSnapshot) -> Option<usize> {
    [Some(&workspace.root), workspace.canonical_root.as_ref()]
        .into_iter()
        .flatten()
        .filter(|root| path.starts_with(root))
        .map(|root| root.components().count())
        .max()
}

fn workspace_symbol_options_for_uri(
    context: &WorkspaceSymbolContext,
    uri: &types::Url,
) -> WorkspaceSymbolFeatureOptions {
    let path = uri.to_file_path().ok();
    workspace_symbol_options_for_path(context, path.as_deref())
}

fn workspace_symbol_options_for_path(
    context: &WorkspaceSymbolContext,
    path: Option<&Path>,
) -> WorkspaceSymbolFeatureOptions {
    path.and_then(|path| workspace_settings_for_path(context, path))
        .and_then(|workspace| workspace.options.as_ref())
        .map(|options| options.server.workspace_symbols)
        .unwrap_or(context.options)
}

fn workspace_path_owned_by_root(
    context: &WorkspaceSymbolContext,
    path: &Path,
    root: &Path,
) -> bool {
    workspace_settings_for_path(context, path)
        .map(|workspace| workspace.root == root)
        .unwrap_or(true)
}

fn workspace_settings_for_path<'a>(
    context: &'a WorkspaceSymbolContext,
    path: &Path,
) -> Option<&'a WorkspaceSettingsSnapshot> {
    context
        .workspace_settings
        .iter()
        .filter_map(|workspace| {
            workspace_root_match_len(path, workspace).map(|len| (workspace, len))
        })
        .max_by_key(|(_, len)| *len)
        .map(|(workspace, _)| workspace)
}

fn workspace_symbols_have_enabled_scope(context: &WorkspaceSymbolContext) -> bool {
    context
        .open_documents
        .iter()
        .any(|document| workspace_symbol_options_for_uri(context, &document.uri).enabled)
        || context
            .workspace_roots
            .iter()
            .any(|root| workspace_symbol_options_for_path(context, Some(root)).enabled)
}

fn workspace_symbol_candidates(
    summaries: &[WorkspaceSymbolSummary],
    query: &str,
) -> Vec<WorkspaceSymbolCandidate> {
    let query_folded = query.trim().to_lowercase();
    let query = query_folded.as_str();
    let mut candidates = summaries
        .iter()
        .flat_map(|summary| {
            summary.symbols.iter().filter_map(move |symbol| {
                let name_folded = symbol.name.to_lowercase();
                symbol_query_score(&name_folded, query).map(|score| {
                    ScoredWorkspaceSymbolCandidate {
                        score,
                        name_folded,
                        candidate: WorkspaceSymbolCandidate {
                            uri: summary.uri.clone(),
                            symbol: symbol.clone(),
                        },
                    }
                })
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.name_folded.cmp(&right.name_folded))
            .then_with(|| left.candidate.symbol.name.cmp(&right.candidate.symbol.name))
            .then_with(|| {
                left.candidate
                    .uri
                    .as_str()
                    .cmp(right.candidate.uri.as_str())
            })
            .then_with(|| {
                range_sort_key(left.candidate.symbol.selection_range)
                    .cmp(&range_sort_key(right.candidate.symbol.selection_range))
            })
    });

    candidates
        .into_iter()
        .map(|candidate| candidate.candidate)
        .collect()
}

fn workspace_uri_keys(uri: &types::Url) -> Vec<String> {
    let mut keys = vec![uri.as_str().to_owned()];
    if let Ok(path) = uri.to_file_path()
        && let Ok(canonical) = std::fs::canonicalize(path)
        && let Ok(uri) = types::Url::from_file_path(canonical)
    {
        keys.push(uri.as_str().to_owned());
    }
    keys
}

fn open_document_path_keys(
    context: &WorkspaceSymbolContext,
) -> std::collections::BTreeSet<PathBuf> {
    let mut paths = std::collections::BTreeSet::new();
    for document in &context.open_documents {
        let Ok(path) = document.uri.to_file_path() else {
            continue;
        };
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            paths.insert(canonical);
        }
        paths.insert(path);
    }
    paths
}

fn symbol_query_score(name: &str, query: &str) -> Option<SymbolQueryScore> {
    if query.is_empty() {
        return Some(SymbolQueryScore {
            rank: 0,
            penalty: 0,
        });
    }

    if name == query {
        return Some(SymbolQueryScore {
            rank: 0,
            penalty: 0,
        });
    }
    if name.starts_with(query) {
        return Some(SymbolQueryScore {
            rank: 1,
            penalty: name.len().saturating_sub(query.len()),
        });
    }
    if let Some(index) = name.find(query) {
        return Some(SymbolQueryScore {
            rank: 2,
            penalty: index,
        });
    }
    subsequence_penalty(name, query).map(|penalty| SymbolQueryScore { rank: 3, penalty })
}

fn subsequence_penalty(name: &str, query: &str) -> Option<usize> {
    let mut chars = name.char_indices();
    let mut first = None;
    let mut last = 0;
    for needle in query.chars() {
        let (index, _) = chars.find(|(_, candidate)| *candidate == needle)?;
        first.get_or_insert(index);
        last = index;
    }
    Some(
        last.saturating_sub(first.unwrap_or(0))
            .saturating_sub(query.len()),
    )
}

fn to_lsp_workspace_symbol(candidate: WorkspaceSymbolCandidate) -> types::WorkspaceSymbol {
    types::WorkspaceSymbol {
        name: candidate.symbol.name,
        kind: candidate.symbol.kind,
        tags: None,
        container_name: candidate.symbol.container_name,
        location: lsp_types::OneOf::Left(types::Location::new(
            candidate.uri,
            candidate.symbol.selection_range,
        )),
        data: None,
    }
}

fn content_hash(source: &str) -> [u8; 32] {
    Sha256::digest(source.as_bytes()).into()
}

fn range_sort_key(range: types::Range) -> (u32, u32, u32, u32) {
    (
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
    )
}

fn clone_cached_summaries(cache: &WorkspaceSymbolCache) -> (Vec<WorkspaceSymbolSummary>, bool) {
    (cache.summaries.values().cloned().collect(), cache.partial)
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait_or_recover<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match condvar.wait(guard) {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, DocumentSymbolClientCapabilities, DocumentSymbolParams,
        DocumentSymbolResponse, FileChangeType, FileEvent, PartialResultParams,
        PositionEncodingKind, TextDocumentClientCapabilities, TextDocumentIdentifier, Url,
        WorkDoneProgressParams, WorkspaceSymbolParams,
    };

    use super::*;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

    fn position_encoding_kind(encoding: PositionEncoding) -> PositionEncodingKind {
        match encoding {
            PositionEncoding::UTF8 => PositionEncodingKind::UTF8,
            PositionEncoding::UTF16 => PositionEncodingKind::UTF16,
            PositionEncoding::UTF32 => PositionEncodingKind::UTF32,
        }
    }

    fn make_snapshot(
        source: &str,
        encoding: PositionEncoding,
        hierarchical_document_symbols: bool,
    ) -> (DocumentSnapshot, Client, Url) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_root = std::env::temp_dir().join("shuck-server-symbol-tests");
        let workspace_uri =
            Url::from_file_path(&workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![position_encoding_kind(encoding)]),
                    ..Default::default()
                }),
                text_document: Some(TextDocumentClientCapabilities {
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(hierarchical_document_symbols),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            encoding,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_root.join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new(source.to_owned(), 1).with_language_id("shellscript"),
        );

        (
            session
                .take_snapshot(uri.clone())
                .expect("test document should produce a snapshot"),
            client,
            uri,
        )
    }

    fn make_session(
        workspace_root: &std::path::Path,
        encoding: PositionEncoding,
    ) -> (Session, Client) {
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(workspace_root).expect("workspace path should convert to a URL");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let session = Session::new(
            &ClientCapabilities {
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![position_encoding_kind(encoding)]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            encoding,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        (session, client)
    }

    fn workspace_symbol_params(query: &str) -> WorkspaceSymbolParams {
        WorkspaceSymbolParams {
            query: query.to_owned(),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        }
    }

    fn workspace_symbol_response_names(response: types::WorkspaceSymbolResponse) -> Vec<String> {
        let types::WorkspaceSymbolResponse::Nested(symbols) = response else {
            panic!("expected nested workspace symbols");
        };
        symbols.into_iter().map(|symbol| symbol.name).collect()
    }

    fn workspace_symbol_response(
        response: types::WorkspaceSymbolResponse,
    ) -> Vec<types::WorkspaceSymbol> {
        let types::WorkspaceSymbolResponse::Nested(symbols) = response else {
            panic!("expected nested workspace symbols");
        };
        symbols
    }

    fn summary_with_names(uri: &str, names: &[&str]) -> WorkspaceSymbolSummary {
        WorkspaceSymbolSummary {
            uri: Url::parse(uri).expect("test URI should parse"),
            version: None,
            content_hash: [0; 32],
            symbols: names
                .iter()
                .map(|name| WorkspaceSymbolEntry {
                    name: (*name).to_owned(),
                    kind: types::SymbolKind::FUNCTION,
                    container_name: None,
                    range: types::Range::new(
                        types::Position::new(0, 0),
                        types::Position::new(0, 0),
                    ),
                    selection_range: types::Range::new(
                        types::Position::new(0, 0),
                        types::Position::new(0, 0),
                    ),
                })
                .collect(),
        }
    }

    #[test]
    fn document_symbols_return_nested_lsp_symbols() {
        let source = "\
#!/bin/bash
VERSION=1
build() {
  local artifact
}
";
        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, true);
        let response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");

        let DocumentSymbolResponse::Nested(symbols) = response else {
            panic!("expected nested document symbols");
        };
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "VERSION");
        assert_eq!(symbols[0].kind, types::SymbolKind::VARIABLE);
        assert_eq!(symbols[0].selection_range.start.line, 1);
        assert_eq!(symbols[0].selection_range.start.character, 0);
        assert_eq!(symbols[0].selection_range.end.character, 7);

        assert_eq!(symbols[1].name, "build");
        assert_eq!(symbols[1].kind, types::SymbolKind::FUNCTION);
        assert_eq!(symbols[1].selection_range.start.line, 2);
        assert_eq!(symbols[1].selection_range.start.character, 0);
        assert_eq!(symbols[1].selection_range.end.character, 5);

        let children = symbols[1]
            .children
            .as_ref()
            .expect("function symbol should have children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "artifact");
        assert_eq!(children[0].kind, types::SymbolKind::VARIABLE);
        assert_eq!(children[0].selection_range.start.line, 3);
        assert_eq!(children[0].selection_range.start.character, 8);
    }

    #[test]
    fn document_symbols_fall_back_to_flat_response_without_hierarchical_client_support() {
        let source = "\
#!/bin/bash
VERSION=1
build() {
  local artifact
}
";
        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, false);
        let response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");

        let DocumentSymbolResponse::Flat(symbols) = response else {
            panic!("expected flat document symbols");
        };
        assert_eq!(
            symbols
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            ["VERSION", "build", "artifact"]
        );
        assert_eq!(symbols[0].container_name, None);
        assert_eq!(symbols[2].container_name.as_deref(), Some("build"));
    }

    #[test]
    fn document_symbol_ranges_use_negotiated_position_encoding() {
        let source = "build() { echo \"é\"; local cafe; }\n";

        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF16, true);
        let utf16_response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");
        let DocumentSymbolResponse::Nested(utf16_symbols) = utf16_response else {
            panic!("expected nested document symbols");
        };
        let utf16_child = &utf16_symbols[0]
            .children
            .as_ref()
            .expect("function should have children")[0];
        assert_eq!(utf16_child.name, "cafe");
        assert_eq!(utf16_child.selection_range.start.character, 26);
        assert_eq!(utf16_child.selection_range.end.character, 30);

        let (snapshot, client, uri) = make_snapshot(source, PositionEncoding::UTF8, true);
        let utf8_response = document_symbols(
            snapshot,
            &client,
            DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            },
        )
        .expect("document symbol request should succeed")
        .expect("document symbol response should be present");
        let DocumentSymbolResponse::Nested(utf8_symbols) = utf8_response else {
            panic!("expected nested document symbols");
        };
        let utf8_child = &utf8_symbols[0]
            .children
            .as_ref()
            .expect("function should have children")[0];
        assert_eq!(utf8_child.name, "cafe");
        assert_eq!(utf8_child.selection_range.start.character, 27);
        assert_eq!(utf8_child.selection_range.end.character, 31);
    }

    #[test]
    fn workspace_symbol_query_ranks_exact_prefix_contains_and_subsequence_matches() {
        let summaries = vec![
            summary_with_names(
                "file:///tmp/one.sh",
                &["prebuild", "build", "builder", "bxxuild"],
            ),
            summary_with_names("file:///tmp/two.sh", &["other"]),
        ];

        let candidates = workspace_symbol_candidates(&summaries, "build");
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.symbol.name.as_str())
                .collect::<Vec<_>>(),
            ["build", "builder", "prebuild", "bxxuild"]
        );
    }

    #[test]
    fn workspace_symbol_queries_are_case_insensitive_and_empty_queries_are_deterministic() {
        let summaries = vec![
            summary_with_names("file:///tmp/z.sh", &["shared", "gamma"]),
            summary_with_names("file:///tmp/a.sh", &["shared", "Alpha"]),
        ];

        let exact = workspace_symbol_candidates(&summaries, "alpha");
        assert_eq!(
            exact
                .iter()
                .map(|candidate| candidate.symbol.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha"]
        );

        let empty = workspace_symbol_candidates(&summaries, "");
        assert_eq!(
            empty
                .iter()
                .map(|candidate| (candidate.symbol.name.as_str(), candidate.uri.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Alpha", "file:///tmp/a.sh"),
                ("gamma", "file:///tmp/z.sh"),
                ("shared", "file:///tmp/a.sh"),
                ("shared", "file:///tmp/z.sh"),
            ]
        );
    }

    #[test]
    fn workspace_symbol_ranges_use_negotiated_position_encoding() {
        let source = "build() { echo \"é\"; local cafe; }\n";
        let path = std::path::Path::new("/tmp/script.sh");
        let uri = Url::parse("file:///tmp/script.sh").expect("test URI should parse");

        let utf16 = workspace_symbol_summary_from_source(
            uri.clone(),
            source,
            path,
            ShellDialect::Bash,
            PositionEncoding::UTF16,
        );
        let utf16_child = utf16
            .symbols
            .iter()
            .find(|symbol| symbol.name == "cafe")
            .expect("expected local symbol");
        assert_eq!(utf16_child.selection_range.start.character, 26);
        assert_eq!(utf16_child.selection_range.end.character, 30);

        let utf8 = workspace_symbol_summary_from_source(
            uri,
            source,
            path,
            ShellDialect::Bash,
            PositionEncoding::UTF8,
        );
        let utf8_child = utf8
            .symbols
            .iter()
            .find(|symbol| symbol.name == "cafe")
            .expect("expected local symbol");
        assert_eq!(utf8_child.selection_range.start.character, 27);
        assert_eq!(utf8_child.selection_range.end.character, 31);
    }

    #[test]
    fn workspace_symbols_can_be_disabled_without_rebuilding_closed_index() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(tempdir.path().join("script.sh"), "hidden_symbol() { :; }\n")
            .expect("fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.enabled = false;
        session.update_client_options(options);
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert!(workspace_symbol_response_names(response).is_empty());

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.dirty);
        assert!(cache.summaries.is_empty());
    }

    #[test]
    fn workspace_symbols_honor_workspace_level_disable() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(tempdir.path().join("script.sh"), "hidden_symbol() { :; }\n")
            .expect("fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let workspace_uri =
            Url::from_file_path(tempdir.path()).expect("workspace URI should convert");
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.enabled = false;
        let mut workspace_options = crate::session::WorkspaceOptionsMap::default();
        workspace_options.insert(workspace_uri, options);
        session.update_configuration(crate::ClientOptions::default(), Some(workspace_options));
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert!(workspace_symbol_response_names(response).is_empty());

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.dirty);
        assert!(cache.summaries.is_empty());
    }

    #[test]
    fn workspace_symbols_include_language_id_only_open_buffers() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_uri = Url::from_file_path(tempdir.path().join("scratch.txt"))
            .expect("scratch URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("language_only_symbol() { :; }\n".to_owned(), 3)
                .with_language_id("shellscript"),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("language_only"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert_eq!(
            workspace_symbol_response_names(response),
            ["language_only_symbol"]
        );
    }

    #[test]
    fn workspace_symbols_return_concrete_locations_and_container_names() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_path = tempdir.path().join("script.sh");
        let open_uri = Url::from_file_path(&open_path).expect("open URI should convert");
        session.open_text_document(
            open_uri.clone(),
            TextDocument::new("build() {\n  local artifact\n}\n".to_owned(), 4)
                .with_language_id("shellscript"),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("artifact"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        let symbols = workspace_symbol_response(response);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "artifact");
        assert_eq!(symbols[0].container_name.as_deref(), Some("build"));

        let lsp_types::OneOf::Left(location) = &symbols[0].location else {
            panic!("workspace symbols should use concrete locations");
        };
        assert_eq!(location.uri, open_uri);
        assert_eq!(location.range.start.line, 1);
        assert_eq!(location.range.start.character, 8);
        assert_eq!(location.range.end.character, 16);
    }

    #[test]
    fn workspace_symbols_prefer_unsaved_open_buffers_over_disk_summaries() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let open_path = tempdir.path().join("open.sh");
        let closed_path = tempdir.path().join("closed.sh");
        std::fs::write(&open_path, "disk_workspace_symbol() { :; }\n")
            .expect("open disk fixture should be written");
        std::fs::write(&closed_path, "closed_workspace_symbol() { :; }\n")
            .expect("closed fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_uri = Url::from_file_path(&open_path).expect("open URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("buffer_workspace_symbol() { :; }\n".to_owned(), 7)
                .with_language_id("shellscript"),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("workspace_symbol"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        let names = workspace_symbol_response_names(response);
        assert!(names.contains(&"buffer_workspace_symbol".to_owned()));
        assert!(names.contains(&"closed_workspace_symbol".to_owned()));
        assert!(!names.contains(&"disk_workspace_symbol".to_owned()));
    }

    #[test]
    fn workspace_symbols_shadow_closed_summaries_for_unclassified_open_buffers() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let open_path = tempdir.path().join("script");
        std::fs::write(&open_path, "#!/bin/sh\ndisk_shadow_symbol() { :; }\n")
            .expect("open disk fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_uri = Url::from_file_path(&open_path).expect("open URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("echo no shell hint\n".to_owned(), 8),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("disk_shadow"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert!(workspace_symbol_response_names(response).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_symbols_shadow_closed_summaries_when_open_uri_is_symlink() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let real_path = tempdir.path().join("real.sh");
        let symlink_path = tempdir.path().join("link.sh");
        std::fs::write(&real_path, "disk_symlink_symbol() { :; }\n")
            .expect("real fixture should be written");
        std::os::unix::fs::symlink(&real_path, &symlink_path)
            .expect("symlink fixture should be created");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_uri = Url::from_file_path(&symlink_path).expect("symlink URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("buffer_symlink_symbol() { :; }\n".to_owned(), 5)
                .with_language_id("shellscript"),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("symlink_symbol"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        let names = workspace_symbol_response_names(response);
        assert_eq!(names, ["buffer_symlink_symbol"]);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_symbols_apply_workspace_options_for_symlinked_workspace_root() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let real_root = tempdir.path().join("real");
        let symlink_root = tempdir.path().join("link");
        std::fs::create_dir(&real_root).expect("real workspace should be created");
        std::os::unix::fs::symlink(&real_root, &symlink_root)
            .expect("workspace symlink should be created");
        std::fs::write(
            real_root.join("script.sh"),
            "function zsh_target zsh_alias() { :; }\n",
        )
        .expect("fixture should be written");

        let (mut session, client) = make_session(&symlink_root, PositionEncoding::UTF16);
        let workspace_uri =
            Url::from_file_path(&symlink_root).expect("workspace URI should convert");
        let mut workspace_options = crate::session::WorkspaceOptionsMap::default();
        workspace_options.insert(
            workspace_uri,
            crate::ClientOptions {
                lint: Some(shuck_config::LintConfig {
                    per_file_shell: Some(std::collections::BTreeMap::from([(
                        "*.sh".to_owned(),
                        "zsh".to_owned(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        session.update_configuration(crate::ClientOptions::default(), Some(workspace_options));

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("zsh_alias"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert_eq!(workspace_symbol_response_names(response), ["zsh_alias"]);
    }

    #[test]
    fn workspace_symbols_use_source_declared_shell_for_closed_files() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(
            tempdir.path().join("script.sh"),
            "# shellcheck shell=zsh\nfunction declared_header declared_alias() { :; }\n",
        )
        .expect("fixture should be written");

        let (session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("declared_alias"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert_eq!(
            workspace_symbol_response_names(response),
            ["declared_alias"]
        );
    }

    #[test]
    fn workspace_symbols_respect_max_files_and_mark_partial_indexes() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(tempdir.path().join("a.sh"), "alpha_symbol() { :; }\n")
            .expect("alpha fixture should be written");
        std::fs::write(tempdir.path().join("b.sh"), "beta_symbol() { :; }\n")
            .expect("beta fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.max_files = 1;
        session.update_client_options(options);
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        let names = workspace_symbol_response_names(response);
        assert_eq!(names, ["alpha_symbol"]);

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.partial);
        assert!(!cache.dirty);
    }

    #[test]
    fn workspace_symbols_honor_workspace_level_max_files() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(tempdir.path().join("a.sh"), "alpha_symbol() { :; }\n")
            .expect("alpha fixture should be written");
        std::fs::write(tempdir.path().join("b.sh"), "beta_symbol() { :; }\n")
            .expect("beta fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let workspace_uri =
            Url::from_file_path(tempdir.path()).expect("workspace URI should convert");
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.max_files = 1;
        let mut workspace_options = crate::session::WorkspaceOptionsMap::default();
        workspace_options.insert(workspace_uri, options);
        session.update_configuration(crate::ClientOptions::default(), Some(workspace_options));
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert_eq!(workspace_symbol_response_names(response), ["alpha_symbol"]);

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.partial);
        assert!(!cache.dirty);
    }

    #[test]
    fn workspace_symbols_enforce_max_files_across_workspace_roots() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let first_root = tempdir.path().join("a-root");
        let second_root = tempdir.path().join("b-root");
        std::fs::create_dir(&first_root).expect("first root should be created");
        std::fs::create_dir(&second_root).expect("second root should be created");
        std::fs::write(first_root.join("a.sh"), "alpha_symbol() { :; }\n")
            .expect("alpha fixture should be written");
        std::fs::write(second_root.join("b.sh"), "beta_symbol() { :; }\n")
            .expect("beta fixture should be written");

        let (mut session, client) = make_session(&first_root, PositionEncoding::UTF16);
        let second_uri =
            Url::from_file_path(&second_root).expect("second workspace URI should convert");
        session
            .open_workspace_folder(second_uri, &client)
            .expect("second workspace should open");
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.max_files = 1;
        session.update_client_options(options);
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert_eq!(workspace_symbol_response_names(response), ["alpha_symbol"]);

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.partial);
        assert!(!cache.dirty);
    }

    #[test]
    fn workspace_symbols_do_not_index_nested_workspace_files_from_parent_root() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let parent_root = tempdir.path().join("parent");
        let nested_root = parent_root.join("nested");
        std::fs::create_dir_all(&nested_root).expect("nested root should be created");
        std::fs::write(parent_root.join("a.sh"), "parent_symbol() { :; }\n")
            .expect("parent fixture should be written");
        std::fs::write(nested_root.join("b.sh"), "nested_symbol() { :; }\n")
            .expect("nested fixture should be written");

        let (mut session, client) = make_session(&parent_root, PositionEncoding::UTF16);
        let nested_uri =
            Url::from_file_path(&nested_root).expect("nested workspace URI should convert");
        session
            .open_workspace_folder(nested_uri.clone(), &client)
            .expect("nested workspace should open");
        let mut nested_options = crate::ClientOptions::default();
        nested_options.server.workspace_symbols.enabled = false;
        let mut workspace_options = crate::session::WorkspaceOptionsMap::default();
        workspace_options.insert(nested_uri, nested_options);
        session.update_configuration(crate::ClientOptions::default(), Some(workspace_options));

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("symbol"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert_eq!(workspace_symbol_response_names(response), ["parent_symbol"]);
    }

    #[test]
    fn workspace_symbols_tolerate_discovery_failure_for_one_root() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let healthy_root = tempdir.path().join("healthy");
        let missing_root = tempdir.path().join("missing");
        std::fs::create_dir(&healthy_root).expect("healthy root should be created");
        std::fs::create_dir(&missing_root).expect("missing root should initially exist");
        std::fs::write(healthy_root.join("a.sh"), "healthy_symbol() { :; }\n")
            .expect("healthy fixture should be written");

        let (mut session, client) = make_session(&healthy_root, PositionEncoding::UTF16);
        let missing_uri =
            Url::from_file_path(&missing_root).expect("missing workspace URI should convert");
        session
            .open_workspace_folder(missing_uri, &client)
            .expect("missing workspace should open");
        let open_uri = Url::from_file_path(missing_root.join("scratch.sh"))
            .expect("open buffer URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("open_buffer_symbol() { :; }\n".to_owned(), 3)
                .with_language_id("shellscript"),
        );
        std::fs::remove_dir(&missing_root).expect("missing root should be removed");
        let context = session.workspace_symbol_context();

        let response = workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should tolerate one bad root")
            .expect("workspace symbol response should be present");
        assert_eq!(
            workspace_symbol_response_names(response),
            ["healthy_symbol", "open_buffer_symbol"]
        );

        let cache = lock_or_recover(&context.index.cache);
        assert!(cache.partial);
        assert!(!cache.dirty);
    }

    #[test]
    fn workspace_symbols_apply_max_files_after_excluding_open_documents() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let open_path = tempdir.path().join("a.sh");
        let closed_path = tempdir.path().join("b.sh");
        std::fs::write(&open_path, "disk_alpha_symbol() { :; }\n")
            .expect("alpha fixture should be written");
        std::fs::write(&closed_path, "beta_symbol() { :; }\n")
            .expect("beta fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let open_uri = Url::from_file_path(&open_path).expect("open URI should convert");
        session.open_text_document(
            open_uri,
            TextDocument::new("buffer_alpha_symbol() { :; }\n".to_owned(), 9)
                .with_language_id("shellscript"),
        );
        let mut options = crate::ClientOptions::default();
        options.server.workspace_symbols.max_files = 1;
        session.update_client_options(options);

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("beta_symbol"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");

        assert_eq!(workspace_symbol_response_names(response), ["beta_symbol"]);
    }

    #[test]
    fn workspace_symbols_rebuild_disk_summary_after_open_document_closes() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let script = tempdir.path().join("script.sh");
        std::fs::write(&script, "old_closed_symbol() { :; }\n")
            .expect("script fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("old_closed"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        assert_eq!(
            workspace_symbol_response_names(response),
            ["old_closed_symbol"]
        );

        std::fs::write(&script, "new_closed_symbol() { :; }\n")
            .expect("script fixture should be updated");
        let uri = Url::from_file_path(&script).expect("script URI should convert");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("new_closed_symbol() { :; }\n".to_owned(), 2)
                .with_language_id("shellscript"),
        );

        let response = workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params("new_closed"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        assert_eq!(
            workspace_symbol_response_names(response),
            ["new_closed_symbol"]
        );

        let key = session.key_from_url(uri);
        session
            .close_document(&key)
            .expect("document close should succeed");
        let context = session.workspace_symbol_context();
        assert!(lock_or_recover(&context.index.cache).dirty);

        let stale = workspace_symbols(
            context.clone(),
            &client,
            workspace_symbol_params("old_closed"),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        assert!(workspace_symbol_response_names(stale).is_empty());

        let fresh = workspace_symbols(context, &client, workspace_symbol_params("new_closed"))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert_eq!(
            workspace_symbol_response_names(fresh),
            ["new_closed_symbol"]
        );
    }

    #[test]
    fn workspace_symbol_index_invalidates_for_file_config_and_workspace_changes() {
        let tempdir = tempfile::tempdir().expect("workspace should be created");
        let script = tempdir.path().join("script.sh");
        std::fs::write(&script, "cached_symbol() { :; }\n")
            .expect("script fixture should be written");

        let (mut session, client) = make_session(tempdir.path(), PositionEncoding::UTF16);
        let context = session.workspace_symbol_context();
        workspace_symbols(context.clone(), &client, workspace_symbol_params(""))
            .expect("workspace symbol request should succeed")
            .expect("workspace symbol response should be present");
        assert!(!lock_or_recover(&context.index.cache).dirty);

        session.reload_settings(
            &[FileEvent {
                uri: Url::from_file_path(&script).expect("script URI should convert"),
                typ: FileChangeType::CHANGED,
            }],
            &client,
        );
        assert!(lock_or_recover(&context.index.cache).dirty);

        workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params(""),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        assert!(!lock_or_recover(&context.index.cache).dirty);

        session.update_configuration(crate::ClientOptions::default(), None);
        assert!(lock_or_recover(&context.index.cache).dirty);

        workspace_symbols(
            session.workspace_symbol_context(),
            &client,
            workspace_symbol_params(""),
        )
        .expect("workspace symbol request should succeed")
        .expect("workspace symbol response should be present");
        assert!(!lock_or_recover(&context.index.cache).dirty);

        let workspace_uri =
            Url::from_file_path(tempdir.path()).expect("workspace URI should convert");
        session
            .close_workspace_folder(&workspace_uri)
            .expect("workspace close should succeed");
        assert!(lock_or_recover(&context.index.cache).dirty);
    }

    #[test]
    fn workspace_symbol_index_waits_for_an_active_rebuild_commit() {
        let index = Arc::new(WorkspaceSymbolIndex::default());
        let summary = summary_with_names("file:///tmp/cached.sh", &["cached_symbol"]);
        {
            let mut cache = lock_or_recover(&index.cache);
            cache
                .summaries
                .insert(summary.uri.as_str().to_owned(), summary);
            cache.dirty = true;
            cache.rebuilding = true;
        }

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let context = WorkspaceSymbolContext {
            index: index.clone(),
            options: WorkspaceSymbolFeatureOptions::default(),
            global_options: crate::ClientOptions::default(),
            workspace_settings: Vec::new(),
            workspace_roots: Vec::new(),
            settings_workspace_roots: Vec::new(),
            open_documents: Vec::new(),
            encoding: PositionEncoding::UTF16,
        };

        let index_for_thread = index.clone();
        let (started_sender, started_receiver) = channel::bounded(1);
        let handle = std::thread::spawn(move || {
            started_sender
                .send(())
                .expect("test thread should signal start");
            index_for_thread
                .closed_summaries(&context, &client)
                .expect("closed summary wait should succeed")
        });

        started_receiver.recv().expect("test thread should start");
        {
            let mut cache = lock_or_recover(&index.cache);
            cache.dirty = false;
            cache.rebuilding = false;
        }
        index.rebuild_finished.notify_all();

        let (summaries, partial) = handle.join().expect("test thread should finish");
        assert!(!partial);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].symbols[0].name, "cached_symbol");
    }
}
