use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shuck::shellcheck_runtime::{
    ShellCheckDiagnostic, ShellCheckProbe, ShellCheckRun, decode_shellcheck_diagnostics,
    parse_shellcheck_supported_shells, probe_shellcheck, run_shellcheck_json1 as run_shellcheck,
    shellcheck_parse_aborted, shellcheck_supported_shells,
};

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
const LARGE_CORPUS_TIMING_ENV: &str = "SHUCK_LARGE_CORPUS_TIMING";

const LARGE_CORPUS_DEFAULT_SHELLCHECK_TIMEOUT: Duration = Duration::from_secs(300);
const LARGE_CORPUS_DEFAULT_SHUCK_TIMEOUT: Duration = Duration::from_secs(30);
const LARGE_CORPUS_AUTOSCALED_SHUCK_TIMEOUT_BUFFER: Duration = Duration::from_secs(15);
const LARGE_CORPUS_MAX_AUTOSCALED_SHUCK_TIMEOUT: Duration = Duration::from_secs(150);
const LARGE_CORPUS_AUTOSCALED_SHUCK_LINES_PER_SEC: usize = 175;
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";
const LARGE_CORPUS_MAX_WORKER_COUNT: usize = 4;
const LARGE_CORPUS_TIMEOUT_FAILURE_CAP: usize = 5;
const LARGE_CORPUS_PROGRESS_PERCENT_STEP: usize = 5;
const LARGE_CORPUS_PROGRESS_BUCKET_COUNT: usize = 100 / LARGE_CORPUS_PROGRESS_PERCENT_STEP;
const LARGE_CORPUS_TIMING_LIMIT: usize = 25;
const RULE_CORPUS_METADATA_DIR: &str = "tests/testdata/corpus-metadata";
const LARGE_CORPUS_ALLOWED_FAILING_RULE_REASON: &str = "known large-corpus rule allowlist";
const LARGE_CORPUS_STATIC_IGNORE_SUFFIXES: &[&str] = &[
    "super-linter__super-linter__test__linters__bash__shell_bad_1.sh",
    "super-linter__super-linter__test__linters__bash_exec__shell_bad_1.sh",
    "alpinelinux__aports__community__starship__starship.plugin.zsh",
    "CISOfy__lynis__include__tests_ports_packages",
    "google__oss-fuzz__infra__chronos__coverage_test_collection.py",
    "moovweb__gvm__examples__native__configure",
    "moovweb__gvm__examples__native__ltmain.sh",
    "ohmyzsh__ohmyzsh__plugins__alias-finder__tests__test_run.sh",
];
const LARGE_CORPUS_STATIC_ZSH_OVERRIDE_SUFFIXES: &[&str] = &[
    "ohmyzsh__ohmyzsh__oh-my-zsh.sh",
    "ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh",
];
const LARGE_CORPUS_SHELLCHECK_UNSUPPORTED_REPO_PREFIXES: &[&str] = &["ohmyzsh__ohmyzsh__"];
const LARGE_CORPUS_SHELLCHECK_PARSE_IGNORE_SUFFIXES: &[&str] = &[
    "SlackBuildsOrg__slackbuilds__network__modemu2k__modemu2k.SlackBuild",
    "SlackBuildsOrg__slackbuilds__system__firmware-gobi-2000__firmware-gobi-2000.SlackBuild",
    "alpinelinux__aports__community__dnscrypt-proxy__dnscrypt-proxy.setup",
    "bittorf__kalua__openwrt-addons__www__cgi-bin-status.sh",
    "docker__docker-bench-security__tests__2_docker_daemon_configuration.sh",
    "google__oss-fuzz__projects__apache-tika__build.sh",
    "leebaird__discover__passive.sh",
    "nvm-sh__nvm__test__install_script__install_nvm_from_git",
    "pyenv__pyenv__plugins__python-build__bin__python-build",
    "romkatv__powerlevel10k__gitstatus__install",
    "rvm__rvm__scripts__functions__build_config_system",
    "rvm__rvm__scripts__functions__requirements__gentoo_portage",
    "rvm__rvm__scripts__functions__rvmrc",
];
const SHELLCHECK_CACHE_SCHEMA: u32 = 3;
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
    timing_mode: bool,
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

fn source_line_count(source: &[u8]) -> usize {
    if source.is_empty() {
        return 0;
    }

    let newline_count = source.iter().filter(|&&byte| byte == b'\n').count();
    if source.last() == Some(&b'\n') {
        newline_count
    } else {
        newline_count + 1
    }
}

fn effective_shuck_timeout(source: &[u8], base_timeout: Duration) -> Duration {
    if env::var_os(LARGE_CORPUS_SHUCK_TIMEOUT_ENV).is_some() {
        return base_timeout;
    }

    let line_count = source_line_count(source);
    let scaled_timeout_secs = line_count.div_ceil(LARGE_CORPUS_AUTOSCALED_SHUCK_LINES_PER_SEC);
    if scaled_timeout_secs <= base_timeout.as_secs() as usize {
        return base_timeout;
    }

    let scaled_timeout = Duration::from_secs(scaled_timeout_secs as u64)
        + LARGE_CORPUS_AUTOSCALED_SHUCK_TIMEOUT_BUFFER;
    scaled_timeout.min(LARGE_CORPUS_MAX_AUTOSCALED_SHUCK_TIMEOUT)
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
struct RuleCorpusMetadataDocument {
    #[serde(default)]
    review_all_divergences: bool,
    #[serde(default)]
    reviewed_divergences: Vec<ReviewedDivergenceRecord>,
}

impl RuleCorpusMetadataDocument {
    fn has_entries(&self) -> bool {
        self.review_all_divergences || !self.reviewed_divergences.is_empty()
    }

    fn validate(&self, rule_code: &str, path: &Path) -> Result<(), String> {
        for (index, entry) in self.reviewed_divergences.iter().enumerate() {
            entry.validate(rule_code, path, index + 1)?;
        }

        Ok(())
    }
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
    rule_wide: bool,
    #[serde(default)]
    path_suffix: Option<String>,
    #[serde(default)]
    path_contains: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
    #[serde(default)]
    column: Option<usize>,
    #[serde(default)]
    end_column: Option<usize>,
    reason: String,
}

impl ReviewedDivergenceRecord {
    fn has_scope(&self) -> bool {
        self.path_suffix.is_some()
            || self.path_contains.is_some()
            || self.line.is_some()
            || self.end_line.is_some()
            || self.column.is_some()
            || self.end_column.is_some()
    }

    fn validate(&self, rule_code: &str, path: &Path, entry_index: usize) -> Result<(), String> {
        match (self.rule_wide, self.has_scope()) {
            (false, false) => Err(format!(
                "invalid reviewed divergence {} entry {} in {}: add a scope field or set rule_wide: true",
                rule_code,
                entry_index,
                path.display()
            )),
            (true, true) => Err(format!(
                "invalid reviewed divergence {} entry {} in {}: rule-wide entries cannot also set path or range constraints",
                rule_code,
                entry_index,
                path.display()
            )),
            _ => Ok(()),
        }
    }
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
    rule_codes: Vec<String>,
    shellcheck_code: String,
    range: DiagnosticRange,
    message: String,
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
    Warning,
    Other,
    Timeout,
}

impl FixtureFailureKind {
    fn blocks(self) -> bool {
        matches!(self, Self::Other | Self::Timeout)
    }
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
    harness_failure: Option<FixtureFailure>,
}

#[derive(Debug, Default)]
struct FixtureFailureCollection {
    implementation_diffs: Vec<String>,
    mapping_issues: Vec<String>,
    reviewed_divergences: Vec<String>,
    harness_warnings: Vec<String>,
    harness_failures: Vec<String>,
    unsupported_shells: usize,
}

impl FixtureFailureCollection {
    fn blocking_failures(&self) -> usize {
        self.implementation_diffs.len() + self.harness_failures.len()
    }

    fn nonblocking_issue_count(&self) -> usize {
        self.mapping_issues.len() + self.reviewed_divergences.len() + self.harness_warnings.len()
    }

    fn has_nonblocking_items(&self) -> bool {
        self.unsupported_shells > 0 || self.nonblocking_issue_count() > 0
    }
}

#[derive(Clone, Copy)]
enum LargeCorpusReportMode {
    Full,
    BlockingOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LargeCorpusTimingOutcome {
    Ok,
    ParseError,
    Timeout,
    Error,
}

impl LargeCorpusTimingOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ParseError => "parse-error",
            Self::Timeout => "timeout",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LargeCorpusTimingRecord {
    fixture_label: String,
    elapsed: Duration,
    outcome: LargeCorpusTimingOutcome,
}

#[derive(Debug, Default)]
struct LargeCorpusTimingCollection {
    records: Vec<LargeCorpusTimingRecord>,
    timeout_cap_reached: bool,
}

fn collect_fixture_timings(
    fixtures: &[&LargeCorpusFixture],
    shuck_timeout: Duration,
    linter_settings: &shuck_linter::LinterSettings,
    shuck_path_resolver: Arc<LargeCorpusPathResolver>,
) -> LargeCorpusTimingCollection {
    collect_fixture_timing_records(fixtures, |fixture| {
        measure_fixture_timing(
            fixture,
            shuck_timeout,
            linter_settings,
            Arc::clone(&shuck_path_resolver),
        )
    })
}

fn collect_fixture_timing_records<F>(
    fixtures: &[&LargeCorpusFixture],
    evaluate: F,
) -> LargeCorpusTimingCollection
where
    F: Fn(&LargeCorpusFixture) -> LargeCorpusTimingRecord,
{
    let mut collection = LargeCorpusTimingCollection::default();
    let mut timeout_count = 0;

    for fixture in fixtures {
        let record = evaluate(fixture);
        if record.outcome == LargeCorpusTimingOutcome::Timeout {
            timeout_count += 1;
        }
        collection.records.push(record);

        if timeout_count >= LARGE_CORPUS_TIMEOUT_FAILURE_CAP {
            collection.timeout_cap_reached = true;
            break;
        }
    }

    collection
}

fn measure_fixture_timing(
    fixture: &LargeCorpusFixture,
    base_shuck_timeout: Duration,
    linter_settings: &shuck_linter::LinterSettings,
    shuck_path_resolver: Arc<LargeCorpusPathResolver>,
) -> LargeCorpusTimingRecord {
    let start = Instant::now();
    let outcome = match panic::catch_unwind(AssertUnwindSafe(|| {
        let source = fs::read(&fixture.path).unwrap_or_default();
        let shuck_timeout = effective_shuck_timeout(&source, base_shuck_timeout);
        run_shuck_with_timeout(
            fixture,
            linter_settings,
            shuck_timeout,
            Some(Arc::clone(&shuck_path_resolver)),
        )
    })) {
        Ok(Ok(run)) => {
            if run.parse_error.is_some() {
                LargeCorpusTimingOutcome::ParseError
            } else {
                LargeCorpusTimingOutcome::Ok
            }
        }
        Ok(Err(err)) => {
            if is_timeout_message(&err, "shuck") {
                LargeCorpusTimingOutcome::Timeout
            } else {
                LargeCorpusTimingOutcome::Error
            }
        }
        Err(_) => LargeCorpusTimingOutcome::Error,
    };

    LargeCorpusTimingRecord {
        fixture_label: fixture_progress_label(fixture),
        elapsed: start.elapsed(),
        outcome,
    }
}

fn ranked_large_corpus_timings(
    records: &[LargeCorpusTimingRecord],
) -> Vec<LargeCorpusTimingRecord> {
    let mut ranked = records.to_vec();
    ranked.sort_by(|left, right| {
        right
            .elapsed
            .cmp(&left.elapsed)
            .then_with(|| left.fixture_label.cmp(&right.fixture_label))
    });
    ranked.truncate(LARGE_CORPUS_TIMING_LIMIT);
    ranked
}

fn timing_timeout_cap_note() -> String {
    format!(
        "large corpus timing note: stopped after {} timed-out fixture(s) to avoid leaving additional timed-out workers running.",
        LARGE_CORPUS_TIMEOUT_FAILURE_CAP
    )
}

fn format_large_corpus_timing_report(collection: &LargeCorpusTimingCollection) -> String {
    let ranked = ranked_large_corpus_timings(&collection.records);
    if ranked.is_empty() {
        return "large corpus timing: no supported fixtures selected".into();
    }

    let mut lines = vec![format!(
        "large corpus timing: showing {} slowest shuck fixture(s) out of {} measured fixture(s)",
        ranked.len(),
        collection.records.len()
    )];
    lines.extend(ranked.iter().enumerate().map(|(index, record)| {
        format!(
            "{:>2}. {} [{}] {}",
            index + 1,
            format_fixture_elapsed(record.elapsed),
            record.outcome.as_str(),
            record.fixture_label
        )
    }));
    if collection.timeout_cap_reached {
        lines.push(timing_timeout_cap_note());
    }
    lines.join("\n")
}

fn select_supported_large_corpus_fixtures<'a>(
    fixtures: &'a [LargeCorpusFixture],
    shellcheck_supported_shells: Option<&HashMap<&'static str, ()>>,
) -> Vec<&'a LargeCorpusFixture> {
    fixtures
        .iter()
        .filter(|fixture| fixture_supported_for_large_corpus(fixture, shellcheck_supported_shells))
        .collect()
}

