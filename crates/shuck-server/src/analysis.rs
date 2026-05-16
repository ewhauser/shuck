use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use lsp_types::Url;
use shuck_indexer::{Indexer, LineIndex};
use shuck_parser::{
    ShellProfile,
    parser::{ParseResult, Parser},
};
use shuck_semantic::{SemanticBuildOptions, SemanticModel};

use crate::TextDocument;
use crate::edit::{DocumentVersion, PositionEncoding};
use crate::lint::RawDocumentDiagnostics;
use crate::session::DocumentSnapshot;

const MAX_ANALYSIS_DOCUMENTS: usize = 64;
const MAX_ANALYSIS_SOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AnalysisCacheKey {
    uri: Url,
    version: DocumentVersion,
    document_id: usize,
    settings_epoch: u64,
    encoding: PositionEncoding,
}

struct AnalysisCacheEntry {
    key: AnalysisCacheKey,
    source_bytes: usize,
    analysis: Arc<DocumentAnalysis>,
}

#[derive(Default)]
struct AnalysisCacheState {
    entries: VecDeque<AnalysisCacheEntry>,
    source_bytes: usize,
}

pub(crate) struct DocumentAnalysisCache {
    state: Mutex<AnalysisCacheState>,
    settings_epoch: AtomicU64,
}

pub(crate) struct DocumentAnalysis {
    document: Arc<TextDocument>,
    path: Option<PathBuf>,
    shell_profile: ShellProfile,
    parse_result: ParseResult,
    indexer: Indexer,
    semantic: OnceLock<SemanticModel>,
    raw_diagnostics: OnceLock<RawDocumentDiagnostics>,
}

impl DocumentAnalysisCache {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(AnalysisCacheState::default()),
            settings_epoch: AtomicU64::new(0),
        }
    }

    pub(crate) fn current_settings_epoch(&self) -> u64 {
        self.settings_epoch.load(Ordering::Acquire)
    }

    pub(crate) fn get_or_build(
        &self,
        snapshot: &DocumentSnapshot,
    ) -> Option<Arc<DocumentAnalysis>> {
        let key = analysis_cache_key(snapshot);
        if let Some(cached) = self.get(&key) {
            return Some(cached);
        }

        let analysis = Arc::new(DocumentAnalysis::new(snapshot)?);
        self.insert(key, analysis.clone());
        Some(analysis)
    }

    pub(crate) fn invalidate_uri(&self, uri: &Url) {
        let mut state = lock_or_recover(&self.state);
        let mut retained = VecDeque::with_capacity(state.entries.len());
        let mut retained_bytes = 0usize;
        while let Some(entry) = state.entries.pop_front() {
            if entry.key.uri == *uri {
                continue;
            }
            retained_bytes += entry.source_bytes;
            retained.push_back(entry);
        }
        state.entries = retained;
        state.source_bytes = retained_bytes;
    }

    pub(crate) fn clear(&self) {
        let mut state = lock_or_recover(&self.state);
        state.entries.clear();
        state.source_bytes = 0;
        self.settings_epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn get(&self, key: &AnalysisCacheKey) -> Option<Arc<DocumentAnalysis>> {
        let mut state = lock_or_recover(&self.state);
        let index = state.entries.iter().position(|entry| entry.key == *key)?;
        let entry = state.entries.remove(index)?;
        let analysis = entry.analysis.clone();
        state.entries.push_back(entry);
        Some(analysis)
    }

    fn insert(&self, key: AnalysisCacheKey, analysis: Arc<DocumentAnalysis>) {
        let mut state = lock_or_recover(&self.state);
        if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
            let Some(entry) = state.entries.remove(index) else {
                return;
            };
            state.entries.push_back(entry);
            return;
        }

        let source_bytes = analysis.source().len();
        state.source_bytes += source_bytes;
        state.entries.push_back(AnalysisCacheEntry {
            key,
            source_bytes,
            analysis,
        });
        evict_over_budget(&mut state);
    }
}

impl DocumentAnalysis {
    fn new(snapshot: &DocumentSnapshot) -> Option<Self> {
        let query = snapshot.query();
        let document = query.document().clone();
        let source = document.contents();
        let path = query.file_path();
        let shell = crate::lint::infer_document_shell_from_parts(
            snapshot.shuck_settings(),
            query.language_id(),
            source,
            path.as_deref(),
        )?;
        let shell_profile = shell.shell_profile();
        let parse_result = Parser::with_profile(source, shell_profile.clone()).parse();
        let indexer = Indexer::new(source, &parse_result);

        Some(Self {
            document,
            path,
            shell_profile,
            parse_result,
            indexer,
            semantic: OnceLock::new(),
            raw_diagnostics: OnceLock::new(),
        })
    }

    pub(crate) fn source(&self) -> &str {
        self.document.contents()
    }

    pub(crate) fn line_index(&self) -> &LineIndex {
        self.document.index()
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn parse_result(&self) -> &ParseResult {
        &self.parse_result
    }

    pub(crate) fn indexer(&self) -> &Indexer {
        &self.indexer
    }

    pub(crate) fn semantic(&self) -> &SemanticModel {
        self.semantic.get_or_init(|| {
            SemanticModel::build_with_options(
                &self.parse_result.file,
                self.source(),
                &self.indexer,
                SemanticBuildOptions {
                    source_path: self.path(),
                    shell_profile: Some(self.shell_profile.clone()),
                    resolve_source_closure: false,
                    ..SemanticBuildOptions::default()
                },
            )
        })
    }

    pub(crate) fn raw_diagnostics(&self, snapshot: &DocumentSnapshot) -> &RawDocumentDiagnostics {
        self.raw_diagnostics
            .get_or_init(|| crate::lint::collect_raw_diagnostics_for_analysis(snapshot, self))
    }
}

fn analysis_cache_key(snapshot: &DocumentSnapshot) -> AnalysisCacheKey {
    let query = snapshot.query();
    AnalysisCacheKey {
        uri: query.file_url().clone(),
        version: query.document().version(),
        document_id: Arc::as_ptr(query.document()) as usize,
        settings_epoch: snapshot.analysis_settings_epoch(),
        encoding: snapshot.encoding(),
    }
}

fn evict_over_budget(state: &mut AnalysisCacheState) {
    while state.entries.len() > MAX_ANALYSIS_DOCUMENTS
        || (state.source_bytes > MAX_ANALYSIS_SOURCE_BYTES && state.entries.len() > 1)
    {
        let Some(entry) = state.entries.pop_front() else {
            break;
        };
        state.source_bytes = state.source_bytes.saturating_sub(entry.source_bytes);
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
