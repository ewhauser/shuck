use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shuck_formatter::{FormattedSource, ShellDialect, ShellFormatOptions, format_file_ast};
use shuck_parser::parser::Parser;
use similar::TextDiff;

const MAX_ORACLE_DIFF_LINES: usize = 200;
const MAX_LARGE_CORPUS_FAILURES: usize = 25;
const SHFMT_LARGE_CORPUS_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LARGE_CORPUS_MAX_DURATION_SECS: usize = 60;
const LARGE_CORPUS_PROGRESS_INTERVAL: usize = 1_000;
const LARGE_CORPUS_SLOW_FIXTURE: Duration = Duration::from_secs(3);
const LARGE_CORPUS_ENV: &str = "SHUCK_TEST_LARGE_CORPUS";
const LARGE_CORPUS_ROOT_ENV: &str = "SHUCK_LARGE_CORPUS_ROOT";
const LARGE_CORPUS_SHARD_ENV: &str = "TEST_SHARD_INDEX";
const LARGE_CORPUS_SHARDS_ENV: &str = "TEST_TOTAL_SHARDS";
const LARGE_CORPUS_SAMPLE_PERCENT_ENV: &str = "SHUCK_LARGE_CORPUS_SAMPLE_PERCENT";
const LARGE_CORPUS_MAX_DURATION_ENV: &str = "SHUCK_SHFMT_ORACLE_MAX_DURATION_SECS";
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";
const SHFMT_ALLOWLIST_UPDATE_ENV: &str = "SHUCK_UPDATE_SHFMT_LARGE_CORPUS_ALLOWLIST";
const SHFMT_ALLOWLIST_SCHEMA: u32 = 1;
const SHFMT_ALLOWLIST_REL_PATH: &str = "tests/testdata/shfmt-large-corpus-allowlist.yaml";
const DEFAULT_SHFMT_ALLOWLIST_REASON: &str =
    "Known formatter difference from the shfmt oracle at this corpus snapshot.";
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
const LARGE_CORPUS_IGNORED_EXTENSIONS: &[&str] = &["fish"];
const LARGE_CORPUS_IGNORED_FILE_PREFIXES: &[&str] = &["._"];
const LARGE_CORPUS_IGNORED_FILE_SUFFIXES: &[&str] = &[
    ".sample",
    ".patch",
    ".diff",
    ".dpatch",
    ".guess",
    "config.sub",
];
const LARGE_CORPUS_IGNORED_FILE_CONTAINS: &[&str] = &["__.git__"];

struct OracleCase {
    name: &'static str,
    fixture: &'static str,
    filename: &'static str,
    shfmt_flags: &'static [&'static str],
    options: ShellFormatOptions,
}

struct ShfmtProbe {
    supported_flags: String,
}

#[derive(Debug, Clone)]
struct LargeCorpusConfig {
    corpus_dir: PathBuf,
    shard_index: usize,
    total_shards: usize,
    sample_percent: usize,
}

impl LargeCorpusConfig {
    fn covers_full_corpus(&self) -> bool {
        self.shard_index == 0 && self.total_shards == 1 && self.sample_percent == 100
    }
}

#[derive(Debug, Clone)]
struct LargeCorpusFixture {
    path: PathBuf,
    cache_rel_path: PathBuf,
}

struct FormatterOracleConfig {
    shfmt_language: &'static str,
    options: ShellFormatOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShfmtDiffHunk {
    start_line: usize,
    end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShfmtLargeCorpusAllowlistEntry {
    path_suffix: String,
    hunks: Vec<ShfmtDiffHunk>,
    diff_hash: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShfmtLargeCorpusAllowlistDocument {
    schema: u32,
    #[serde(default)]
    entries: Vec<ShfmtLargeCorpusAllowlistEntry>,
}

impl Default for ShfmtLargeCorpusAllowlistDocument {
    fn default() -> Self {
        Self {
            schema: SHFMT_ALLOWLIST_SCHEMA,
            entries: Vec::new(),
        }
    }
}

impl ShfmtLargeCorpusAllowlistDocument {
    fn validate(&self, path: &Path) -> Result<(), String> {
        if self.schema != SHFMT_ALLOWLIST_SCHEMA {
            return Err(format!(
                "unsupported shfmt large-corpus allowlist schema {} in {}; expected {}",
                self.schema,
                path.display(),
                SHFMT_ALLOWLIST_SCHEMA
            ));
        }

        let mut seen_paths = HashSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            entry.validate(path, index + 1)?;
            if !seen_paths.insert(entry.path_suffix.as_str()) {
                return Err(format!(
                    "duplicate shfmt allowlist path_suffix `{}` in {}",
                    entry.path_suffix,
                    path.display()
                ));
            }
        }

        Ok(())
    }
}

impl ShfmtLargeCorpusAllowlistEntry {
    fn validate(&self, path: &Path, entry_index: usize) -> Result<(), String> {
        if self.path_suffix.trim().is_empty() {
            return Err(format!(
                "invalid shfmt allowlist entry {entry_index} in {}: path_suffix cannot be empty",
                path.display()
            ));
        }
        if self.hunks.is_empty() {
            return Err(format!(
                "invalid shfmt allowlist entry {entry_index} in {}: hunks cannot be empty",
                path.display()
            ));
        }
        if self.diff_hash.len() != 64 || !self.diff_hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid shfmt allowlist entry {entry_index} in {}: diff_hash must be a SHA-256 hex digest",
                path.display()
            ));
        }
        if self.reason.trim().is_empty() {
            return Err(format!(
                "invalid shfmt allowlist entry {entry_index} in {}: reason cannot be empty",
                path.display()
            ));
        }

