use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Environment variable names (matching the Go test)
// ---------------------------------------------------------------------------

const LARGE_CORPUS_ENV: &str = "SHUCK_TEST_LARGE_CORPUS";
const LARGE_CORPUS_ROOT_ENV: &str = "SHUCK_LARGE_CORPUS_ROOT";
const LARGE_CORPUS_TIMEOUT_ENV: &str = "SHUCK_LARGE_CORPUS_TIMEOUT_SECS";
const LARGE_CORPUS_SHARD_ENV: &str = "TEST_SHARD_INDEX";
const LARGE_CORPUS_SHARDS_ENV: &str = "TEST_TOTAL_SHARDS";

const LARGE_CORPUS_DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const LARGE_CORPUS_CACHE_DIR_NAME: &str = ".cache/large-corpus";

const SHELLCHECK_CACHE_SCHEMA: u32 = 2;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LargeCorpusConfig {
    corpus_dir: PathBuf,
    cache_dir: PathBuf,
    timeout: Duration,
    shard_index: usize,
    total_shards: usize,
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LargeCorpusFixture {
    path: PathBuf,
    shell: String,
    source_hash: String,
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
    invocation_hash: String,
}

impl ShellCheckCache {
    fn new(cache_root: &Path, shellcheck_path: &str) -> Self {
        let invocation_hash = shellcheck_invocation_hash(shellcheck_path);
        Self {
            dir: cache_root.join("shellcheck"),
            invocation_hash,
        }
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
            "path": fixture.path.to_string_lossy(),
            "shell": fixture.shell,
            "sourceHash": fixture.source_hash,
            "invocationHash": self.invocation_hash,
        });
        let key = hash_bytes(key_data.to_string().as_bytes());
        self.dir.join(format!("{key}.json"))
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

    let shellcheck_path = find_shellcheck()
        .expect("shellcheck not found on PATH; install it to run the large corpus test");

    let supported_shells = shellcheck_supported_shells(&shellcheck_path);
    let shellcheck_index = build_shellcheck_index();
    let shellcheck_cache = ShellCheckCache::new(&cfg.cache_dir, &shellcheck_path);
    let linter_settings = shuck_linter::LinterSettings::default();

    for fixture in &fixtures {
        if !supported_shells.contains_key(fixture.shell.as_str()) {
            continue;
        }

        let mut issues: Vec<String> = Vec::new();

        let shuck_run = run_shuck(fixture, &linter_settings);
        if let Some(ref err) = shuck_run.parse_error {
            issues.push(format!("shuck parse error: {err}"));
        }

        match shellcheck_cache.run_fixture(fixture, &shellcheck_path, cfg.timeout) {
            Ok(sc_run) => {
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
                            &shellcheck_index,
                        ));
                    }
                }
            }
            Err(err) => {
                issues.push(format!("shellcheck error: {err}"));
            }
        }

        if !issues.is_empty() {
            panic!(
                "{}\n{}",
                fixture.path.display(),
                indent_detail(&issues.join("\n\n"))
            );
        }
    }
}

#[test]
#[ignore = "requires the large corpus; run `make test-large-corpus`"]
fn large_corpus_parses_without_panic() {
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

    run_parse_only(&fixtures);
}

fn run_parse_only(fixtures: &[LargeCorpusFixture]) {
    let linter_settings = shuck_linter::LinterSettings::default();
    let mut parse_errors = 0usize;
    let mut parse_successes = 0usize;

    for fixture in fixtures {
        let source = match fs::read_to_string(&fixture.path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  SKIP {}: {e}", fixture.path.display());
                continue;
            }
        };

        let output = match shuck_parser::parser::Parser::new(&source).parse() {
            Ok(o) => o,
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };

        // Run the full pipeline to catch panics in the indexer/semantic/linter.
        let indexer = shuck_indexer::Indexer::new(&source, &output);
        let semantic = shuck_semantic::SemanticModel::build(&output.script, &source, &indexer);
        let _ = shuck_linter::lint_file(
            &output.script,
            &source,
            &semantic,
            &indexer,
            &linter_settings,
            None,
        );

        parse_successes += 1;
    }

    eprintln!(
        "parsed {} fixtures: {} ok, {} errors",
        fixtures.len(),
        parse_successes,
        parse_errors
    );
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
                LARGE_CORPUS_TIMEOUT_ENV,
                LARGE_CORPUS_DEFAULT_TIMEOUT.as_secs() as usize,
            );
            let total_shards = positive_env_int(LARGE_CORPUS_SHARDS_ENV, 1);
            let shard_index = non_negative_env_int(LARGE_CORPUS_SHARD_ENV, 0);

            assert!(
                shard_index < total_shards,
                "{LARGE_CORPUS_SHARD_ENV}={shard_index}, want value in [0,{total_shards})"
            );

            return Some(LargeCorpusConfig {
                corpus_dir,
                cache_dir: default_root,
                timeout: Duration::from_secs(timeout_secs as u64),
                shard_index,
                total_shards,
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

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn load_fixtures(cfg: &LargeCorpusConfig) -> Vec<LargeCorpusFixture> {
    let mut fixtures = collect_fixtures(&cfg.corpus_dir);
    fixtures = shard_fixtures(fixtures, cfg.shard_index, cfg.total_shards);
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
        let src = match fs::read(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let shell = resolve_shell(&path, &src);
        let source_hash = hash_bytes(&src);

        fixtures.push(LargeCorpusFixture {
            path,
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
    let semantic = shuck_semantic::SemanticModel::build(&output.script, &source, &indexer);
    let diagnostics = shuck_linter::lint_file(
        &output.script,
        &source,
        &semantic,
        &indexer,
        linter_settings,
        None,
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

fn build_shellcheck_index() -> HashMap<String, String> {
    // Maps shuck rule codes to shellcheck codes.
    // As rules are added with shellcheck equivalents, add them here.
    HashMap::new()
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

fn find_shellcheck() -> Option<String> {
    Command::new("shellcheck")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| "shellcheck".into())
}

fn run_shellcheck(
    path: &Path,
    shell: &str,
    shellcheck_path: &str,
    _timeout: Duration,
) -> Result<ShellCheckRun, String> {
    let output = Command::new(shellcheck_path)
        .args(["--norc", "-s", shell, "-f", "json1"])
        .arg(path)
        .output()
        .map_err(|e| format!("shellcheck exec: {e}"))?;

    // shellcheck exits 1 when it finds issues, which is normal
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        if code != 1 {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("shellcheck exit {code}: {stderr}"));
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
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

fn decode_shellcheck_diagnostics(data: &[u8]) -> Result<Vec<ShellCheckDiagnostic>, String> {
    let data = data.iter().copied().collect::<Vec<_>>();
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
        if let Some(start) = line.find('(') {
            if let Some(end) = line[start + 1..].find(')') {
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
    }
    supported
}

fn shellcheck_invocation_hash(shellcheck_path: &str) -> String {
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
    fn shard_fixtures_contiguous_split() {
        let fixtures: Vec<LargeCorpusFixture> = (0..100)
            .map(|i| LargeCorpusFixture {
                path: PathBuf::from(format!("script-{i:03}.sh")),
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
}
