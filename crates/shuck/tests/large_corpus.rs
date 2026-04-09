use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

// ---------------------------------------------------------------------------
// Environment variable names (matching the Go test)
// ---------------------------------------------------------------------------

const LARGE_CORPUS_ENV: &str = "SHUCK_TEST_LARGE_CORPUS";
const LARGE_CORPUS_ROOT_ENV: &str = "SHUCK_LARGE_CORPUS_ROOT";
const LARGE_CORPUS_SHELLCHECK_TIMEOUT_ENV: &str = "SHUCK_LARGE_CORPUS_TIMEOUT_SECS";
const LARGE_CORPUS_SHUCK_TIMEOUT_ENV: &str = "SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS";
const LARGE_CORPUS_SHARD_ENV: &str = "TEST_SHARD_INDEX";
const LARGE_CORPUS_SHARDS_ENV: &str = "TEST_TOTAL_SHARDS";
const LARGE_CORPUS_RULES_ENV: &str = "SHUCK_LARGE_CORPUS_RULES";
const LARGE_CORPUS_SAMPLE_PERCENT_ENV: &str = "SHUCK_LARGE_CORPUS_SAMPLE_PERCENT";
const LARGE_CORPUS_MAPPED_ONLY_ENV: &str = "SHUCK_LARGE_CORPUS_MAPPED_ONLY";
const LARGE_CORPUS_KEEP_GOING_ENV: &str = "SHUCK_LARGE_CORPUS_KEEP_GOING";

const LARGE_CORPUS_DEFAULT_SHELLCHECK_TIMEOUT: Duration = Duration::from_secs(300);
const LARGE_CORPUS_DEFAULT_SHUCK_TIMEOUT: Duration = Duration::from_secs(30);
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";
const LARGE_CORPUS_WORKER_COUNT: usize = 4;
const LARGE_CORPUS_TIMEOUT_FAILURE_CAP: usize = 5;
const LARGE_CORPUS_PROGRESS_PERCENT_STEP: usize = 5;
const LARGE_CORPUS_PROGRESS_BUCKET_COUNT: usize = 100 / LARGE_CORPUS_PROGRESS_PERCENT_STEP;
const RULE_CORPUS_METADATA_DIR: &str = "tests/testdata/corpus-metadata";

const SHELLCHECK_CACHE_SCHEMA: u32 = 2;
const SHELLCHECK_CACHE_MIGRATION_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LargeCorpusConfig {
    corpus_dir: PathBuf,
    cache_dir: PathBuf,
    shellcheck_timeout: Duration,
    shuck_timeout: Duration,
    shard_index: usize,
    total_shards: usize,
    selected_rules: Option<shuck_linter::RuleSet>,
    sample_percent: usize,
    mapped_only: bool,
    keep_going: bool,
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LargeCorpusFixture {
    path: PathBuf,
    cache_rel_path: PathBuf,
    shell: String,
    source_hash: String,
}

impl LargeCorpusFixture {
    fn cache_rel_path_key(&self) -> String {
        normalize_cache_rel_path(&self.cache_rel_path)
    }
}

#[derive(Debug)]
struct LargeCorpusPathResolver {
    cache_rel_by_path: HashMap<PathBuf, PathBuf>,
    path_by_cache_rel: HashMap<PathBuf, PathBuf>,
}

impl LargeCorpusPathResolver {
    fn new(fixtures: &[&LargeCorpusFixture]) -> Self {
        let mut cache_rel_by_path = HashMap::new();
        let mut path_by_cache_rel = HashMap::new();

        for fixture in fixtures {
            let canonical_path = canonicalize_for_resolver(&fixture.path);
            cache_rel_by_path.insert(fixture.path.clone(), fixture.cache_rel_path.clone());
            cache_rel_by_path.insert(canonical_path.clone(), fixture.cache_rel_path.clone());
            path_by_cache_rel.insert(fixture.cache_rel_path.clone(), canonical_path);
        }

        Self {
            cache_rel_by_path,
            path_by_cache_rel,
        }
    }
}

impl shuck_semantic::SourcePathResolver for LargeCorpusPathResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        let Some(source_cache_rel_path) = self.cache_rel_by_path.get(source_path) else {
            return Vec::new();
        };
        let mut resolved = Vec::new();
        let mut seen = HashSet::new();

        for candidate_cache_rel_path in [
            resolve_large_corpus_candidate_cache_rel_path(source_cache_rel_path, candidate),
            resolve_large_corpus_repo_relative_candidate_cache_rel_path(
                source_cache_rel_path,
                candidate,
            ),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(path) = self.path_by_cache_rel.get(&candidate_cache_rel_path)
                && seen.insert(path.clone())
            {
                resolved.push(path.clone());
            }
        }

        resolved
    }
}

fn canonicalize_for_resolver(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_large_corpus_candidate_cache_rel_path(
    source_cache_rel_path: &Path,
    candidate: &str,
) -> Option<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return None;
    }

    let mut resolved = source_cache_rel_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut saw_component = false;

    for component in candidate_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !resolved.pop() {
                    return None;
                }
                saw_component = true;
            }
            std::path::Component::Normal(part) => {
                resolved.push(part);
                saw_component = true;
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }

    saw_component.then_some(resolved)
}

fn resolve_large_corpus_repo_relative_candidate_cache_rel_path(
    source_cache_rel_path: &Path,
    candidate: &str,
) -> Option<PathBuf> {
    if source_cache_rel_path.components().count() != 1 {
        return None;
    }

    let source_name = source_cache_rel_path.file_name()?.to_str()?;
    let mut source_parts = source_name.split("__");
    let owner = source_parts.next()?;
    let repo = source_parts.next()?;

    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return None;
    }

    let mut flattened = Vec::new();
    for component in candidate_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
            std::path::Component::Normal(part) => {
                flattened.push(part.to_string_lossy().into_owned());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }

    (!flattened.is_empty())
        .then(|| PathBuf::from(format!("{owner}__{repo}__{}", flattened.join("__"))))
}

// ---------------------------------------------------------------------------
// Progress logging
// ---------------------------------------------------------------------------

static LARGE_CORPUS_PROGRESS_LOG: Mutex<()> = Mutex::new(());

struct LargeCorpusProgress {
    total: usize,
    completed: AtomicUsize,
    logged_bucket: AtomicUsize,
}

impl LargeCorpusProgress {
    fn new(total: usize) -> Self {
        Self {
            total,
            completed: AtomicUsize::new(0),
            logged_bucket: AtomicUsize::new(0),
        }
    }

    fn finish_fixture(&self) {
        let completed = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        let bucket = progress_bucket(self.total, completed);

        loop {
            let logged_bucket = self.logged_bucket.load(Ordering::Relaxed);
            if bucket <= logged_bucket {
                return;
            }

            if self
                .logged_bucket
                .compare_exchange(logged_bucket, bucket, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let percent = bucket * LARGE_CORPUS_PROGRESS_PERCENT_STEP;
                log_large_corpus_progress(&format!(
                    "processed {completed}/{} fixtures ({percent}%)",
                    self.total
                ));
                return;
            }
        }
    }
}

fn log_large_corpus_progress(message: &str) {
    let _guard = LARGE_CORPUS_PROGRESS_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    eprintln!("large corpus: {message}");
}

fn fixture_progress_label(fixture: &LargeCorpusFixture) -> String {
    fixture.cache_rel_path_key()
}

fn log_large_corpus_timeout(fixture: &LargeCorpusFixture) {
    log_large_corpus_progress(&format!("timeout {}", fixture_progress_label(fixture)));
}

fn progress_bucket(total: usize, completed: usize) -> usize {
    if total == 0 {
        return 0;
    }

    let completed = completed.min(total) as u128;
    let total = total as u128;
    let bucket_count = LARGE_CORPUS_PROGRESS_BUCKET_COUNT as u128;

    ((completed * bucket_count) / total) as usize
}

fn format_fixture_elapsed(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1_000 {
        return format!("{millis}ms");
    }

    format!("{:.3}s", duration.as_secs_f64())
}

fn format_timeout_message(label: &str, timeout: Duration) -> String {
    format!(
        "{label} timed out after {}",
        format_fixture_elapsed(timeout)
    )
}

fn run_with_timeout<T, F>(label: &'static str, timeout: Duration, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = work();
        let _ = sender.send(result);
    });

    match receiver.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format_timeout_message(label, timeout)),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("{label} worker thread exited before returning"))
        }
    }
}

// ---------------------------------------------------------------------------
// ShellCheck types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShellCheckDiagnostic {
    #[serde(default)]
    file: String,
    code: u32,
    line: usize,
    #[serde(rename = "endLine")]
    end_line: usize,
    column: usize,
    #[serde(rename = "endColumn")]
    end_column: usize,
    level: String,
    message: String,
}