        for hunk in &self.hunks {
            if hunk.end_line < hunk.start_line {
                return Err(format!(
                    "invalid shfmt allowlist entry {entry_index} in {}: hunk end_line is before start_line",
                    path.display()
                ));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShfmtOracleMismatch {
    filename: String,
    hunks: Vec<ShfmtDiffHunk>,
    diff_hash: String,
    diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShfmtMismatchClassification {
    Reviewed { entry_index: usize },
    ChangedKnown { reason: String },
    New,
}

#[derive(Debug)]
struct LargeCorpusFixtureResult {
    filename: String,
    elapsed: Duration,
    comparison: LargeCorpusComparison,
}

#[derive(Debug)]
enum LargeCorpusComparison {
    Matched,
    Mismatch(ShfmtOracleMismatch),
    ShuckError(String),
    ShfmtError(String),
    ShfmtSkip,
    UnsupportedDialect,
    NonUtf8,
}

#[derive(Default)]
struct LargeCorpusProgress {
    processed: AtomicUsize,
    compared: AtomicUsize,
    matched: AtomicUsize,
    mismatches: AtomicUsize,
    shuck_errors: AtomicUsize,
    shfmt_errors: AtomicUsize,
    shfmt_skips: AtomicUsize,
    unsupported_dialects: AtomicUsize,
    non_utf8: AtomicUsize,
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1 and shfmt on PATH (for example via `nix develop`)"]
fn selected_fixtures_match_shfmt() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the shfmt oracle");
        return;
    }

    let shfmt = probe_shfmt().expect("shfmt not found on PATH; run under `nix develop`");

    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
    let mut ran_case = false;
    let mut mismatches = Vec::new();
    for case in oracle_cases() {
        if !case.is_supported(&shfmt) {
            eprintln!(
                "skipping oracle case `{}` because installed shfmt does not support {:?}",
                case.name, case.shfmt_flags
            );
            continue;
        }

        let source = fs::read_to_string(fixture_root.join(case.fixture)).unwrap();
        let shuck = match try_run_shuck_formatter(&source, case.filename, &case.options).unwrap() {
            FormattedSource::Unchanged => source.to_string(),
            FormattedSource::Formatted(formatted) => formatted,
        };
        let shfmt = run_shfmt(&source, case.filename, case.shfmt_flags);

        if let Some(mismatch) = render_oracle_mismatch(case.name, case.filename, &shfmt, &shuck) {
            mismatches.push(mismatch);
        }
        ran_case = true;
    }

    assert!(
        ran_case,
        "no oracle cases were compatible with this shfmt binary"
    );
    assert!(
        mismatches.is_empty(),
        "fixture oracle diverged from shfmt:\n\n{}",
        mismatches.join("\n\n")
    );
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1, SHUCK_TEST_LARGE_CORPUS=1, and shfmt on PATH"]
fn large_corpus_matches_shfmt() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the shfmt oracle");
        return;
    }
    let Some(cfg) = resolve_large_corpus_config() else {
        eprintln!("large corpus shfmt oracle skipped (set {LARGE_CORPUS_ENV}=1 to enable)");
        return;
    };

    let update_allowlist = env_truthy(SHFMT_ALLOWLIST_UPDATE_ENV, false);
    let allowlist = (!update_allowlist).then(load_shfmt_large_corpus_allowlist);
    let large_corpus_started = Instant::now();
    probe_shfmt().expect("shfmt not found on PATH; run under `nix develop`");

    let all_fixtures = collect_large_corpus_fixtures(&cfg.corpus_dir);
    assert!(
        !all_fixtures.is_empty(),
        "no large-corpus fixtures found in {}",
        cfg.corpus_dir.join("scripts").display()
    );

    let fixtures = sample_fixtures(
        shard_fixtures(all_fixtures, cfg.shard_index, cfg.total_shards),
        cfg.sample_percent,
    );
    assert!(
        !fixtures.is_empty(),
        "no large-corpus fixtures selected from {}",
        cfg.corpus_dir.join("scripts").display()
    );
    eprintln!(
        "large corpus shfmt oracle using Rayon: fixtures={} workers={}",
        fixtures.len(),
        rayon::current_num_threads()
    );

    let progress = LargeCorpusProgress::default();
    let results = fixtures
        .par_iter()
        .map(|fixture| {
            let result = compare_large_corpus_fixture(fixture);
            progress.observe(&result, fixtures.len());
            result
        })
        .collect::<Vec<_>>();

    let mut compared = 0usize;
    let mut matched = 0usize;
    let mut unsupported_dialects = 0usize;
    let mut non_utf8 = 0usize;
    let mut shfmt_skips = 0usize;
    let mut shuck_errors = Vec::new();
    let mut shfmt_errors = Vec::new();
    let mut mismatches = Vec::<ShfmtOracleMismatch>::new();

    for result in results {
        if result.elapsed >= LARGE_CORPUS_SLOW_FIXTURE {
            eprintln!(
                "large corpus shfmt oracle slow fixture: {} took {:.1}s",
                result.filename,
                result.elapsed.as_secs_f64(),
            );
        }
        match result.comparison {
            LargeCorpusComparison::Matched => {
                compared += 1;
                matched += 1;
            }
            LargeCorpusComparison::Mismatch(mismatch) => {
                compared += 1;
                mismatches.push(mismatch);
            }
            LargeCorpusComparison::ShuckError(error) => {
                compared += 1;
                shuck_errors.push(error);
            }
            LargeCorpusComparison::ShfmtError(error) => {
                shfmt_errors.push(error);
            }
            LargeCorpusComparison::ShfmtSkip => {
                shfmt_skips += 1;
            }
            LargeCorpusComparison::UnsupportedDialect => {
                unsupported_dialects += 1;
            }
            LargeCorpusComparison::NonUtf8 => {
                non_utf8 += 1;
            }
        }
    }

    if update_allowlist {
        write_shfmt_large_corpus_allowlist(&mismatches);
    }

    let mut reviewed_mismatches = 0usize;
    let mut blocking_mismatches = Vec::new();
    let mut matched_allowlist_indices = HashSet::new();

    if let Some(allowlist) = &allowlist {
        for mismatch in &mismatches {
            match classify_shfmt_mismatch(allowlist, mismatch) {
                ShfmtMismatchClassification::Reviewed { entry_index } => {
                    reviewed_mismatches += 1;
                    matched_allowlist_indices.insert(entry_index);
                }
                ShfmtMismatchClassification::ChangedKnown { reason } => {
                    blocking_mismatches.push(format_shfmt_mismatch_failure(mismatch, &reason));
                }
                ShfmtMismatchClassification::New => {
                    blocking_mismatches.push(format_shfmt_mismatch_failure(
                        mismatch,
                        "new formatter difference from the shfmt oracle",
                    ));
                }
            }
        }
    }

    let stale_allowlist_entries = allowlist
        .as_ref()
        .filter(|_| cfg.covers_full_corpus())
        .map(|allowlist| stale_shfmt_allowlist_entries(allowlist, &matched_allowlist_indices))
        .unwrap_or_default();
    if !stale_allowlist_entries.is_empty() {
        eprintln!(
            "{}",
            format_failure_list(
                "stale shfmt formatter allowlist entries (nonblocking cleanup hints)",
                &stale_allowlist_entries,
            )
        );
    }

    let elapsed = large_corpus_started.elapsed();
    let max_duration = large_corpus_max_duration();
    eprintln!(
        "large corpus shfmt oracle summary: fixtures={} compared={} matched={} mismatches={} reviewed_mismatches={} blocking_mismatches={} shuck_errors={} shfmt_errors={} shfmt_skips={} unsupported_dialects={} non_utf8={} stale_allowlist={} elapsed={:.2}s max_elapsed={}s",
        fixtures.len(),
        compared,
        matched,
        mismatches.len(),
        reviewed_mismatches,
        blocking_mismatches.len(),
        shuck_errors.len(),
        shfmt_errors.len(),
        shfmt_skips,
        unsupported_dialects,
        non_utf8,
        stale_allowlist_entries.len(),
        elapsed.as_secs_f64(),
        max_duration.as_secs(),
    );

    let timing_failure = if elapsed > max_duration {
        format!(
            "large corpus shfmt oracle exceeded {}s limit (took {:.2}s)\n\n",
            max_duration.as_secs(),
            elapsed.as_secs_f64(),
        )
    } else {
        String::new()
    };

    assert!(
        large_corpus_oracle_passes(
            elapsed,
            max_duration,
            &shuck_errors,
            &shfmt_errors,
            &blocking_mismatches
        ),
        "large corpus shfmt oracle found {} shuck error(s), {} shfmt error(s), {} blocking mismatch(es), and {} reviewed mismatch(es) in {:.2}s (limit {}s):\n\n{}{}{}{}",
        shuck_errors.len(),
        shfmt_errors.len(),
        blocking_mismatches.len(),
        reviewed_mismatches,
        elapsed.as_secs_f64(),
        max_duration.as_secs(),
        timing_failure,
        format_failure_list("shuck formatter errors", &shuck_errors),
        format_failure_list("shfmt harness errors", &shfmt_errors),
        format_failure_list("blocking formatter mismatches", &blocking_mismatches),
    );
}

impl LargeCorpusProgress {
    fn observe(&self, result: &LargeCorpusFixtureResult, total: usize) {
        let (counts_comparison, counter) = match &result.comparison {
            LargeCorpusComparison::Matched => (true, &self.matched),
            LargeCorpusComparison::Mismatch(_) => (true, &self.mismatches),
            LargeCorpusComparison::ShuckError(_) => (true, &self.shuck_errors),
            LargeCorpusComparison::ShfmtError(_) => (false, &self.shfmt_errors),
            LargeCorpusComparison::ShfmtSkip => (false, &self.shfmt_skips),
            LargeCorpusComparison::UnsupportedDialect => (false, &self.unsupported_dialects),
            LargeCorpusComparison::NonUtf8 => (false, &self.non_utf8),
        };
        if counts_comparison {
            self.compared.fetch_add(1, Ordering::Relaxed);
        }
        counter.fetch_add(1, Ordering::Relaxed);

        let processed = self.processed.fetch_add(1, Ordering::Relaxed) + 1;
        if processed.is_multiple_of(LARGE_CORPUS_PROGRESS_INTERVAL) {
            eprintln!(
                "large corpus shfmt oracle progress: processed={processed}/{total} compared={} matched={} mismatches={} shuck_errors={} shfmt_errors={} shfmt_skips={} unsupported_dialects={} non_utf8={}",
                self.compared.load(Ordering::Relaxed),
                self.matched.load(Ordering::Relaxed),
                self.mismatches.load(Ordering::Relaxed),
                self.shuck_errors.load(Ordering::Relaxed),
                self.shfmt_errors.load(Ordering::Relaxed),
                self.shfmt_skips.load(Ordering::Relaxed),
                self.unsupported_dialects.load(Ordering::Relaxed),
                self.non_utf8.load(Ordering::Relaxed),
            );
        }
    }
}

fn compare_large_corpus_fixture(fixture: &LargeCorpusFixture) -> LargeCorpusFixtureResult {
    let fixture_started = Instant::now();
    let filename = fixture.cache_rel_path.to_string_lossy().into_owned();
    let finish = |filename: String, comparison: LargeCorpusComparison| LargeCorpusFixtureResult {
        filename,
        elapsed: fixture_started.elapsed(),
        comparison,
    };

    let source = match fs::read_to_string(&fixture.path) {
        Ok(source) => source,
        Err(_) => return finish(filename, LargeCorpusComparison::NonUtf8),
    };
    let Some(format_config) = formatter_oracle_config(&source, &fixture.path) else {
        return finish(filename, LargeCorpusComparison::UnsupportedDialect);
    };

    let shfmt = match try_run_shfmt(&source, &filename, format_config.shfmt_language) {
        Ok(output) => output,
        Err(ShfmtRunError::Unsupported) => {
            return finish(filename, LargeCorpusComparison::ShfmtSkip);
        }
        Err(ShfmtRunError::Harness(error)) => {
            return finish(
                filename.clone(),
                LargeCorpusComparison::ShfmtError(format!("{filename}: {error}")),
            );
        }
    };

    let shuck = match try_run_shuck_formatter(&source, &filename, &format_config.options) {
        Ok(FormattedSource::Unchanged) => source.clone(),
        Ok(FormattedSource::Formatted(formatted)) => formatted,
        Err(error) => {
            return finish(
                filename.clone(),
                LargeCorpusComparison::ShuckError(format!("{filename}: {error}")),
            );
        }
    };

    let comparison = build_oracle_mismatch(&filename, &shfmt, &shuck)
        .map(LargeCorpusComparison::Mismatch)
        .unwrap_or(LargeCorpusComparison::Matched);

    finish(filename, comparison)
}

impl OracleCase {
    fn new(name: &'static str, fixture: &'static str) -> Self {
        Self {
            name,
            fixture,
            filename: fixture,
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
        }
    }

    fn with_filename(mut self, filename: &'static str) -> Self {
        self.filename = filename;
        self
    }

    fn with_shfmt_flags(mut self, shfmt_flags: &'static [&'static str]) -> Self {
        self.shfmt_flags = shfmt_flags;
        self
    }

    fn with_options(mut self, options: ShellFormatOptions) -> Self {
        self.options = options;
        self
    }

    fn is_supported(&self, shfmt: &ShfmtProbe) -> bool {
        self.shfmt_flags
            .iter()
            .all(|flag| shfmt.supports_flag(flag))
    }
}

impl ShfmtProbe {
    fn supports_flag(&self, flag: &str) -> bool {
        match flag {
            "-ln=mksh" => self.supported_flags.contains("-ln, --language-dialect"),
            other => self.supported_flags.contains(other),
        }
    }
}

fn probe_shfmt() -> Option<ShfmtProbe> {
    let version = Command::new("shfmt")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !version.success() {
        return None;
    }

    let help = Command::new("shfmt").arg("--help").output().ok()?;
    if !help.status.success() {
        return None;
    }

    let mut supported_flags = String::from_utf8_lossy(&help.stdout).into_owned();
    supported_flags.push_str(&String::from_utf8_lossy(&help.stderr));

    Some(ShfmtProbe { supported_flags })
}

fn try_run_shuck_formatter(
    source: &str,
    filename: &str,
    options: &ShellFormatOptions,
) -> Result<FormattedSource, String> {
    let path = Path::new(filename);
    let resolved = options.resolve(source, Some(path));
    let parsed = Parser::with_dialect(source, resolved.dialect()).parse();
    if parsed.is_err() {
        return Err(parsed.strict_error().to_string());
    }
    format_file_ast(source, parsed.file, Some(path), options).map_err(|error| error.to_string())
}

fn run_shfmt(source: &str, filename: &str, flags: &[&str]) -> String {
    let mut command = Command::new("shfmt");
    command.arg("-filename").arg(filename);
    for flag in flags {
        command.arg(flag);
    }
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());