fn select_large_corpus_timing_fixtures(
    fixtures: &[LargeCorpusFixture],
) -> Vec<&LargeCorpusFixture> {
    fixtures
        .iter()
        .filter(|fixture| fixture_supported_for_large_corpus_timing(fixture))
        .collect()
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

    if cfg.timing_mode {
        let supported_fixtures = select_large_corpus_timing_fixtures(&fixtures);
        let linter_settings =
            build_large_corpus_linter_settings(cfg.selected_rules, cfg.mapped_only);
        let shuck_path_resolver = Arc::new(LargeCorpusPathResolver::new(&supported_fixtures));
        let timings = collect_fixture_timings(
            &supported_fixtures,
            cfg.shuck_timeout,
            &linter_settings,
            shuck_path_resolver,
        );
        eprintln!("{}", format_large_corpus_timing_report(&timings));
        return;
    }

    let shellcheck = probe_shellcheck()
        .expect("shellcheck not found on PATH; install it to run the large corpus test");

    let supported_shells = shellcheck_supported_shells(&shellcheck.command);
    let shellcheck_index = build_rule_to_shellcheck_index(cfg.selected_rules.as_ref());
    let shellcheck_rule_index = build_shellcheck_to_rule_index(cfg.selected_rules.as_ref());
    let corpus_metadata = load_all_rule_corpus_metadata();
    let shellcheck_filter_codes =
        build_shellcheck_filter_codes(cfg.selected_rules, cfg.mapped_only);
    let shellcheck_cache = ShellCheckCache::new(&cfg.cache_dir, &shellcheck);
    shellcheck_cache.prepare(&fixtures, &discover_worktree_roots());
    let linter_settings =
        build_large_corpus_compat_linter_settings(cfg.selected_rules, cfg.mapped_only);
    let supported_fixtures =
        select_supported_large_corpus_fixtures(&fixtures, Some(&supported_shells));
    let skipped_unsupported_shells = fixtures.len().saturating_sub(supported_fixtures.len());
    let shuck_path_resolver = None;

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
                cfg.selected_rules.as_ref(),
                shellcheck_filter_codes.as_ref(),
                shuck_path_resolver.clone(),
            )
        });
    let mut failure_collection = failure_collection;
    failure_collection.unsupported_shells = skipped_unsupported_shells;

    eprintln!(
        "large corpus compatibility summary: blocking={} warnings={} fixtures={} unsupported_shells={} implementation_diffs={} mapping_issues={} reviewed_divergences={} harness_warnings={} harness_failures={}",
        failure_collection.blocking_failures(),
        failure_collection.nonblocking_issue_count(),
        fixtures.len(),
        skipped_unsupported_shells,
        failure_collection.implementation_diffs.len(),
        failure_collection.mapping_issues.len(),
        failure_collection.reviewed_divergences.len(),
        failure_collection.harness_warnings.len(),
        failure_collection.harness_failures.len(),
    );
    if failure_collection.blocking_failures() == 0
        && failure_collection.nonblocking_issue_count() > 0
    {
        eprintln!("{}", format_large_corpus_report(&failure_collection));
    }

    assert!(
        failure_collection.blocking_failures() == 0,
        "large corpus compatibility had {} blocking issue(s) across {} fixture(s) ({} skipped unsupported shells):\n\n{}",
        failure_collection.blocking_failures(),
        fixtures.len(),
        skipped_unsupported_shells,
        format_large_corpus_failure_report(&failure_collection)
    );
}

#[test]
#[ignore = "requires the large corpus; run `make test-large-corpus-zsh`"]
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
    assert!(
        failure_collection.blocking_failures() == 0,
        "large corpus zsh parse had {} blocking issue(s) across {} fixture(s):\n\n{}",
        failure_collection.blocking_failures(),
        zsh_fixtures.len(),
        format_large_corpus_failure_report(&failure_collection)
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
            large_corpus_worker_count(fixtures.len()),
            &evaluate,
            &progress,
        );
    }

    collect_fixture_failures_sequential(fixtures, &evaluate, &progress)
}

fn large_corpus_worker_count(fixtures_len: usize) -> usize {
    let available_parallelism = thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);

    clamp_large_corpus_worker_count(available_parallelism, fixtures_len)
}