#[derive(Debug, Clone)]
struct ShellCheckRun {
    diagnostics: Vec<ShellCheckDiagnostic>,
    parse_aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellCheckProbe {
    command: String,
    version_text: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
struct RuleCorpusMetadataDocument {
    #[serde(default)]
    reviewed_divergences: Vec<ReviewedDivergenceRecord>,
    #[serde(default)]
    comparison_target_notes: Vec<ComparisonTargetNote>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum CompatibilitySide {
    ShellcheckOnly,
    ShuckOnly,
}

impl CompatibilitySide {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ShellcheckOnly => "shellcheck-only",
            Self::ShuckOnly => "shuck-only",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReviewedDivergenceRecord {
    side: CompatibilitySide,
    #[serde(default)]
    path_suffix: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
    #[serde(default)]
    column: Option<usize>,
    #[serde(default)]
    end_column: Option<usize>,
    #[serde(default)]
    labels: Vec<String>,
    reason: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ComparisonTargetNote {
    current_shellcheck_code: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DiagnosticRange {
    line: usize,
    end_line: usize,
    column: usize,
    end_column: usize,
}

impl DiagnosticRange {
    fn display(&self) -> String {
        format_range(self.line, self.column, self.end_line, self.end_column)
    }
}

#[derive(Debug, Clone)]
struct CompatibilityRecord {
    side: CompatibilitySide,
    rule_code: Option<String>,
    shellcheck_code: String,
    range: DiagnosticRange,
    message: String,
    labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CompatibilityRecordKey {
    shellcheck_code: String,
    range: DiagnosticRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatibilityClassification {
    Implementation,
    MappingIssue,
    ReviewedDivergence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorpusNoiseKind {
    UnsupportedShell,
    Patch,
    Fish,
    ParseAbort,
    ShellCollapse,
}

impl CorpusNoiseKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedShell => "unsupported-shell",
            Self::Patch => "patch",
            Self::Fish => "fish",
            Self::ParseAbort => "parse-abort",
            Self::ShellCollapse => "shell-collapse",
        }
    }
}

// ---------------------------------------------------------------------------
// ShellCheck cache
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShellCheckCacheEntry {
    schema: u32,
    diagnostics: Vec<ShellCheckDiagnostic>,
    parse_aborted: bool,
}

struct ShellCheckCache {
    dir: PathBuf,
    version_text: String,
    legacy_invocation_hash: String,
}

impl ShellCheckCache {
    fn new(cache_root: &Path, probe: &ShellCheckProbe) -> Self {
        Self {
            dir: cache_root.join("shellcheck"),
            version_text: probe.version_text.clone(),
            legacy_invocation_hash: legacy_shellcheck_invocation_hash(&probe.command),
        }
    }

    fn prepare(&self, fixtures: &[LargeCorpusFixture], worktree_roots: &[PathBuf]) {
        if fixtures.is_empty() {
            return;
        }

        let _ = fs::create_dir_all(&self.dir);

        let sentinel = self.migration_sentinel_path(fixtures);
        if sentinel.is_file() {
            return;
        }

        for fixture in fixtures {
            let stable_path = self.cache_path(fixture);
            let mut stable_exists = stable_path.is_file();

            for legacy_path in self.legacy_cache_paths(fixture, worktree_roots) {
                if !legacy_path.is_file() {
                    continue;
                }

                if stable_exists {
                    let _ = fs::remove_file(&legacy_path);
                    continue;
                }

                if let Some(parent) = stable_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                match fs::rename(&legacy_path, &stable_path) {
                    Ok(()) => stable_exists = true,
                    Err(_) if stable_path.is_file() => {
                        stable_exists = true;
                        let _ = fs::remove_file(&legacy_path);
                    }
                    Err(_) => {}
                }
            }
        }

        let _ = fs::write(&sentinel, shellcheck_cache_migration_fingerprint(fixtures));
    }

    fn run_fixture(
        &self,
        fixture: &LargeCorpusFixture,
        shellcheck_path: &str,
        timeout: Duration,
    ) -> Result<ShellCheckRun, String> {
        if let Some(cached) = self.read_cache(fixture) {
            return Ok(cached);
        }

        let run = run_shellcheck(&fixture.path, &fixture.shell, shellcheck_path, timeout)?;
        self.write_cache(fixture, &run);
        Ok(run)
    }

    fn cache_path(&self, fixture: &LargeCorpusFixture) -> PathBuf {
        let key_data = serde_json::json!({
            "schema": SHELLCHECK_CACHE_SCHEMA,
            "path": fixture.cache_rel_path_key(),
            "shell": fixture.shell,
            "sourceHash": fixture.source_hash,
            "versionText": self.version_text,
        });
        let key = hash_bytes(key_data.to_string().as_bytes());
        self.dir.join(format!("{key}.json"))
    }

    fn legacy_cache_paths(
        &self,
        fixture: &LargeCorpusFixture,
        worktree_roots: &[PathBuf],
    ) -> Vec<PathBuf> {
        let mut legacy_paths = Vec::new();
        let mut seen = HashSet::new();

        let direct = self.legacy_cache_path_for_absolute_path(fixture, &fixture.path);
        if seen.insert(direct.clone()) {
            legacy_paths.push(direct);
        }

        for absolute_path in
            projected_worktree_fixture_paths(&fixture.cache_rel_path, worktree_roots)
        {
            let legacy = self.legacy_cache_path_for_absolute_path(fixture, &absolute_path);
            if seen.insert(legacy.clone()) {
                legacy_paths.push(legacy);
            }
        }

        legacy_paths
    }

    fn legacy_cache_path_for_absolute_path(
        &self,
        fixture: &LargeCorpusFixture,
        absolute_path: &Path,
    ) -> PathBuf {
        let key_data = serde_json::json!({
            "schema": SHELLCHECK_CACHE_SCHEMA,
            "path": absolute_path.to_string_lossy(),
            "shell": fixture.shell,
            "sourceHash": fixture.source_hash,
            "invocationHash": self.legacy_invocation_hash,
        });
        let key = hash_bytes(key_data.to_string().as_bytes());
        self.dir.join(format!("{key}.json"))
    }

    fn migration_sentinel_path(&self, fixtures: &[LargeCorpusFixture]) -> PathBuf {
        let fingerprint = shellcheck_cache_migration_fingerprint(fixtures);
        self.dir.join(format!(
            ".migration-v{SHELLCHECK_CACHE_MIGRATION_VERSION}-{fingerprint}.done"
        ))
    }

    fn read_cache(&self, fixture: &LargeCorpusFixture) -> Option<ShellCheckRun> {
        let path = self.cache_path(fixture);
        let data = fs::read_to_string(&path).ok()?;
        let entry: ShellCheckCacheEntry = serde_json::from_str(&data).ok()?;
        if entry.schema != SHELLCHECK_CACHE_SCHEMA {
            return None;
        }
        Some(ShellCheckRun {
            diagnostics: entry.diagnostics,
            parse_aborted: entry.parse_aborted,
        })
    }

    fn write_cache(&self, fixture: &LargeCorpusFixture, run: &ShellCheckRun) {
        let path = self.cache_path(fixture);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let entry = ShellCheckCacheEntry {
            schema: SHELLCHECK_CACHE_SCHEMA,
            diagnostics: run.diagnostics.clone(),
            parse_aborted: run.parse_aborted,
        };
        if let Ok(data) = serde_json::to_string(&entry) {
            let _ = fs::write(&path, data);
        }
    }
}

// ---------------------------------------------------------------------------
// Shuck runner result
// ---------------------------------------------------------------------------

struct ShuckRun {
    diagnostics: Vec<shuck_linter::Diagnostic>,
    parse_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureFailureKind {
    Other,
    Timeout,
}

#[derive(Debug, Clone)]
struct FixtureFailure {
    message: String,
    kind: FixtureFailureKind,
}

#[derive(Debug, Default)]
struct FixtureEvaluation {
    implementation_diffs: Vec<String>,
    mapping_issues: Vec<String>,
    reviewed_divergences: Vec<String>,
    corpus_noise: Vec<String>,
    harness_failure: Option<FixtureFailure>,
}

#[derive(Debug, Default)]
struct FixtureFailureCollection {
    implementation_diffs: Vec<String>,
    mapping_issues: Vec<String>,
    reviewed_divergences: Vec<String>,
    corpus_noise: Vec<String>,
    harness_failures: Vec<String>,
    unsupported_shells: usize,
    timeout_cap_reached: bool,
}

impl FixtureFailureCollection {
    fn blocking_failures(&self) -> usize {
        self.implementation_diffs.len() + self.mapping_issues.len() + self.harness_failures.len()
    }

    fn has_nonblocking_items(&self) -> bool {
        self.unsupported_shells > 0
            || !self.reviewed_divergences.is_empty()
            || !self.corpus_noise.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires the large corpus; run `make test-large-corpus`"]
fn large_corpus_conforms_with_shellcheck() {
    let cfg = match resolve_large_corpus_config() {
        Some(cfg) => cfg,
        None => {
            eprintln!("large corpus test skipped (set {LARGE_CORPUS_ENV}=1 to enable)");
            return;
        }
    };

    let fixtures = load_fixtures(&cfg);
    if fixtures.is_empty() {
        panic!(
            "no fixtures found in {}",
            cfg.corpus_dir.join("scripts").display()
        );
    }

    let shellcheck = probe_shellcheck()
        .expect("shellcheck not found on PATH; install it to run the large corpus test");

    let supported_shells = shellcheck_supported_shells(&shellcheck.command);
    let shellcheck_index = build_rule_to_shellcheck_index();
    let shellcheck_rule_index = build_shellcheck_to_rule_index();
    let corpus_metadata = load_all_rule_corpus_metadata();
    let shellcheck_filter_codes =
        build_shellcheck_filter_codes(cfg.selected_rules, cfg.mapped_only);
    let shellcheck_cache = ShellCheckCache::new(&cfg.cache_dir, &shellcheck);
    shellcheck_cache.prepare(&fixtures, &discover_worktree_roots());
    let linter_settings = build_large_corpus_linter_settings(cfg.selected_rules, cfg.mapped_only);
    let supported_fixtures: Vec<_> = fixtures
        .iter()
        .filter(|fixture| fixture_supported_for_large_corpus(fixture, Some(&supported_shells)))
        .collect();
    let skipped_unsupported_shells = fixtures.len().saturating_sub(supported_fixtures.len());
    let shuck_path_resolver = Arc::new(LargeCorpusPathResolver::new(&supported_fixtures));

    let failure_collection =
        collect_fixture_failures(&supported_fixtures, cfg.keep_going, |fixture| {
            evaluate_fixture_compatibility(
                fixture,
                &shellcheck_cache,
                &shellcheck.command,
                cfg.shellcheck_timeout,
                cfg.shuck_timeout,
                &linter_settings,
                &shellcheck_index,
                &shellcheck_rule_index,
                &corpus_metadata,
                shellcheck_filter_codes.as_ref(),
                Arc::clone(&shuck_path_resolver),
            )
        });
    let mut failure_collection = failure_collection;
    failure_collection.unsupported_shells = skipped_unsupported_shells;
    let timeout_cap_note = if failure_collection.timeout_cap_reached {
        format!(
            "; stopped after reaching timeout cap of {} fixture timeouts",
            LARGE_CORPUS_TIMEOUT_FAILURE_CAP
        )
    } else {
        String::new()
    };

    if failure_collection.blocking_failures() == 0 && failure_collection.has_nonblocking_items() {
        eprintln!(
            "large corpus non-blocking summary: reviewed divergence={}, corpus noise={}, unsupported-shell={}",
            failure_collection.reviewed_divergences.len(),
            failure_collection.corpus_noise.len(),
            failure_collection.unsupported_shells,
        );
    }

    assert!(
        failure_collection.blocking_failures() == 0,
        "large corpus compatibility had {} blocking issue(s) across {} fixture(s) ({} skipped unsupported shells){}:\n\n{}",
        failure_collection.blocking_failures(),
        fixtures.len(),
        skipped_unsupported_shells,
        timeout_cap_note,
        format_large_corpus_report(&failure_collection)
    );
}

#[test]
#[ignore = "requires the large corpus; run `make test-large-corpus`"]
fn large_corpus_zsh_fixtures_parse() {
    let cfg = match resolve_large_corpus_config() {
        Some(cfg) => cfg,
        None => {
            eprintln!("large corpus test skipped (set {LARGE_CORPUS_ENV}=1 to enable)");
            return;
        }
    };

    let fixtures = load_fixtures(&cfg);
    if fixtures.is_empty() {
        panic!(
            "no fixtures found in {}",
            cfg.corpus_dir.join("scripts").display()
        );
    }

    let zsh_fixtures: Vec<_> = fixtures
        .iter()
        .filter(|fixture| fixture_selected_for_large_corpus_zsh_parse(fixture))
        .collect();
    if zsh_fixtures.is_empty() {
        eprintln!("large corpus zsh parse skipped (no zsh fixtures found for this shard/sample)");
        return;
    }

    let failure_collection = collect_fixture_failures(&zsh_fixtures, cfg.keep_going, |fixture| {
        evaluate_fixture_zsh_parse(fixture, cfg.shuck_timeout)
    });
    let timeout_cap_note = if failure_collection.timeout_cap_reached {
        format!(
            "; stopped after reaching timeout cap of {} fixture timeouts",
            LARGE_CORPUS_TIMEOUT_FAILURE_CAP
        )
    } else {
        String::new()
    };

    assert!(
        failure_collection.blocking_failures() == 0,
        "large corpus zsh parse had {} blocking issue(s) across {} fixture(s){}:\n\n{}",
        failure_collection.blocking_failures(),
        zsh_fixtures.len(),
        timeout_cap_note,
        format_large_corpus_report(&failure_collection)
    );
}

fn collect_fixture_failures<F>(
    fixtures: &[&LargeCorpusFixture],
    keep_going: bool,
    evaluate: F,
) -> FixtureFailureCollection
where
    F: Fn(&LargeCorpusFixture) -> FixtureEvaluation + Sync,
{
    let progress = LargeCorpusProgress::new(fixtures.len());

    if keep_going {
        return collect_fixture_failures_in_parallel(
            fixtures,
            LARGE_CORPUS_WORKER_COUNT,
            &evaluate,
            &progress,
        );
    }

    collect_fixture_failures_sequential(fixtures, &evaluate, &progress)
}

fn collect_fixture_failures_sequential<F>(
    fixtures: &[&LargeCorpusFixture],
    evaluate: &F,
    progress: &LargeCorpusProgress,
) -> FixtureFailureCollection
where
    F: Fn(&LargeCorpusFixture) -> FixtureEvaluation,
{
    let mut collection = FixtureFailureCollection::default();

    for fixture in fixtures {
        let evaluation = evaluate(fixture);
        let has_blocking = evaluation.harness_failure.is_some()
            || !evaluation.implementation_diffs.is_empty()
            || !evaluation.mapping_issues.is_empty();
        let timeout = evaluation
            .harness_failure
            .as_ref()
            .is_some_and(|failure| failure.kind == FixtureFailureKind::Timeout);
        progress.finish_fixture();

        merge_fixture_evaluation(&mut collection, evaluation);

        if has_blocking {
            if timeout {
                log_large_corpus_timeout(fixture);
            }
            panic!("{}", format_large_corpus_report(&collection));
        }
    }

    collection
}

fn collect_fixture_failures_in_parallel<F>(
    fixtures: &[&LargeCorpusFixture],
    worker_count: usize,
    evaluate: &F,
    progress: &LargeCorpusProgress,
) -> FixtureFailureCollection
where
    F: Fn(&LargeCorpusFixture) -> FixtureEvaluation + Sync,
{
    if fixtures.is_empty() {
        return FixtureFailureCollection::default();
    }

    let worker_count = worker_count.max(1).min(fixtures.len());
    let next_index = AtomicUsize::new(0);
    let timeout_failures = AtomicUsize::new(0);
    let timeout_cap_reached = AtomicBool::new(false);
    let collection = Mutex::new(Vec::<(usize, FixtureEvaluation)>::new());

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let collection = &collection;
            let next_index = &next_index;
            let timeout_failures = &timeout_failures;
            let timeout_cap_reached = &timeout_cap_reached;
            scope.spawn(move || {
                let mut local_evaluations = Vec::new();
                loop {
                    if timeout_cap_reached.load(Ordering::Relaxed) {
                        break;
                    }

                    let index = next_index.fetch_add(1, Ordering::Relaxed);
                    if index >= fixtures.len() {
                        break;
                    }

                    let fixture = fixtures[index];
                    let result = panic::catch_unwind(AssertUnwindSafe(|| evaluate(fixture)));
                    progress.finish_fixture();

                    match result {
                        Ok(evaluation) => {
                            if evaluation
                                .harness_failure
                                .as_ref()
                                .is_some_and(|failure| failure.kind == FixtureFailureKind::Timeout)
                            {
                                log_large_corpus_timeout(fixture);
                                let timeout_count =
                                    timeout_failures.fetch_add(1, Ordering::Relaxed) + 1;
                                if timeout_count <= LARGE_CORPUS_TIMEOUT_FAILURE_CAP {
                                    local_evaluations.push((index, evaluation));
                                }
                                if timeout_count >= LARGE_CORPUS_TIMEOUT_FAILURE_CAP {
                                    timeout_cap_reached.store(true, Ordering::Relaxed);
                                }
                                continue;
                            }

                            local_evaluations.push((index, evaluation));
                        }
                        Err(payload) => {
                            local_evaluations.push((
                                index,
                                FixtureEvaluation {
                                    harness_failure: Some(FixtureFailure {
                                        kind: FixtureFailureKind::Other,
                                        message: format_fixture_panic(fixture, payload),
                                    }),
                                    ..FixtureEvaluation::default()
                                },
                            ));
                        }
                    }
                }

                if !local_evaluations.is_empty() {
                    collection.lock().unwrap().extend(local_evaluations);
                }
            });
        }
    });

    let mut evaluations = collection.into_inner().unwrap();
    evaluations.sort_by_key(|(index, _)| *index);
    let mut failures = FixtureFailureCollection::default();
    for (_, evaluation) in evaluations {
        merge_fixture_evaluation(&mut failures, evaluation);
    }
    failures.timeout_cap_reached = timeout_cap_reached.load(Ordering::Relaxed);
    failures
}

fn format_fixture_panic(fixture: &LargeCorpusFixture, payload: Box<dyn Any + Send>) -> String {
    format_fixture_failure(
        &fixture.path,
        &[format!("fixture panic: {}", panic_payload_message(payload))],
    )
}

fn panic_payload_message(payload: Box<dyn Any + Send>) -> String {
    let payload = &*payload;
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    "non-string panic payload".into()
}

#[allow(clippy::too_many_arguments)]
fn evaluate_fixture_compatibility(
    fixture: &LargeCorpusFixture,
    shellcheck_cache: &ShellCheckCache,
    shellcheck_path: &str,
    shellcheck_timeout: Duration,
    shuck_timeout: Duration,
    linter_settings: &shuck_linter::LinterSettings,
    shellcheck_index: &HashMap<String, String>,
    shellcheck_rule_index: &HashMap<u32, String>,
    corpus_metadata: &HashMap<String, RuleCorpusMetadataDocument>,
    shellcheck_filter_codes: Option<&HashSet<u32>>,
    shuck_path_resolver: Arc<LargeCorpusPathResolver>,
) -> FixtureEvaluation {
    let mut evaluation = FixtureEvaluation::default();
    let src = fs::read(&fixture.path).unwrap_or_default();

    let shuck_run = match run_shuck_with_timeout(
        fixture,
        linter_settings,
        shuck_timeout,
        shuck_path_resolver,
    ) {
        Ok(run) => run,
        Err(err) => {
            evaluation.harness_failure = Some(FixtureFailure {
                kind: fixture_failure_kind_for_message(&err, "shuck"),
                message: format_fixture_failure(&fixture.path, &[err]),
            });
            return evaluation;
        }
    };
    if let Some(ref err) = shuck_run.parse_error {
        if shuck_parse_aborted(err) {
            let noise_kind = classify_fixture_noise(fixture, &src, true, false);
            evaluation.corpus_noise.push(format_fixture_failure(
                &fixture.path,
                &[
                    format!("corpus noise [{}]", noise_kind.as_str()),
                    format!("shuck parse error: {err}"),
                ],
            ));
        } else {
            evaluation.harness_failure = Some(FixtureFailure {
                kind: FixtureFailureKind::Other,
                message: format_fixture_failure(&fixture.path, &[format!("shuck error: {err}")]),
            });
        }
        return evaluation;
    }

    match shellcheck_cache.run_fixture(fixture, shellcheck_path, shellcheck_timeout) {
        Ok(sc_run) => {
            let sc_run = filter_shellcheck_run(sc_run, shellcheck_filter_codes);
            if sc_run.parse_aborted {
                let noise_kind = classify_fixture_noise(fixture, &src, false, true);
                let mut details = vec![format!("corpus noise [{}]", noise_kind.as_str())];
                details.push("shellcheck parse aborted".into());
                evaluation
                    .corpus_noise
                    .push(format_fixture_failure(&fixture.path, &details));
                return evaluation;
            }

            let labels = compatibility_context_labels(&fixture.cache_rel_path, &src);
            let shellcheck_records = shellcheck_compatibility_records(
                &sc_run.diagnostics,
                shellcheck_rule_index,
                &labels,
            );
            let shuck_records =
                shuck_compatibility_records(&shuck_run.diagnostics, shellcheck_index, &labels);
            let (shellcheck_only, shuck_only) =
                unmatched_compatibility_records(&shellcheck_records, &shuck_records);
            let location_only_codes = location_only_shellcheck_codes(
                &shellcheck_records,
                &shuck_records,
                &shellcheck_only,
                &shuck_only,
            );

            let mut implementation_details = Vec::new();
            let mut mapping_details = Vec::new();
            let mut reviewed_details = Vec::new();

            for record in shellcheck_only.into_iter().chain(shuck_only) {
                let (classification, reason) =
                    classify_compatibility_record(&record, &fixture.path, corpus_metadata);
                let detail = format_compatibility_record(
                    &record,
                    reason.as_deref(),
                    classification == CompatibilityClassification::Implementation
                        && location_only_codes.contains(record.shellcheck_code.as_str()),
                );
                match classification {
                    CompatibilityClassification::Implementation => {
                        implementation_details.push(detail)
                    }
                    CompatibilityClassification::MappingIssue => mapping_details.push(detail),
                    CompatibilityClassification::ReviewedDivergence => {
                        reviewed_details.push(detail)
                    }
                }
            }

            if !implementation_details.is_empty() {
                evaluation.implementation_diffs.push(format_fixture_failure(
                    &fixture.path,
                    &implementation_details,
                ));
            }
            if !mapping_details.is_empty() {
                evaluation
                    .mapping_issues
                    .push(format_fixture_failure(&fixture.path, &mapping_details));
            }
            if !reviewed_details.is_empty() {
                evaluation
                    .reviewed_divergences
                    .push(format_fixture_failure(&fixture.path, &reviewed_details));
            }
        }
        Err(err) => {
            evaluation.harness_failure = Some(FixtureFailure {
                kind: fixture_failure_kind_for_message(&err, "shellcheck"),
                message: format_fixture_failure(
                    &fixture.path,
                    &[format!("shellcheck error: {err}")],
                ),
            });
        }
    }

    evaluation
}

fn evaluate_fixture_zsh_parse(
    fixture: &LargeCorpusFixture,
    shuck_timeout: Duration,
) -> FixtureEvaluation {
    let mut evaluation = FixtureEvaluation::default();
    let parse_result =
        match parse_fixture_for_effective_large_corpus_shell_with_timeout(fixture, shuck_timeout) {
            Ok(result) => result,
            Err(err) => {
                evaluation.harness_failure = Some(FixtureFailure {
                    kind: fixture_failure_kind_for_message(&err, "shuck"),
                    message: format_fixture_failure(&fixture.path, &[err]),
                });
                return evaluation;
            }
        };

    if let Err(err) = parse_result {
        evaluation.harness_failure = Some(FixtureFailure {
            kind: FixtureFailureKind::Other,
            message: format_fixture_failure(&fixture.path, &[format!("shuck parse error: {err}")]),
        });
    }

    evaluation
}

fn fixture_failure_kind_for_message(message: &str, label: &str) -> FixtureFailureKind {
    if is_timeout_message(message, label) {
        FixtureFailureKind::Timeout
    } else {
        FixtureFailureKind::Other
    }
}

fn is_timeout_message(message: &str, label: &str) -> bool {
    message.starts_with(label) && message.contains(" timed out after ")
}

fn filter_shellcheck_run(
    mut run: ShellCheckRun,
    shellcheck_filter_codes: Option<&HashSet<u32>>,
) -> ShellCheckRun {
    let Some(shellcheck_filter_codes) = shellcheck_filter_codes else {
        return run;
    };

    run.diagnostics
        .retain(|diag| shellcheck_filter_codes.contains(&diag.code));
    run.parse_aborted = shellcheck_parse_aborted(&run.diagnostics);
    run
}

fn merge_fixture_evaluation(
    collection: &mut FixtureFailureCollection,
    evaluation: FixtureEvaluation,
) {
    collection
        .implementation_diffs
        .extend(evaluation.implementation_diffs);
    collection.mapping_issues.extend(evaluation.mapping_issues);
    collection
        .reviewed_divergences
        .extend(evaluation.reviewed_divergences);
    collection.corpus_noise.extend(evaluation.corpus_noise);
    if let Some(failure) = evaluation.harness_failure {
        collection.harness_failures.push(failure.message);
    }
}

fn rule_corpus_metadata_path(rule_code: &str) -> PathBuf {
    let filename = format!("{}.yaml", rule_code.to_ascii_lowercase());
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(RULE_CORPUS_METADATA_DIR)
        .join(filename)
}

fn load_rule_corpus_metadata(rule_code: &str) -> RuleCorpusMetadataDocument {
    let path = rule_corpus_metadata_path(rule_code);
    if !path.exists() {
        return RuleCorpusMetadataDocument::default();
    }
    let data =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_yaml::from_str(&data).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

fn load_all_rule_corpus_metadata() -> HashMap<String, RuleCorpusMetadataDocument> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(RULE_CORPUS_METADATA_DIR);
    let mut map = HashMap::new();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return map,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "yaml")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            let rule_code = stem.to_ascii_uppercase();
            let metadata = load_rule_corpus_metadata(&rule_code);
            if !metadata.reviewed_divergences.is_empty()
                || !metadata.comparison_target_notes.is_empty()
            {
                map.insert(rule_code, metadata);
            }
        }
    }
    map
}

fn reviewed_divergence_reason<'a>(
    metadata: &'a RuleCorpusMetadataDocument,
    record: &CompatibilityRecord,
    path: &Path,
) -> Option<&'a str> {
    let path = path.to_string_lossy();
    metadata.reviewed_divergences.iter().find_map(|entry| {
        (entry.side == record.side
            && entry
                .path_suffix
                .as_ref()
                .is_none_or(|suffix| path.ends_with(suffix))
            && entry.line.is_none_or(|line| line == record.range.line)
            && entry
                .end_line
                .is_none_or(|end_line| end_line == record.range.end_line)
            && entry
                .column
                .is_none_or(|column| column == record.range.column)
            && entry
                .end_column
                .is_none_or(|end_column| end_column == record.range.end_column)
            && entry.labels.iter().all(|label| {
                record
                    .labels
                    .iter()
                    .any(|record_label| record_label == label)
            }))
        .then_some(entry.reason.as_str())
    })
}

fn comparison_target_note_reason<'a>(
    metadata: &'a RuleCorpusMetadataDocument,
    shellcheck_code: &str,
) -> Option<&'a str> {
    metadata.comparison_target_notes.iter().find_map(|note| {
        (note.current_shellcheck_code == shellcheck_code).then_some(note.reason.as_str())
    })
}

