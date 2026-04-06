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
const SHELLCHECK_RULE_ALLOWLIST_DIR: &str = "tests/testdata/allowlists";

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
        let Some(candidate_cache_rel_path) =
            resolve_large_corpus_candidate_cache_rel_path(source_cache_rel_path, candidate)
        else {
            return Vec::new();
        };

        self.path_by_cache_rel
            .get(&candidate_cache_rel_path)
            .cloned()
            .into_iter()
            .collect()
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ShellCheckRuleAllowlistDocument {
    entries: Vec<ShellCheckRuleAllowlistEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ShellCheckRuleAllowlistEntry {
    path_suffix: String,
    line: usize,
    end_line: usize,
    column: usize,
    end_column: usize,
    reason: String,
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
    locations: HashMap<String, usize>,
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
struct FixtureFailureCollection {
    failures: Vec<String>,
    timeout_cap_reached: bool,
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
    let shellcheck_index = build_shellcheck_index();
    let sc2034_allowlist =
        load_shellcheck_rule_allowlist("SC2034").expect("failed to load SC2034 allowlist");
    let shellcheck_filter_codes =
        build_shellcheck_filter_codes(cfg.selected_rules, cfg.mapped_only);
    let shellcheck_cache = ShellCheckCache::new(&cfg.cache_dir, &shellcheck);
    shellcheck_cache.prepare(&fixtures, &discover_worktree_roots());
    let linter_settings = build_large_corpus_linter_settings(cfg.selected_rules);
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
                &sc2034_allowlist,
                shellcheck_filter_codes.as_ref(),
                Arc::clone(&shuck_path_resolver),
            )
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
        failure_collection.failures.is_empty(),
        "large corpus compatibility had {} failure(s) across {} fixture(s) ({} skipped unsupported shells){}:\n\n{}",
        failure_collection.failures.len(),
        fixtures.len(),
        skipped_unsupported_shells,
        timeout_cap_note,
        failure_collection.failures.join("\n\n")
    );
}

fn collect_fixture_failures<F>(
    fixtures: &[&LargeCorpusFixture],
    keep_going: bool,
    evaluate: F,
) -> FixtureFailureCollection
where
    F: Fn(&LargeCorpusFixture) -> Option<FixtureFailure> + Sync,
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
    F: Fn(&LargeCorpusFixture) -> Option<FixtureFailure>,
{
    for fixture in fixtures {
        let failure = evaluate(fixture);
        progress.finish_fixture();

        if let Some(failure) = failure {
            if failure.kind == FixtureFailureKind::Timeout {
                log_large_corpus_timeout(fixture);
            }
            panic!("{}", failure.message);
        }
    }

    FixtureFailureCollection::default()
}