    let mut child = command.spawn().expect("spawn shfmt");
    child
        .stdin
        .as_mut()
        .expect("shfmt stdin")
        .write_all(source.as_bytes())
        .expect("write source to shfmt");
    let output = child.wait_with_output().expect("wait for shfmt");
    assert!(
        output.status.success(),
        "shfmt exited with {}",
        output.status
    );
    String::from_utf8(output.stdout).expect("utf8 shfmt output")
}

#[derive(Debug)]
enum ShfmtRunError {
    Unsupported,
    Harness(String),
}

fn try_run_shfmt(source: &str, filename: &str, language: &str) -> Result<String, ShfmtRunError> {
    let mut command = Command::new("shfmt");
    command
        .arg("-filename")
        .arg(filename)
        .arg(format!("-ln={language}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| ShfmtRunError::Harness(error.to_string()))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| ShfmtRunError::Harness("failed to open shfmt stdin".to_string()))?
        .write_all(source.as_bytes())
        .map_err(|error| ShfmtRunError::Harness(error.to_string()))?;
    let output = child
        .wait_with_output_timeout(SHFMT_LARGE_CORPUS_TIMEOUT)
        .map_err(ShfmtRunError::Harness)?;
    if !output.status.success() {
        return Err(ShfmtRunError::Unsupported);
    }
    String::from_utf8(output.stdout).map_err(|error| ShfmtRunError::Harness(error.to_string()))
}

trait ChildTimeoutExt {
    fn wait_with_output_timeout(self, timeout: Duration) -> Result<Output, String>;
}

impl ChildTimeoutExt for Child {
    fn wait_with_output_timeout(mut self, timeout: Duration) -> Result<Output, String> {
        drop(self.stdin.take());

        let stdout_reader = self.stdout.take().map(spawn_reader);
        let stderr_reader = self.stderr.take().map(spawn_reader);
        let started = Instant::now();

        let status = loop {
            if let Some(status) = self.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if started.elapsed() >= timeout {
                let _ = self.kill();
                let _ = self.wait();
                let _ = collect_reader(stdout_reader);
                let _ = collect_reader(stderr_reader);
                return Err(format!("shfmt timed out after {}s", timeout.as_secs()));
            }
            thread::sleep(Duration::from_millis(10));
        };

        Ok(Output {
            status,
            stdout: collect_reader(stdout_reader)?,
            stderr: collect_reader(stderr_reader)?,
        })
    }
}

fn spawn_reader<R>(mut reader: R) -> thread::JoinHandle<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .map_err(|error| error.to_string())?;
        Ok(output)
    })
}