fn clamp_large_corpus_worker_count(available_parallelism: usize, fixtures_len: usize) -> usize {
    available_parallelism
        .min(LARGE_CORPUS_MAX_WORKER_COUNT)
        .min(fixtures_len)
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
        let has_blocking = evaluation
            .harness_failure
            .as_ref()
            .is_some_and(|failure| failure.kind.blocks())
            || !evaluation.implementation_diffs.is_empty();
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
            panic!("{}", format_large_corpus_failure_report(&collection));
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
    let collection = Mutex::new(Vec::<(usize, FixtureEvaluation)>::new());

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let collection = &collection;
            let next_index = &next_index;
            scope.spawn(move || {
                let mut local_evaluations = Vec::new();
                loop {
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
    shellcheck_rule_index: &HashMap<u32, Vec<String>>,
    corpus_metadata: &HashMap<String, RuleCorpusMetadataDocument>,
    selected_rules: Option<&shuck_linter::RuleSet>,
    shellcheck_filter_codes: Option<&HashSet<u32>>,
    shuck_path_resolver: Option<Arc<LargeCorpusPathResolver>>,
) -> FixtureEvaluation {
    let mut evaluation = FixtureEvaluation::default();
    let src = fs::read(&fixture.path).unwrap_or_default();
    let shuck_timeout = effective_shuck_timeout(&src, shuck_timeout);

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
    let shellcheck_run = match shuck_run.parse_error.as_deref() {
        Some(err) if shuck_parse_aborted(err) => {
            match shellcheck_cache.run_fixture(fixture, shellcheck_path, shellcheck_timeout) {
                Ok(run) => Some(filter_shellcheck_run(run, shellcheck_filter_codes)),
                Err(err) => {
                    evaluation.harness_failure = Some(FixtureFailure {
                        kind: fixture_failure_kind_for_message(&err, "shellcheck"),
                        message: format_fixture_failure(
                            &fixture.path,
                            &[format!("shellcheck error: {err}")],
                        ),
                    });
                    return evaluation;
                }
            }
        }
        _ => None,
    };
    if let Some(ref err) = shuck_run.parse_error {
        if shuck_parse_aborted(err) {
            let Some(sc_run) = shellcheck_run.as_ref() else {
                unreachable!("parse-aborted shuck runs should probe shellcheck")
            };
            let mut details = vec![format!("shuck parse error: {err}")];
            if sc_run.parse_aborted {
                details.push("shellcheck parse aborted".into());
            } else {
                details.push("shellcheck parsed fixture successfully".into());
            }
            evaluation.harness_failure = Some(FixtureFailure {
                kind: shuck_parse_failure_kind(selected_rules, shellcheck_filter_codes),
                message: format_fixture_failure(&fixture.path, &details),
            });
        } else {
            evaluation.harness_failure = Some(FixtureFailure {
                kind: shuck_parse_failure_kind(selected_rules, shellcheck_filter_codes),
                message: format_fixture_failure(&fixture.path, &[format!("shuck error: {err}")]),
            });
        }
        return evaluation;
    }

    match shellcheck_run.map(Ok).unwrap_or_else(|| {
        shellcheck_cache
            .run_fixture(fixture, shellcheck_path, shellcheck_timeout)
            .map(|run| filter_shellcheck_run(run, shellcheck_filter_codes))
    }) {
        Ok(sc_run) => {
            if sc_run.parse_aborted {
                let details = vec!["shellcheck parse aborted".into()];
                evaluation.harness_failure = Some(FixtureFailure {
                    kind: FixtureFailureKind::Other,
                    message: format_fixture_failure(&fixture.path, &details),
                });
                return evaluation;
            }

            let shellcheck_records =
                shellcheck_compatibility_records(&sc_run.diagnostics, shellcheck_rule_index);
            let shuck_records =
                shuck_compatibility_records(&shuck_run.diagnostics, shellcheck_index);
            let (shellcheck_only, shuck_only) =
                unmatched_compatibility_records(&shellcheck_records, &shuck_records);

            let cache_rel_path = fixture.cache_rel_path_key();
            let mut implementation_details = Vec::new();
            let mut mapping_details = Vec::new();
            let mut reviewed_details = Vec::new();

            for record in shellcheck_only.into_iter().chain(shuck_only) {
                let (classification, reason) =
                    classify_compatibility_record(&record, &cache_rel_path, corpus_metadata);
                let detail = format_compatibility_record(&record, reason.as_deref());
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
        match probe_invalid_zsh_fixture(&fixture.path, shuck_timeout) {
            Ok(Some(zsh_err)) => {
                evaluation.harness_failure = Some(FixtureFailure {
                    kind: FixtureFailureKind::Other,
                    message: format_fixture_failure(
                        &fixture.path,
                        &[
                            format!("shuck zsh parse/option extraction error: {err}"),
                            format!("zsh -n parse error: {zsh_err}"),
                        ],
                    ),
                });
            }
            Ok(None) | Err(_) => {
                evaluation.harness_failure = Some(FixtureFailure {
                    kind: FixtureFailureKind::Other,
                    message: format_fixture_failure(
                        &fixture.path,
                        &[format!("shuck zsh parse/option extraction error: {err}")],
                    ),
                });
            }
        }
    }

    evaluation
}

fn probe_invalid_zsh_fixture(path: &Path, timeout: Duration) -> Result<Option<String>, String> {
    let path = path.to_path_buf();
    let status = run_with_timeout("zsh", timeout, move || {
        Command::new("zsh")
            .arg("-n")
            .arg(&path)
            .output()
            .map_err(|err| format!("failed to run `zsh -n`: {err}"))
    })??;

    if status.status.success() {
        return Ok(None);
    }

    let stderr = String::from_utf8_lossy(&status.stderr).trim().to_owned();
    if stderr.is_empty() || !zsh_stderr_looks_like_parse_error(&stderr) {
        return Ok(None);
    }

    Ok(Some(stderr))
}

fn zsh_stderr_looks_like_parse_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("parse error") || lower.contains("unmatched ")
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
    run.parse_aborted |= shellcheck_parse_aborted(&run.diagnostics);
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
    if let Some(failure) = evaluation.harness_failure {
        match failure.kind {
            FixtureFailureKind::Warning => collection.harness_warnings.push(failure.message),
            FixtureFailureKind::Other | FixtureFailureKind::Timeout => {
                collection.harness_failures.push(failure.message)
            }
        }
    }
}

fn shuck_parse_failure_kind(
    selected_rules: Option<&shuck_linter::RuleSet>,
    shellcheck_filter_codes: Option<&HashSet<u32>>,
) -> FixtureFailureKind {
    if large_corpus_uses_single_file_c001_oracle(selected_rules, shellcheck_filter_codes) {
        FixtureFailureKind::Warning
    } else {
        FixtureFailureKind::Other
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
    let metadata: RuleCorpusMetadataDocument =
        serde_yaml::from_str(&data).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()));
    metadata
        .validate(rule_code, &path)
        .unwrap_or_else(|err| panic!("{err}"));
    metadata
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
            if metadata.has_entries() {
                map.insert(rule_code, metadata);
            }
        }
    }
    map
}

fn reviewed_divergence_reason<'a>(
    metadata: &'a RuleCorpusMetadataDocument,
    record: &CompatibilityRecord,
    cache_rel_path: &str,
) -> Option<&'a str> {
    let cache_rel_path_variants = cache_rel_path_match_variants(cache_rel_path);
    metadata.reviewed_divergences.iter().find_map(|entry| {
        (entry.side == record.side
            && entry.path_suffix.as_ref().is_none_or(|suffix| {
                cache_rel_path_variants
                    .iter()
                    .any(|candidate| candidate.ends_with(suffix))
            })
            && entry.path_contains.as_ref().is_none_or(|needle| {
                cache_rel_path_variants
                    .iter()
                    .any(|candidate| candidate.contains(needle))
            })
            && entry.line.is_none_or(|line| line == record.range.line)
            && entry
                .end_line
                .is_none_or(|end_line| end_line == record.range.end_line)
            && entry
                .column
                .is_none_or(|column| column == record.range.column)
            && entry
                .end_column
                .is_none_or(|end_column| end_column == record.range.end_column))
        .then_some(entry.reason.as_str())
    })
}

fn cache_rel_path_match_variants(cache_rel_path: &str) -> [String; 3] {
    [
        cache_rel_path.to_owned(),
        format!("scripts/{cache_rel_path}"),
        format!("corpus/scripts/{cache_rel_path}"),
    ]
}

fn classify_compatibility_record(
    record: &CompatibilityRecord,
    cache_rel_path: &str,
    corpus_metadata: &HashMap<String, RuleCorpusMetadataDocument>,
) -> (CompatibilityClassification, Option<String>) {
    let rule_codes = compatibility_record_rule_codes(record);
    if rule_codes.is_empty() {
        return (
            CompatibilityClassification::MappingIssue,
            Some(format!("no Shuck rule maps {}", record.shellcheck_code)),
        );
    }
    if rule_codes.iter().all(|rule_code| {
        corpus_metadata
            .get(*rule_code)
            .is_some_and(|metadata| metadata.review_all_divergences)
    }) {
        return (
            CompatibilityClassification::ReviewedDivergence,
            Some(LARGE_CORPUS_ALLOWED_FAILING_RULE_REASON.to_owned()),
        );
    }
    let mut reviewed_reason = None;

    for rule_code in rule_codes {
        let Some(metadata) = corpus_metadata.get(rule_code) else {
            return (CompatibilityClassification::Implementation, None);
        };

        if let Some(reason) = reviewed_divergence_reason(metadata, record, cache_rel_path) {
            reviewed_reason.get_or_insert_with(|| reason.to_owned());
            continue;
        }

        return (CompatibilityClassification::Implementation, None);
    }

    if let Some(reason) = reviewed_reason {
        return (
            CompatibilityClassification::ReviewedDivergence,
            Some(reason),
        );
    }

    (CompatibilityClassification::Implementation, None)
}
fn shellcheck_compatibility_records(
    diagnostics: &[ShellCheckDiagnostic],
    shellcheck_rule_index: &HashMap<u32, Vec<String>>,
) -> Vec<CompatibilityRecord> {
    diagnostics
        .iter()
        .map(|diag| {
            let rule_codes = shellcheck_rule_index
                .get(&diag.code)
                .cloned()
                .unwrap_or_default();
            CompatibilityRecord {
                side: CompatibilitySide::ShellcheckOnly,
                rule_code: (rule_codes.len() == 1).then(|| rule_codes[0].clone()),
                rule_codes,
                shellcheck_code: format!("SC{:04}", diag.code),
                range: DiagnosticRange {
                    line: diag.line,
                    end_line: diag.end_line,
                    column: diag.column,
                    end_column: diag.end_column,
                },
                message: format!("{} {}", diag.level, diag.message),
            }
        })
        .collect()
}