fn classify_compatibility_record(
    record: &CompatibilityRecord,
    path: &Path,
    corpus_metadata: &HashMap<String, RuleCorpusMetadataDocument>,
) -> (CompatibilityClassification, Option<String>) {
    let Some(rule_code) = record.rule_code.as_deref() else {
        return (
            CompatibilityClassification::MappingIssue,
            Some(format!("no Shuck rule maps {}", record.shellcheck_code)),
        );
    };

    let Some(metadata) = corpus_metadata.get(rule_code) else {
        return (CompatibilityClassification::Implementation, None);
    };

    if let Some(reason) = reviewed_divergence_reason(metadata, record, path) {
        return (
            CompatibilityClassification::ReviewedDivergence,
            Some(reason.to_owned()),
        );
    }

    if let Some(reason) = comparison_target_note_reason(metadata, record.shellcheck_code.as_str()) {
        return (
            CompatibilityClassification::MappingIssue,
            Some(reason.to_owned()),
        );
    }

    (CompatibilityClassification::Implementation, None)
}

fn shellcheck_compatibility_records(
    diagnostics: &[ShellCheckDiagnostic],
    shellcheck_rule_index: &HashMap<u32, String>,
    labels: &[String],
) -> Vec<CompatibilityRecord> {
    diagnostics
        .iter()
        .map(|diag| CompatibilityRecord {
            side: CompatibilitySide::ShellcheckOnly,
            rule_code: shellcheck_rule_index.get(&diag.code).cloned(),
            shellcheck_code: format!("SC{:04}", diag.code),
            range: DiagnosticRange {
                line: diag.line,
                end_line: diag.end_line,
                column: diag.column,
                end_column: diag.end_column,
            },
            message: format!("{} {}", diag.level, diag.message),
            labels: labels.to_vec(),
        })
        .collect()
}

fn shuck_compatibility_records(
    diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
    labels: &[String],
) -> Vec<CompatibilityRecord> {
    diagnostics
        .iter()
        .filter_map(|diag| {
            shellcheck_index
                .get(diag.code())
                .map(|shellcheck_code| CompatibilityRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_code: Some(diag.code().to_owned()),
                    shellcheck_code: shellcheck_code.clone(),
                    range: DiagnosticRange {
                        line: diag.span.start.line,
                        end_line: diag.span.end.line,
                        column: diag.span.start.column,
                        end_column: diag.span.end.column,
                    },
                    message: diag.message.clone(),
                    labels: labels.to_vec(),
                })
        })
        .collect()
}

fn compatibility_record_key(record: &CompatibilityRecord) -> CompatibilityRecordKey {
    CompatibilityRecordKey {
        shellcheck_code: record.shellcheck_code.clone(),
        range: record.range.clone(),
    }
}

fn unmatched_compatibility_records(
    shellcheck_records: &[CompatibilityRecord],
    shuck_records: &[CompatibilityRecord],
) -> (Vec<CompatibilityRecord>, Vec<CompatibilityRecord>) {
    let mut shuck_counts = HashMap::new();
    for record in shuck_records {
        *shuck_counts
            .entry(compatibility_record_key(record))
            .or_insert(0usize) += 1;
    }

    let mut shellcheck_counts = HashMap::new();
    for record in shellcheck_records {
        *shellcheck_counts
            .entry(compatibility_record_key(record))
            .or_insert(0usize) += 1;
    }

    let mut shellcheck_only = Vec::new();
    for record in shellcheck_records {
        let key = compatibility_record_key(record);
        let matched = shuck_counts.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        });
        if !matched {
            shellcheck_only.push(record.clone());
        }
    }

    let mut shuck_only = Vec::new();
    for record in shuck_records {
        let key = compatibility_record_key(record);
        let matched = shellcheck_counts.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        });
        if !matched {
            shuck_only.push(record.clone());
        }
    }

    (shellcheck_only, shuck_only)
}

fn location_only_shellcheck_codes(
    shellcheck_records: &[CompatibilityRecord],
    shuck_records: &[CompatibilityRecord],
    shellcheck_only: &[CompatibilityRecord],
    shuck_only: &[CompatibilityRecord],
) -> HashSet<String> {
    let shellcheck_counts = count_codes(
        &shellcheck_records
            .iter()
            .map(|record| record.shellcheck_code.clone())
            .collect::<Vec<_>>(),
    );
    let shuck_counts = count_codes(
        &shuck_records
            .iter()
            .map(|record| record.shellcheck_code.clone())
            .collect::<Vec<_>>(),
    );
    let shellcheck_unmatched = count_codes(
        &shellcheck_only
            .iter()
            .map(|record| record.shellcheck_code.clone())
            .collect::<Vec<_>>(),
    );
    let shuck_unmatched = count_codes(
        &shuck_only
            .iter()
            .map(|record| record.shellcheck_code.clone())
            .collect::<Vec<_>>(),
    );

    shellcheck_counts
        .keys()
        .chain(shuck_counts.keys())
        .filter(|code| {
            shellcheck_counts.get(*code) == shuck_counts.get(*code)
                && shellcheck_unmatched.get(*code).copied().unwrap_or(0) > 0
                && shuck_unmatched.get(*code).copied().unwrap_or(0) > 0
        })
        .cloned()
        .collect()
}