fn collect_reader(
    reader: Option<thread::JoinHandle<Result<Vec<u8>, String>>>,
) -> Result<Vec<u8>, String> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| "failed to join shfmt pipe reader".to_string())?,
        None => Ok(Vec::new()),
    }
}

fn render_oracle_mismatch(
    case_name: &str,
    filename: &str,
    shfmt: &str,
    shuck: &str,
) -> Option<String> {
    build_oracle_mismatch(filename, shfmt, shuck).map(|mismatch| {
        format!(
            "oracle mismatch for {case_name}\n{}",
            truncate_diff(&mismatch.diff)
        )
    })
}

fn build_oracle_mismatch(filename: &str, shfmt: &str, shuck: &str) -> Option<ShfmtOracleMismatch> {
    if shfmt == shuck {
        return None;
    }

    let raw_diff = TextDiff::from_lines(shfmt, shuck)
        .unified_diff()
        .header(&format!("shfmt/{filename}"), &format!("shuck/{filename}"))
        .to_string();
    let diff = normalize_diff_body(&raw_diff);
    let hunks = parse_shfmt_diff_hunks(&diff);

    Some(ShfmtOracleMismatch {
        filename: filename.to_owned(),
        hunks,
        diff_hash: sha256_hex(&diff),
        diff,
    })
}