fn collect_fixture_failures_in_parallel<F>(
    fixtures: &[&LargeCorpusFixture],
    worker_count: usize,
    evaluate: &F,
    progress: &LargeCorpusProgress,
) -> FixtureFailureCollection
where
    F: Fn(&LargeCorpusFixture) -> Option<FixtureFailure> + Sync,
{
    if fixtures.is_empty() {
        return FixtureFailureCollection::default();
    }

    let worker_count = worker_count.max(1).min(fixtures.len());
    let next_index = AtomicUsize::new(0);
    let timeout_failures = AtomicUsize::new(0);
    let timeout_cap_reached = AtomicBool::new(false);
    let failures = Mutex::new(Vec::<(usize, String)>::new());

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let failures = &failures;
            let next_index = &next_index;
            let progress = progress;
            let timeout_failures = &timeout_failures;
            let timeout_cap_reached = &timeout_cap_reached;
            scope.spawn(move || {
                let mut local_failures = Vec::new();
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
                        Ok(Some(failure)) => {
                            if failure.kind == FixtureFailureKind::Timeout {
                                log_large_corpus_timeout(fixture);
                                let timeout_count =
                                    timeout_failures.fetch_add(1, Ordering::Relaxed) + 1;
                                if timeout_count <= LARGE_CORPUS_TIMEOUT_FAILURE_CAP {
                                    local_failures.push((index, failure.message));
                                }
                                if timeout_count >= LARGE_CORPUS_TIMEOUT_FAILURE_CAP {
                                    timeout_cap_reached.store(true, Ordering::Relaxed);
                                }
                                continue;
                            }

                            local_failures.push((index, failure.message));
                        }
                        Ok(None) => {}
                        Err(payload) => {
                            local_failures.push((index, format_fixture_panic(fixture, payload)));
                        }
                    }
                }

                if !local_failures.is_empty() {
                    failures.lock().unwrap().extend(local_failures);
                }
            });
        }
    });

    let mut failures = failures.into_inner().unwrap();
    failures.sort_by_key(|(index, _)| *index);
    FixtureFailureCollection {
        failures: failures.into_iter().map(|(_, failure)| failure).collect(),
        timeout_cap_reached: timeout_cap_reached.load(Ordering::Relaxed),
    }
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
    sc2034_allowlist: &[ShellCheckRuleAllowlistEntry],
    shellcheck_filter_codes: Option<&HashSet<u32>>,
    shuck_path_resolver: Arc<LargeCorpusPathResolver>,
) -> Option<FixtureFailure> {
    let mut issues: Vec<String> = Vec::new();
    let mut failure_kind = FixtureFailureKind::Other;

    let shuck_run = match run_shuck_with_timeout(
        fixture,
        linter_settings,
        shuck_timeout,
        shuck_path_resolver,
    ) {
        Ok(run) => run,
        Err(err) => {
            return Some(FixtureFailure {
                kind: fixture_failure_kind_for_message(&err, "shuck"),
                message: format_fixture_failure(&fixture.path, &[err]),
            });
        }
    };
    if let Some(ref err) = shuck_run.parse_error {
        issues.push(format!("shuck parse error: {err}"));
    }

    match shellcheck_cache.run_fixture(fixture, shellcheck_path, shellcheck_timeout) {
        Ok(sc_run) => {
            let sc_run = apply_shellcheck_allowlist(
                sc2034_allowlist,
                2034,
                fixture,
                filter_shellcheck_run(sc_run, shellcheck_filter_codes),
            );
            if shuck_run.parse_error.is_none() {
                let sc_locations =
                    count_codes(&shellcheck_compatibility_locations(&sc_run.diagnostics));
                let shuck_locations = &shuck_run.locations;
                if let Some(diff) =
                    compatibility_code_diff(&sc_locations, shuck_locations, DiffMode::All)
                {
                    let src = fs::read(&fixture.path).unwrap_or_default();
                    issues.push(format_compatibility_diff(
                        &diff,
                        &src,
                        &sc_locations,
                        shuck_locations,
                        &sc_run,
                        &shuck_run.diagnostics,
                        shellcheck_index,
                    ));
                }
            }
        }
        Err(err) => {
            if fixture_failure_kind_for_message(&err, "shellcheck") == FixtureFailureKind::Timeout {
                failure_kind = FixtureFailureKind::Timeout;
            }
            issues.push(format!("shellcheck error: {err}"));
        }
    }

    (!issues.is_empty()).then(|| FixtureFailure {
        kind: failure_kind,
        message: format_fixture_failure(&fixture.path, &issues),
    })
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

fn apply_shellcheck_allowlist(
    allowlist: &[ShellCheckRuleAllowlistEntry],
    code: u32,
    fixture: &LargeCorpusFixture,
    mut run: ShellCheckRun,
) -> ShellCheckRun {
    run.diagnostics
        .retain(|diag| shellcheck_allowlist_reason(allowlist, code, &fixture.path, diag).is_none());
    run.parse_aborted = shellcheck_parse_aborted(&run.diagnostics);
    run
}

fn shellcheck_allowlist_reason<'a>(
    allowlist: &'a [ShellCheckRuleAllowlistEntry],
    code: u32,
    path: &Path,
    diag: &ShellCheckDiagnostic,
) -> Option<&'a str> {
    if diag.code != code {
        return None;
    }

    let path = path.to_string_lossy();
    allowlist.iter().find_map(|entry| {
        (path.ends_with(entry.path_suffix.as_str())
            && diag.line == entry.line
            && diag.end_line == entry.end_line
            && diag.column == entry.column
            && diag.end_column == entry.end_column)
            .then_some(entry.reason.as_str())
    })
}