fn shuck_compatibility_records(
    diagnostics: &[shuck_linter::Diagnostic],
    shellcheck_index: &HashMap<String, String>,
) -> Vec<CompatibilityRecord> {
    diagnostics
        .iter()
        .filter_map(|diag| {
            shellcheck_index
                .get(diag.code())
                .map(|shellcheck_code| CompatibilityRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_code: Some(diag.code().to_owned()),
                    rule_codes: Vec::new(),
                    shellcheck_code: shellcheck_code.clone(),
                    range: DiagnosticRange {
                        line: diag.span.start.line,
                        end_line: diag.span.end.line,
                        column: diag.span.start.column,
                        end_column: diag.span.end.column,
                    },
                    message: diag.message.clone(),
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

fn compatibility_record_rule_codes(record: &CompatibilityRecord) -> Vec<&str> {
    if !record.rule_codes.is_empty() {
        return record.rule_codes.iter().map(String::as_str).collect();
    }

    record.rule_code.iter().map(String::as_str).collect()
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

fn shuck_parse_aborted(error: &str) -> bool {
    !error.starts_with("read error:")
}

fn format_compatibility_record(record: &CompatibilityRecord, reason: Option<&str>) -> String {
    let reason = reason
        .map(|reason| format!(" reason={reason}"))
        .unwrap_or_default();
    let rule_codes = compatibility_record_rule_codes(record);
    let rule_code = if rule_codes.is_empty() {
        "(unmapped)".to_owned()
    } else {
        rule_codes.join(",")
    };
    format!(
        "{} {}/{} {} {}{}",
        record.side.as_str(),
        rule_code,
        record.shellcheck_code,
        record.range.display(),
        record.message,
        reason,
    )
}

fn format_large_corpus_failure_report(collection: &FixtureFailureCollection) -> String {
    let report =
        format_large_corpus_report_with_mode(collection, LargeCorpusReportMode::BlockingOnly);
    if !collection.has_nonblocking_items() {
        return report;
    }

    format!(
        "{}\n\nNonblocking issue buckets were omitted from the failing log output. See the compatibility summary counts above for skipped unsupported shells, mapping issues, reviewed divergences, and harness warnings.",
        report
    )
}

fn format_large_corpus_report(collection: &FixtureFailureCollection) -> String {
    format_large_corpus_report_with_mode(collection, LargeCorpusReportMode::Full)
}

fn format_large_corpus_report_with_mode(
    collection: &FixtureFailureCollection,
    mode: LargeCorpusReportMode,
) -> String {
    let mut sections = Vec::new();

    if let Some(section) =
        format_report_section("Implementation Diffs", &collection.implementation_diffs)
    {
        sections.push(section);
    }
    if matches!(mode, LargeCorpusReportMode::Full) {
        if let Some(section) = format_report_section("Mapping Issues", &collection.mapping_issues) {
            sections.push(section);
        }
        if let Some(section) =
            format_report_section("Reviewed Divergence", &collection.reviewed_divergences)
        {
            sections.push(section);
        }

        if let Some(section) =
            format_report_section("Harness Warnings", &collection.harness_warnings)
        {
            sections.push(section);
        }
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
    fixture_supported_for_large_corpus_with_options(fixture, shellcheck_supported_shells, true)
}

fn fixture_supported_for_large_corpus_timing(fixture: &LargeCorpusFixture) -> bool {
    fixture_supported_for_large_corpus_with_options(fixture, None, false)
}

fn fixture_supported_for_large_corpus_with_options(
    fixture: &LargeCorpusFixture,
    shellcheck_supported_shells: Option<&HashMap<&'static str, ()>>,
    exclude_shellcheck_parse_ignored: bool,
) -> bool {
    if path_is_statically_ignored_large_corpus_fixture(&fixture.path)
        || path_is_shellcheck_unsupported_large_corpus_fixture(&fixture.path)
        || path_is_sample_file(&fixture.path)
        || path_is_fish_file(&fixture.path)
        || path_is_patch_file(&fixture.path)
        || path_is_guess_file(&fixture.path)
        || path_is_config_sub_file(&fixture.path)
        || fixture_is_repo_git_entry(fixture)
    {
        return false;
    }

    if exclude_shellcheck_parse_ignored
        && path_is_ignored_large_corpus_shellcheck_parse_fixture(&fixture.path)
    {
        return false;
    }

    shell_supported_for_large_corpus(
        effective_large_corpus_shell(fixture),
        shellcheck_supported_shells,
    )
}

fn fixture_selected_for_large_corpus_zsh_parse(fixture: &LargeCorpusFixture) -> bool {
    if path_is_statically_ignored_large_corpus_fixture(&fixture.path)
        || path_is_sample_file(&fixture.path)
        || path_is_fish_file(&fixture.path)
        || path_is_patch_file(&fixture.path)
        || path_is_guess_file(&fixture.path)
        || path_is_config_sub_file(&fixture.path)
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
        .unwrap_or_else(|| default_supported_large_corpus_shell(shell))
}

fn default_supported_large_corpus_shell(shell: &str) -> bool {
    matches!(shell, "sh" | "bash" | "dash" | "ksh")
}

fn effective_large_corpus_shell(fixture: &LargeCorpusFixture) -> &str {
    if fixture_looks_like_zsh(fixture) {
        "zsh"
    } else {
        fixture.shell.as_str()
    }
}

fn fixture_looks_like_zsh(fixture: &LargeCorpusFixture) -> bool {
    if path_has_large_corpus_static_zsh_override(&fixture.path) {
        return true;
    }

    let Some(name) = fixture.path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    name.ends_with(".zsh") || name.ends_with(".zsh-theme") || name.starts_with(".zsh")
}

fn path_matches_large_corpus_suffix(path: &Path, suffixes: &[&str]) -> bool {
    let path = path.to_string_lossy();
    suffixes.iter().any(|suffix| path.ends_with(suffix))
}

fn path_is_statically_ignored_large_corpus_fixture(path: &Path) -> bool {
    path_matches_large_corpus_suffix(path, LARGE_CORPUS_STATIC_IGNORE_SUFFIXES)
}

fn path_is_ignored_large_corpus_shellcheck_parse_fixture(path: &Path) -> bool {
    path_matches_large_corpus_suffix(path, LARGE_CORPUS_SHELLCHECK_PARSE_IGNORE_SUFFIXES)
}

fn path_has_large_corpus_static_zsh_override(path: &Path) -> bool {
    path_matches_large_corpus_suffix(path, LARGE_CORPUS_STATIC_ZSH_OVERRIDE_SUFFIXES)
}

fn path_is_shellcheck_unsupported_large_corpus_fixture(path: &Path) -> bool {
    path.components().any(|component| {
        let std::path::Component::Normal(part) = component else {
            return false;
        };

        let part = part.to_string_lossy();
        LARGE_CORPUS_SHELLCHECK_UNSUPPORTED_REPO_PREFIXES
            .iter()
            .any(|prefix| part.starts_with(prefix))
    })
}

fn path_is_sample_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".sample"))
}

fn path_is_patch_file(path: &Path) -> bool {
    let path = path.to_string_lossy().to_lowercase();
    path.ends_with(".patch") || path.ends_with(".diff") || path.ends_with(".dpatch")
}

fn path_is_fish_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fish"))
}

fn path_is_appledouble_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("._"))
}

fn path_is_guess_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".guess"))
}

fn path_is_config_sub_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with("config.sub"))
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
                timing_mode: env_truthy(LARGE_CORPUS_TIMING_ENV, false),
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
    // crates/shuck-cli -> workspace root
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
        if path_is_statically_ignored_large_corpus_fixture(&path)
            || path_is_sample_file(&path)
            || path_is_fish_file(&path)
            || path_is_patch_file(&path)
            || path_is_appledouble_file(&path)
            || path_is_guess_file(&path)
            || path_is_config_sub_file(&path)
        {
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
    let source = String::from_utf8_lossy(src);
    let source = source.strip_prefix('\u{feff}').unwrap_or(source.as_ref());
    let trimmed_first_line = source
        .lines()
        .next()
        .map(|line| line.trim_start().to_ascii_lowercase())
        .unwrap_or_default();

    if trimmed_first_line.starts_with("#compdef") || trimmed_first_line.starts_with("#autoload") {
        return "zsh".into();
    }
    if path_has_large_corpus_static_zsh_override(path) {
        return "zsh".into();
    }

    match shuck_linter::ShellDialect::infer(source, Some(path)) {
        shuck_linter::ShellDialect::Bash => "bash".into(),
        shuck_linter::ShellDialect::Ksh | shuck_linter::ShellDialect::Mksh => "ksh".into(),
        shuck_linter::ShellDialect::Zsh => "zsh".into(),
        shuck_linter::ShellDialect::Sh | shuck_linter::ShellDialect::Dash => "sh".into(),
        shuck_linter::ShellDialect::Unknown => {
            unsupported_large_corpus_shebang_shell(&trimmed_first_line)
                .map(|shell| shell.to_ascii_lowercase())
                .unwrap_or_else(|| "sh".into())
        }
    }
}

fn unsupported_large_corpus_shebang_shell(first_line: &str) -> Option<&str> {
    let interpreter = shuck_parser::shebang::interpreter_name(first_line)?;

    match interpreter.to_ascii_lowercase().as_str() {
        "fish" | "csh" | "tcsh" => Some(interpreter),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shuck runner
// ---------------------------------------------------------------------------

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
    let parsed = shuck_parser::parser::Parser::with_dialect(&source, parse_dialect).parse();
    let diagnostics = lint_large_corpus_output(
        fixture,
        &source,
        &parsed,
        &linter_settings,
        source_path_resolver,
    );
    ShuckRun {
        diagnostics,
        parse_error: parsed.is_err().then(|| parsed.strict_error().to_string()),
    }
}

fn lint_large_corpus_output(
    fixture: &LargeCorpusFixture,
    source: &str,
    parse_result: &shuck_parser::parser::ParseResult,
    linter_settings: &shuck_linter::LinterSettings,
    source_path_resolver: Option<&(dyn shuck_semantic::SourcePathResolver + Send + Sync)>,
) -> Vec<shuck_linter::Diagnostic> {
    let indexer = shuck_indexer::Indexer::new_arena(source, &parse_result.arena_file);
    let shellcheck_map = shuck_linter::ShellCheckCodeMap::default();
    let directives = shuck_linter::parse_directives(
        source,
        &parse_result.arena_file,
        indexer.comment_index(),
        &shellcheck_map,
    );
    let suppression_index = (!directives.is_empty()).then(|| {
        shuck_linter::SuppressionIndex::new(
            &directives,
            &parse_result.arena_file,
            shuck_linter::first_statement_line(&parse_result.arena_file).unwrap_or(u32::MAX),
        )
    });

    shuck_linter::lint_file_at_path_with_resolver_and_parse_result(
        parse_result,
        source,
        &indexer,
        linter_settings,
        suppression_index.as_ref(),
        Some(&fixture.path),
        source_path_resolver,
    )
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
        shuck_linter::ShellDialect::from_name(&fixture.shell).parser_dialect(),
        &fixture.shell,
    )
}

fn run_shuck_with_timeout(
    fixture: &LargeCorpusFixture,
    linter_settings: &shuck_linter::LinterSettings,
    timeout: Duration,
    source_path_resolver: Option<Arc<LargeCorpusPathResolver>>,
) -> Result<ShuckRun, String> {
    let fixture = fixture.clone();
    let linter_settings = linter_settings.clone();
    run_with_timeout("shuck", timeout, move || {
        let source_path_resolver = source_path_resolver.as_ref().map(|resolver| {
            resolver.as_ref() as &(dyn shuck_semantic::SourcePathResolver + Send + Sync)
        });
        run_shuck(&fixture, &linter_settings, source_path_resolver)
    })
}

fn parse_fixture_for_effective_large_corpus_shell_with_timeout(
    fixture: &LargeCorpusFixture,
    timeout: Duration,
) -> Result<Result<(), String>, String> {
    let fixture = fixture.clone();
    let source = match fs::read_to_string(&fixture.path) {
        Ok(source) => source,
        Err(err) => return Ok(Err(format!("read error: {err}"))),
    };
    let timeout = effective_shuck_timeout(source.as_bytes(), timeout);
    run_with_timeout("shuck", timeout, move || {
        let shell = effective_large_corpus_shell(&fixture);
        let shell_profile = shuck_linter::ShellDialect::from_name(shell).shell_profile();
        let parsed =
            shuck_parser::parser::Parser::with_profile(&source, shell_profile.clone()).parse();
        if parsed.is_err() {
            return Err(parsed.strict_error().to_string());
        }

        if shell == "zsh" {
            extract_large_corpus_zsh_option_state(&fixture.path, &source, &parsed, shell_profile)?;
        }

        Ok(())
    })
}