fn normalize_diff_body(diff: &str) -> String {
    diff.replace("\r\n", "\n").replace('\r', "\n")
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn parse_shfmt_diff_hunks(diff: &str) -> Vec<ShfmtDiffHunk> {
    diff.lines().filter_map(parse_shfmt_hunk_header).collect()
}

fn parse_shfmt_hunk_header(line: &str) -> Option<ShfmtDiffHunk> {
    let header = line.strip_prefix("@@ ")?;
    let old_spec = header.split_whitespace().next()?.strip_prefix('-')?;
    let (start_line, len) = parse_unified_diff_range(old_spec)?;
    let end_line = if len == 0 {
        start_line
    } else {
        start_line + len - 1
    };

    Some(ShfmtDiffHunk {
        start_line,
        end_line,
    })
}

fn parse_unified_diff_range(spec: &str) -> Option<(usize, usize)> {
    let (start, len) = spec
        .split_once(',')
        .map_or((spec, "1"), |(start, len)| (start, len));
    Some((start.parse().ok()?, len.parse().ok()?))
}

fn truncate_diff(diff: &str) -> String {
    let lines = diff.lines().collect::<Vec<_>>();
    if lines.len() <= MAX_ORACLE_DIFF_LINES {
        return diff.to_string();
    }

    let omitted = lines.len() - MAX_ORACLE_DIFF_LINES;
    let mut truncated = lines[..MAX_ORACLE_DIFF_LINES].join("\n");
    truncated.push_str(&format!(
        "\n... diff truncated, omitted {omitted} additional lines ..."
    ));
    truncated
}

fn formatter_oracle_config(source: &str, path: &Path) -> Option<FormatterOracleConfig> {
    match shuck_linter::ShellDialect::infer(source, Some(path)) {
        shuck_linter::ShellDialect::Ksh | shuck_linter::ShellDialect::Mksh => {
            Some(FormatterOracleConfig {
                shfmt_language: "mksh",
                options: ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            })
        }
        shuck_linter::ShellDialect::Unknown
        | shuck_linter::ShellDialect::Sh
        | shuck_linter::ShellDialect::Dash
        | shuck_linter::ShellDialect::Bash => Some(FormatterOracleConfig {
            shfmt_language: "bash",
            options: ShellFormatOptions::default().with_dialect(ShellDialect::Bash),
        }),
        shuck_linter::ShellDialect::Zsh => None,
    }
}

fn resolve_large_corpus_config() -> Option<LargeCorpusConfig> {
    if !env_truthy(LARGE_CORPUS_ENV, false) {
        return None;
    }

    let repo_root = repo_root();
    let root_hint = std::env::var(LARGE_CORPUS_ROOT_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    let candidates = if let Some(root_hint) = root_hint {
        vec![PathBuf::from(root_hint)]
    } else {
        vec![
            repo_root.join(LARGE_CORPUS_CACHE_DIR_NAME),
            repo_root.join("..").join("shell-checks"),
        ]
    };

    for candidate in candidates {
        if let Some(corpus_dir) = normalize_large_corpus_root(&candidate) {
            let total_shards = filtered_env_int(LARGE_CORPUS_SHARDS_ENV, 1, |value| value > 0);
            let shard_index = filtered_env_int(LARGE_CORPUS_SHARD_ENV, 0, |_| true);
            assert!(
                shard_index < total_shards,
                "{LARGE_CORPUS_SHARD_ENV}={shard_index}, want value in [0,{total_shards})"
            );

            return Some(LargeCorpusConfig {
                corpus_dir,
                shard_index,
                total_shards,
                sample_percent: filtered_env_int(LARGE_CORPUS_SAMPLE_PERCENT_ENV, 100, |value| {
                    (1..=100).contains(&value)
                }),
            });
        }
    }

    panic!("large corpus not found; set {LARGE_CORPUS_ROOT_ENV} to an existing corpus directory");
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
    manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("failed to resolve repo root")
        .to_path_buf()
}

fn collect_large_corpus_fixtures(corpus_dir: &Path) -> Vec<LargeCorpusFixture> {
    let scripts_dir = corpus_dir.join("scripts");
    let mut fixtures = Vec::new();
    let mut pending = vec![scripts_dir.clone()];

    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && fixture_path_is_supported(&path) {
                let cache_rel_path = path
                    .strip_prefix(&scripts_dir)
                    .unwrap_or(path.as_path())
                    .to_path_buf();
                fixtures.push(LargeCorpusFixture {
                    path,
                    cache_rel_path,
                });
            }
        }
    }

    fixtures.sort_by(|a, b| a.cache_rel_path.cmp(&b.cache_rel_path));
    fixtures
}