fn compatibility_context_labels(path: &Path, src: &[u8]) -> Vec<String> {
    let mut labels = Vec::new();
    let source = String::from_utf8_lossy(src);
    let shell = shuck_linter::ShellDialect::from_name(resolve_shell(path, src).as_str());
    let file_context = shuck_linter::classify_file_context(&source, Some(path), shell);

    for tag in file_context.tags() {
        labels.push(tag.label().to_owned());
    }
    if source_starts_with_unknown_shell_comment(src) {
        labels.push("shell-collapse".into());
    }
    labels.sort();
    labels.dedup();
    labels
}

fn shuck_parse_aborted(error: &str) -> bool {
    !error.starts_with("read error:")
}

fn classify_fixture_noise(
    fixture: &LargeCorpusFixture,
    src: &[u8],
    _shuck_parse_aborted: bool,
    _shellcheck_parse_aborted: bool,
) -> CorpusNoiseKind {
    if fixture_looks_like_patch(fixture) {
        CorpusNoiseKind::Patch
    } else if fixture_looks_like_fish(fixture, src) {
        CorpusNoiseKind::Fish
    } else if source_starts_with_unknown_shell_comment(src) {
        CorpusNoiseKind::ShellCollapse
    } else {
        CorpusNoiseKind::ParseAbort
    }
}

fn fixture_looks_like_patch(fixture: &LargeCorpusFixture) -> bool {
    let path = fixture.path.to_string_lossy().to_lowercase();
    path.ends_with(".patch") || path.ends_with(".diff") || path.ends_with(".dpatch")
}

fn fixture_looks_like_fish(fixture: &LargeCorpusFixture, src: &[u8]) -> bool {
    if fixture
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fish"))
    {
        return true;
    }

    let first_line = src
        .split(|&b| b == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).to_lowercase())
        .unwrap_or_default();
    if first_line.contains("fish") {
        return true;
    }

    fixture
        .path
        .to_string_lossy()
        .to_lowercase()
        .contains("oh-my-fish")
}

fn format_compatibility_record(
    record: &CompatibilityRecord,
    reason: Option<&str>,
    location_only: bool,
) -> String {
    let mut labels = record.labels.clone();
    if location_only {
        labels.push("location-only".into());
    }
    labels.sort();
    labels.dedup();
    let labels = if labels.is_empty() {
        String::new()
    } else {
        format!(" labels={}", labels.join(","))
    };
    let reason = reason
        .map(|reason| format!(" reason={reason}"))
        .unwrap_or_default();
    format!(
        "{} {}/{} {} {}{}{}",
        record.side.as_str(),
        record.rule_code.as_deref().unwrap_or("(unmapped)"),
        record.shellcheck_code,
        record.range.display(),
        record.message,
        labels,
        reason,
    )
}

fn format_large_corpus_report(collection: &FixtureFailureCollection) -> String {
    let mut sections = Vec::new();

    if let Some(section) =
        format_report_section("Implementation Diffs", &collection.implementation_diffs)
    {
        sections.push(section);
    }
    if let Some(section) = format_report_section("Mapping Issues", &collection.mapping_issues) {
        sections.push(section);
    }
    if let Some(section) =
        format_report_section("Reviewed Divergence", &collection.reviewed_divergences)
    {
        sections.push(section);
    }

    let mut corpus_noise = collection.corpus_noise.clone();
    if collection.unsupported_shells > 0 {
        corpus_noise.push(format!(
            "{} skipped: {} fixture(s)",
            CorpusNoiseKind::UnsupportedShell.as_str(),
            collection.unsupported_shells
        ));
    }
    if let Some(section) = format_report_section("Corpus Noise", &corpus_noise) {
        sections.push(section);
    }

    if let Some(section) = format_report_section("Harness Failures", &collection.harness_failures) {
        sections.push(section);
    }

    if sections.is_empty() {
        "(none)".into()
    } else {
        sections.join("\n\n")
    }
}

fn format_report_section(title: &str, items: &[String]) -> Option<String> {
    if items.is_empty() {
        None
    } else {
        Some(format!("{}:\n{}", title, items.join("\n\n")))
    }
}

fn format_fixture_failure(path: &Path, issues: &[String]) -> String {
    format!(
        "{}\n{}",
        path.display(),
        indent_detail(&issues.join("\n\n"))
    )
}

fn fixture_supported_for_large_corpus(
    fixture: &LargeCorpusFixture,
    shellcheck_supported_shells: Option<&HashMap<&'static str, ()>>,
) -> bool {
    if path_is_sample_file(&fixture.path)
        || path_is_fish_file(&fixture.path)
        || fixture_is_repo_git_entry(fixture)
    {
        return false;
    }

    shell_supported_for_large_corpus(
        effective_large_corpus_shell(fixture),
        shellcheck_supported_shells,
    )
}

fn fixture_selected_for_large_corpus_zsh_parse(fixture: &LargeCorpusFixture) -> bool {
    if path_is_sample_file(&fixture.path)
        || path_is_fish_file(&fixture.path)
        || fixture_is_repo_git_entry(fixture)
    {
        return false;
    }

    effective_large_corpus_shell(fixture) == "zsh"
}

fn shell_supported_for_large_corpus(
    shell: &str,
    shellcheck_supported_shells: Option<&HashMap<&'static str, ()>>,
) -> bool {
    if shell == "zsh" {
        return false;
    }

    shellcheck_supported_shells
        .map(|supported| supported.contains_key(shell))
        .unwrap_or(true)
}

fn effective_large_corpus_shell(fixture: &LargeCorpusFixture) -> &str {
    if fixture_looks_like_zsh(fixture) {
        "zsh"
    } else {
        fixture.shell.as_str()
    }
}

fn fixture_looks_like_zsh(fixture: &LargeCorpusFixture) -> bool {
    let Some(name) = fixture.path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    name.ends_with(".zsh") || name.ends_with(".zsh-theme") || name.starts_with(".zsh")
}

fn path_is_sample_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".sample"))
}

fn path_is_fish_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fish"))
}

fn fixture_is_repo_git_entry(fixture: &LargeCorpusFixture) -> bool {
    let Some(name) = fixture.path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    name.contains("__.git__")
}

// ---------------------------------------------------------------------------
// Config resolution
// ---------------------------------------------------------------------------

fn resolve_large_corpus_config() -> Option<LargeCorpusConfig> {
    if !env_truthy(LARGE_CORPUS_ENV, false) {
        return None;
    }

    let repo_root = repo_root();
    let default_root = repo_root.join(LARGE_CORPUS_CACHE_DIR_NAME);

    let root_hint = env::var(LARGE_CORPUS_ROOT_ENV)
        .ok()
        .filter(|s| !s.is_empty());

    let candidates: Vec<PathBuf> = if let Some(ref hint) = root_hint {
        vec![PathBuf::from(hint)]
    } else {
        vec![
            default_root.clone(),
            repo_root.join("..").join("shell-checks"),
        ]
    };

    for candidate in &candidates {
        if let Some(corpus_dir) = normalize_large_corpus_root(candidate) {
            let timeout_secs = positive_env_int(
                LARGE_CORPUS_SHELLCHECK_TIMEOUT_ENV,
                LARGE_CORPUS_DEFAULT_SHELLCHECK_TIMEOUT.as_secs() as usize,
            );
            let shuck_timeout_secs = positive_env_int(
                LARGE_CORPUS_SHUCK_TIMEOUT_ENV,
                LARGE_CORPUS_DEFAULT_SHUCK_TIMEOUT.as_secs() as usize,
            );
            let total_shards = positive_env_int(LARGE_CORPUS_SHARDS_ENV, 1);
            let shard_index = non_negative_env_int(LARGE_CORPUS_SHARD_ENV, 0);
            let selected_rules = parse_large_corpus_rule_filter_env(LARGE_CORPUS_RULES_ENV);
            let sample_percent = percentage_env_int(LARGE_CORPUS_SAMPLE_PERCENT_ENV, 100);

            assert!(
                shard_index < total_shards,
                "{LARGE_CORPUS_SHARD_ENV}={shard_index}, want value in [0,{total_shards})"
            );

            return Some(LargeCorpusConfig {
                corpus_dir,
                cache_dir: default_root,
                shellcheck_timeout: Duration::from_secs(timeout_secs as u64),
                shuck_timeout: Duration::from_secs(shuck_timeout_secs as u64),
                shard_index,
                total_shards,
                selected_rules,
                sample_percent,
                mapped_only: env_truthy(LARGE_CORPUS_MAPPED_ONLY_ENV, false),
                keep_going: env_truthy(LARGE_CORPUS_KEEP_GOING_ENV, false),
            });
        }
    }

    panic!(
        "large corpus not found; set {LARGE_CORPUS_ROOT_ENV} to an existing corpus directory, \
         run scripts/corpus-download.sh to populate {}, or place shell-checks at {}",
        default_root.display(),
        repo_root.join("..").join("shell-checks").display(),
    );
}

fn normalize_large_corpus_root(root: &Path) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }

    let repo_corpus = root.join("corpus");
    if corpus_dir_looks_valid(&repo_corpus) {
        return Some(repo_corpus);
    }
    if corpus_dir_looks_valid(root) {
        return Some(root.to_path_buf());
    }
    None
}

fn corpus_dir_looks_valid(dir: &Path) -> bool {
    dir.join("scripts").is_dir() && dir.join("manifest.yaml").is_file()
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/shuck -> workspace root
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to resolve repo root")
        .to_path_buf()
}

fn discover_worktree_roots() -> Vec<PathBuf> {
    let repo_root = repo_root();
    let output = Command::new("git")
        .current_dir(&repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output();

    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    if let Ok(output) = output
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let Some(path) = line.strip_prefix("worktree ") else {
                continue;
            };
            let root = PathBuf::from(path);
            if seen.insert(root.clone()) {
                roots.push(root);
            }
        }
    }

    if seen.insert(repo_root.clone()) {
        roots.push(repo_root);
    }

    roots
}

fn normalize_cache_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn projected_worktree_fixture_paths(
    cache_rel_path: &Path,
    worktree_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut absolute_paths = Vec::new();
    let mut seen = HashSet::new();

    for root in worktree_roots {
        let corpus_base = root.join(".cache").join("large-corpus");
        for projected in [
            corpus_base.join("scripts").join(cache_rel_path),
            corpus_base
                .join("corpus")
                .join("scripts")
                .join(cache_rel_path),
        ] {
            if seen.insert(projected.clone()) {
                absolute_paths.push(projected);
            }
        }
    }

    absolute_paths
}

fn shellcheck_cache_migration_fingerprint(fixtures: &[LargeCorpusFixture]) -> String {
    let mut hasher = Sha256::new();
    update_hash_component(&mut hasher, &SHELLCHECK_CACHE_MIGRATION_VERSION.to_string());

    for fixture in fixtures {
        update_hash_component(&mut hasher, &fixture.cache_rel_path_key());
        update_hash_component(&mut hasher, &fixture.shell);
        update_hash_component(&mut hasher, &fixture.source_hash);
    }

    let result = hasher.finalize();
    result.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn update_hash_component(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_fixtures(cfg: &LargeCorpusConfig) -> Vec<LargeCorpusFixture> {
    let mut fixtures = collect_fixtures(&cfg.corpus_dir);
    fixtures = shard_fixtures(fixtures, cfg.shard_index, cfg.total_shards);
    fixtures = sample_fixtures(fixtures, cfg.sample_percent);
    fixtures
}

fn collect_fixtures(corpus_dir: &Path) -> Vec<LargeCorpusFixture> {
    let scripts_dir = corpus_dir.join("scripts");
    let mut fixtures = Vec::new();

    for entry in walkdir::WalkDir::new(&scripts_dir)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".shuck_cache" && name != ".shellck_cache"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path().to_path_buf();
        if path_is_sample_file(&path) || path_is_fish_file(&path) {
            continue;
        }
        let cache_rel_path = path
            .strip_prefix(&scripts_dir)
            .unwrap_or(path.as_path())
            .to_path_buf();
        let src = match fs::read(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let shell = resolve_shell(&path, &src);
        let source_hash = hash_bytes(&src);

        fixtures.push(LargeCorpusFixture {
            path,
            cache_rel_path,
            shell,
            source_hash,
        });
    }

    fixtures.sort_by(|a, b| a.path.cmp(&b.path));
    fixtures
}

fn shard_fixtures(
    fixtures: Vec<LargeCorpusFixture>,
    shard_index: usize,
    total_shards: usize,
) -> Vec<LargeCorpusFixture> {
    if fixtures.is_empty() || total_shards <= 1 {
        return fixtures;
    }
    let start = shard_index * fixtures.len() / total_shards;
    let end = (shard_index + 1) * fixtures.len() / total_shards;
    fixtures[start..end].to_vec()
}

fn sample_fixtures(
    fixtures: Vec<LargeCorpusFixture>,
    sample_percent: usize,
) -> Vec<LargeCorpusFixture> {
    if fixtures.is_empty() || sample_percent >= 100 {
        return fixtures;
    }

    fixtures
        .into_iter()
        .filter(|fixture| fixture_selected_for_sample(fixture, sample_percent))
        .collect()
}

fn fixture_selected_for_sample(fixture: &LargeCorpusFixture, sample_percent: usize) -> bool {
    if sample_percent >= 100 {
        return true;
    }

    let mut hasher = Sha256::new();
    update_hash_component(&mut hasher, "large-corpus-sample-v1");
    update_hash_component(&mut hasher, &fixture.cache_rel_path_key());

    let digest = hasher.finalize();
    let sample_value = u64::from_be_bytes(digest[..8].try_into().expect("slice has 8 bytes"));
    let threshold = ((u64::MAX as u128) + 1) * sample_percent as u128 / 100;

    (sample_value as u128) < threshold
}

fn resolve_shell(path: &Path, src: &[u8]) -> String {
    let first_line = src
        .split(|&b| b == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).to_lowercase())
        .unwrap_or_default();

    if first_line.contains("bash") {
        return "bash".into();
    }
    if first_line.contains("ksh") {
        return "ksh".into();
    }
    if first_line.contains("zsh") {
        return "zsh".into();
    }
    if first_line.contains("dash") {
        return "sh".into();
    }
    if first_line.contains("sh") {
        return "sh".into();
    }

    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("bash") => "bash".into(),
        Some("ksh") => "ksh".into(),
        Some("zsh") => "zsh".into(),
        Some("sh" | "dash") => "sh".into(),
        _ => "sh".into(),
    }
}