fn load_shellcheck_rule_allowlist(
    rule_code: &str,
) -> Result<Vec<ShellCheckRuleAllowlistEntry>, String> {
    let path = shellcheck_rule_allowlist_path(rule_code);
    let data =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let document: ShellCheckRuleAllowlistDocument =
        serde_yaml::from_str(&data).map_err(|err| format!("parse {}: {err}", path.display()))?;
    Ok(document.entries)
}

fn shellcheck_rule_allowlist_path(rule_code: &str) -> PathBuf {
    let filename = format!("{}.yaml", rule_code.to_ascii_lowercase());
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(SHELLCHECK_RULE_ALLOWLIST_DIR)
        .join(filename)
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
    if fixture_looks_like_zsh(fixture) || fixture_is_repo_git_entry(fixture) {
        return false;
    }

    shell_supported_for_large_corpus(fixture.shell.as_str(), shellcheck_supported_shells)
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

fn fixture_looks_like_zsh(fixture: &LargeCorpusFixture) -> bool {
    let Some(name) = fixture.path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    name.ends_with(".zsh") || name.ends_with(".zsh-theme") || name.starts_with(".zsh")
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
        for projected in [
            root.join(LARGE_CORPUS_CACHE_DIR_NAME)
                .join("scripts")
                .join(cache_rel_path),
            root.join(LARGE_CORPUS_CACHE_DIR_NAME)
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

fn run_shuck(
    fixture: &LargeCorpusFixture,
    linter_settings: &shuck_linter::LinterSettings,
    source_path_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)>,
) -> ShuckRun {
    let source = match fs::read_to_string(&fixture.path) {
        Ok(s) => s,
        Err(e) => {
            return ShuckRun {
                locations: HashMap::new(),
                diagnostics: Vec::new(),
                parse_error: Some(format!("read error: {e}")),
            };
        }
    };

    let output = match shuck_parser::parser::Parser::new(&source).parse() {
        Ok(o) => o,
        Err(e) => {
            return ShuckRun {
                locations: HashMap::new(),
                diagnostics: Vec::new(),
                parse_error: Some(e.to_string()),
            };
        }
    };

    let indexer = shuck_indexer::Indexer::new(&source, &output);
    let linter_settings = linter_settings
        .clone()
        .with_shell(shuck_linter::ShellDialect::from_name(&fixture.shell));
    let diagnostics = shuck_linter::lint_file_at_path_with_resolver(
        &output.script,
        &source,
        &indexer,
        &linter_settings,
        None,
        Some(&fixture.path),
        source_path_resolver,
    );

    let shellcheck_index = build_shellcheck_index();
    let locations = count_codes(&shuck_compatibility_locations(
        &diagnostics,
        &shellcheck_index,
    ));

    ShuckRun {
        locations,
        diagnostics,
        parse_error: None,
    }
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

fn build_shellcheck_index() -> HashMap<String, String> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .map(|(sc_code, rule)| (rule.code().to_owned(), format!("SC{sc_code}")))
        .collect()
}

fn build_large_corpus_linter_settings(
    selected_rules: Option<shuck_linter::RuleSet>,
) -> shuck_linter::LinterSettings {
    selected_rules.map_or_else(shuck_linter::LinterSettings::default, |rules| {
        shuck_linter::LinterSettings::for_rules(rules.iter())
    })
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

fn shuck_compatibility_locations(
    diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> Vec<String> {
    let mut locations = Vec::new();
    for diag in diagnostics {
        let code = diag.code();
        if let Some(sc_code) = shellcheck_index.get(code) {
            locations.push(format!(
                "{} {}",
                sc_code,
                format_range(
                    diag.span.start.line,
                    diag.span.start.column,
                    diag.span.end.line,
                    diag.span.end.column,
                )
            ));
        }
    }
    locations
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

fn shellcheck_compatibility_locations(diagnostics: &[ShellCheckDiagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diag| {
            format!(
                "SC{:04} {}",
                diag.code,
                format_range(diag.line, diag.column, diag.end_line, diag.end_column)
            )
        })
        .collect()
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

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

fn format_compatibility_diff(
    diff: &str,
    src: &[u8],
    shellcheck_locations: &HashMap<String, usize>,
    shuck_locations: &HashMap<String, usize>,
    shellcheck_run: &ShellCheckRun,
    shuck_diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> String {
    let labels = compatibility_labels(
        src,
        shellcheck_locations,
        shuck_locations,
        shellcheck_run,
        shuck_diagnostics,
        shellcheck_index,
    );
    format!(
        "compatibility diff (code + location):\n\
         {diff}\n\
         labels:\n\
         {}\n\
         shellcheck parse aborted: {}\n\
         shellcheck locations:\n\
         {}\n\
         shuck locations:\n\
         {}\n\
         shellcheck diagnostics:\n\
         {}\n\
         shuck diagnostics:\n\
         {}",
        format_labels(&labels),
        shellcheck_run.parse_aborted,
        format_compatibility_counts(shellcheck_locations),
        format_compatibility_counts(shuck_locations),
        format_shellcheck_diagnostics(shellcheck_run),
        format_shuck_diagnostics(shuck_diagnostics, shellcheck_index),
    )
}

fn compatibility_labels(
    src: &[u8],
    shellcheck_locations: &HashMap<String, usize>,
    shuck_locations: &HashMap<String, usize>,
    shellcheck_run: &ShellCheckRun,
    shuck_diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> Vec<String> {
    let mut labels = Vec::new();

    if compatibility_location_only(
        shellcheck_locations,
        shuck_locations,
        &shellcheck_run.diagnostics,
        shuck_diagnostics,
        shellcheck_index,
    ) {
        labels.push("location-only".into());
    }
    if shellcheck_run.parse_aborted {
        labels.push("shellcheck-parse-abort".into());
    }
    if source_has_directive_handling_hints(src) {
        labels.push("directive-handling".into());
    }
    if source_has_project_closure_hints(src, shellcheck_run) {
        labels.push("project-closure".into());
    }
    if source_starts_with_unknown_shell_comment(src) {
        labels.push("unknown-shell-collapse".into());
    }

    labels.sort();
    labels
}

fn compatibility_location_only(
    shellcheck_locations: &HashMap<String, usize>,
    shuck_locations: &HashMap<String, usize>,
    shellcheck_diagnostics: &[ShellCheckDiagnostic],
    shuck_diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> bool {
    if compatibility_code_diff(shellcheck_locations, shuck_locations, DiffMode::All).is_none() {
        return false;
    }
    let sc_codes = count_codes(&shellcheck_code_list(shellcheck_diagnostics));
    let shuck_codes = count_codes(&shuck_compatibility_codes(
        shuck_diagnostics,
        shellcheck_index,
    ));
    compatibility_code_diff(&sc_codes, &shuck_codes, DiffMode::All).is_none()
}

fn shellcheck_code_list(diags: &[ShellCheckDiagnostic]) -> Vec<String> {
    diags.iter().map(|d| format!("SC{}", d.code)).collect()
}

fn shuck_compatibility_codes(
    diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> Vec<String> {
    diagnostics
        .iter()
        .filter_map(|diag| shellcheck_index.get(diag.code()).cloned())
        .collect()
}

fn source_has_directive_handling_hints(src: &[u8]) -> bool {
    let text = String::from_utf8_lossy(src);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(body) = trimmed.strip_prefix('#') {
            let lower = body.trim().to_lowercase();
            if lower.starts_with("shellcheck ")
                || lower == "shellcheck"
                || lower.starts_with("shuck:")
            {
                return true;
            }
        }
    }
    false
}

fn source_has_project_closure_hints(src: &[u8], shellcheck_run: &ShellCheckRun) -> bool {
    let text = String::from_utf8_lossy(src);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(body) = trimmed.strip_prefix('#') {
            let lower = body.trim().to_lowercase();
            if lower.starts_with("shellcheck source=") {
                return true;
            }
            continue;
        }
        if trimmed.starts_with("source ") || trimmed.starts_with(". ") {
            return true;
        }
    }
    for diag in &shellcheck_run.diagnostics {
        if matches!(diag.code, 1090 | 1091 | 2119 | 2120) {
            return true;
        }
    }
    false
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

fn format_labels(labels: &[String]) -> String {
    if labels.is_empty() {
        "(none)".into()
    } else {
        labels.join("\n")
    }
}

fn format_compatibility_counts(counts: &HashMap<String, usize>) -> String {
    if counts.is_empty() {
        return "(none)".into();
    }
    let mut codes: Vec<_> = counts.iter().collect();
    codes.sort_by_key(|(k, _)| (*k).clone());
    codes
        .iter()
        .map(|(code, count)| format!("{code}={count}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_shellcheck_diagnostics(run: &ShellCheckRun) -> String {
    if run.diagnostics.is_empty() {
        return "(none)".into();
    }
    let mut lines = Vec::new();
    if run.parse_aborted {
        lines.push("parse-aborted=true".into());
    }
    for diag in &run.diagnostics {
        lines.push(format!(
            "SC{:04} {} {} {}",
            diag.code,
            format_range(diag.line, diag.column, diag.end_line, diag.end_column),
            diag.level,
            diag.message,
        ));
    }
    lines.join("\n")
}

fn format_shuck_diagnostics(
    diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> String {
    if diagnostics.is_empty() {
        return "(none)".into();
    }
    let mut lines = Vec::new();
    for diag in diagnostics {
        let mapped = shellcheck_index
            .get(diag.code())
            .map(|sc| format!("=>{sc}"))
            .unwrap_or_default();
        lines.push(format!(
            "{}{} {} {} {}",
            diag.code(),
            mapped,
            format_range(
                diag.span.start.line,
                diag.span.start.column,
                diag.span.end.line,
                diag.span.end.column,
            ),
            diag.severity.as_str(),
            diag.message,
        ));
    }
    lines.join("\n")
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
            thread::sleep(Duration::from_millis(50));
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
        let settings = build_large_corpus_linter_settings(Some(rules));

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
    fn mapped_only_filter_drops_unmapped_shellcheck_diagnostics() {
        let mapped_shellcheck_codes = build_mapped_shellcheck_codes();
        let run = ShellCheckRun {
            diagnostics: vec![
                shellcheck_diagnostic(2034),
                shellcheck_diagnostic(2086),
                shellcheck_diagnostic(1072),
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
    fn shellcheck_rule_allowlist_path_uses_rule_code_convention() {
        assert_eq!(
            shellcheck_rule_allowlist_path("SC2034"),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/testdata/allowlists")
                .join("sc2034.yaml")
        );
    }

    #[test]
    fn sc2034_allowlist_filters_exact_matches_only() {
        let allowlist =
            load_shellcheck_rule_allowlist("SC2034").expect("failed to load SC2034 allowlist");
        let fixture =
            fixture("SlackBuildsOrg__slackbuilds__libraries__bluez-alsa__bluez-alsa.conf");
        let run = ShellCheckRun {
            diagnostics: vec![
                ShellCheckDiagnostic {
                    file: String::new(),
                    code: 2034,
                    line: 19,
                    end_line: 19,
                    column: 1,
                    end_column: 14,
                    level: "warning".into(),
                    message: "allowlisted".into(),
                },
                ShellCheckDiagnostic {
                    file: String::new(),
                    code: 2034,
                    line: 19,
                    end_line: 19,
                    column: 1,
                    end_column: 15,
                    level: "warning".into(),
                    message: "still visible".into(),
                },
            ],
            parse_aborted: false,
        };

        let filtered = apply_shellcheck_allowlist(&allowlist, 2034, &fixture, run);

        assert_eq!(filtered.diagnostics.len(), 1);
        assert_eq!(filtered.diagnostics[0].end_column, 15);
    }

    #[test]
    fn sc2034_allowlist_loads_documented_entries() {
        let allowlist =
            load_shellcheck_rule_allowlist("SC2034").expect("failed to load SC2034 allowlist");

        assert!(allowlist.iter().any(|entry| {
            entry.path_suffix
                == "SlackBuildsOrg__slackbuilds__libraries__bluez-alsa__bluez-alsa.conf"
                && entry.line == 19
                && entry.end_line == 19
                && entry.column == 1
                && entry.end_column == 14
                && entry.reason
                    == "config value is consumed by rc.bluez-alsa after sourcing the file"
        }));
        assert!(allowlist.iter().any(|entry| {
            entry.path_suffix == "233boy__v2ray__install.sh"
                && entry.line == 424
                && entry.end_line == 424
                && entry.column == 5
                && entry.end_column == 19
                && entry.reason
                    == "install mode flag is consumed by the dynamically sourced core.sh helper"
        }));
        assert!(allowlist.iter().any(|entry| {
            entry.path_suffix
                == "GameServerManagers__LinuxGSM__lgsm__modules__command_details.sh"
                && entry.line == 19
                && entry.end_line == 19
                && entry.column == 2
                && entry.end_column == 5
                && entry.reason
                    == "loop variable is consumed by the sibling query_gamedig.sh helper invoked in the loop"
        }));
    }

    #[test]
    fn keep_going_collects_multiple_fixture_failures() {
        let first = fixture("first.sh");
        let second = fixture("second.sh");
        let fixtures = vec![&first, &second];
        let seen = Mutex::new(Vec::new());

        let failures = collect_fixture_failures(&fixtures, true, |fixture| {
            seen.lock().unwrap().push(fixture.path.clone());
            Some(FixtureFailure {
                kind: FixtureFailureKind::Other,
                message: format_fixture_failure(
                    &fixture.path,
                    &[format!("{} failed", fixture.path.display())],
                ),
            })
        });

        let mut seen = seen.into_inner().unwrap();
        seen.sort();

        assert_eq!(
            seen,
            vec![PathBuf::from("first.sh"), PathBuf::from("second.sh")]
        );
        assert!(!failures.timeout_cap_reached);
        assert_eq!(failures.failures.len(), 2);
        assert!(failures.failures[0].contains("first.sh"));
        assert!(failures.failures[1].contains("second.sh"));
    }

    #[test]
    fn keep_going_captures_fixture_panics() {
        let fixture = fixture("panic.sh");
        let fixtures = vec![&fixture];

        let failures = collect_fixture_failures(&fixtures, true, |_| -> Option<FixtureFailure> {
            panic!("boom");
        });

        assert!(!failures.timeout_cap_reached);
        assert_eq!(failures.failures.len(), 1);
        assert!(failures.failures[0].contains("panic.sh"));
        assert!(failures.failures[0].contains("fixture panic: boom"));
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
            Some(FixtureFailure {
                kind: FixtureFailureKind::Timeout,
                message: format_fixture_failure(
                    &fixture.path,
                    &[format!(
                        "shuck error: {}",
                        format_timeout_message("shuck", Duration::from_secs(30))
                    )],
                ),
            })
        });

        assert!(failures.timeout_cap_reached);
        assert_eq!(failures.failures.len(), LARGE_CORPUS_TIMEOUT_FAILURE_CAP);
        assert!(seen.load(Ordering::Relaxed) <= fixture_refs.len());
        assert!(
            failures
                .failures
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
    fn source_directive_hints_detected() {
        assert!(source_has_directive_handling_hints(
            b"#!/bin/sh\n# shellcheck disable=SC2086\necho $x\n"
        ));
        assert!(!source_has_directive_handling_hints(
            b"#!/bin/sh\necho ok\n"
        ));
    }

    #[test]
    fn source_project_closure_hints_detected() {
        let run = ShellCheckRun {
            diagnostics: Vec::new(),
            parse_aborted: false,
        };
        assert!(source_has_project_closure_hints(
            b"#!/bin/sh\nsource ./lib.sh\n",
            &run
        ));
        assert!(!source_has_project_closure_hints(
            b"#!/bin/sh\necho ok\n",
            &run
        ));
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
            &alternate_root.join(".cache/large-corpus/corpus/scripts/example.sh"),
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