fn fixture_path_is_supported(path: &Path) -> bool {
    let path_text = path.to_string_lossy();
    if LARGE_CORPUS_STATIC_IGNORE_SUFFIXES
        .iter()
        .any(|suffix| path_text.ends_with(suffix))
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                LARGE_CORPUS_IGNORED_EXTENSIONS
                    .iter()
                    .any(|ignored| ext.eq_ignore_ascii_case(ignored))
            })
    {
        return false;
    }

    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };

    let lower_name = name.to_ascii_lowercase();
    !(LARGE_CORPUS_IGNORED_FILE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || LARGE_CORPUS_IGNORED_FILE_SUFFIXES
            .iter()
            .any(|suffix| lower_name.ends_with(suffix))
        || LARGE_CORPUS_IGNORED_FILE_CONTAINS
            .iter()
            .any(|needle| name.contains(needle)))
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
        .filter(|fixture| {
            let key = fixture.cache_rel_path.to_string_lossy();
            let mut hash = 0xcbf29ce484222325u64;
            for byte in key.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
            hash % 100 < sample_percent as u64
        })
        .collect()
}

fn shfmt_allowlist_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SHFMT_ALLOWLIST_REL_PATH)
}

fn load_shfmt_large_corpus_allowlist() -> ShfmtLargeCorpusAllowlistDocument {
    let path = shfmt_allowlist_path();
    if !path.exists() {
        return ShfmtLargeCorpusAllowlistDocument::default();
    }

    let data =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let allowlist: ShfmtLargeCorpusAllowlistDocument =
        serde_yaml::from_str(&data).unwrap_or_else(|err| {
            panic!(
                "parse shfmt large-corpus allowlist {}: {err}",
                path.display()
            )
        });
    allowlist
        .validate(&path)
        .unwrap_or_else(|err| panic!("{err}"));
    allowlist
}

