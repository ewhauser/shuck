use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use lsp_types as types;
use sha2::{Digest, Sha256};
use shuck_discover::{DiscoveryOptions, FileKind, discover_files_with_status};

use crate::TextDocument;
use crate::lint::generate_diagnostics;
use crate::server::Result;
use crate::session::{
    Client, ClientOptions, ClientSettings, DocumentSnapshot, RequestCancellationToken,
    ShuckSettings, WorkspaceDiagnosticsFeatureOptions, WorkspaceDocumentSnapshotFactory,
    WorkspaceSettingsSnapshot,
};

const PARTIAL_RESULT_BATCH_SIZE: usize = 25;

#[derive(Clone)]
pub(crate) struct WorkspaceDiagnosticContext {
    pub(crate) options: WorkspaceDiagnosticsFeatureOptions,
    pub(crate) global_options: ClientOptions,
    pub(crate) workspace_settings: Vec<WorkspaceSettingsSnapshot>,
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) settings_workspace_roots: Vec<PathBuf>,
    pub(crate) open_documents: Vec<WorkspaceDiagnosticOpenDocument>,
    pub(crate) snapshot_factory: WorkspaceDocumentSnapshotFactory,
    pub(crate) cache: Arc<WorkspaceDiagnosticCache>,
    pub(crate) cache_generation: u64,
    pub(crate) cancellation: RequestCancellationToken,
}

#[derive(Clone)]
pub(crate) struct WorkspaceDiagnosticOpenDocument {
    pub(crate) uri: types::Url,
    pub(crate) path: PathBuf,
    pub(crate) snapshot: DocumentSnapshot,
}

#[derive(Default)]
pub(crate) struct WorkspaceDiagnosticCache {
    entries: Mutex<BTreeMap<String, CachedWorkspaceDiagnostic>>,
    generation: AtomicU64,
}

#[derive(Clone)]
struct CachedWorkspaceDiagnostic {
    source_hash: [u8; 32],
    result_id: String,
    diagnostics: Vec<types::Diagnostic>,
}

struct WorkDoneGuard {
    client: Client,
    token: Option<types::ProgressToken>,
}

struct ReportSink<'a> {
    client: &'a Client,
    partial_token: Option<&'a types::ProgressToken>,
    buffered: Vec<types::WorkspaceDocumentDiagnosticReport>,
    completed: usize,
}

impl WorkspaceDiagnosticCache {
    pub(crate) fn invalidate_all(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        lock_or_recover(&self.entries).clear();
    }

    pub(crate) fn invalidate_uri(&self, uri: &types::Url) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        lock_or_recover(&self.entries).remove(uri.as_str());
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn get(
        &self,
        uri: &types::Url,
        source_hash: [u8; 32],
        generation: u64,
    ) -> Option<CachedWorkspaceDiagnostic> {
        if self.generation() != generation {
            return None;
        }
        let entries = lock_or_recover(&self.entries);
        if self.generation() != generation {
            return None;
        }
        entries
            .get(uri.as_str())
            .filter(|entry| entry.source_hash == source_hash)
            .cloned()
    }

    fn insert(&self, uri: &types::Url, entry: CachedWorkspaceDiagnostic, generation: u64) {
        if self.generation() != generation {
            return;
        }
        let mut entries = lock_or_recover(&self.entries);
        if self.generation() == generation {
            entries.insert(uri.to_string(), entry);
        }
    }

    fn retain(&self, uris: &BTreeSet<String>, generation: u64) {
        if self.generation() != generation {
            return;
        }
        let mut entries = lock_or_recover(&self.entries);
        if self.generation() == generation {
            entries.retain(|uri, _| uris.contains(uri));
        }
    }
}

impl WorkDoneGuard {
    fn new(client: &Client, token: Option<types::ProgressToken>) -> Self {
        if let Some(token) = token.as_ref() {
            let _ = client.send_notification_value(
                "$/progress",
                serde_json::json!({
                    "token": token,
                    "value": {
                        "kind": "begin",
                        "title": "Shuck workspace diagnostics",
                        "cancellable": true,
                    },
                }),
            );
        }
        Self {
            client: client.clone(),
            token,
        }
    }

    fn report(&self, completed: usize) {
        let Some(token) = self.token.as_ref() else {
            return;
        };
        let _ = self.client.send_notification_value(
            "$/progress",
            serde_json::json!({
                "token": token,
                "value": {
                    "kind": "report",
                    "cancellable": true,
                    "message": format!("Analyzed {completed} shell files"),
                },
            }),
        );
    }
}