fn extract_large_corpus_zsh_option_state(
    path: &Path,
    source: &str,
    output: &shuck_parser::parser::ParseResult,
    shell_profile: shuck_parser::ShellProfile,
) -> Result<(), String> {
    let indexer = shuck_indexer::Indexer::new(source, output);
    let semantic = shuck_semantic::SemanticModel::build_arena_with_options(
        &output.arena_file,
        source,
        &indexer,
        shuck_semantic::SemanticBuildOptions {
            source_path: Some(path),
            shell_profile: Some(shell_profile),
            ..shuck_semantic::SemanticBuildOptions::default()
        },
    );

    if semantic.shell_profile().zsh_options().is_none() {
        return Err("semantic model did not retain a zsh shell profile".to_owned());
    }

    for stmt in output.file.body.iter() {
        if semantic.zsh_options_at(stmt.span.start.offset).is_none() {
            return Err(format!(
                "missing zsh option snapshot at {}:{}",
                stmt.span.start.line, stmt.span.start.column
            ));
        }
    }

    Ok(())
}

fn build_rule_to_shellcheck_index(
    selected_rules: Option<&shuck_linter::RuleSet>,
) -> HashMap<String, String> {
    let shellcheck_map = shuck_linter::ShellCheckCodeMap::default();

    if let Some(selected_rules) = selected_rules {
        return selected_rules
            .iter()
            .filter_map(|rule| {
                shellcheck_map
                    .code_for_rule(rule)
                    .map(|sc_code| (rule.code().to_owned(), format!("SC{sc_code}")))
            })
            .collect();
    }

    shellcheck_map
        .mappings()
        .map(|(sc_code, rule)| (rule.code().to_owned(), format!("SC{sc_code}")))
        .collect::<HashMap<_, _>>()
}

fn build_shellcheck_to_rule_index(
    selected_rules: Option<&shuck_linter::RuleSet>,
) -> HashMap<u32, Vec<String>> {
    let shellcheck_map = shuck_linter::ShellCheckCodeMap::default();

    if let Some(selected_rules) = selected_rules {
        let mut index = HashMap::<u32, Vec<String>>::new();
        for rule in selected_rules.iter() {
            if let Some(sc_code) = shellcheck_map.code_for_rule(rule) {
                index
                    .entry(sc_code)
                    .or_default()
                    .push(rule.code().to_owned());
            }
        }
        for rule_codes in index.values_mut() {
            rule_codes.sort();
            rule_codes.dedup();
        }
        return index;
    }

    let mut index = HashMap::<u32, Vec<String>>::new();
    for (sc_code, rule) in shellcheck_map.mappings() {
        index
            .entry(sc_code)
            .or_default()
            .push(rule.code().to_owned());
    }
    for rule_codes in index.values_mut() {
        rule_codes.sort();
        rule_codes.dedup();
    }
    index
}

fn build_large_corpus_linter_settings(
    selected_rules: Option<shuck_linter::RuleSet>,
    mapped_only: bool,
) -> shuck_linter::LinterSettings {
    let settings = if let Some(rules) = selected_rules {
        shuck_linter::LinterSettings::for_rules(rules.iter())
    } else if mapped_only {
        let mapped_rules: HashSet<_> = shuck_linter::ShellCheckCodeMap::default()
            .mappings()
            .map(|(_, rule)| rule)
            .collect();
        shuck_linter::LinterSettings::for_rules(mapped_rules)
    } else {
        shuck_linter::LinterSettings::default()
    };

    settings.with_c063_report_unreached_nested_definitions(true)
}

fn build_large_corpus_compat_linter_settings(
    selected_rules: Option<shuck_linter::RuleSet>,
    mapped_only: bool,
) -> shuck_linter::LinterSettings {
    build_large_corpus_linter_settings(selected_rules, mapped_only)
        .with_resolve_source_closure(false)
}

fn large_corpus_uses_single_file_c001_oracle(
    selected_rules: Option<&shuck_linter::RuleSet>,
    shellcheck_filter_codes: Option<&HashSet<u32>>,
) -> bool {
    if let Some(rules) = selected_rules {
        return rules.len() == 1 && rules.contains(shuck_linter::Rule::UnusedAssignment);
    }

    shellcheck_filter_codes.is_some_and(|codes| {
        codes.len() == 1
            && shuck_linter::ShellCheckCodeMap::default()
                .code_for_rule(shuck_linter::Rule::UnusedAssignment)
                .is_some_and(|code| codes.contains(&code))
    })
}

fn build_shellcheck_filter_codes(
    selected_rules: Option<shuck_linter::RuleSet>,
    mapped_only: bool,
) -> Option<HashSet<u32>> {
    selected_rules
        .map(|rules| {
            validate_selected_rules_for_large_corpus(&rules)
                .unwrap_or_else(|err| panic!("{LARGE_CORPUS_RULES_ENV}, {err}"));
            build_selected_shellcheck_codes(&rules)
        })
        .or_else(|| mapped_only.then(build_mapped_shellcheck_codes))
}

fn build_mapped_shellcheck_codes() -> HashSet<u32> {
    shuck_linter::ShellCheckCodeMap::default()
        .mappings()
        .map(|(sc_code, _)| sc_code)
        .collect()
}

fn build_selected_shellcheck_codes(selected_rules: &shuck_linter::RuleSet) -> HashSet<u32> {
    let shellcheck_map = shuck_linter::ShellCheckCodeMap::default();

    selected_rules
        .iter()
        .filter_map(|rule| shellcheck_map.code_for_rule(rule))
        .collect()
}

fn validate_selected_rules_for_large_corpus(
    selected_rules: &shuck_linter::RuleSet,
) -> Result<(), String> {
    let _ = selected_rules;
    // Selected large-corpus runs may target rules without an active ShellCheck comparison code.
    // Those runs simply execute with an empty ShellCheck filter and compare no records.
    Ok(())
}