// ---------------------------------------------------------------------------
// Shuck runner
// ---------------------------------------------------------------------------

fn parser_dialect_for_large_corpus_shell(shell: &str) -> shuck_parser::ShellDialect {
    match shuck_linter::ShellDialect::from_name(shell) {
        shuck_linter::ShellDialect::Sh
        | shuck_linter::ShellDialect::Dash
        | shuck_linter::ShellDialect::Ksh => shuck_parser::ShellDialect::Posix,
        shuck_linter::ShellDialect::Mksh => shuck_parser::ShellDialect::Mksh,
        shuck_linter::ShellDialect::Zsh => shuck_parser::ShellDialect::Zsh,
        shuck_linter::ShellDialect::Unknown | shuck_linter::ShellDialect::Bash => {
            shuck_parser::ShellDialect::Bash
        }
    }
}

fn run_shuck_with_parse_dialect(
    fixture: &LargeCorpusFixture,
    linter_settings: &shuck_linter::LinterSettings,
    source_path_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)>,
    parse_dialect: shuck_parser::ShellDialect,
    shell: &str,
) -> ShuckRun {
    let source = match fs::read_to_string(&fixture.path) {
        Ok(s) => s,
        Err(e) => {
            return ShuckRun {
                diagnostics: Vec::new(),
                parse_error: Some(format!("read error: {e}")),
            };
        }
    };

    let linter_settings = linter_settings
        .clone()
        .with_shell(shuck_linter::ShellDialect::from_name(shell))
        .with_analyzed_paths([fixture.path.clone()]);
    let parsed = match shuck_parser::parser::Parser::with_dialect(&source, parse_dialect).parse() {
        Ok(parsed) => parsed,
        Err(error) => {
            let recovered = shuck_parser::parser::Parser::with_dialect(&source, parse_dialect)
                .parse_recovered();
            let output = shuck_parser::parser::ParseOutput {
                file: recovered.file,
            };
            let diagnostics = lint_large_corpus_output(
                fixture,
                &source,
                &output,
                &recovered.diagnostics,
                &linter_settings,
                source_path_resolver,
            );
            let handled_parse_diagnostic = linter_settings
                .rules
                .contains(shuck_linter::Rule::MissingFi)
                && parse_diagnostics_include_missing_fi(&recovered.diagnostics);

            if !diagnostics.is_empty() || handled_parse_diagnostic {
                return ShuckRun {
                    diagnostics,
                    parse_error: None,
                };
            }

            return ShuckRun {
                diagnostics: Vec::new(),
                parse_error: Some(error.to_string()),
            };
        }
    };
    let diagnostics = lint_large_corpus_output(
        fixture,
        &source,
        &parsed,
        &[],
        &linter_settings,
        source_path_resolver,
    );

    ShuckRun {
        diagnostics,
        parse_error: None,
    }
}

fn lint_large_corpus_output(
    fixture: &LargeCorpusFixture,
    source: &str,
    output: &shuck_parser::parser::ParseOutput,
    parse_diagnostics: &[shuck_parser::parser::ParseDiagnostic],
    linter_settings: &shuck_linter::LinterSettings,
    source_path_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)>,
) -> Vec<shuck_linter::Diagnostic> {
    let indexer = shuck_indexer::Indexer::new(source, output);
    let shellcheck_map = shuck_linter::ShellCheckCodeMap::default();
    let directives =
        shuck_linter::parse_directives(source, indexer.comment_index(), &shellcheck_map);
    let suppression_index = (!directives.is_empty()).then(|| {
        shuck_linter::SuppressionIndex::new(
            &directives,
            &output.file,
            shuck_linter::first_statement_line(&output.file).unwrap_or(u32::MAX),
        )
    });

    shuck_linter::lint_file_at_path_with_resolver_and_parse_diagnostics(
        &output.file,
        source,
        &indexer,
        linter_settings,
        suppression_index.as_ref(),
        Some(&fixture.path),
        source_path_resolver,
        parse_diagnostics,
    )
}

fn parse_diagnostics_include_missing_fi(
    parse_diagnostics: &[shuck_parser::parser::ParseDiagnostic],
) -> bool {
    parse_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.starts_with("expected 'fi'"))
}

fn run_shuck(
    fixture: &LargeCorpusFixture,
    linter_settings: &shuck_linter::LinterSettings,
    source_path_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)>,
) -> ShuckRun {
    run_shuck_with_parse_dialect(
        fixture,
        linter_settings,
        source_path_resolver,
        shuck_parser::ShellDialect::Bash,
        &fixture.shell,
    )
}

fn run_shuck_with_timeout(
    fixture: &LargeCorpusFixture,
    linter_settings: &shuck_linter::LinterSettings,
    timeout: Duration,
    source_path_resolver: Arc<LargeCorpusPathResolver>,
) -> Result<ShuckRun, String> {
    let fixture = fixture.clone();
    let linter_settings = linter_settings.clone();
    let source_path_resolver = Arc::clone(&source_path_resolver);
    run_with_timeout("shuck", timeout, move || {
        run_shuck(
            &fixture,
            &linter_settings,
            Some(source_path_resolver.as_ref()
                as &(dyn shuck_semantic::SourcePathResolver + Send + Sync)),
        )
    })
}

fn parse_fixture_for_effective_large_corpus_shell_with_timeout(
    fixture: &LargeCorpusFixture,
    timeout: Duration,
) -> Result<Result<(), String>, String> {
    let fixture = fixture.clone();
    run_with_timeout("shuck", timeout, move || {
        let source =
            fs::read_to_string(&fixture.path).map_err(|err| format!("read error: {err}"))?;
        let shell = effective_large_corpus_shell(&fixture);
        let parse_dialect = parser_dialect_for_large_corpus_shell(shell);
        shuck_parser::parser::Parser::with_dialect(&source, parse_dialect)
            .parse()
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

fn build_rule_to_shellcheck_index() -> HashMap<String, String> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .map(|(sc_code, rule)| (rule.code().to_owned(), format!("SC{sc_code}")))
        .collect()
}

fn build_shellcheck_to_rule_index() -> HashMap<u32, String> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .map(|(sc_code, rule)| (sc_code, rule.code().to_owned()))
        .collect()
}

fn build_shellcheck_index() -> HashMap<String, String> {
    build_rule_to_shellcheck_index()
}

fn build_large_corpus_linter_settings(
    selected_rules: Option<shuck_linter::RuleSet>,
    mapped_only: bool,
) -> shuck_linter::LinterSettings {
    if let Some(rules) = selected_rules {
        return shuck_linter::LinterSettings::for_rules(rules.iter());
    }
    if mapped_only {
        let map = shuck_linter::ShellCheckCodeMap::default();
        let mapped_rules: Vec<_> = map.mappings().map(|(_, rule)| rule).collect();
        return shuck_linter::LinterSettings::for_rules(mapped_rules);
    }
    shuck_linter::LinterSettings::default()
}

fn build_shellcheck_filter_codes(
    selected_rules: Option<shuck_linter::RuleSet>,
    mapped_only: bool,
) -> Option<HashSet<u32>> {
    selected_rules
        .map(|rules| build_selected_shellcheck_codes(&rules))
        .or_else(|| mapped_only.then(build_mapped_shellcheck_codes))
}

fn build_mapped_shellcheck_codes() -> HashSet<u32> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .map(|(sc_code, _)| sc_code)
        .collect()
}

fn build_selected_shellcheck_codes(selected_rules: &shuck_linter::RuleSet) -> HashSet<u32> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .filter_map(|(sc_code, rule)| selected_rules.contains(rule).then_some(sc_code))
        .collect()
}

// ---------------------------------------------------------------------------
// ShellCheck runner
// ---------------------------------------------------------------------------

fn probe_shellcheck() -> Option<ShellCheckProbe> {
    let output = Command::new("shellcheck").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let version_text = normalize_shellcheck_version_text(&output.stdout);
    if version_text.is_empty() {
        return None;
    }

    Some(ShellCheckProbe {
        command: "shellcheck".into(),
        version_text,
    })
}

fn run_shellcheck(
    path: &Path,
    shell: &str,
    shellcheck_path: &str,
    timeout: Duration,
) -> Result<ShellCheckRun, String> {
    let mut child = Command::new(shellcheck_path)
        .args(["--norc", "-s", shell, "-f", "json1"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("shellcheck exec: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "shellcheck exec: failed to capture stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "shellcheck exec: failed to capture stderr".to_owned())?;
    let stdout_reader = thread::spawn(move || read_shellcheck_pipe(stdout, "stdout"));
    let stderr_reader = thread::spawn(move || read_shellcheck_pipe(stderr, "stderr"));
    let status = match child
        .wait_timeout(timeout)
        .map_err(|e| format!("shellcheck wait: {e}"))?
    {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(format_timeout_message("shellcheck", timeout));
        }
    };
    let stdout = join_shellcheck_pipe(stdout_reader, "stdout")?;
    let stderr = join_shellcheck_pipe(stderr_reader, "stderr")?;

    // shellcheck exits 1 when it finds issues, which is normal
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        if code != 1 {
            let stderr = String::from_utf8_lossy(&stderr);
            return Err(format!("shellcheck exit {code}: {stderr}"));
        }
    }

    let stdout = String::from_utf8_lossy(&stdout);
    if stdout.trim().is_empty() {
        return Ok(ShellCheckRun {
            diagnostics: Vec::new(),
            parse_aborted: false,
        });
    }

    let diagnostics = decode_shellcheck_diagnostics(stdout.as_bytes())?;
    let parse_aborted = shellcheck_parse_aborted(&diagnostics);
    Ok(ShellCheckRun {
        diagnostics,
        parse_aborted,
    })
}

fn read_shellcheck_pipe<R: Read>(mut pipe: R, label: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    pipe.read_to_end(&mut data)
        .map_err(|err| format!("shellcheck {label}: {err}"))?;
    Ok(data)
}

fn join_shellcheck_pipe(
    reader: thread::JoinHandle<Result<Vec<u8>, String>>,
    label: &str,
) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| format!("shellcheck {label} reader panicked"))?
}

fn decode_shellcheck_diagnostics(data: &[u8]) -> Result<Vec<ShellCheckDiagnostic>, String> {
    let data = data.to_vec();
    let trimmed = String::from_utf8_lossy(&data);
    let trimmed = trimmed.trim();

    if trimmed.is_empty() {
        return Err("empty shellcheck json output".into());
    }

    if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<ShellCheckDiagnostic>>(trimmed)
            .map_err(|e| format!("decode shellcheck json array: {e}"))
    } else if trimmed.starts_with('{') {
        #[derive(Deserialize)]
        struct Wrapper {
            comments: Vec<ShellCheckDiagnostic>,
        }
        let wrapper: Wrapper = serde_json::from_str(trimmed)
            .map_err(|e| format!("decode shellcheck json object: {e}"))?;
        Ok(wrapper.comments)
    } else {
        Err(format!(
            "decode shellcheck json: unexpected leading byte {:?}",
            trimmed.chars().next()
        ))
    }
}

fn shellcheck_parse_aborted(diags: &[ShellCheckDiagnostic]) -> bool {
    for diag in diags {
        if diag.level != "error" {
            continue;
        }
        if diag.code == 1072 || diag.code == 1073 {
            return true;
        }
        let lower = diag.message.to_lowercase();
        if lower.contains("fix to allow more checks")
            || lower.contains("fix any mentioned problems and try again")
        {
            return true;
        }
    }
    false
}

fn shellcheck_supported_shells(shellcheck_path: &str) -> HashMap<&'static str, ()> {
    let output = Command::new(shellcheck_path)
        .arg("--help")
        .output()
        .expect("failed to run shellcheck --help");

    let help = String::from_utf8_lossy(&output.stdout);
    let mut supported = parse_shellcheck_supported_shells(&help);

    // Always include common shells even if parsing fails
    if supported.is_empty() {
        for shell in &["sh", "bash", "dash", "ksh"] {
            supported.insert(shell, ());
        }
    }

    supported
}

fn parse_shellcheck_supported_shells(help: &str) -> HashMap<&'static str, ()> {
    let mut supported = HashMap::new();
    for line in help.lines() {
        if !line.contains("--shell=") {
            continue;
        }
        if let Some(start) = line.find('(')
            && let Some(end) = line[start + 1..].find(')')
        {
            let shells = &line[start + 1..start + 1 + end];
            for shell in shells.split(',') {
                let shell = shell.trim();
                match shell {
                    "sh" => {
                        supported.insert("sh", ());
                    }
                    "bash" => {
                        supported.insert("bash", ());
                    }
                    "dash" => {
                        supported.insert("dash", ());
                    }
                    "ksh" => {
                        supported.insert("ksh", ());
                    }
                    "zsh" => {
                        supported.insert("zsh", ());
                    }
                    "busybox" => {
                        supported.insert("busybox", ());
                    }
                    _ => {}
                }
            }
        }
    }
    supported
}