impl Drop for WorkDoneGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.as_ref() else {
            return;
        };
        let _ = self.client.send_notification_value(
            "$/progress",
            serde_json::json!({
                "token": token,
                "value": { "kind": "end" },
            }),
        );
    }
}

impl<'a> ReportSink<'a> {
    fn new(client: &'a Client, partial_token: Option<&'a types::ProgressToken>) -> Self {
        Self {
            client,
            partial_token,
            buffered: Vec::new(),
            completed: 0,
        }
    }

    fn completed(&self) -> usize {
        self.completed
    }

    fn push(&mut self, report: types::WorkspaceDocumentDiagnosticReport) -> Result<()> {
        self.buffered.push(report);
        self.completed += 1;
        if self.partial_token.is_some() && self.buffered.len() >= PARTIAL_RESULT_BATCH_SIZE {
            self.flush_partial()?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<types::WorkspaceDocumentDiagnosticReport>> {
        if self.partial_token.is_some() {
            self.flush_partial()?;
            return Ok(Vec::new());
        }
        Ok(self.buffered)
    }

    fn flush_partial(&mut self) -> Result<()> {
        let Some(token) = self.partial_token else {
            return Ok(());
        };
        if self.buffered.is_empty() {
            return Ok(());
        }
        self.client.send_notification_value(
            "$/progress",
            serde_json::json!({
                "token": token,
                "value": { "items": self.buffered },
            }),
        )?;
        self.buffered.clear();
        Ok(())
    }
}

pub(crate) fn workspace_diagnostics(
    context: WorkspaceDiagnosticContext,
    client: &Client,
    params: &types::WorkspaceDiagnosticParams,
) -> Result<types::WorkspaceDiagnosticReportResult> {
    let progress = WorkDoneGuard::new(
        client,
        params.work_done_progress_params.work_done_token.clone(),
    );
    if !workspace_diagnostics_have_enabled_scope(&context) || context.cancellation.is_cancelled() {
        return Ok(empty_report());
    }

    let previous = params
        .previous_result_ids
        .iter()
        .map(|previous| (previous.uri.to_string(), previous.value.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut reports = ReportSink::new(
        client,
        params.partial_result_params.partial_result_token.as_ref(),
    );
    let mut seen = BTreeSet::new();
    let mut open_paths = BTreeSet::new();
    let mut source_bytes = 0usize;
    let mut coverage_complete = true;

    let mut open_documents = context.open_documents.clone();
    open_documents.sort_by(|left, right| left.uri.as_str().cmp(right.uri.as_str()));
    for document in open_documents {
        if reports.completed() >= context.options.max_files {
            coverage_complete = false;
            break;
        }
        if context.cancellation.is_cancelled() {
            coverage_complete = false;
            break;
        }
        let options = workspace_diagnostic_options_for_path(&context, &document.path);
        if !options.enabled || !path_in_workspace(&context, &document.path) {
            continue;
        }
        let document_bytes = document.snapshot.query().document().contents().len();
        if source_bytes.saturating_add(document_bytes) > context.options.max_source_bytes
            || source_bytes.saturating_add(document_bytes) > options.max_source_bytes
        {
            coverage_complete = false;
            continue;
        }
        source_bytes += document_bytes;
        open_paths.insert(canonical_or_original(&document.path));
        let diagnostics = generate_diagnostics(&document.snapshot);
        let result_id = diagnostic_result_id(&diagnostics);
        seen.insert(document.uri.to_string());
        reports.push(document_report(
            document.uri,
            Some(i64::from(document.snapshot.query().document().version())),
            diagnostics,
            result_id,
            &previous,
        ))?;
        progress.report(reports.completed());
    }

    let remaining = context
        .options
        .max_files
        .saturating_sub(reports.completed());
    let mut discovered = BTreeMap::new();
    let mut discovery_entries = 0usize;
    if remaining > 0 && !context.cancellation.is_cancelled() {
        for root in &context.workspace_roots {
            let options = workspace_diagnostic_options_for_path(&context, root);
            if !options.enabled {
                continue;
            }
            if discovered.len() >= remaining {
                coverage_complete = false;
                break;
            }
            let root_remaining = remaining
                .saturating_sub(discovered.len())
                .min(options.max_files);
            let entry_remaining = context
                .options
                .max_entries
                .saturating_sub(discovery_entries)
                .min(options.max_entries);
            if root_remaining == 0 || entry_remaining == 0 {
                coverage_complete = false;
                break;
            }
            let result = match discover_files_with_status(
                std::slice::from_ref(root),
                root,
                &DiscoveryOptions {
                    respect_gitignore: true,
                    parallel: false,
                    use_config_roots: true,
                    max_files: Some(root_remaining),
                    max_entries: Some(entry_remaining),
                    cancellation: Some(context.cancellation.discovery_token()),
                    include_embedded: false,
                    excluded_subtrees: context
                        .workspace_roots
                        .iter()
                        .filter(|other| *other != root && other.starts_with(root))
                        .cloned()
                        .collect(),
                    ..DiscoveryOptions::default()
                },
            ) {
                Ok(files) => files,
                Err(error) => {
                    tracing::warn!(
                        "Failed to discover workspace diagnostic files in {}: {error}",
                        root.display()
                    );
                    coverage_complete = false;
                    continue;
                }
            };
            discovery_entries += result.visited_entries;
            coverage_complete &= result.complete;
            for file in result.files {
                if file.kind != FileKind::Shell
                    || !path_owned_by_root(&context, &file.absolute_path, root)
                    || open_paths.contains(&canonical_or_original(&file.absolute_path))
                {
                    continue;
                }
                discovered.entry(file.absolute_path.clone()).or_insert(file);
            }
        }
    } else if !context.workspace_roots.is_empty() {
        coverage_complete = false;
    }

    for file in discovered.values() {
        if reports.completed() >= context.options.max_files {
            coverage_complete = false;
            break;
        }
        if context.cancellation.is_cancelled() {
            coverage_complete = false;
            break;
        }
        let options = workspace_diagnostic_options_for_path(&context, &file.absolute_path);
        let remaining_source_bytes = context
            .options
            .max_source_bytes
            .saturating_sub(source_bytes)
            .min(options.max_source_bytes.saturating_sub(source_bytes));
        let source = match read_source_bounded(&file.absolute_path, remaining_source_bytes) {
            Ok(Some(source)) => source,
            Ok(None) => {
                coverage_complete = false;
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to read workspace diagnostic file {}: {error}",
                    file.absolute_path.display()
                );
                coverage_complete = false;
                continue;
            }
        };
        source_bytes += source.len();
        let Ok(uri) = types::Url::from_file_path(&file.absolute_path) else {
            continue;
        };
        let source_hash: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let cached = context
            .cache
            .get(&uri, source_hash, context.cache_generation);
        let (diagnostics, result_id) = if let Some(cached) = cached {
            (cached.diagnostics, cached.result_id)
        } else {
            let (settings, client_settings) = settings_for_path(&context, &file.absolute_path);
            let document = Arc::new(TextDocument::new(source, 0));
            let snapshot = context.snapshot_factory.snapshot(
                uri.clone(),
                document,
                Arc::new(settings),
                Arc::new(client_settings),
            );
            let diagnostics = generate_diagnostics(&snapshot);
            let result_id = diagnostic_result_id(&diagnostics);
            context.cache.insert(
                &uri,
                CachedWorkspaceDiagnostic {
                    source_hash,
                    result_id: result_id.clone(),
                    diagnostics: diagnostics.clone(),
                },
                context.cache_generation,
            );
            (diagnostics, result_id)
        };
        seen.insert(uri.to_string());
        reports.push(document_report(
            uri,
            None,
            diagnostics,
            result_id,
            &previous,
        ))?;
        progress.report(reports.completed());
    }

    if coverage_complete && !context.cancellation.is_cancelled() {
        let remaining_reports = context
            .options
            .max_files
            .saturating_sub(reports.completed());
        for previous_uri in previous
            .keys()
            .filter(|uri| !seen.contains(*uri))
            .take(remaining_reports)
        {
            let Ok(uri) = previous_uri.parse::<types::Url>() else {
                continue;
            };
            let Ok(path) = uri.to_file_path() else {
                continue;
            };
            if !path_in_workspace(&context, &path) {
                continue;
            }
            reports.push(document_report(
                uri,
                None,
                Vec::new(),
                diagnostic_result_id(&[]),
                &previous,
            ))?;
        }
    }
    context.cache.retain(&seen, context.cache_generation);

    let reports = reports.finish()?;

    Ok(types::WorkspaceDiagnosticReportResult::Report(
        types::WorkspaceDiagnosticReport { items: reports },
    ))
}

fn read_source_bounded(path: &Path, max_bytes: usize) -> std::io::Result<Option<String>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > max_bytes as u64 {
        return Ok(None);
    }
    let file = std::fs::File::open(path)?;
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut source = String::new();
    file.take(limit).read_to_string(&mut source)?;
    if source.len() > max_bytes {
        return Ok(None);
    }
    Ok(Some(source))
}

fn empty_report() -> types::WorkspaceDiagnosticReportResult {
    types::WorkspaceDiagnosticReportResult::Report(types::WorkspaceDiagnosticReport {
        items: Vec::new(),
    })
}

fn document_report(
    uri: types::Url,
    version: Option<i64>,
    diagnostics: Vec<types::Diagnostic>,
    result_id: String,
    previous: &BTreeMap<String, &str>,
) -> types::WorkspaceDocumentDiagnosticReport {
    if previous.get(uri.as_str()).copied() == Some(result_id.as_str()) {
        return types::WorkspaceDocumentDiagnosticReport::Unchanged(
            types::WorkspaceUnchangedDocumentDiagnosticReport {
                uri,
                version,
                unchanged_document_diagnostic_report: types::UnchangedDocumentDiagnosticReport {
                    result_id,
                },
            },
        );
    }
    types::WorkspaceDocumentDiagnosticReport::Full(types::WorkspaceFullDocumentDiagnosticReport {
        uri,
        version,
        full_document_diagnostic_report: types::FullDocumentDiagnosticReport {
            result_id: Some(result_id),
            items: diagnostics,
        },
    })
}

fn diagnostic_result_id(diagnostics: &[types::Diagnostic]) -> String {
    let serialized = serde_json::to_vec(diagnostics).unwrap_or_default();
    let digest = Sha256::digest(&serialized);
    let mut result = String::with_capacity(32);
    for byte in &digest[..16] {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn settings_for_path(
    context: &WorkspaceDiagnosticContext,
    path: &Path,
) -> (ShuckSettings, ClientSettings) {
    let workspace_options =
        workspace_settings_for_path(context, path).and_then(|workspace| workspace.options.as_ref());
    if let Some(workspace_options) = workspace_options {
        let layers = [&context.global_options, workspace_options];
        return (
            ShuckSettings::resolve(path.into(), &context.settings_workspace_roots, &layers),
            ClientSettings::from_layered_options(&layers),
        );
    }
    let layers = [&context.global_options];
    (
        ShuckSettings::resolve(path.into(), &context.settings_workspace_roots, &layers),
        ClientSettings::from_layered_options(&layers),
    )
}

fn workspace_diagnostic_options_for_path(
    context: &WorkspaceDiagnosticContext,
    path: &Path,
) -> WorkspaceDiagnosticsFeatureOptions {
    workspace_settings_for_path(context, path)
        .and_then(|workspace| workspace.options.as_ref())
        .map(|options| {
            options
                .server
                .workspace_diagnostics_layered_over(context.options)
        })
        .unwrap_or(context.options)
}

fn workspace_settings_for_path<'a>(
    context: &'a WorkspaceDiagnosticContext,
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

fn workspace_root_match_len(path: &Path, workspace: &WorkspaceSettingsSnapshot) -> Option<usize> {
    [Some(&workspace.root), workspace.canonical_root.as_ref()]
        .into_iter()
        .flatten()
        .filter(|root| path.starts_with(root))
        .map(|root| root.components().count())
        .max()
}

fn workspace_diagnostics_have_enabled_scope(context: &WorkspaceDiagnosticContext) -> bool {
    context
        .workspace_roots
        .iter()
        .any(|root| workspace_diagnostic_options_for_path(context, root).enabled)
}

fn path_in_workspace(context: &WorkspaceDiagnosticContext, path: &Path) -> bool {
    workspace_settings_for_path(context, path).is_some()
        || context
            .workspace_roots
            .iter()
            .any(|root| path.starts_with(root))
}

fn path_owned_by_root(context: &WorkspaceDiagnosticContext, path: &Path, root: &Path) -> bool {
    workspace_settings_for_path(context, path)
        .map(|workspace| workspace.root == root)
        .unwrap_or(true)
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_request_cannot_repopulate_an_invalidated_cache() {
        let cache = WorkspaceDiagnosticCache::default();
        let uri = types::Url::parse("file:///workspace/script.sh").unwrap();
        let stale_generation = cache.generation();
        cache.invalidate_all();
        cache.insert(
            &uri,
            CachedWorkspaceDiagnostic {
                source_hash: [7; 32],
                result_id: "stale".to_owned(),
                diagnostics: Vec::new(),
            },
            stale_generation,
        );

        assert!(cache.get(&uri, [7; 32], cache.generation()).is_none());
    }

    #[test]
    fn bounded_reader_rejects_a_file_before_loading_past_the_budget() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.sh");
        std::fs::write(&path, vec![b'x'; 4096]).unwrap();

        assert!(read_source_bounded(&path, 16).unwrap().is_none());
    }
}