// ---------------------------------------------------------------------------
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
    fn resolve_shell_env_split_bash_shebang() {
        assert_eq!(
            resolve_shell(Path::new("script"), b"#!/usr/bin/env -S bash -e\n"),
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
    fn resolve_shell_fish_shebang_stays_unsupported() {
        assert_eq!(
            resolve_shell(Path::new("script"), b"#!/usr/bin/env fish\n"),
            "fish"
        );
    }

    #[test]
    fn resolve_shell_env_split_fish_shebang_stays_unsupported() {
        assert_eq!(
            resolve_shell(Path::new("script"), b"#!/usr/bin/env -S fish -e\n"),
            "fish"
        );
    }

    #[test]
    fn resolve_shell_csh_shebang_stays_unsupported() {
        assert_eq!(resolve_shell(Path::new("script"), b"#!/bin/csh\n"), "csh");
    }

    #[test]
    fn resolve_shell_respects_shellcheck_header_over_unsupported_shebang() {
        assert_eq!(
            resolve_shell(
                Path::new("script"),
                b"#!/usr/bin/env fish\n# shellcheck shell=bash\necho hi\n",
            ),
            "bash"
        );
    }

    #[test]
    fn resolve_shell_compdef_header_is_zsh() {
        assert_eq!(resolve_shell(Path::new("_wd.sh"), b"#compdef wd\n"), "zsh");
    }

    #[test]
    fn resolve_shell_static_zsh_override_is_zsh() {
        assert_eq!(
            resolve_shell(
                Path::new("ohmyzsh__ohmyzsh__oh-my-zsh.sh"),
                b"# comment only\n",
            ),
            "zsh"
        );
    }

    #[test]
    fn resolve_shell_bash_extension_fallback() {
        assert_eq!(
            resolve_shell(Path::new("example.bash"), b"echo hi\n"),
            "bash"
        );
    }

    #[test]
    fn resolve_shell_bom_prefixed_bash_shebang_without_extension() {
        assert_eq!(
            resolve_shell(
                Path::new("example"),
                b"\xEF\xBB\xBF#!/usr/bin/env bash\necho hi\n",
            ),
            "bash"
        );
    }

    #[test]
    fn resolve_shell_ignores_generic_shell_script_comments_for_bash_files() {
        assert_eq!(
            resolve_shell(
                Path::new("example.bash"),
                b"# -*- shell-script -*-\nfor ((i = 0; i < 5; i++)); do :; done\n",
            ),
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
    fn ranked_large_corpus_timings_sorts_descending_and_truncates_to_limit() {
        let records = (0..30)
            .map(|i| {
                timing_record(
                    &format!("script-{i:02}.sh"),
                    (i + 1) as u64,
                    LargeCorpusTimingOutcome::Ok,
                )
            })
            .collect::<Vec<_>>();

        let ranked = ranked_large_corpus_timings(&records);

        assert_eq!(ranked.len(), LARGE_CORPUS_TIMING_LIMIT);
        assert_eq!(ranked[0].fixture_label, "script-29.sh");
        assert_eq!(ranked[0].elapsed, Duration::from_millis(30));
        assert_eq!(ranked.last().unwrap().fixture_label, "script-05.sh");
    }

    #[test]
    fn ranked_large_corpus_timings_break_ties_by_fixture_label() {
        let ranked = ranked_large_corpus_timings(&[
            timing_record("zeta.sh", 50, LargeCorpusTimingOutcome::Ok),
            timing_record("alpha.sh", 50, LargeCorpusTimingOutcome::Timeout),
            timing_record("mid.sh", 50, LargeCorpusTimingOutcome::Error),
        ]);

        assert_eq!(
            ranked
                .iter()
                .map(|record| record.fixture_label.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha.sh", "mid.sh", "zeta.sh"]
        );
    }

    #[test]
    fn ranked_large_corpus_timings_keep_non_ok_statuses() {
        let ranked = ranked_large_corpus_timings(&[
            timing_record("timeout.sh", 90, LargeCorpusTimingOutcome::Timeout),
            timing_record("error.sh", 80, LargeCorpusTimingOutcome::Error),
            timing_record("ok.sh", 70, LargeCorpusTimingOutcome::Ok),
        ]);

        assert_eq!(
            ranked
                .iter()
                .map(|record| record.outcome)
                .collect::<Vec<_>>(),
            vec![
                LargeCorpusTimingOutcome::Timeout,
                LargeCorpusTimingOutcome::Error,
                LargeCorpusTimingOutcome::Ok,
            ]
        );
    }

    #[test]
    fn format_large_corpus_timing_report_handles_fewer_than_limit() {
        let report = format_large_corpus_timing_report(&LargeCorpusTimingCollection {
            records: vec![
                timing_record("slow.sh", 900, LargeCorpusTimingOutcome::Ok),
                timing_record("faster.sh", 300, LargeCorpusTimingOutcome::Ok),
            ],
            timeout_cap_reached: false,
        });

        assert!(report.contains("showing 2 slowest shuck fixture(s) out of 2 measured fixture(s)"));
        assert!(report.contains("1. 900ms [ok] slow.sh"));
        assert!(report.contains("2. 300ms [ok] faster.sh"));
    }

    #[test]
    fn format_large_corpus_timing_report_includes_status_labels() {
        let report = format_large_corpus_timing_report(&LargeCorpusTimingCollection {
            records: vec![
                timing_record("ok.sh", 40, LargeCorpusTimingOutcome::Ok),
                timing_record("parse.sh", 30, LargeCorpusTimingOutcome::ParseError),
                timing_record("timeout.sh", 20, LargeCorpusTimingOutcome::Timeout),
                timing_record("error.sh", 10, LargeCorpusTimingOutcome::Error),
            ],
            timeout_cap_reached: false,
        });

        assert!(report.contains("[ok] ok.sh"));
        assert!(report.contains("[parse-error] parse.sh"));
        assert!(report.contains("[timeout] timeout.sh"));
        assert!(report.contains("[error] error.sh"));
    }

    #[test]
    fn format_large_corpus_timing_report_handles_empty_input() {
        assert_eq!(
            format_large_corpus_timing_report(&LargeCorpusTimingCollection::default()),
            "large corpus timing: no supported fixtures selected"
        );
    }

    #[test]
    fn collect_fixture_timing_records_stops_after_timeout_cap() {
        let fixtures = (0..10)
            .map(|i| fixture(&format!("timeout-{i}.sh")))
            .collect::<Vec<_>>();
        let fixture_refs = fixtures.iter().collect::<Vec<_>>();
        let seen = AtomicUsize::new(0);

        let collection = collect_fixture_timing_records(&fixture_refs, |fixture| {
            seen.fetch_add(1, Ordering::Relaxed);
            timing_record(
                fixture.path.to_string_lossy().as_ref(),
                100,
                LargeCorpusTimingOutcome::Timeout,
            )
        });

        assert!(collection.timeout_cap_reached);
        assert_eq!(collection.records.len(), LARGE_CORPUS_TIMEOUT_FAILURE_CAP);
        assert_eq!(
            seen.load(Ordering::Relaxed),
            LARGE_CORPUS_TIMEOUT_FAILURE_CAP
        );
    }

    #[test]
    fn format_large_corpus_timing_report_includes_timeout_cap_note() {
        let report = format_large_corpus_timing_report(&LargeCorpusTimingCollection {
            records: vec![timing_record(
                "timeout.sh",
                100,
                LargeCorpusTimingOutcome::Timeout,
            )],
            timeout_cap_reached: true,
        });

        assert!(report.contains("large corpus timing note: stopped after"));
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
            shuck_linter::ShellDialect::from_name("zsh").parser_dialect(),
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
    fn unsupported_shebang_shells_are_skipped_without_shellcheck_metadata() {
        assert!(!shell_supported_for_large_corpus("fish", None));
        assert!(!shell_supported_for_large_corpus("csh", None));
        assert!(shell_supported_for_large_corpus("sh", None));
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
    fn ohmyzsh_repo_fixtures_are_skipped_for_shellcheck_conformance() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("ohmyzsh__ohmyzsh__plugins__wd__wd.sh"),
            cache_rel_path: PathBuf::from("ohmyzsh__ohmyzsh__plugins__wd__wd.sh"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_shellcheck_unsupported_large_corpus_fixture(
            &fixture.path
        ));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
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
    fn static_zsh_override_paths_are_selected_for_large_corpus_zsh_parse() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh"),
            cache_rel_path: PathBuf::from("ohmyzsh__ohmyzsh__tools__check_for_upgrade.sh"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert_eq!(effective_large_corpus_shell(&fixture), "zsh");
        assert!(fixture_selected_for_large_corpus_zsh_parse(&fixture));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
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
    fn patch_files_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("patches/fixup.patch"),
            cache_rel_path: PathBuf::from("patches/fixup.patch"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_patch_file(&fixture.path));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn static_ignore_paths_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("super-linter__super-linter__test__linters__bash__shell_bad_1.sh"),
            cache_rel_path: PathBuf::from(
                "super-linter__super-linter__test__linters__bash__shell_bad_1.sh",
            ),
            shell: "bash".into(),
            source_hash: String::new(),
        };

        assert!(path_is_statically_ignored_large_corpus_fixture(
            &fixture.path
        ));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn shellcheck_parse_ignore_paths_are_skipped_for_conformance() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("pyenv__pyenv__plugins__python-build__bin__python-build"),
            cache_rel_path: PathBuf::from("pyenv__pyenv__plugins__python-build__bin__python-build"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_ignored_large_corpus_shellcheck_parse_fixture(
            &fixture.path
        ));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn shellcheck_parse_ignore_paths_are_kept_for_timing() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("pyenv__pyenv__plugins__python-build__bin__python-build"),
            cache_rel_path: PathBuf::from("pyenv__pyenv__plugins__python-build__bin__python-build"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_ignored_large_corpus_shellcheck_parse_fixture(
            &fixture.path
        ));
        assert!(fixture_supported_for_large_corpus_timing(&fixture));
    }

    #[test]
    fn guess_files_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("termux__termux-packages__scripts__config.guess"),
            cache_rel_path: PathBuf::from("termux__termux-packages__scripts__config.guess"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_guess_file(&fixture.path));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
    }

    #[test]
    fn config_sub_files_are_skipped_for_large_corpus() {
        let fixture = LargeCorpusFixture {
            path: PathBuf::from("termux__termux-packages__scripts__config.sub"),
            cache_rel_path: PathBuf::from("termux__termux-packages__scripts__config.sub"),
            shell: "sh".into(),
            source_hash: String::new(),
        };

        assert!(path_is_config_sub_file(&fixture.path));
        assert!(!fixture_supported_for_large_corpus(&fixture, None));
        assert!(!fixture_selected_for_large_corpus_zsh_parse(&fixture));
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
                fix: None,
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
                fix: None,
            },
        ];
        assert!(shellcheck_parse_aborted(&aborted));

        let parsing_stopped = vec![ShellCheckDiagnostic {
            file: String::new(),
            code: 1088,
            line: 3,
            end_line: 3,
            column: 1,
            end_column: 1,
            level: "error".into(),
            message: "Parsing stopped here. Invalid use of parentheses?".into(),
            fix: None,
        }];
        assert!(shellcheck_parse_aborted(&parsing_stopped));

        let non_aborted = vec![ShellCheckDiagnostic {
            file: String::new(),
            code: 3011,
            line: 1,
            end_line: 1,
            column: 1,
            end_column: 1,
            level: "warning".into(),
            message: "In POSIX sh, here-strings are undefined.".into(),
            fix: None,
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
        assert!(filtered.parse_aborted);
    }

    #[test]
    fn large_corpus_uses_the_shared_shellcheck_map() {
        let index = build_shellcheck_to_rule_index(None);

        assert_eq!(index.get(&3034), Some(&vec!["X026".to_string()]));
        assert_eq!(index.get(&2096), Some(&vec!["S053".to_string()]));
        assert_eq!(index.get(&2086), Some(&vec!["S001".to_string()]));
    }

    #[test]
    fn selected_rule_shellcheck_filters_include_active_rule_codes_without_aliases() {
        let selected_rules =
            shuck_linter::RuleSet::from_iter([shuck_linter::Rule::FunctionKeywordInSh]);
        let codes = build_selected_shellcheck_codes(&selected_rules);

        assert_eq!(codes, HashSet::from([2112]));
    }

    #[test]
    fn c001_only_large_corpus_filter_uses_single_file_oracle() {
        let selected_rules =
            shuck_linter::RuleSet::from_iter([shuck_linter::Rule::UnusedAssignment]);
        let shellcheck_codes = HashSet::from([2034]);

        assert!(large_corpus_uses_single_file_c001_oracle(
            Some(&selected_rules),
            None
        ));
        assert!(large_corpus_uses_single_file_c001_oracle(
            None,
            Some(&shellcheck_codes)
        ));
    }

    #[test]
    fn mixed_rule_large_corpus_filter_is_not_c001_only_even_with_c001_only_codes() {
        let selected_rules = shuck_linter::RuleSet::from_iter([
            shuck_linter::Rule::UnusedAssignment,
            shuck_linter::Rule::MixedAndOrInCondition,
        ]);
        let shellcheck_codes = build_selected_shellcheck_codes(&selected_rules);

        assert_eq!(shellcheck_codes, HashSet::from([2034]));
        assert!(!large_corpus_uses_single_file_c001_oracle(
            Some(&selected_rules),
            Some(&shellcheck_codes)
        ));
    }

    #[test]
    fn default_large_corpus_comparison_disables_source_closure() {
        let settings = build_large_corpus_compat_linter_settings(None, false);

        assert!(!settings.resolve_source_closure);
    }

    #[test]
    fn mixed_c001_c006_large_corpus_comparison_disables_source_closure() {
        let selected_rules = shuck_linter::RuleSet::from_iter([
            shuck_linter::Rule::UnusedAssignment,
            shuck_linter::Rule::UndefinedVariable,
        ]);

        let settings = build_large_corpus_compat_linter_settings(Some(selected_rules), false);

        assert!(!settings.resolve_source_closure);
        assert!(
            settings
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert!(
            settings
                .rules
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
    }

    #[test]
    fn large_corpus_timing_settings_keep_source_closure_enabled() {
        let settings = build_large_corpus_linter_settings(None, false);

        assert!(settings.resolve_source_closure);
    }

    #[test]
    fn parse_failure_kind_uses_selected_rules_before_collapsed_shellcheck_codes() {
        let selected_rules = shuck_linter::RuleSet::from_iter([
            shuck_linter::Rule::UnusedAssignment,
            shuck_linter::Rule::MixedAndOrInCondition,
        ]);
        let shellcheck_codes = build_selected_shellcheck_codes(&selected_rules);

        assert_eq!(shellcheck_codes, HashSet::from([2034]));
        assert_eq!(
            shuck_parse_failure_kind(Some(&selected_rules), Some(&shellcheck_codes)),
            FixtureFailureKind::Other
        );
    }

    #[test]
    fn parse_failure_kind_downgrades_true_c001_only_runs() {
        let selected_rules =
            shuck_linter::RuleSet::from_iter([shuck_linter::Rule::UnusedAssignment]);
        let shellcheck_codes = build_selected_shellcheck_codes(&selected_rules);

        assert_eq!(
            shuck_parse_failure_kind(Some(&selected_rules), Some(&shellcheck_codes)),
            FixtureFailureKind::Warning
        );
    }

    #[test]
    fn rule_corpus_metadata_path_uses_rule_code_convention() {
        assert_eq!(
            rule_corpus_metadata_path("X123"),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/testdata/corpus-metadata")
                .join("x123.yaml")
        );
    }

    #[test]
    fn load_all_rule_corpus_metadata_keeps_scoped_entries() {
        let metadata = load_all_rule_corpus_metadata();

        assert!(!metadata.contains_key("C001"));
        assert!(metadata.values().any(|rule_metadata| {
            rule_metadata.reviewed_divergences.iter().any(|entry| {
                !entry.rule_wide && entry.path_suffix.is_some() && entry.line.is_some()
            })
        }));
    }

    #[test]
    fn reviewed_divergence_classification_matches_exact_shellcheck_record() {
        let metadata = HashMap::from([(
            "C999".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShellcheckOnly,
                    rule_wide: false,
                    path_suffix: Some("repo__script.sh".into()),
                    path_contains: None,
                    line: Some(19),
                    end_line: Some(19),
                    column: Some(1),
                    end_column: Some(14),
                    reason: "exact-match reviewed divergence".into(),
                }],
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShellcheckOnly,
            rule_code: Some("C999".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 19,
                end_line: 19,
                column: 1,
                end_column: 14,
            },
            message: "warning reviewed divergence".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "repo__script.sh", &metadata);

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(reason.as_deref(), Some("exact-match reviewed divergence"));
    }

    #[test]
    fn reviewed_divergence_classification_matches_scripts_prefixed_path_suffix_record() {
        let metadata = HashMap::from([(
            "C999".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_wide: false,
                    path_suffix: Some("scripts/repo__script.sh".into()),
                    path_contains: None,
                    line: Some(19),
                    end_line: Some(19),
                    column: Some(1),
                    end_column: Some(14),
                    reason: "scripts-prefixed reviewed divergence".into(),
                }],
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C999".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 19,
                end_line: 19,
                column: 1,
                end_column: 14,
            },
            message: "warning reviewed divergence".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "repo__script.sh", &metadata);

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(
            reason.as_deref(),
            Some("scripts-prefixed reviewed divergence")
        );
    }

    #[test]
    fn reviewed_divergence_classification_matches_rule_wide_record() {
        let metadata = HashMap::from([(
            "C999".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_wide: true,
                    path_suffix: None,
                    path_contains: None,
                    line: None,
                    end_line: None,
                    column: None,
                    end_column: None,
                    reason: "rule-wide reviewed divergence".into(),
                }],
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C999".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC1090".into(),
            range: DiagnosticRange {
                line: 20,
                end_line: 20,
                column: 1,
                end_column: 10,
            },
            message: "dynamic source path".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "repo__script.sh", &metadata);

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(reason.as_deref(), Some("rule-wide reviewed divergence"));
    }

    #[test]
    fn reviewed_divergence_classification_requires_path_and_line_constraints() {
        let metadata = HashMap::from([(
            "X065".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_wide: false,
                    path_suffix: Some("alpinelinux__aports__scripts__mkimage.sh".into()),
                    path_contains: None,
                    line: Some(120),
                    end_line: Some(120),
                    column: None,
                    end_column: None,
                    reason: "scoped reviewed divergence".into(),
                }],
            },
        )]);
        let matching = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("X065".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC3026".into(),
            range: DiagnosticRange {
                line: 120,
                end_line: 120,
                column: 22,
                end_column: 34,
            },
            message: "caret negation in bracket expressions is not portable to POSIX sh".into(),
        };
        let wrong_path = CompatibilityRecord { ..matching.clone() };
        let wrong_line = CompatibilityRecord {
            range: DiagnosticRange {
                line: 121,
                end_line: 121,
                column: 22,
                end_column: 34,
            },
            ..matching.clone()
        };

        let (matching_classification, matching_reason) = classify_compatibility_record(
            &matching,
            "alpinelinux__aports__scripts__mkimage.sh",
            &metadata,
        );
        let (wrong_path_classification, wrong_path_reason) =
            classify_compatibility_record(&wrong_path, "other__repo__script.sh", &metadata);
        let (wrong_line_classification, wrong_line_reason) = classify_compatibility_record(
            &wrong_line,
            "alpinelinux__aports__scripts__mkimage.sh",
            &metadata,
        );

        assert_eq!(
            matching_classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(
            matching_reason.as_deref(),
            Some("scoped reviewed divergence")
        );
        assert_eq!(
            wrong_path_classification,
            CompatibilityClassification::Implementation
        );
        assert!(wrong_path_reason.is_none());
        assert_eq!(
            wrong_line_classification,
            CompatibilityClassification::Implementation
        );
        assert!(wrong_line_reason.is_none());
    }

    #[test]
    fn reviewed_divergence_classification_matches_path_contains_record() {
        let metadata = HashMap::from([(
            "C999".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_wide: false,
                    path_suffix: None,
                    path_contains: Some("termux__termux-packages__".into()),
                    line: None,
                    end_line: None,
                    column: None,
                    end_column: None,
                    reason: "path-contains reviewed divergence".into(),
                }],
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C999".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 13,
                end_line: 13,
                column: 1,
                end_column: 32,
            },
            message: "warning termux variable".into(),
        };

        let (classification, reason) = classify_compatibility_record(
            &record,
            "fixtures/termux__termux-packages__packages__foo__build.sh",
            &metadata,
        );

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(reason.as_deref(), Some("path-contains reviewed divergence"));
    }

    #[test]
    fn reviewed_divergence_path_contains_uses_cache_relative_path_only() {
        let metadata = HashMap::from([(
            "C999".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: false,
                reviewed_divergences: vec![ReviewedDivergenceRecord {
                    side: CompatibilitySide::ShuckOnly,
                    rule_wide: false,
                    path_suffix: None,
                    path_contains: Some("termux__termux-packages__".into()),
                    line: None,
                    end_line: None,
                    column: None,
                    end_column: None,
                    reason: "path-contains reviewed divergence".into(),
                }],
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C999".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 13,
                end_line: 13,
                column: 1,
                end_column: 32,
            },
            message: "warning unrelated variable".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "fixtures/unrelated.sh", &metadata);

        assert_eq!(classification, CompatibilityClassification::Implementation);
        assert!(reason.is_none());
    }

    #[test]
    fn reviewed_divergence_validation_rejects_implicit_rule_wide_entry() {
        let metadata = RuleCorpusMetadataDocument {
            review_all_divergences: false,
            reviewed_divergences: vec![ReviewedDivergenceRecord {
                side: CompatibilitySide::ShellcheckOnly,
                rule_wide: false,
                path_suffix: None,
                path_contains: None,
                line: None,
                end_line: None,
                column: None,
                end_column: None,
                reason: "implicit rule-wide reviewed divergence".into(),
            }],
        };

        let error = metadata
            .validate(
                "C999",
                Path::new("tests/testdata/corpus-metadata/c999.yaml"),
            )
            .unwrap_err();

        assert!(error.contains("add a scope field or set rule_wide: true"));
    }

    #[test]
    fn reviewed_divergence_validation_rejects_rule_wide_entry_with_scope() {
        let metadata = RuleCorpusMetadataDocument {
            review_all_divergences: false,
            reviewed_divergences: vec![ReviewedDivergenceRecord {
                side: CompatibilitySide::ShellcheckOnly,
                rule_wide: true,
                path_suffix: Some("repo__script.sh".into()),
                path_contains: None,
                line: None,
                end_line: None,
                column: None,
                end_column: None,
                reason: "conflicting reviewed divergence".into(),
            }],
        };

        let error = metadata
            .validate(
                "C999",
                Path::new("tests/testdata/corpus-metadata/c999.yaml"),
            )
            .unwrap_err();

        assert!(error.contains("rule-wide entries cannot also set path or range constraints"));
    }

    #[test]
    fn allowlisted_large_corpus_rule_is_nonblocking() {
        let metadata = HashMap::from([(
            "C001".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: true,
                reviewed_divergences: Vec::new(),
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShuckOnly,
            rule_code: Some("C001".into()),
            rule_codes: Vec::new(),
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 13,
                end_line: 13,
                column: 1,
                end_column: 32,
            },
            message: "warning unused assignment".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "fixtures/unrelated.sh", &metadata);

        assert_eq!(
            classification,
            CompatibilityClassification::ReviewedDivergence
        );
        assert_eq!(
            reason.as_deref(),
            Some(LARGE_CORPUS_ALLOWED_FAILING_RULE_REASON)
        );
    }

    #[test]
    fn allowlisted_large_corpus_rule_requires_every_mapped_rule_to_be_allowlisted() {
        let metadata = HashMap::from([(
            "C001".to_string(),
            RuleCorpusMetadataDocument {
                review_all_divergences: true,
                reviewed_divergences: Vec::new(),
            },
        )]);
        let record = CompatibilityRecord {
            side: CompatibilitySide::ShellcheckOnly,
            rule_code: None,
            rule_codes: vec!["C001".into(), "C999".into()],
            shellcheck_code: "SC2034".into(),
            range: DiagnosticRange {
                line: 13,
                end_line: 13,
                column: 1,
                end_column: 32,
            },
            message: "warning mixed mappings".into(),
        };

        let (classification, reason) =
            classify_compatibility_record(&record, "fixtures/unrelated.sh", &metadata);

        assert_eq!(classification, CompatibilityClassification::Implementation);
        assert!(reason.is_none());
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
        assert_eq!(failures.implementation_diffs.len(), 2);
        assert!(failures.implementation_diffs[0].contains("first.sh"));
        assert!(failures.implementation_diffs[1].contains("second.sh"));
    }

    #[test]
    fn clamp_large_corpus_worker_count_uses_machine_parallelism_with_cap() {
        assert_eq!(clamp_large_corpus_worker_count(2, 10), 2);
        assert_eq!(
            clamp_large_corpus_worker_count(8, 10),
            LARGE_CORPUS_MAX_WORKER_COUNT
        );
        assert_eq!(clamp_large_corpus_worker_count(8, 3), 3);
    }

    #[test]
    fn mapping_issues_are_nonblocking() {
        let fixture = fixture("mapping.sh");
        let fixtures = vec![&fixture];

        let failures = collect_fixture_failures(&fixtures, false, |fixture| FixtureEvaluation {
            mapping_issues: vec![format_fixture_failure(
                &fixture.path,
                &[format!("{} needs mapping review", fixture.path.display())],
            )],
            ..FixtureEvaluation::default()
        });

        assert_eq!(failures.blocking_failures(), 0);
        assert_eq!(failures.mapping_issues.len(), 1);
        assert!(failures.has_nonblocking_items());
    }

    #[test]
    fn sequential_timeouts_fail_immediately() {
        let fixture = fixture("timeout.sh");
        let fixtures = vec![&fixture];

        let panic = panic::catch_unwind(AssertUnwindSafe(|| {
            collect_fixture_failures(&fixtures, false, |fixture| FixtureEvaluation {
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
            })
        }))
        .unwrap_err();

        let message = panic_payload_message(panic);
        assert!(message.contains("timeout.sh"));
        assert!(message.contains("timed out after"));
    }

    #[test]
    fn failure_report_omits_nonblocking_sections() {
        let report = format_large_corpus_failure_report(&FixtureFailureCollection {
            implementation_diffs: vec![
                "/tmp/blocking.sh\n  shellcheck-only C001/SC2000 1:1-1:5 error blocking".into(),
            ],
            mapping_issues: vec![
                "/tmp/mapping.sh\n  shellcheck-only C001/SC2000 1:1-1:5 warning mapping".into(),
            ],
            reviewed_divergences: vec![
                "/tmp/reviewed.sh\n  shuck-only C001/SC2000 1:1-1:5 warning reviewed".into(),
            ],
            harness_warnings: vec!["/tmp/timeout.sh\n  shuck error: timed out".into()],
            unsupported_shells: 2,
            ..FixtureFailureCollection::default()
        });

        assert!(report.contains("Implementation Diffs:"));
        assert!(!report.contains("Mapping Issues:"));
        assert!(!report.contains("Reviewed Divergence:"));
        assert!(!report.contains("Harness Warnings:"));
        assert!(report.contains("Nonblocking issue buckets were omitted"));
    }

    #[test]
    fn keep_going_captures_fixture_panics() {
        let fixture = fixture("panic.sh");
        let fixtures = vec![&fixture];

        let failures = collect_fixture_failures(&fixtures, true, |_| -> FixtureEvaluation {
            panic!("boom");
        });

        assert_eq!(failures.harness_failures.len(), 1);
        assert!(failures.harness_failures[0].contains("panic.sh"));
        assert!(failures.harness_failures[0].contains("fixture panic: boom"));
    }

    #[test]
    fn keep_going_records_timeout_failures() {
        let fixtures: Vec<_> = (0..10)
            .map(|i| fixture(&format!("timeout-{i}.sh")))
            .collect();
        let fixture_refs: Vec<_> = fixtures.iter().collect();
        let seen = AtomicUsize::new(0);
        let progress = LargeCorpusProgress::new(fixture_refs.len());

        let failures = collect_fixture_failures_in_parallel(
            &fixture_refs,
            1,
            &|fixture| {
                seen.fetch_add(1, Ordering::Relaxed);
                if fixture.path.ends_with("timeout-9.sh") {
                    return FixtureEvaluation {
                        implementation_diffs: vec![format_fixture_failure(
                            &fixture.path,
                            &[format!("{} failed", fixture.path.display())],
                        )],
                        ..FixtureEvaluation::default()
                    };
                }

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
            },
            &progress,
        );

        assert_eq!(seen.load(Ordering::Relaxed), fixture_refs.len());
        assert_eq!(failures.implementation_diffs.len(), 1);
        assert_eq!(failures.harness_failures.len(), fixture_refs.len() - 1);
        assert!(failures.harness_warnings.is_empty());
        assert!(failures.implementation_diffs[0].contains("timeout-9.sh"));
        assert!(
            failures
                .harness_failures
                .iter()
                .all(|failure| failure.contains("timed out after"))
        );
    }

    #[test]
    fn zsh_stderr_parse_error_detection_ignores_non_parse_failures() {
        assert!(zsh_stderr_looks_like_parse_error(
            "script.zsh:4: parse error near `foo'"
        ));
        assert!(zsh_stderr_looks_like_parse_error(
            "script.zsh:8: unmatched \""
        ));
        assert!(!zsh_stderr_looks_like_parse_error(
            "script.zsh:32: no such file or directory:"
        ));
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
    fn shellcheck_cache_ignores_entries_with_stale_schema() {
        let tempdir = tempfile::tempdir().unwrap();
        let fixture = fixture_at(
            &tempdir
                .path()
                .join("worktree/.cache/large-corpus/scripts/example.sh"),
            Path::new("example.sh"),
        );
        let cache = ShellCheckCache::new(tempdir.path(), &probe("version: 0.10.0"));
        let stale_path = cache.cache_path(&fixture);

        if let Some(parent) = stale_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            &stale_path,
            serde_json::to_string(&ShellCheckCacheEntry {
                schema: SHELLCHECK_CACHE_SCHEMA - 1,
                diagnostics: vec![shellcheck_diagnostic(1088)],
                parse_aborted: false,
            })
            .unwrap(),
        )
        .unwrap();

        assert!(cache.read_cache(&fixture).is_none());
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
    fn collect_fixtures_skips_known_non_corpus_inputs() {
        let tempdir = tempfile::tempdir().unwrap();
        let scripts_dir = tempdir.path().join("scripts");
        let nested_dir = scripts_dir.join("nested");
        fs::create_dir_all(&nested_dir).unwrap();

        fs::write(scripts_dir.join("keep.sh"), "#!/bin/sh\necho keep\n").unwrap();
        fs::write(scripts_dir.join("._keep.sh"), "skip\n").unwrap();
        fs::write(scripts_dir.join("config.guess"), "not a shell script\n").unwrap();
        fs::write(
            scripts_dir.join("build-aux-config.sub"),
            "not a shell script\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("termux__termux-packages__scripts__config.guess"),
            "not a shell script\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("termux__termux-packages__scripts__config.sub"),
            "not a shell script\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("pre-commit.sample"),
            "#!/bin/sh\necho skip\n",
        )
        .unwrap();
        fs::write(scripts_dir.join("config.fish"), "echo skip\n").unwrap();
        fs::write(scripts_dir.join("fixup.patch"), "--- a/file\n+++ b/file\n").unwrap();
        fs::write(
            scripts_dir.join("super-linter__super-linter__test__linters__bash__shell_bad_1.sh"),
            "#!/usr/bin/env bash\necho skip\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("alpinelinux__aports__community__starship__starship.plugin.zsh"),
            "echo skip\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("google__oss-fuzz__infra__chronos__coverage_test_collection.py"),
            "#!/bin/bash -eux\nprint('skip')\n",
        )
        .unwrap();
        fs::write(
            scripts_dir.join("ohmyzsh__ohmyzsh__plugins__alias-finder__tests__test_run.sh"),
            "#!/usr/bin/env zunit\n@setup {\n  echo skip\n}\n",
        )
        .unwrap();
        fs::write(
            nested_dir.join("post-checkout.sample"),
            "#!/bin/sh\necho skip\n",
        )
        .unwrap();
        fs::write(nested_dir.join("prompt.fish"), "echo skip\n").unwrap();
        fs::write(nested_dir.join("fixup.diff"), "--- a/file\n+++ b/file\n").unwrap();
        fs::write(nested_dir.join("._nested.sh"), "skip\n").unwrap();
        fs::write(nested_dir.join("toolchain.guess"), "not a shell script\n").unwrap();
        fs::write(
            nested_dir.join("toolchain-config.sub"),
            "not a shell script\n",
        )
        .unwrap();

        let fixtures = collect_fixtures(tempdir.path());
        let collected_paths: Vec<_> = fixtures
            .into_iter()
            .map(|fixture| fixture.cache_rel_path)
            .collect();

        assert_eq!(collected_paths, vec![PathBuf::from("keep.sh")]);
    }

    #[test]
    fn collect_fixtures_keeps_ohmyzsh_entries_for_non_conformance_harnesses() {
        let tempdir = tempfile::tempdir().unwrap();
        let scripts_dir = tempdir.path().join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();

        fs::write(
            scripts_dir.join("ohmyzsh__ohmyzsh__plugins__wd__wd.sh"),
            "#!/bin/sh\necho keep\n",
        )
        .unwrap();

        let fixtures = collect_fixtures(tempdir.path());
        let collected_paths: Vec<_> = fixtures
            .into_iter()
            .map(|fixture| fixture.cache_rel_path)
            .collect();

        assert_eq!(
            collected_paths,
            vec![PathBuf::from("ohmyzsh__ohmyzsh__plugins__wd__wd.sh")]
        );
    }

    fn fixture(path: &str) -> LargeCorpusFixture {
        LargeCorpusFixture {
            path: PathBuf::from(path),
            cache_rel_path: PathBuf::from(path),
            shell: "sh".into(),
            source_hash: String::new(),
        }
    }

    fn timing_record(
        fixture_label: &str,
        millis: u64,
        outcome: LargeCorpusTimingOutcome,
    ) -> LargeCorpusTimingRecord {
        LargeCorpusTimingRecord {
            fixture_label: fixture_label.into(),
            elapsed: Duration::from_millis(millis),
            outcome,
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
            fix: None,
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

    #[test]
    fn run_shuck_reports_parse_error_when_parse_rule_is_disabled() {
        let tempdir = tempfile::tempdir().unwrap();
        let fixture_path = tempdir.path().join("broken.sh");
        fs::write(&fixture_path, "#!/bin/sh\nif true; then\n  :\n").unwrap();
        let fixture = fixture_at(&fixture_path, Path::new("broken.sh"));

        let run = run_shuck_with_parse_dialect(
            &fixture,
            &shuck_linter::LinterSettings::for_rule(shuck_linter::Rule::UnusedAssignment),
            None,
            shuck_parser::ShellDialect::Posix,
            "sh",
        );

        assert!(run.diagnostics.is_empty());
        assert!(
            run.parse_error
                .as_deref()
                .is_some_and(|error| error.contains("expected 'fi'"))
        );
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
                fix: None,
            }],
            parse_aborted: false,
        })
        .unwrap()
    }
}