fn write_shfmt_large_corpus_allowlist(mismatches: &[ShfmtOracleMismatch]) {
    let path = shfmt_allowlist_path();
    let allowlist = shfmt_allowlist_document_from_mismatches(mismatches);
    allowlist
        .validate(&path)
        .unwrap_or_else(|err| panic!("{err}"));
    let yaml = serde_yaml::to_string(&allowlist)
        .unwrap_or_else(|err| panic!("serialize {}: {err}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("create {}: {err}", parent.display()));
    }
    fs::write(&path, yaml).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    eprintln!(
        "wrote {} shfmt large-corpus allowlist entrie(s) to {}",
        allowlist.entries.len(),
        path.display(),
    );
}

fn shfmt_allowlist_document_from_mismatches(
    mismatches: &[ShfmtOracleMismatch],
) -> ShfmtLargeCorpusAllowlistDocument {
    let mut entries = mismatches
        .iter()
        .map(|mismatch| ShfmtLargeCorpusAllowlistEntry {
            path_suffix: mismatch.filename.clone(),
            hunks: mismatch.hunks.clone(),
            diff_hash: mismatch.diff_hash.clone(),
            reason: DEFAULT_SHFMT_ALLOWLIST_REASON.to_owned(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        a.path_suffix
            .cmp(&b.path_suffix)
            .then_with(|| a.hunks.cmp(&b.hunks))
            .then_with(|| a.diff_hash.cmp(&b.diff_hash))
    });

    ShfmtLargeCorpusAllowlistDocument {
        schema: SHFMT_ALLOWLIST_SCHEMA,
        entries,
    }
}

fn classify_shfmt_mismatch(
    allowlist: &ShfmtLargeCorpusAllowlistDocument,
    mismatch: &ShfmtOracleMismatch,
) -> ShfmtMismatchClassification {
    let mut path_matched = false;
    let mut hunk_matched = false;

    for (entry_index, entry) in allowlist.entries.iter().enumerate() {
        if !mismatch_path_matches(&mismatch.filename, &entry.path_suffix) {
            continue;
        }
        path_matched = true;

        if entry.hunks != mismatch.hunks {
            continue;
        }
        hunk_matched = true;

        if entry.diff_hash == mismatch.diff_hash {
            return ShfmtMismatchClassification::Reviewed { entry_index };
        }
    }

    if hunk_matched {
        return ShfmtMismatchClassification::ChangedKnown {
            reason: "known formatter oracle location changed output; diff hash no longer matches"
                .to_owned(),
        };
    }
    if path_matched {
        return ShfmtMismatchClassification::ChangedKnown {
            reason: "known formatter oracle fixture now differs at a different shfmt line range"
                .to_owned(),
        };
    }

    ShfmtMismatchClassification::New
}

fn mismatch_path_matches(filename: &str, path_suffix: &str) -> bool {
    filename == path_suffix || filename.ends_with(path_suffix)
}

fn stale_shfmt_allowlist_entries(
    allowlist: &ShfmtLargeCorpusAllowlistDocument,
    matched_allowlist_indices: &HashSet<usize>,
) -> Vec<String> {
    allowlist
        .entries
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched_allowlist_indices.contains(index))
        .map(|(_, entry)| {
            format!(
                "{} shfmt_lines={} diff_hash={}",
                entry.path_suffix,
                format_hunk_ranges(&entry.hunks),
                entry.diff_hash,
            )
        })
        .collect()
}

fn format_shfmt_mismatch_failure(mismatch: &ShfmtOracleMismatch, reason: &str) -> String {
    format!(
        "{}\n{}",
        mismatch.filename,
        indent_detail(&format!(
            "reason: {reason}\nshfmt_lines: {}\ndiff_hash: {}\n{}",
            format_hunk_ranges(&mismatch.hunks),
            mismatch.diff_hash,
            truncate_diff(&mismatch.diff),
        )),
    )
}

fn format_hunk_ranges(hunks: &[ShfmtDiffHunk]) -> String {
    hunks
        .iter()
        .map(|hunk| format!("{}-{}", hunk.start_line, hunk.end_line))
        .collect::<Vec<_>>()
        .join(",")
}

fn indent_detail(detail: &str) -> String {
    detail
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn large_corpus_oracle_passes(
    elapsed: Duration,
    max_duration: Duration,
    shuck_errors: &[String],
    shfmt_errors: &[String],
    blocking_mismatches: &[String],
) -> bool {
    elapsed <= max_duration
        && shuck_errors.is_empty()
        && shfmt_errors.is_empty()
        && blocking_mismatches.is_empty()
}

fn large_corpus_max_duration() -> Duration {
    Duration::from_secs(filtered_env_int(
        LARGE_CORPUS_MAX_DURATION_ENV,
        DEFAULT_LARGE_CORPUS_MAX_DURATION_SECS,
        |value| value > 0,
    ) as u64)
}

fn format_failure_list(title: &str, failures: &[String]) -> String {
    if failures.is_empty() {
        return String::new();
    }

    let limit = failures.len().min(MAX_LARGE_CORPUS_FAILURES);
    let mut report = format!("{title} (showing {limit}/{}):\n", failures.len());
    for failure in failures.iter().take(limit) {
        report.push_str(failure);
        report.push_str("\n\n");
    }
    report
}

fn env_truthy(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(default)
}

fn filtered_env_int(name: &str, default: usize, predicate: impl Fn(usize) -> bool) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| predicate(*value))
        .unwrap_or(default)
}