fn normalize_shellcheck_version_text(output: &[u8]) -> String {
    let normalized = String::from_utf8_lossy(output).replace("\r\n", "\n");
    normalized
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn legacy_shellcheck_invocation_hash(shellcheck_path: &str) -> String {
    let meta = fs::metadata(shellcheck_path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let key = format!("{}:{}:shellcheck", shellcheck_path, size);
    hash_bytes(key.as_bytes())
}

// ---------------------------------------------------------------------------
// Compatibility comparison
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffMode {
    All,
    Overreports,
}

fn count_codes(codes: &[String]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for code in codes {
        *counts.entry(code.clone()).or_insert(0) += 1;
    }
    counts
}

fn compatibility_code_diff(
    want: &HashMap<String, usize>,
    got: &HashMap<String, usize>,
    mode: DiffMode,
) -> Option<String> {
    let mut keys: Vec<&String> = want.keys().chain(got.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut lines = Vec::new();
    for key in &keys {
        let w = want.get(*key).copied().unwrap_or(0);
        let g = got.get(*key).copied().unwrap_or(0);
        if w == g {
            continue;
        }
        if mode == DiffMode::Overreports && g <= w {
            continue;
        }
        lines.push(format!("{key}: shellcheck={w} shuck={g}"));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn source_starts_with_unknown_shell_comment(src: &[u8]) -> bool {
    if src.is_empty() {
        return false;
    }
    let text = String::from_utf8_lossy(src);
    let mut saw_plain_comment = false;
    for line in text.lines() {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text.starts_with("#!") || text.starts_with("# !") {
            break;
        }
        if let Some(body) = text.strip_prefix('#') {
            let lower = body.trim().to_lowercase();
            if lower.starts_with("shellcheck shell=") || lower.starts_with("shuck:") {
                break;
            }
            saw_plain_comment = true;
        } else {
            break;
        }
    }
    saw_plain_comment
}

fn format_range(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> String {
    let el = if end_line == 0 { start_line } else { end_line };
    let ec = if end_col == 0 { start_col } else { end_col };
    format!("{start_line}:{start_col}-{el}:{ec}")
}

fn indent_detail(detail: &str) -> String {
    detail
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

fn env_truthy(key: &str, default: bool) -> bool {
    match env::var(key).ok().as_deref() {
        None | Some("") => default,
        Some(v) => matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
    }
}

fn positive_env_int(key: &str, default: usize) -> usize {
    match env::var(key).ok().filter(|s| !s.is_empty()) {
        None => default,
        Some(v) => {
            let parsed: usize = v
                .parse()
                .unwrap_or_else(|_| panic!("{key}={v:?}, want positive integer"));
            assert!(parsed > 0, "{key}={v:?}, want positive integer");
            parsed
        }
    }
}

fn parse_large_corpus_rule_filter_env(key: &str) -> Option<shuck_linter::RuleSet> {
    let value = env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(
        parse_large_corpus_rule_set(trimmed)
            .unwrap_or_else(|err| panic!("{key}={trimmed:?}, {err}")),
    )
}

fn parse_large_corpus_rule_set(value: &str) -> Result<shuck_linter::RuleSet, String> {
    let selectors: Vec<shuck_linter::RuleSelector> = value
        .split(',')
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
        .map(|selector| {
            selector
                .parse::<shuck_linter::RuleSelector>()
                .map_err(|err| err.to_string())
        })
        .collect::<Result<_, _>>()?;

    if selectors.is_empty() {
        return Err("want at least one comma-separated rule selector".into());
    }

    Ok(shuck_linter::LinterSettings::from_selectors(&selectors, &[]).rules)
}

fn percentage_env_int(key: &str, default: usize) -> usize {
    match env::var(key).ok().filter(|s| !s.is_empty()) {
        None => default,
        Some(v) => {
            let parsed: usize = v
                .parse()
                .unwrap_or_else(|_| panic!("{key}={v:?}, want integer percentage in [1,100]"));
            assert!(
                (1..=100).contains(&parsed),
                "{key}={v:?}, want integer percentage in [1,100]"
            );
            parsed
        }
    }
}

fn non_negative_env_int(key: &str, default: usize) -> usize {
    match env::var(key).ok().filter(|s| !s.is_empty()) {
        None => default,
        Some(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("{key}={v:?}, want non-negative integer")),
    }
}

// ---------------------------------------------------------------------------
// Unit tests for helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_shell_bash_shebang() {
        assert_eq!(
            resolve_shell(Path::new("script"), b"#!/usr/bin/env bash\n"),
            "bash"
        );
    }

    #[test]
    fn resolve_shell_dash_maps_to_sh() {
        assert_eq!(resolve_shell(Path::new("script"), b"#!/bin/dash\n"), "sh");
    }

    #[test]
    fn resolve_shell_zsh_shebang() {
        assert_eq!(resolve_shell(Path::new("script"), b"#!/bin/zsh\n"), "zsh");
    }

    #[test]
    fn resolve_shell_bash_extension_fallback() {
        assert_eq!(
            resolve_shell(Path::new("example.bash"), b"echo hi\n"),
            "bash"
        );
    }

    #[test]
    fn resolve_shell_sh_extension_fallback() {
        assert_eq!(resolve_shell(Path::new("example.sh"), b"echo hi\n"), "sh");
    }

    #[test]
    fn resolve_shell_default_sh() {
        assert_eq!(resolve_shell(Path::new("example"), b"echo hi\n"), "sh");
    }

    #[test]
    fn fixture_progress_label_uses_cache_relative_path() {
        let fixture = fixture_at(
            Path::new("/tmp/worktree-a/.cache/large-corpus/scripts/nested/example.sh"),
            Path::new("nested/example.sh"),
        );

        assert_eq!(fixture_progress_label(&fixture), "nested/example.sh");
    }

    #[test]
    fn progress_bucket_waits_until_a_full_five_percent_is_complete() {
        assert_eq!(progress_bucket(21, 1), 0);
        assert_eq!(progress_bucket(21, 2), 1);
    }

    #[test]
    fn progress_bucket_can_skip_ahead_for_small_fixture_sets() {
        assert_eq!(progress_bucket(3, 1), 6);
        assert_eq!(progress_bucket(3, 2), 13);
        assert_eq!(progress_bucket(3, 3), 20);
    }

    #[test]
    fn progress_bucket_clamps_to_one_hundred_percent() {
        assert_eq!(progress_bucket(20, 25), 20);
    }

    #[test]
    fn format_fixture_elapsed_uses_millis_for_short_runs() {
        assert_eq!(format_fixture_elapsed(Duration::from_millis(842)), "842ms");
    }

    #[test]
    fn format_fixture_elapsed_uses_seconds_for_longer_runs() {
        assert_eq!(
            format_fixture_elapsed(Duration::from_millis(1_234)),
            "1.234s"
        );
    }

    #[test]
    fn run_with_timeout_returns_completed_result() {
        let result = run_with_timeout("test worker", Duration::from_millis(50), || 42).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn run_with_timeout_reports_timeout() {
        let err = run_with_timeout("test worker", Duration::from_millis(10), || {
            thread::sleep(Duration::from_secs(5));
            42
        })
        .unwrap_err();

        assert_eq!(err, "test worker timed out after 10ms");
    }

    #[cfg(unix)]
    #[test]
    fn run_shellcheck_reports_timeout() {
        use std::os::unix::fs::PermissionsExt;

        let tempdir = tempfile::tempdir().unwrap();
        let fixture_path = tempdir.path().join("fixture.sh");
        let shellcheck_path = tempdir.path().join("fake-shellcheck");

        fs::write(&fixture_path, "echo hi\n").unwrap();
        fs::write(&shellcheck_path, "#!/bin/sh\nsleep 1\nprintf '[]'\n").unwrap();

        let mut permissions = fs::metadata(&shellcheck_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shellcheck_path, permissions).unwrap();

        let err = run_shellcheck(
            &fixture_path,
            "sh",
            shellcheck_path.to_str().unwrap(),
            Duration::from_millis(10),
        )
        .unwrap_err();

        assert_eq!(err, "shellcheck timed out after 10ms");
    }

    #[test]
    fn shard_fixtures_contiguous_split() {
        let fixtures: Vec<LargeCorpusFixture> = (0..100)
            .map(|i| LargeCorpusFixture {
                path: PathBuf::from(format!("script-{i:03}.sh")),
                cache_rel_path: PathBuf::from(format!("script-{i:03}.sh")),
                shell: "sh".into(),
                source_hash: String::new(),
            })
            .collect();

        let shard0 = shard_fixtures(fixtures.clone(), 0, 4);
        assert_eq!(shard0.len(), 25);
        assert_eq!(shard0[0].path, PathBuf::from("script-000.sh"));

        let shard1 = shard_fixtures(fixtures.clone(), 1, 4);
        assert_eq!(shard1.len(), 25);
        assert_eq!(shard1[0].path, PathBuf::from("script-025.sh"));

        let shard3 = shard_fixtures(fixtures, 3, 4);
        assert_eq!(shard3.len(), 25);
        assert_eq!(shard3[0].path, PathBuf::from("script-075.sh"));
    }

    #[test]
    fn sample_fixtures_full_percentage_keeps_everything() {
        let fixtures: Vec<LargeCorpusFixture> = (0..10)
            .map(|i| fixture(&format!("script-{i:03}.sh")))
            .collect();

        let sampled = sample_fixtures(fixtures.clone(), 100);

        assert_eq!(sampled.len(), fixtures.len());
        assert_eq!(
            sampled
                .iter()
                .map(|fixture| &fixture.path)
                .collect::<Vec<_>>(),
            fixtures
                .iter()
                .map(|fixture| &fixture.path)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn sample_fixtures_uses_cache_relative_path_for_stability() {
        let left = fixture_at(
            Path::new("/tmp/worktree-a/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let right = fixture_at(
            Path::new("/tmp/worktree-b/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );

        assert_eq!(
            fixture_selected_for_sample(&left, 17),
            fixture_selected_for_sample(&right, 17)
        );
    }

    #[test]
    fn sample_fixtures_membership_is_order_independent() {
        let fixtures: Vec<LargeCorpusFixture> = (0..200)
            .map(|i| fixture(&format!("script-{i:03}.sh")))
            .collect();
        let mut reversed = fixtures.clone();
        reversed.reverse();

        let mut forward = sample_fixtures(fixtures, 10)
            .into_iter()
            .map(|fixture| fixture.cache_rel_path)
            .collect::<Vec<_>>();
        let mut backward = sample_fixtures(reversed, 10)
            .into_iter()
            .map(|fixture| fixture.cache_rel_path)
            .collect::<Vec<_>>();

        forward.sort();
        backward.sort();

        assert_eq!(forward, backward);
    }

    #[test]
    fn parse_shellcheck_supported_shells_parses_help() {
        let help = "Usage: shellcheck [OPTIONS...] FILES...\n  \
                     -s SHELLNAME        --shell=SHELLNAME          \
                     Specify dialect (sh, bash, dash, ksh, busybox)\n";
        let supported = parse_shellcheck_supported_shells(help);
        for shell in &["sh", "bash", "dash", "ksh", "busybox"] {
            assert!(
                supported.contains_key(shell),
                "expected {shell} in supported shells"
            );
        }
        assert!(
            !supported.contains_key("zsh"),
            "zsh should not be in supported shells"
        );
    }

    #[test]
    fn parser_dialect_for_large_corpus_zsh_shell_is_zsh() {
        assert_eq!(
            parser_dialect_for_large_corpus_shell("zsh"),
            shuck_parser::ShellDialect::Zsh
        );
    }

    #[test]
    fn zsh_is_not_supported_for_large_corpus_even_if_shellcheck_supports_it() {
        let supported = HashMap::from([("sh", ()), ("bash", ()), ("zsh", ())]);

        assert!(!shell_supported_for_large_corpus("zsh", Some(&supported)));
        assert!(shell_supported_for_large_corpus("sh", Some(&supported)));
        assert!(shell_supported_for_large_corpus("bash", Some(&supported)));
    }

    #[test]
    fn zsh_paths_are_skipped_even_when_resolved_shell_is_sh() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("example.zsh"),
            cache_rel_path: PathBuf::from("example.zsh"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(fixture_looks_like_zsh(&fixture));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
    }

    #[test]
    fn zsh_shebangs_are_selected_for_large_corpus_zsh_parse() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("bin/plugin"),
            cache_rel_path: PathBuf::from("bin/plugin"),
            shell: "zsh".into(),
            source_hash: String::new(),
        };

        assert_eq!(effective_large_corpus_shell(&fixture), "zsh");
        assert!(fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn zsh_paths_force_effective_zsh_shell() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from(".zshrc"),
            cache_rel_path: PathBuf::from(".zshrc"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert_eq!(effective_large_corpus_shell(&fixture), "zsh");
        assert!(fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn flattened_repo_git_entries_are_skipped() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("repo__.git__hooks__pre-commit.sample"),
            cache_rel_path: PathBuf::from("repo__.git__hooks__pre-commit.sample"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(fixture_is_repo_git_entry(&fixture));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
    }

    #[test]
    fn sample_files_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("hooks/pre-commit.sample"),
            cache_rel_path: PathBuf::from("hooks/pre-commit.sample"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_sample_file(&fixture.path));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
    }

    #[test]
    fn fish_files_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("functions/prompt.fish"),
            cache_rel_path: PathBuf::from("functions/prompt.fish"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_fish_file(&fixture.path));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
    }

    #[test]
    fn shellcheck_parse_abort_classification() {
        let aborted = vec![
            ShellCheckDiagnostic {
                file: String::new(),
                code: 1073,
                line: 1,
                end_line: 1,
                column: 1,
                end_column: 1,
                level: "error".into(),
                message: "Couldn't parse this test expression.".into(),
            },
            ShellCheckDiagnostic {
                file: String::new(),
                code: 1072,
                line: 2,
                end_line: 2,
                column: 1,
                end_column: 1,
                level: "error".into(),
                message: "Expected comparison operator. Fix any mentioned problems and try again."
                    .into(),
            },
        ];
        assert!(shellcheck_parse_aborted(&aborted));

        let non_aborted = vec![ShellCheckDiagnostic {
            file: String::new(),
            code: 3011,
            line: 1,
            end_line: 1,
            column: 1,
            end_column: 1,
            level: "warning".into(),
            message: "In POSIX sh, here-strings are undefined.".into(),
        }];
        assert!(!shellcheck_parse_aborted(&non_aborted));
    }

    #[test]
    fn compatibility_diff_detects_over_and_underreporting() {
        let want = HashMap::from([("SC2086".to_string(), 1)]);
        let got_over = HashMap::from([("SC2086".to_string(), 2)]);
        let got_under = HashMap::from([("SC2086".to_string(), 0)]);

        assert!(compatibility_code_diff(&want, &got_over, DiffMode::All).is_some());
        assert!(compatibility_code_diff(&want, &got_under, DiffMode::All).is_some());
    }

    #[test]
    fn count_codes_aggregates() {
        let codes = vec!["SC2086".into(), "SC2086".into(), "SC2154".into()];
        let counts = count_codes(&codes);
        assert_eq!(counts["SC2086"], 2);
        assert_eq!(counts["SC2154"], 1);
    }

    #[test]
    fn build_shellcheck_index_uses_linter_mappings() {
        let index = build_shellcheck_index();

        // Spot-check known mappings.
        assert_eq!(index.get("C001").map(String::as_str), Some("SC2034"));
        assert_eq!(index.get("S001").map(String::as_str), Some("SC2086"));
        assert_eq!(index.get("C006").map(String::as_str), Some("SC2154"));
        assert_eq!(index.get("C124").map(String::as_str), Some("SC2365"));
    }

    #[test]
    fn parse_large_corpus_rule_set_accepts_csv_and_prefix_selectors() {
        let rules = parse_large_corpus_rule_set(" C001, S001 , C02 ").unwrap();

        assert!(rules.contains(shuck_linter::Rule::UnusedAssignment));
        assert!(rules.contains(shuck_linter::Rule::UnquotedExpansion));
        assert!(rules.contains(shuck_linter::Rule::TruthyLiteralTest));
        assert!(rules.contains(shuck_linter::Rule::ConstantCaseSubject));
        assert!(rules.contains(shuck_linter::Rule::EmptyTest));
        assert!(!rules.contains(shuck_linter::Rule::UndefinedVariable));
    }

    #[test]
    fn parse_large_corpus_rule_set_rejects_unknown_selectors() {
        let err = parse_large_corpus_rule_set("C001,NOPE").unwrap_err();

        assert_eq!(err, "unknown rule selector `NOPE`");
    }

    #[test]
    fn selected_rule_filter_builds_matching_shellcheck_codes() {
        let rules = parse_large_corpus_rule_set("C001,S001").unwrap();
        let codes = build_selected_shellcheck_codes(&rules);

        assert_eq!(codes, HashSet::from([2034, 2086]));
    }

    #[test]
    fn selected_rule_filter_limits_shellcheck_codes_even_when_mapped_only_is_enabled() {
        let rules = parse_large_corpus_rule_set("C001").unwrap();
        let codes = build_shellcheck_filter_codes(Some(rules), true).unwrap();

        assert_eq!(codes, HashSet::from([2034]));
    }

    #[test]
    fn selected_rule_filter_builds_matching_linter_settings() {
        let rules = parse_large_corpus_rule_set("S001").unwrap();
        let settings = build_large_corpus_linter_settings(Some(rules), false);

        assert!(
            settings
                .rules
                .contains(shuck_linter::Rule::UnquotedExpansion)
        );
        assert!(
            !settings
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
    }

    #[test]
    fn mapped_only_enables_all_mapped_rules_in_linter_settings() {
        let settings = build_large_corpus_linter_settings(None, true);
        // Style rules like S003 (LoopFromCommandOutput) should be included
        assert!(
            settings
                .rules
                .contains(shuck_linter::Rule::LoopFromCommandOutput)
        );
        // All mapped rules should be present
        let map = shuck_linter::ShellCheckCodeMap::default();
        for (_, rule) in map.mappings() {
            assert!(
                settings.rules.contains(rule),
                "mapped rule {:?} missing from mapped_only linter settings",
                rule
            );
        }
    }

    #[test]
    fn mapped_only_filter_drops_unmapped_shellcheck_diagnostics() {
        let mapped_shellcheck_codes = build_mapped_shellcheck_codes();
        let run = ShellCheckRun {
            diagnostics: vec![
                shellcheck_diagnostic(2034),
                shellcheck_diagnostic(2086),
                shellcheck_diagnostic(9999),
            ],
            parse_aborted: true,
        };

        let filtered = filter_shellcheck_run(run, Some(&mapped_shellcheck_codes));
        let codes = filtered
            .diagnostics
            .iter()
            .map(|diag| diag.code)
            .collect::<Vec<_>>();

        assert_eq!(codes, vec![2034, 2086]);
        assert!(!filtered.parse_aborted);
    }

    #[test]
    fn run_shuck_respects_shellcheck_disable_directives() {
        let tempdir = tempfile::tempdir().unwrap();
        let fixture_path = tempdir.path().join("fixture.sh");
        let source = "\
# shellcheck shell=bash disable=SC2155
demo() {
  local value=$(date)
}
";
        fs::write(&fixture_path, source).unwrap();

        let fixture = LargeCorpusFixture {
            path: fixture_path.clone(),
            cache_rel_path: PathBuf::from("fixture.sh"),
            shell: "bash".into(),
            source_hash: hash_bytes(source.as_bytes()),
        };
        let run = run_shuck(
            &fixture,
            &shuck_linter::LinterSettings::for_rule(shuck_linter::Rule::ExportCommandSubstitution),
            None,
        );

        assert!(run.parse_error.is_none());
        assert!(run.diagnostics.is_empty());
    }

    #[test]
    fn run_shuck_reports_missing_fi_as_c035() {
        let tempdir = tempfile::tempdir().unwrap();
        let fixture_path = tempdir.path().join("fixture.sh");
        let source = "#!/bin/sh\nif true; then\n  :\n";
        fs::write(&fixture_path, source).unwrap();

        let fixture = LargeCorpusFixture {
            path: fixture_path.clone(),
            cache_rel_path: PathBuf::from("fixture.sh"),
            shell: "sh".into(),
            source_hash: hash_bytes(source.as_bytes()),
        };
        let run = run_shuck(
            &fixture,
            &shuck_linter::LinterSettings::for_rule(shuck_linter::Rule::MissingFi),
            None,
        );

        assert!(run.parse_error.is_none());
        assert_eq!(run.diagnostics.len(), 1);
        assert_eq!(run.diagnostics[0].code(), "C035");
        assert_eq!(run.diagnostics[0].span.start.line, 4);
        assert_eq!(run.diagnostics[0].span.start.column, 1);
    }

    #[test]
    fn rule_corpus_metadata_path_uses_rule_code_convention() {
        assert_eq!(
            rule_corpus_metadata_path("C001"),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/testdata/corpus-metadata")
                .join("c001.yaml")
        );
    }

    #[test]
    fn c001_metadata_loads_documented_reviewed_divergences() {
        let metadata = load_rule_corpus_metadata("C001");

        assert!(metadata.reviewed_divergences.iter().any(|entry| {
            entry.side == CompatibilitySide::ShellcheckOnly
                && entry.path_suffix.as_deref()
                    == Some("SlackBuildsOrg__slackbuilds__libraries__bluez-alsa__bluez-alsa.conf")
                && entry.line == Some(19)
                && entry.end_line == Some(19)
                && entry.column == Some(1)
                && entry.end_column == Some(14)
                && entry.reason
                    == "config value is consumed by rc.bluez-alsa after sourcing the file"
        }));
        assert!(metadata.reviewed_divergences.iter().any(|entry| {
            entry.path_suffix.as_deref() == Some("233boy__v2ray__install.sh")
                && entry.line == Some(424)
                && entry.end_line == Some(424)
                && entry.column == Some(5)
                && entry.end_column == Some(19)
                && entry.reason
                    == "install mode flag is consumed by the dynamically sourced core.sh helper"
        }));
        assert!(metadata.reviewed_divergences.iter().any(|entry| {
            entry.path_suffix.as_deref()
                == Some("GameServerManagers__LinuxGSM__lgsm__modules__command_details.sh")
                && entry.line == Some(19)
                && entry.end_line == Some(19)
                && entry.column == Some(2)
                && entry.end_column == Some(5)
                && entry.reason
                    == "loop variable is consumed by the sibling query_gamedig.sh helper invoked in the loop"
        }));
    }

    #[test]
    fn reviewed_divergence_classification_matches_exact_shellcheck_record() {
        let metadata = HashMap::from([("C001".to_string(), load_rule_corpus_metadata("C001"))]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShellcheckOnly,
            rule_code: Some("C001".into()),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 19,
                end_line: 19,
                column: 1,
                end_column: 14,
            },
            message: "warning reviewed divergence".into(),
            labels: Vec::new(),
        };

        let (classification, reason) = classify_compatibility_record(
            &record,
            Path::new("SlackBuildsOrg__slackbuilds__libraries__bluez-alsa__bluez-alsa.conf"),
            &metadata,
        );

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(
            reason.as_deref(),
            Some("config value is consumed by rc.bluez-alsa after sourcing the file")
        );
    }

    #[test]
    fn reviewed_divergence_classification_matches_label_qualified_record() {
        let metadata = HashMap::from([("C002".to_string(), load_rule_corpus_metadata("C002"))]);
        let matching = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C002".into()),
            shellcheck_code: "SC1090".into(),
            range: DiagnosticRange {
                line: 20,
                end_line: 20,
                column: 1,
                end_column: 10,
            },
            message: "dynamic source path".into(),
            labels: vec!["project-closure".into()],
        };
        let non_matching = CompatibilityRecord {
            labels: Vec::new(),
            ..matching.clone()
        };

        let (matching_classification, _) =
            classify_compatibility_record(&matching, Path::new("repo__script.sh"), &metadata);
        let (non_matching_classification, _) =
            classify_compatibility_record(&non_matching, Path::new("repo__script.sh"), &metadata);

        assert_eq!(
            matching_classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(
            non_matching_classification,
            CompatibilityClassification::Implementation
        );
    }

    #[test]
    fn comparison_target_note_classification_stays_blocking() {
        let metadata = HashMap::from([("C046".to_string(), load_rule_corpus_metadata("C046"))]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShellcheckOnly,
            rule_code: Some("C046".into()),
            shellcheck_code: "SC2124".into(),
            range: DiagnosticRange {
                line: 287,
                end_line: 287,
                column: 1,
                end_column: 10,
            },
            message: "warning array-to-scalar assignment".into(),
            labels: Vec::new(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, Path::new("wifi.sh"), &metadata);

        assert_eq!(classification, CompatibilityClassification::MappingIssue);
        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("SC2124"))
        );
    }

    #[test]
    fn load_all_rule_corpus_metadata_reads_seeded_documents() {
        let metadata = load_all_rule_corpus_metadata();

        assert!(metadata.contains_key("C001"));
        assert!(metadata.contains_key("C002"));
        assert!(metadata.contains_key("C019"));
        assert!(metadata.contains_key("C046"));
        assert!(metadata.contains_key("C048"));
        assert!(metadata.contains_key("C050"));
        assert!(metadata.contains_key("C055"));
    }

    #[test]
    fn keep_going_collects_multiple_fixture_failures() {
        let first = fixture("first.sh");
        let second = fixture("second.sh");
        let fixtures = vec![&first, &second];
        let seen = Mutex::new(Vec::new());

        let failures = collect_fixture_failures(&fixtures, true, |fixture| {
            seen.lock().unwrap().push(fixture.path.clone());
            FixtureEvaluation {
                implementation_diffs: vec![format_fixture_failure(
                    &fixture.path,
                    &[format!("{} failed", fixture.path.display())],
                )],
                ..FixtureEvaluation::default()
            }
        });

        let mut seen = seen.into_inner().unwrap();
        seen.sort();

        assert_eq!(
            seen,
            vec![PathBuf::from("first.sh"), PathBuf::from("second.sh")]
        );
        assert!(!failures.timeout_cap_reached);
        assert_eq!(failures.implementation_diffs.len(), 2);
        assert!(failures.implementation_diffs[0].contains("first.sh"));
        assert!(failures.implementation_diffs[1].contains("second.sh"));
    }

    #[test]
    fn keep_going_captures_fixture_panics() {
        let fixture = fixture("panic.sh");
        let fixtures = vec![&fixture];

        let failures = collect_fixture_failures(&fixtures, true, |_| -> FixtureEvaluation {
            panic!("boom");
        });

        assert!(!failures.timeout_cap_reached);
        assert_eq!(failures.harness_failures.len(), 1);
        assert!(failures.harness_failures[0].contains("panic.sh"));
        assert!(failures.harness_failures[0].contains("fixture panic: boom"));
    }

    #[test]
    fn keep_going_stops_after_five_timeouts() {
        let fixtures: Vec<_> = (0..10)
            .map(|i| fixture(&format!("timeout-{i}.sh")))
            .collect();
        let fixture_refs: Vec<_> = fixtures.iter().collect();
        let seen = AtomicUsize::new(0);

        let failures = collect_fixture_failures(&fixture_refs, true, |fixture| {
            seen.fetch_add(1, Ordering::Relaxed);
            FixtureEvaluation {
                harness_failure: Some(FixtureFailure {
                    kind: FixtureFailureKind::Timeout,
                    message: format_fixture_failure(
                        &fixture.path,
                        &[format!(
                            "shuck error: {}",
                            format_timeout_message("shuck", Duration::from_secs(30))
                        )],
                    ),
                }),
                ..FixtureEvaluation::default()
            }
        });

        assert!(failures.timeout_cap_reached);
        assert_eq!(
            failures.harness_failures.len(),
            LARGE_CORPUS_TIMEOUT_FAILURE_CAP
        );
        assert!(seen.load(Ordering::Relaxed) <= fixture_refs.len());
        assert!(
            failures
                .harness_failures
                .iter()
                .all(|failure| failure.contains("timed out after"))
        );
    }

    #[test]
    fn decode_shellcheck_json_array() {
        let data = br#"[{"code":1091,"line":12,"endLine":12,"column":3,"endColumn":15,"level":"info","message":"missing source"}]"#;
        let diags = decode_shellcheck_diagnostics(data).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, 1091);
        assert_eq!(diags[0].line, 12);
    }

    #[test]
    fn decode_shellcheck_json_object() {
        let data = br#"{"comments":[{"code":1091,"line":12,"endLine":12,"column":3,"endColumn":15,"level":"info","message":"missing source"}]}"#;
        let diags = decode_shellcheck_diagnostics(data).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, 1091);
    }

    #[test]
    fn compatibility_labels_include_shared_context_tags() {
        let labels = compatibility_context_labels(
            Path::new("ko1nksm__shellspec__spec__core__clone_spec.sh"),
            b"#!/bin/sh\n# shellcheck disable=SC2086\nDescribe 'clone'\nParameters\n  \"test\"\nEnd\nsource ./helper.sh\n",
        );

        assert!(labels.contains(&"directive-handling".to_owned()));
        assert!(labels.contains(&"project-closure".to_owned()));
        assert!(labels.contains(&"shellspec".to_owned()));
        assert!(labels.contains(&"test-harness".to_owned()));
    }

    #[test]
    fn compatibility_labels_include_generated_configure() {
        let labels = compatibility_context_labels(
            Path::new("examples/native/configure"),
            b"# Generated by GNU Autoconf 2.71\nas_lineno=${as_lineno-$LINENO}\n",
        );

        assert!(labels.contains(&"generated-configure".to_owned()));
    }

    #[test]
    fn unknown_shell_comment_detected() {
        assert!(source_starts_with_unknown_shell_comment(
            b"# leading comment\necho hi\n"
        ));
        assert!(!source_starts_with_unknown_shell_comment(
            b"#!/bin/sh\necho hi\n"
        ));
    }

    #[test]
    fn classify_fixture_noise_marks_patch_inputs() {
        let fixture = fixture("example.patch");
        assert_eq!(
            classify_fixture_noise(&fixture, b"not shell", true, false),
            CorpusNoiseKind::Patch
        );
    }

    #[test]
    fn classify_fixture_noise_marks_fish_inputs() {
        let fixture = fixture("generate-authors.fish");
        assert_eq!(
            classify_fixture_noise(&fixture, b"#!/usr/bin/env fish\necho hi\n", true, false),
            CorpusNoiseKind::Fish
        );
    }

    #[test]
    fn classify_fixture_noise_marks_parse_aborts() {
        let fixture = fixture("script.sh");
        assert_eq!(
            classify_fixture_noise(&fixture, b"#!/bin/sh\necho hi\n", true, false),
            CorpusNoiseKind::ParseAbort
        );
        assert_eq!(
            classify_fixture_noise(&fixture, b"#!/bin/sh\necho hi\n", false, true),
            CorpusNoiseKind::ParseAbort
        );
    }

    #[test]
    fn classify_fixture_noise_marks_shell_collapse_inputs() {
        let fixture = fixture("script.sh");
        assert_eq!(
            classify_fixture_noise(&fixture, b"# leading comment\necho hi\n", true, false),
            CorpusNoiseKind::ShellCollapse
        );
    }

    #[test]
    fn env_truthy_parses_values() {
        // Can't easily test with real env vars in unit tests,
        // so just verify the default behavior
        assert!(!env_truthy("__SHUCK_TEST_NONEXISTENT_VAR__", false));
    }

    #[test]
    fn format_range_handles_zeros() {
        assert_eq!(format_range(1, 2, 0, 0), "1:2-1:2");
        assert_eq!(format_range(1, 2, 3, 4), "1:2-3:4");
    }

    #[test]
    fn shellcheck_cache_key_is_stable_across_worktree_paths() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let left = fixture_at(
            Path::new("/tmp/worktree-a/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let right = fixture_at(
            Path::new("/tmp/worktree-b/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );

        assert_eq!(cache.cache_path(&left), cache.cache_path(&right));
    }

    #[test]
    fn shellcheck_cache_key_changes_with_shellcheck_version_text() {
        let tempdir = tempfile::tempdir().unwrap();
        let fixture = fixture_at(
            Path::new("/tmp/worktree/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let first = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let second = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.1"));

        assert_ne!(first.cache_path(&fixture), second.cache_path(&fixture));
    }

    #[test]
    fn shellcheck_cache_prepare_renames_legacy_current_worktree_files() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path().join("worktree");
        let fixture = fixture_at(
            &root.join(".cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let stable_path = cache.cache_path(&fixture);
        let legacy_path = cache.legacy_cache_path_for_absolute_path(&fixture, &fixture.path);

        write_cache_file(&legacy_path, "legacy-current");

        cache.prepare(std::slice::from_ref(&fixture), std::slice::from_ref(&root));

        assert!(stable_path.is_file());
        assert!(!legacy_path.exists());
        assert_eq!(
            fs::read_to_string(&stable_path).unwrap(),
            cache_file_data("legacy-current")
        );
    }

    #[test]
    fn shellcheck_cache_prepare_renames_legacy_alternate_worktree_files() {
        let tempdir = tempfile::tempdir().unwrap();
        let current_root = tempdir.path().join("current");
        let alternate_root = tempdir.path().join("alternate");
        let fixture = fixture_at(
            &current_root.join(".cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let stable_path = cache.cache_path(&fixture);
        let alternate_legacy_path = cache.legacy_cache_path_for_absolute_path(
            &fixture,
            &alternate_root
                .join(".cache")
                .join("large-corpus")
                .join("corpus")
                .join("scripts")
                .join("example.sh"),
        );

        write_cache_file(&alternate_legacy_path, "legacy-alternate");

        cache.prepare(
            std::slice::from_ref(&fixture),
            &[current_root.clone(), alternate_root.clone()],
        );

        assert!(stable_path.is_file());
        assert!(!alternate_legacy_path.exists());
        assert_eq!(
            fs::read_to_string(&stable_path).unwrap(),
            cache_file_data("legacy-alternate")
        );
    }

    #[test]
    fn shellcheck_cache_prepare_is_idempotent() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path().join("worktree");
        let fixture = fixture_at(
            &root.join(".cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let stable_path = cache.cache_path(&fixture);
        let legacy_path = cache.legacy_cache_path_for_absolute_path(&fixture, &fixture.path);

        write_cache_file(&legacy_path, "legacy-current");
        cache.prepare(std::slice::from_ref(&fixture), std::slice::from_ref(&root));
        cache.prepare(std::slice::from_ref(&fixture), std::slice::from_ref(&root));

        assert!(stable_path.is_file());
        assert!(!legacy_path.exists());
        assert_eq!(
            fs::read_to_string(&stable_path).unwrap(),
            cache_file_data("legacy-current")
        );
    }

    #[test]
    fn shellcheck_cache_prepare_keeps_existing_stable_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path().join("worktree");
        let fixture = fixture_at(
            &root.join(".cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let stable_path = cache.cache_path(&fixture);
        let legacy_path = cache.legacy_cache_path_for_absolute_path(&fixture, &fixture.path);

        write_cache_file(&stable_path, "stable");
        write_cache_file(&legacy_path, "legacy-current");

        cache.prepare(std::slice::from_ref(&fixture), std::slice::from_ref(&root));

        assert!(stable_path.is_file());
        assert!(!legacy_path.exists());
        assert_eq!(
            fs::read_to_string(&stable_path).unwrap(),
            cache_file_data("stable")
        );
    }

    #[test]
    fn large_corpus_candidate_resolution_uses_cache_relative_parent_dirs() {
        let resolved = resolve_large_corpus_candidate_cache_rel_path(
            Path::new("repo/pkg/main.sh"),
            "../common/helper.sh",
        );

        assert_eq!(resolved, Some(PathBuf::from("repo/common/helper.sh")));
    }

    #[test]
    fn large_corpus_path_resolver_keeps_helper_lookup_inside_current_repo() {
        let tempdir = tempfile::tempdir().unwrap();
        let scripts_dir = tempdir.path().join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();

        let source = fixture_at(
            &scripts_dir.join("repo__pkg__main.sh"),
            Path::new("repo/pkg/main.sh"),
        );
        let local = fixture_at(
            &scripts_dir.join("repo__pkg__build.sh"),
            Path::new("repo/pkg/build.sh"),
        );
        let unrelated = fixture_at(
            &scripts_dir.join("other__build.sh"),
            Path::new("other/build.sh"),
        );

        fs::write(&source.path, "#!/bin/sh\n./build.sh\n").unwrap();
        fs::write(&local.path, "echo local\n").unwrap();
        fs::write(&unrelated.path, "echo unrelated\n").unwrap();

        let resolver = LargeCorpusPathResolver::new(&[&source, &local, &unrelated]);
        let resolved = shuck_semantic::SourcePathResolver::resolve_candidate_paths(
            &resolver,
            &source.path,
            "./build.sh",
        );

        assert_eq!(resolved, vec![canonicalize_for_resolver(&local.path)]);
    }

    #[test]
    fn large_corpus_path_resolver_can_resolve_repo_relative_static_tails() {
        let tempdir = tempfile::tempdir().unwrap();
        let scripts_dir = tempdir.path().join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();

        let source = fixture_at(
            &scripts_dir.join("rvm__rvm__tests__fast__sample.sh"),
            Path::new("rvm__rvm__tests__fast__sample.sh"),
        );
        let helper = fixture_at(
            &scripts_dir.join("rvm__rvm__scripts__rvm"),
            Path::new("rvm__rvm__scripts__rvm"),
        );

        fs::write(
            &source.path,
            "#!/bin/sh\nsource \"$rvm_path/scripts/rvm\"\n",
        )
        .unwrap();
        fs::write(&helper.path, "echo helper\n").unwrap();

        let resolver = LargeCorpusPathResolver::new(&[&source, &helper]);
        let resolved = shuck_semantic::SourcePathResolver::resolve_candidate_paths(
            &resolver,
            &source.path,
            "scripts/rvm",
        );

        assert_eq!(resolved, vec![canonicalize_for_resolver(&helper.path)]);
    }

    #[test]
    fn collect_fixtures_skips_sample_and_fish_files() {
        let tempdir = tempfile::tempdir().unwrap();
        let scripts_dir = tempdir.path().join("scripts");
        let nested_dir = scripts_dir.join("nested");
        fs::create_dir_all(&nested_dir).unwrap();

        fs::write(scripts_dir.join("keep.sh"), "#!/bin/sh\necho keep\n").unwrap();
        fs::write(
            scripts_dir.join("pre-commit.sample"),
            "#!/bin/sh\necho skip\n",
        )
        .unwrap();
        fs::write(scripts_dir.join("config.fish"), "echo skip\n").unwrap();
        fs::write(
            nested_dir.join("post-checkout.sample"),
            "#!/bin/sh\necho skip\n",
        )
        .unwrap();
        fs::write(nested_dir.join("prompt.fish"), "echo skip\n").unwrap();

        let fixtures = collect_fixtures(tempdir.path());
        let collected_paths: Vec<_> = fixtures
            .into_iter()
            .map(|fixture| fixture.cache_rel_path)
            .collect();

        assert_eq!(collected_paths, vec![PathBuf::from("keep.sh")]);
    }

    fn fixture(path: &str) -> LargeCorpusFixture {
        LargeCorpusFixture {
            path: PathBuf::from(path),
            cache_rel_path: PathBuf::from(path),
            shell: "sh".into(),
            source_hash: String::new(),
        }
    }

    fn shellcheck_diagnostic(code: u32) -> ShellCheckDiagnostic {
        ShellCheckDiagnostic {
            file: String::new(),
            code,
            line: 1,
            end_line: 1,
            column: 1,
            end_column: 1,
            level: "error".into(),
            message: format!("diagnostic {code}"),
        }
    }

    fn fixture_at(path: &Path, cache_rel_path: &Path) -> LargeCorpusFixture {
        LargeCorpusFixture {
            path: path.to_path_buf(),
            cache_rel_path: cache_rel_path.to_path_buf(),
            shell: "sh".into(),
            source_hash: "source-hash".into(),
        }
    }

    fn probe(version_text: &str) -> ShellCheckProbe {
        ShellCheckProbe {
            command: "shellcheck".into(),
            version_text: version_text.into(),
        }
    }

    fn write_cache_file(path: &Path, label: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, cache_file_data(label)).unwrap();
    }

    fn cache_file_data(label: &str) -> String {
        serde_json::to_string(&ShellCheckCacheEntry {
            schema: SHELLCHECK_CACHE_SCHEMA,
            diagnostics: vec![ShellCheckDiagnostic {
                file: String::new(),
                code: 2034,
                line: 1,
                end_line: 1,
                column: 1,
                end_column: 1,
                level: "warning".into(),
                message: label.into(),
            }],
            parse_aborted: false,
        })
        .unwrap()
    }
}