fn oracle_cases() -> Vec<OracleCase> {
    vec![
        OracleCase::new("function next line", "function_next_line.sh")
            .with_shfmt_flags(&["-fn"])
            .with_options(ShellFormatOptions::default().with_function_next_line(true)),
        OracleCase::new("case arms", "case_default.sh"),
        OracleCase::new("space redirects", "space_redirects.sh")
            .with_shfmt_flags(&["-sr"])
            .with_options(ShellFormatOptions::default().with_space_redirects(true)),
        OracleCase::new("keep padding", "keep_padding.sh")
            .with_shfmt_flags(&["-kp"])
            .with_options(ShellFormatOptions::default().with_keep_padding(true)),
        OracleCase::new("nested heredoc", "nested_heredoc.sh"),
        OracleCase::new("if body comment", "if_body_comment.sh"),
        OracleCase::new("heredoc trailing comment", "heredoc_trailing_comment.sh"),
        OracleCase::new("declare heredoc", "decl_heredoc.sh"),
        OracleCase::new("binary next line", "binary_next_line.sh")
            .with_shfmt_flags(&["-bn"])
            .with_options(ShellFormatOptions::default().with_binary_next_line(true)),
        OracleCase::new("simplify", "simplify.sh")
            .with_filename("simplify.bash")
            .with_shfmt_flags(&["-s"])
            .with_options(ShellFormatOptions::default().with_simplify(true)),
        OracleCase::new("minify", "minify.sh")
            .with_shfmt_flags(&["-mn"])
            .with_options(ShellFormatOptions::default().with_minify(true)),
        OracleCase::new("mksh select", "mksh_select.sh")
            .with_filename("script.mksh")
            .with_shfmt_flags(&["-ln=mksh"])
            .with_options(ShellFormatOptions::default().with_dialect(ShellDialect::Mksh)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(start_line: usize, end_line: usize) -> ShfmtDiffHunk {
        ShfmtDiffHunk {
            start_line,
            end_line,
        }
    }

    fn mismatch(filename: &str, hunks: Vec<ShfmtDiffHunk>, diff_hash: &str) -> ShfmtOracleMismatch {
        ShfmtOracleMismatch {
            filename: filename.to_owned(),
            hunks,
            diff_hash: diff_hash.to_owned(),
            diff: "--- shfmt/file\n+++ shuck/file\n@@ -1 +1 @@\n-a\n+b\n".to_owned(),
        }
    }

    fn allowlist(
        entries: Vec<ShfmtLargeCorpusAllowlistEntry>,
    ) -> ShfmtLargeCorpusAllowlistDocument {
        ShfmtLargeCorpusAllowlistDocument {
            schema: SHFMT_ALLOWLIST_SCHEMA,
            entries,
        }
    }

    fn entry(
        path_suffix: &str,
        hunks: Vec<ShfmtDiffHunk>,
        diff_hash: &str,
    ) -> ShfmtLargeCorpusAllowlistEntry {
        ShfmtLargeCorpusAllowlistEntry {
            path_suffix: path_suffix.to_owned(),
            hunks,
            diff_hash: diff_hash.to_owned(),
            reason: "known local formatter oracle difference".to_owned(),
        }
    }

    #[test]
    fn exact_allowlist_match_is_nonblocking() {
        let allowlist = allowlist(vec![entry(
            "repo__script.sh",
            vec![hunk(10, 12)],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )]);
        let mismatch = mismatch(
            "repo__script.sh",
            vec![hunk(10, 12)],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );

        assert_eq!(
            classify_shfmt_mismatch(&allowlist, &mismatch),
            ShfmtMismatchClassification::Reviewed { entry_index: 0 }
        );
    }

    #[test]
    fn committed_allowlist_loads_and_validates() {
        assert!(shfmt_allowlist_path().is_file());

        let allowlist = load_shfmt_large_corpus_allowlist();

        assert_eq!(allowlist.schema, SHFMT_ALLOWLIST_SCHEMA);
    }

    #[test]
    fn same_location_with_changed_hash_is_blocking() {
        let allowlist = allowlist(vec![entry(
            "repo__script.sh",
            vec![hunk(10, 12)],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )]);
        let mismatch = mismatch(
            "repo__script.sh",
            vec![hunk(10, 12)],
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );

        assert!(matches!(
            classify_shfmt_mismatch(&allowlist, &mismatch),
            ShfmtMismatchClassification::ChangedKnown { .. }
        ));
    }

    #[test]
    fn new_path_line_mismatch_is_blocking() {
        let allowlist = allowlist(vec![entry(
            "repo__script.sh",
            vec![hunk(10, 12)],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )]);
        let mismatch = mismatch(
            "other__script.sh",
            vec![hunk(20, 24)],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );

        assert_eq!(
            classify_shfmt_mismatch(&allowlist, &mismatch),
            ShfmtMismatchClassification::New
        );
    }

    #[test]
    fn missing_allowlist_entry_is_nonblocking_stale_hint() {
        let allowlist = allowlist(vec![
            entry(
                "repo__script.sh",
                vec![hunk(10, 12)],
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            entry(
                "fixed__script.sh",
                vec![hunk(2, 4)],
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
        ]);
        let matched = HashSet::from([0]);

        let stale = stale_shfmt_allowlist_entries(&allowlist, &matched);

        assert_eq!(stale.len(), 1);
        assert!(stale[0].contains("fixed__script.sh"));
    }

    #[test]
    fn shuck_formatter_errors_remain_blocking() {
        assert!(!large_corpus_oracle_passes(
            Duration::from_secs(1),
            Duration::from_secs(60),
            &["repo__script.sh: parse error".to_owned()],
            &[],
            &[],
        ));
    }

    #[test]
    fn parse_shfmt_hunk_header_reads_shfmt_side_range() {
        assert_eq!(
            parse_shfmt_hunk_header("@@ -42,3 +42,4 @@"),
            Some(hunk(42, 44))
        );
        assert_eq!(parse_shfmt_hunk_header("@@ -8 +8,2 @@"), Some(hunk(8, 8)));
    }
}
