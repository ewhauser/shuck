use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{DirEntry, ParallelVisitor, WalkBuilder, WalkState};
use shuck_config::{resolve_project_root_for_file, resolve_project_root_for_input};
use shuck_extract::{is_embedded_host, is_extractable};
use shuck_linter::ShellDialect;

pub const DEFAULT_IGNORED_DIR_NAMES: &[&str] = &[
    ".shuck_cache",
    ".bzr",
    ".cache",
    ".git",
    ".hg",
    ".jj",
    ".svn",
    "node_modules",
    "vendor",
];

const SHEBANG_SNIFF_LIMIT_BYTES: u64 = 4096;

/// File extensions that positively identify a non-shell language. Explicitly
/// named files with these extensions are skipped instead of being parsed (and
/// possibly rewritten) as shell.
const NON_SHELL_EXTENSIONS: &[&str] = &[
    "awk", "bat", "c", "cc", "cjs", "clj", "cmake", "cmd", "cpp", "cs", "css", "cts", "cxx",
    "dart", "el", "erl", "ex", "exs", "fish", "go", "groovy", "h", "hh", "hpp", "hs", "htm",
    "html", "ipynb", "java", "jl", "js", "json", "jsonc", "jsx", "kt", "kts", "lisp", "lua",
    "markdown", "md", "mjs", "ml", "mts", "nim", "nu", "php", "pl", "pm", "ps1", "psm1", "py",
    "pyi", "pyw", "r", "rb", "rs", "rst", "scala", "scss", "sed", "swift", "tcl", "toml", "ts",
    "tsx", "vim", "xml", "zig",
];

/// Shebang interpreters that positively identify a non-shell language. Version
/// suffixes such as `python3.12` are stripped before matching.
const NON_SHELL_INTERPRETERS: &[&str] = &[
    "awk",
    "bun",
    "deno",
    "elixir",
    "escript",
    "expect",
    "fish",
    "gawk",
    "gmake",
    "groovy",
    "guile",
    "julia",
    "kotlin",
    "lua",
    "luajit",
    "make",
    "mawk",
    "nawk",
    "node",
    "nodejs",
    "nu",
    "ocaml",
    "osascript",
    "perl",
    "php",
    "powershell",
    "pwsh",
    "python",
    "racket",
    "raku",
    "rscript",
    "ruby",
    "runghc",
    "runhaskell",
    "sbcl",
    "scala",
    "sed",
    "swift",
    "tclsh",
    "ts-node",
    "tsx",
    "wish",
];

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProjectRoot {
    pub storage_root: PathBuf,
    pub canonical_root: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiscoveredFile {
    pub display_path: PathBuf,
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub project_root: ProjectRoot,
    pub kind: FileKind,
}

/// Files found by a discovery pass plus whether the configured scope was fully visited.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiscoveryResult {
    /// Discovered files in stable path order.
    pub files: Vec<DiscoveredFile>,
    /// Whether discovery finished without hitting a work limit, cancellation,
    /// or an unreadable path.
    pub complete: bool,
    /// Number of filesystem entries examined during this pass.
    pub visited_entries: usize,
    /// Paths that could not be read or walked. Discovery continues past them.
    pub errors: Vec<DiscoveryError>,
    /// Explicitly named inputs that were not processed because they are not shell.
    pub skipped: Vec<SkippedInput>,
}

/// A path that discovery could not process.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiscoveryError {
    /// The affected path, relative to the working directory when possible.
    pub path: PathBuf,
    /// A description of the failure, without the path.
    pub message: String,
}

/// An explicitly named input that discovery declined to process.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SkippedInput {
    /// The skipped path, relative to the working directory when possible.
    pub path: PathBuf,
    /// Why the input was skipped.
    pub reason: &'static str,
}

/// How an explicitly named input should be handled.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExplicitInputKind {
    /// Process the input as the given kind.
    Process(FileKind),
    /// Skip the input; the reason describes why.
    Skip(&'static str),
}

#[derive(Default)]
struct DiscoveryState {
    files: BTreeMap<PathBuf, DiscoveredFile>,
    visited_entries: usize,
    errors: Vec<DiscoveryError>,
    skipped: Vec<SkippedInput>,
}

impl DiscoveryState {
    fn push_error(&mut self, path: &Path, cwd: &Path, message: impl Into<String>) {
        self.errors.push(DiscoveryError {
            path: display_path(path, cwd),
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum FileKind {
    Shell,
    Embedded,
}

#[derive(Debug)]
pub struct DiscoveryOptions {
    pub exclude_patterns: Vec<String>,
    pub extend_exclude_patterns: Vec<String>,
    pub respect_gitignore: bool,
    pub force_exclude: bool,
    pub parallel: bool,
    pub cache_root: Option<PathBuf>,
    pub use_config_roots: bool,
    /// Stop discovery after retaining this many files.
    pub max_files: Option<usize>,
    /// Stop discovery after visiting this many filesystem entries.
    pub max_entries: Option<usize>,
    /// Optional shared cancellation signal checked during directory walks.
    pub cancellation: Option<DiscoveryCancellationToken>,
    /// Whether embedded host documents may be returned.
    pub include_embedded: bool,
    /// Exact directory subtrees to prune from recursive discovery.
    pub excluded_subtrees: Vec<PathBuf>,
}

/// A clonable cancellation signal for bounded file discovery.
#[derive(Clone, Debug, Default)]
pub struct DiscoveryCancellationToken(Arc<AtomicBool>);

impl DiscoveryCancellationToken {
    /// Mark the associated discovery operation as cancelled.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Return whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            exclude_patterns: Vec::new(),
            extend_exclude_patterns: Vec::new(),
            respect_gitignore: false,
            force_exclude: false,
            parallel: false,
            cache_root: None,
            use_config_roots: true,
            max_files: None,
            max_entries: None,
            cancellation: None,
            include_embedded: true,
            excluded_subtrees: Vec::new(),
        }
    }
}

/// Discover files, failing when any path could not be read or walked.
///
/// Use [`discover_files_with_status`] to keep the files that were found
/// alongside per-path errors.
pub fn discover_files(
    inputs: &[PathBuf],
    cwd: &Path,
    options: &DiscoveryOptions,
) -> Result<Vec<DiscoveredFile>> {
    let result = discover_files_with_status(inputs, cwd, options)?;
    if let Some(error) = result.errors.first() {
        return Err(anyhow!("{}: {}", error.path.display(), error.message));
    }
    Ok(result.files)
}

/// Discover files and report whether configured work bounds truncated the walk.
pub fn discover_files_with_status(
    inputs: &[PathBuf],
    cwd: &Path,
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult> {
    let exclude_matcher =
        ExcludeMatcher::new(&options.exclude_patterns, &options.extend_exclude_patterns)?;
    let mut explicit_ignore_cache = ExplicitIgnoreCache::new(cwd);

    let resolved_inputs = if inputs.is_empty() {
        vec![cwd.to_path_buf()]
    } else {
        inputs
            .iter()
            .map(|input| {
                if input.is_absolute() {
                    input.clone()
                } else {
                    cwd.join(input)
                }
            })
            .collect()
    };

    let mut state = DiscoveryState::default();
    for input in resolved_inputs {
        if discovery_stopped(options, state.files.len(), state.visited_entries) {
            break;
        }
        let input = normalize_path(&input);

        // Default ignored directory names apply only beneath the input. An
        // explicitly named path is never skipped because some ancestor (for
        // example `~/.cache` or a `vendor/` checkout) happens to use one.
        if path_matches_cache_root(&input, options.cache_root.as_deref()) {
            continue;
        }
        if options
            .excluded_subtrees
            .iter()
            .any(|excluded| input.starts_with(excluded))
        {
            continue;
        }

        let metadata = match fs::metadata(&input) {
            Ok(metadata) => metadata,
            Err(err) => {
                state.push_error(&input, cwd, format!("cannot access path: {err}"));
                continue;
            }
        };

        let storage_root = resolve_project_root_for_input(&input, options.use_config_roots)
            .with_context(|| format!("resolve project root for {}", input.display()))?;
        let canonical_root = fs::canonicalize(&storage_root)
            .with_context(|| format!("canonicalize {}", storage_root.display()))?;
        let project_root = ProjectRoot {
            storage_root,
            canonical_root,
        };

        // With `--force-exclude`, explicit inputs inside a default ignored
        // directory are skipped like walked files would be. Only components
        // below the working directory (or, outside it, below the project root)
        // count.
        if options.force_exclude
            && input
                .strip_prefix(cwd)
                .or_else(|_| input.strip_prefix(&project_root.storage_root))
                .is_ok_and(path_has_default_ignored_component)
        {
            continue;
        }

        collect_input(
            &input,
            &metadata,
            cwd,
            &project_root,
            &exclude_matcher,
            &mut explicit_ignore_cache,
            options,
            &mut state,
        )?;
    }

    let complete = !discovery_stopped(options, state.files.len(), state.visited_entries)
        && state.errors.is_empty();
    Ok(DiscoveryResult {
        files: state.files.into_values().collect(),
        complete,
        visited_entries: state.visited_entries,
        errors: state.errors,
        skipped: state.skipped,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_input(
    input: &Path,
    metadata: &fs::Metadata,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    explicit_ignore_cache: &mut ExplicitIgnoreCache,
    options: &DiscoveryOptions,
    state: &mut DiscoveryState,
) -> Result<()> {
    if metadata.is_dir() {
        if options.force_exclude && exclude_matcher.matches(input, cwd) {
            return Ok(());
        }
        collect_directory(input, cwd, project_root, exclude_matcher, options, state)?;
    } else if metadata.is_file() {
        state.visited_entries += 1;
        let kind = match explicit_file_kind(input) {
            Ok(ExplicitInputKind::Process(kind)) => kind,
            Ok(ExplicitInputKind::Skip(reason)) => {
                state.skipped.push(SkippedInput {
                    path: display_path(input, cwd),
                    reason,
                });
                return Ok(());
            }
            Err(err) => {
                state.push_error(input, cwd, err);
                return Ok(());
            }
        };
        if kind == FileKind::Embedded && !options.include_embedded {
            return Ok(());
        }
        if options.force_exclude {
            if exclude_matcher.matches(input, cwd) {
                return Ok(());
            }
            if !is_allowed_by_gitignore(input, explicit_ignore_cache, options.respect_gitignore)? {
                return Ok(());
            }
        }

        let fallback_start = input
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if let Err(err) = add_file(
            input,
            cwd,
            &fallback_start,
            project_root,
            options.use_config_roots,
            kind,
            &mut state.files,
        ) {
            state.push_error(input, cwd, format!("{err:#}"));
        }
    }

    Ok(())
}

fn collect_directory(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    options: &DiscoveryOptions,
    state: &mut DiscoveryState,
) -> Result<()> {
    let mut builder = WalkBuilder::new(input);
    configure_walk_builder(
        &mut builder,
        options.respect_gitignore,
        options.cache_root.clone(),
        options.excluded_subtrees.clone(),
    );
    if options.parallel
        && options.max_files.is_none()
        && options.max_entries.is_none()
        && options.cancellation.is_none()
    {
        return collect_directory_parallel(
            input,
            cwd,
            project_root,
            exclude_matcher,
            options.use_config_roots,
            &mut builder,
            state,
        );
    }

    // Limits keep the first files walked, so walk in a stable order rather
    // than whatever order the filesystem returns directory entries in.
    builder.sort_by_file_name(|left, right| left.cmp(right));
    for entry in builder.build() {
        if discovery_stopped(options, state.files.len(), state.visited_entries) {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let (path, message) = walk_error_parts(&err);
                state.push_error(path.unwrap_or(input), cwd, message);
                continue;
            }
        };
        state.visited_entries += 1;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if should_ignore_walked_path(path, input, options.cache_root.as_deref())
            || exclude_matcher.matches(path, cwd)
        {
            continue;
        }
        let kind = match discovered_file_kind(path) {
            Ok(Some(kind)) => kind,
            Ok(None) => continue,
            Err(err) => {
                state.push_error(path, cwd, err);
                continue;
            }
        };
        if kind == FileKind::Embedded && !options.include_embedded {
            continue;
        }

        if let Err(err) = add_file(
            path,
            cwd,
            input,
            project_root,
            options.use_config_roots,
            kind,
            &mut state.files,
        ) {
            state.push_error(path, cwd, format!("{err:#}"));
        }
    }

    Ok(())
}

fn discovery_stopped(
    options: &DiscoveryOptions,
    file_count: usize,
    visited_entries: usize,
) -> bool {
    options
        .cancellation
        .as_ref()
        .is_some_and(DiscoveryCancellationToken::is_cancelled)
        || options
            .max_files
            .is_some_and(|max_files| file_count >= max_files)
        || options
            .max_entries
            .is_some_and(|max_entries| visited_entries >= max_entries)
}

fn collect_directory_parallel(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    use_config_roots: bool,
    builder: &mut WalkBuilder,
    discovery_state: &mut DiscoveryState,
) -> Result<()> {
    builder.threads(
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(12),
    );

    let state = ParallelDiscoveryState::default();
    let walker = builder.build_parallel();
    let mut visitor = ShellFilesVisitorBuilder::new(input, cwd, exclude_matcher, &state);
    walker.visit(&mut visitor);

    let mut errors = state
        .errors
        .into_inner()
        .map_err(|_| anyhow!("parallel discovery error state mutex poisoned"))?;
    errors.sort_by(|left, right| left.path.cmp(&right.path));
    discovery_state.errors.append(&mut errors);

    let mut matched_paths = state
        .matched_paths
        .into_inner()
        .map_err(|_| anyhow!("parallel discovery matched-path state mutex poisoned"))?;
    matched_paths
        .sort_by(|left, right| left.path.cmp(&right.path).then(left.kind.cmp(&right.kind)));
    for matched_path in matched_paths {
        if let Err(err) = add_file(
            &matched_path.path,
            cwd,
            input,
            project_root,
            use_config_roots,
            matched_path.kind,
            &mut discovery_state.files,
        ) {
            discovery_state.push_error(&matched_path.path, cwd, format!("{err:#}"));
        }
    }

    Ok(())
}

fn configure_walk_builder(
    builder: &mut WalkBuilder,
    respect_gitignore: bool,
    cache_root: Option<PathBuf>,
    excluded_subtrees: Vec<PathBuf>,
) {
    builder.hidden(false);
    builder.parents(respect_gitignore);
    builder.ignore(respect_gitignore);
    builder.git_ignore(respect_gitignore);
    builder.git_global(respect_gitignore);
    builder.git_exclude(respect_gitignore);
    builder.require_git(false);
    builder.filter_entry(move |entry| {
        !is_ignored_directory(entry, cache_root.as_deref(), &excluded_subtrees)
    });
}

fn is_ignored_directory(
    entry: &DirEntry,
    cache_root: Option<&Path>,
    excluded_subtrees: &[PathBuf],
) -> bool {
    entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
        && (DEFAULT_IGNORED_DIR_NAMES
            .iter()
            .any(|name| entry.file_name() == OsStr::new(name))
            || path_matches_cache_root(entry.path(), cache_root)
            || excluded_subtrees
                .iter()
                .any(|excluded| entry.path().starts_with(excluded)))
}

#[derive(Debug)]
struct ExplicitIgnoreCache {
    cwd: PathBuf,
    matchers: BTreeMap<PathBuf, Gitignore>,
}

impl ExplicitIgnoreCache {
    fn new(cwd: &Path) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            matchers: BTreeMap::new(),
        }
    }

    fn allows(&mut self, path: &Path) -> Result<bool> {
        if !path.starts_with(&self.cwd) {
            return Ok(true);
        }

        let directory = path.parent().unwrap_or(&self.cwd);
        let key = explicit_ignore_cache_key(directory, &self.cwd);
        if !self.matchers.contains_key(&key) {
            let gitignore = build_gitignore_matcher(directory, &self.cwd)?;
            self.matchers.insert(key.clone(), gitignore);
        }

        let Some(matcher) = self.matchers.get(&key) else {
            unreachable!("gitignore matcher should be cached for the computed key");
        };
        Ok(!matcher.matched_path_or_any_parents(path, false).is_ignore())
    }
}

fn is_allowed_by_gitignore(
    path: &Path,
    cache: &mut ExplicitIgnoreCache,
    respect_gitignore: bool,
) -> Result<bool> {
    if !respect_gitignore {
        return Ok(true);
    }

    cache.allows(path)
}

fn explicit_ignore_cache_key(directory: &Path, cwd: &Path) -> PathBuf {
    directory
        .strip_prefix(cwd)
        .map(normalize_path)
        .unwrap_or_else(|_| normalize_path(directory))
}

fn build_gitignore_matcher(path: &Path, cwd: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(cwd);
    for dir in candidate_ignore_directories(path, cwd) {
        for name in [".ignore", ".gitignore"] {
            let ignore_file = dir.join(name);
            if ignore_file.is_file()
                && let Some(error) = builder.add(&ignore_file)
            {
                return Err(error.into());
            }
        }
    }

    Ok(builder.build()?)
}

fn candidate_ignore_directories(path: &Path, cwd: &Path) -> Vec<PathBuf> {
    let start = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(cwd)
    };

    let mut directories = Vec::new();
    let mut current = Some(start);
    while let Some(dir) = current {
        if !dir.starts_with(cwd) {
            break;
        }
        directories.push(dir.to_path_buf());
        if dir == cwd {
            break;
        }
        current = dir.parent();
    }
    directories.reverse();
    directories
}

/// Returns whether a file found while walking `walk_root` should be skipped.
/// Only components below the walk root are matched against the default ignored
/// directory names, so ancestors of the requested directory never hide it.
fn should_ignore_walked_path(path: &Path, walk_root: &Path, cache_root: Option<&Path>) -> bool {
    path.strip_prefix(walk_root)
        .is_ok_and(path_has_default_ignored_component)
        || path_matches_cache_root(path, cache_root)
}

/// Splits a walk error into the path it concerns (when known) and a message
/// that does not repeat that path.
fn walk_error_parts(err: &ignore::Error) -> (Option<&Path>, String) {
    match err {
        ignore::Error::WithPath { path, err } => (Some(path), walk_error_parts(err).1),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            walk_error_parts(err)
        }
        _ => (None, format!("cannot walk directory: {err}")),
    }
}

fn path_has_default_ignored_component(path: &Path) -> bool {
    path.components().any(|component| {
        let std::path::Component::Normal(part) = component else {
            return false;
        };
        DEFAULT_IGNORED_DIR_NAMES
            .iter()
            .any(|name| part == OsStr::new(name))
    })
}

fn path_matches_cache_root(path: &Path, cache_root: Option<&Path>) -> bool {
    cache_root.is_some_and(|cache_root| path.starts_with(cache_root))
}

#[derive(Default)]
struct ParallelDiscoveryState {
    matched_paths: Mutex<Vec<MatchedPath>>,
    errors: Mutex<Vec<DiscoveryError>>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct MatchedPath {
    path: PathBuf,
    kind: FileKind,
}

struct ShellFilesVisitorBuilder<'a> {
    input: &'a Path,
    cwd: &'a Path,
    exclude_matcher: &'a ExcludeMatcher,
    state: &'a ParallelDiscoveryState,
}

impl<'a> ShellFilesVisitorBuilder<'a> {
    fn new(
        input: &'a Path,
        cwd: &'a Path,
        exclude_matcher: &'a ExcludeMatcher,
        state: &'a ParallelDiscoveryState,
    ) -> Self {
        Self {
            input,
            cwd,
            exclude_matcher,
            state,
        }
    }
}

struct ShellFilesVisitor<'a> {
    input: &'a Path,
    cwd: &'a Path,
    exclude_matcher: &'a ExcludeMatcher,
    state: &'a ParallelDiscoveryState,
    local_paths: Vec<MatchedPath>,
    local_errors: Vec<DiscoveryError>,
}

impl<'a> ignore::ParallelVisitorBuilder<'a> for ShellFilesVisitorBuilder<'a> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 'a> {
        Box::new(ShellFilesVisitor {
            input: self.input,
            cwd: self.cwd,
            exclude_matcher: self.exclude_matcher,
            state: self.state,
            local_paths: Vec::new(),
            local_errors: Vec::new(),
        })
    }
}

impl ParallelVisitor for ShellFilesVisitor<'_> {
    fn visit(&mut self, result: std::result::Result<DirEntry, ignore::Error>) -> WalkState {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                let (path, message) = walk_error_parts(&err);
                self.local_errors.push(DiscoveryError {
                    path: display_path(path.unwrap_or(self.input), self.cwd),
                    message,
                });
                return WalkState::Continue;
            }
        };

        let Some(file_type) = entry.file_type() else {
            return WalkState::Continue;
        };
        if !file_type.is_file() {
            return WalkState::Continue;
        }

        let path = entry.path();
        if self.exclude_matcher.matches(path, self.cwd) {
            return WalkState::Continue;
        }

        match discovered_file_kind(path) {
            Ok(Some(kind)) => self.local_paths.push(MatchedPath {
                path: entry.into_path(),
                kind,
            }),
            Ok(None) => {}
            Err(err) => self.local_errors.push(DiscoveryError {
                path: display_path(path, self.cwd),
                message: err,
            }),
        }

        WalkState::Continue
    }
}

impl Drop for ShellFilesVisitor<'_> {
    fn drop(&mut self) {
        if !self.local_paths.is_empty() {
            let mut matched_paths = lock_or_recover(&self.state.matched_paths);
            matched_paths.append(&mut self.local_paths);
        }

        if !self.local_errors.is_empty() {
            let mut errors = lock_or_recover(&self.state.errors);
            errors.append(&mut self.local_errors);
        }
    }
}

pub fn add_file(
    path: &Path,
    cwd: &Path,
    fallback_start: &Path,
    default_project_root: &ProjectRoot,
    use_config_roots: bool,
    kind: FileKind,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let absolute_path =
        fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    let storage_root = resolve_project_root_for_file(path, fallback_start, use_config_roots)
        .with_context(|| format!("resolve project root for {}", path.display()))?;
    let canonical_root = fs::canonicalize(&storage_root)
        .with_context(|| format!("canonicalize {}", storage_root.display()))?;
    let project_root = if storage_root == default_project_root.storage_root
        && canonical_root == default_project_root.canonical_root
    {
        default_project_root.clone()
    } else {
        ProjectRoot {
            storage_root,
            canonical_root,
        }
    };
    let relative_path = absolute_path
        .strip_prefix(&project_root.canonical_root)
        .map(Path::to_path_buf)
        .with_context(|| {
            format!(
                "file {} is not under project root {}",
                absolute_path.display(),
                project_root.canonical_root.display()
            )
        })?;
    let display_path = display_path(path, cwd);

    files
        .entry(absolute_path.clone())
        .or_insert_with(|| DiscoveredFile {
            display_path,
            absolute_path,
            relative_path,
            project_root,
            kind,
        });

    Ok(())
}

pub fn display_path(path: &Path, cwd: &Path) -> PathBuf {
    path.strip_prefix(cwd)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| normalize_path(path))
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

pub fn is_shell_script(path: &Path) -> Result<bool> {
    if ShellDialect::infer_from_path(path) != ShellDialect::Unknown {
        return Ok(true);
    }

    let bytes = read_shebang_prefix(path).map_err(|err| anyhow!(err))?;
    Ok(source_prefix_is_shell(&bytes))
}

fn source_prefix_is_shell(bytes: &[u8]) -> bool {
    // Zsh completion/autoload functions lead with a `#compdef`/`#autoload` tag on
    // the first line (compinit requires the tag there, so no shebang can precede
    // it). Recognize them so these extensionless zsh files are discovered; the
    // linter already infers the zsh dialect from the same marker.
    infer_shebang_dialect(bytes).is_some() || first_line_is_zsh_autoload_tag(bytes)
}

/// Classifies an explicitly named input (a command-line path or a
/// `--stdin-filename`) from its path and the start of its contents.
///
/// A directly named file is an authoritative request to process it, so files
/// without any recognizable marker are still parsed as shell. Files that
/// positively identify as another language, through a non-shell extension or
/// shebang, are skipped so they are never linted or rewritten as shell, and
/// YAML or other embedded host formats are only processed when an extractor
/// supports them.
pub fn classify_explicit_input(path: &Path, source_prefix: &[u8]) -> ExplicitInputKind {
    if ShellDialect::infer_from_path(path) != ShellDialect::Unknown
        || source_prefix_is_shell(source_prefix)
    {
        return ExplicitInputKind::Process(FileKind::Shell);
    }
    if is_extractable(path) {
        return ExplicitInputKind::Process(FileKind::Embedded);
    }
    if is_embedded_host(path) {
        return ExplicitInputKind::Skip("no supported embedded shell scripts in this file type");
    }
    if has_non_shell_extension(path) || has_non_shell_shebang(source_prefix) {
        return ExplicitInputKind::Skip("not a shell script");
    }
    ExplicitInputKind::Process(FileKind::Shell)
}

fn has_non_shell_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            let extension = extension.to_ascii_lowercase();
            NON_SHELL_EXTENSIONS.contains(&extension.as_str())
        })
}

fn has_non_shell_shebang(src: &[u8]) -> bool {
    let Some(first_line) = src.split(|byte| *byte == b'\n').next() else {
        return false;
    };
    let Some(interpreter) = std::str::from_utf8(first_line)
        .ok()
        .and_then(shuck_parser::shebang::interpreter_name)
    else {
        return false;
    };
    let interpreter = interpreter.to_ascii_lowercase();
    let name = interpreter.trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
    NON_SHELL_INTERPRETERS.contains(&name)
}

fn first_line_is_zsh_autoload_tag(src: &[u8]) -> bool {
    src.split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .is_some_and(ShellDialect::is_zsh_autoload_tag_line)
}

/// Classifies a walked file. Errors are path-free messages.
fn discovered_file_kind(path: &Path) -> std::result::Result<Option<FileKind>, String> {
    if ShellDialect::infer_from_path(path) != ShellDialect::Unknown {
        return Ok(Some(FileKind::Shell));
    }
    if source_prefix_is_shell(&read_shebang_prefix(path)?) {
        return Ok(Some(FileKind::Shell));
    }
    if is_extractable(path) {
        return Ok(Some(FileKind::Embedded));
    }
    Ok(None)
}

/// Classifies a directly named file. Directory walks use
/// `discovered_file_kind` instead so unknown files are not parsed
/// indiscriminately. Errors are path-free messages.
fn explicit_file_kind(path: &Path) -> std::result::Result<ExplicitInputKind, String> {
    if ShellDialect::infer_from_path(path) != ShellDialect::Unknown {
        return Ok(ExplicitInputKind::Process(FileKind::Shell));
    }
    Ok(classify_explicit_input(path, &read_shebang_prefix(path)?))
}

fn read_shebang_prefix(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|err| format!("cannot open file: {err}"))?;
    let mut reader = BufReader::new(file).take(SHEBANG_SNIFF_LIMIT_BYTES);
    let mut bytes = Vec::new();
    reader
        .read_until(b'\n', &mut bytes)
        .map_err(|err| format!("cannot read file: {err}"))?;
    Ok(bytes)
}

fn infer_shebang_dialect(src: &[u8]) -> Option<&'static str> {
    let first_line = src.split(|byte| *byte == b'\n').next()?;
    let line = std::str::from_utf8(first_line).ok()?;
    let interpreter = shuck_parser::shebang::interpreter_name(line)?;

    match interpreter.to_ascii_lowercase().as_str() {
        "bash" => Some("bash"),
        "sh" => Some("sh"),
        "dash" => Some("dash"),
        "ksh" => Some("ksh"),
        "mksh" => Some("mksh"),
        "bats" => Some("bats"),
        "zsh" => Some("zsh"),
        _ => None,
    }
}

struct ExcludeMatcher {
    set: Option<GlobSet>,
}

impl ExcludeMatcher {
    fn new(exclude_patterns: &[String], extend_exclude_patterns: &[String]) -> Result<Self> {
        if exclude_patterns.is_empty() && extend_exclude_patterns.is_empty() {
            return Ok(Self { set: None });
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in exclude_patterns.iter().chain(extend_exclude_patterns) {
            let glob = Glob::new(pattern)
                .map_err(|err| anyhow!("invalid exclude pattern `{pattern}`: {err}"))?;
            builder.add(glob);
        }

        Ok(Self {
            set: Some(builder.build()?),
        })
    }

    fn matches(&self, path: &Path, cwd: &Path) -> bool {
        let Some(set) = &self.set else {
            return false;
        };

        if set.is_match(path) {
            return true;
        }

        if let Ok(relative) = path.strip_prefix(cwd)
            && set.is_match(relative)
        {
            return true;
        }

        path.file_name()
            .map(|name| set.is_match(Path::new(name)))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn detects_extensionless_bash_shebang() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script");
        fs::write(&script, "#!/bin/bash\necho ok\n").unwrap();

        assert!(is_shell_script(&script).unwrap());
    }

    #[test]
    fn detects_env_zsh_shebang() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script");
        fs::write(&script, "#!/usr/bin/env zsh\nprint ok\n").unwrap();

        assert!(is_shell_script(&script).unwrap());
    }

    #[test]
    fn detects_env_split_bash_shebang() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script");
        fs::write(&script, "#!/usr/bin/env -S bash -e\necho ok\n").unwrap();

        assert!(is_shell_script(&script).unwrap());
    }

    #[test]
    fn detects_common_zsh_dotfiles_without_shebangs() {
        let tempdir = tempdir().unwrap();
        for name in [".zshrc", ".zshenv", ".zprofile", ".zlogin", ".zlogout"] {
            let script = tempdir.path().join(name);
            fs::write(&script, "plugins=(git)\n").unwrap();
            assert!(is_shell_script(&script).unwrap(), "{name}");
        }
    }

    #[test]
    fn detects_known_shell_filenames_without_shebangs() {
        let tempdir = tempdir().unwrap();
        for name in ["PKGBUILD", ".bashrc", ".profile"] {
            let script = tempdir.path().join(name);
            fs::write(&script, "echo ok\n").unwrap();
            assert!(is_shell_script(&script).unwrap(), "{name}");
        }
    }

    #[test]
    fn detects_extensionless_compdef_completion_file() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("_zdot");
        fs::write(&script, "#compdef zdot\n_arguments '*:: :->args'\n").unwrap();

        assert!(is_shell_script(&script).unwrap());
    }

    #[test]
    fn detects_extensionless_autoload_tag_file() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("myfunc");
        fs::write(&script, "#autoload\nprint hi\n").unwrap();

        assert!(is_shell_script(&script).unwrap());
    }

    #[test]
    fn ignores_spaced_compdef_comment_without_shebang() {
        let tempdir = tempdir().unwrap();
        let file = tempdir.path().join("notes");
        fs::write(
            &file,
            "# compdef is discussed in these notes\nplain prose\n",
        )
        .unwrap();

        assert!(!is_shell_script(&file).unwrap());
    }

    #[test]
    fn ignores_large_extensionless_non_shell_file() {
        let tempdir = tempdir().unwrap();
        let file = tempdir.path().join("blob");
        fs::write(&file, vec![b'x'; (SHEBANG_SNIFF_LIMIT_BYTES as usize) * 2]).unwrap();

        assert!(!is_shell_script(&file).unwrap());
    }

    #[test]
    fn explicit_regular_files_bypass_recursive_candidate_filtering() {
        let tempdir = tempdir().unwrap();
        let extensionless = tempdir.path().join("script");
        let executable = tempdir.path().join("tool");
        let non_shell = tempdir.path().join("notes.txt");
        for path in [&extensionless, &executable, &non_shell] {
            fs::write(path, "plain text without a shell marker\n").unwrap();
        }
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable, permissions).unwrap();
        }

        let files = discover_files(
            &[extensionless, executable, non_shell],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();

        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|file| file.kind == FileKind::Shell));
    }

    #[test]
    fn explicit_plain_yaml_is_not_treated_as_shell() {
        let tempdir = tempdir().unwrap();
        let plain_yaml = tempdir.path().join("config.yaml");
        let workflows = tempdir.path().join(".github/workflows");
        let workflow = workflows.join("ci.yml");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(&plain_yaml, "key: value\n").unwrap();
        fs::write(
            &workflow,
            "on: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps: []\n",
        )
        .unwrap();

        let files = discover_files(
            &[plain_yaml, workflow],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].display_path,
            PathBuf::from(".github/workflows/ci.yml")
        );
        assert_eq!(files[0].kind, FileKind::Embedded);
    }

    #[test]
    fn directory_walk_keeps_unknown_regular_files_filtered() {
        let tempdir = tempdir().unwrap();
        let executable = tempdir.path().join("tool");
        fs::write(&executable, "plain text without a shell marker\n").unwrap();
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable, permissions).unwrap();
        }
        fs::write(tempdir.path().join("notes.txt"), "not shell\n").unwrap();
        fs::write(tempdir.path().join("script"), "#!/bin/sh\necho ok\n").unwrap();
        fs::write(tempdir.path().join("PKGBUILD"), "echo package\n").unwrap();
        fs::write(tempdir.path().join(".bashrc"), "echo bash\n").unwrap();
        fs::write(tempdir.path().join(".profile"), "echo profile\n").unwrap();
        fs::write(tempdir.path().join(".zshrc"), "print zsh\n").unwrap();

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();
        let display_paths = files
            .iter()
            .map(|file| file.display_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            display_paths,
            vec![
                PathBuf::from(".bashrc"),
                PathBuf::from(".profile"),
                PathBuf::from(".zshrc"),
                PathBuf::from("PKGBUILD"),
                PathBuf::from("script"),
            ]
        );
    }

    #[test]
    fn reuses_explicit_ignore_matcher_for_files_in_same_directory() {
        let tempdir = tempdir().unwrap();
        let ignored_dir = tempdir.path().join("ignored");
        fs::create_dir_all(&ignored_dir).unwrap();
        fs::write(tempdir.path().join(".gitignore"), "ignored/\n").unwrap();

        let first = ignored_dir.join("first.sh");
        let second = ignored_dir.join("second.sh");
        fs::write(&first, "#!/bin/bash\necho one\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho two\n").unwrap();

        let mut cache = ExplicitIgnoreCache::new(tempdir.path());
        assert!(!is_allowed_by_gitignore(&first, &mut cache, true).unwrap());
        assert!(!is_allowed_by_gitignore(&second, &mut cache, true).unwrap());
        assert_eq!(cache.matchers.len(), 1);
    }

    #[test]
    fn discovers_github_actions_workflows_as_embedded_files() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            workflows.join("ci.yml"),
            "on: push\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps: []\n",
        )
        .unwrap();

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].display_path,
            PathBuf::from(".github/workflows/ci.yml")
        );
        assert_eq!(files[0].kind, FileKind::Embedded);
    }

    #[test]
    fn bounded_discovery_stops_after_the_file_limit() {
        let tempdir = tempdir().unwrap();
        // Some filesystems list the newest entries first, so a walk that
        // follows raw directory order would return the highest names here.
        for index in 0..20 {
            fs::write(
                tempdir.path().join(format!("script-{index:02}.sh")),
                "echo ok\n",
            )
            .unwrap();
        }

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions {
                max_files: Some(3),
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();

        let names = files
            .iter()
            .map(|file| file.display_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                PathBuf::from("script-00.sh"),
                PathBuf::from("script-01.sh"),
                PathBuf::from("script-02.sh"),
            ]
        );
    }

    #[test]
    fn bounded_discovery_stops_after_the_entry_work_limit() {
        let tempdir = tempdir().unwrap();
        for index in 0..20 {
            fs::write(
                tempdir.path().join(format!("a-note-{index:02}.txt")),
                "not shell\n",
            )
            .unwrap();
        }
        fs::write(tempdir.path().join("z-script.sh"), "echo late\n").unwrap();

        let result = discover_files_with_status(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions {
                max_files: Some(10),
                max_entries: Some(5),
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();

        assert!(!result.complete);
        assert!(result.visited_entries <= 5);
    }

    #[test]
    fn bounded_discovery_honors_preexisting_cancellation() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("script.sh"), "echo ok\n").unwrap();
        let cancellation = DiscoveryCancellationToken::default();
        cancellation.cancel();

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions {
                cancellation: Some(cancellation),
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();

        assert!(files.is_empty());
    }

    #[test]
    fn discovery_can_omit_embedded_hosts_before_the_limit() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(workflows.join("ci.yml"), "on: push\njobs: {}\n").unwrap();
        fs::write(tempdir.path().join("script.sh"), "echo ok\n").unwrap();

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions {
                max_files: Some(1),
                include_embedded: false,
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, FileKind::Shell);
    }

    #[test]
    fn bounded_discovery_prunes_nested_workspace_subtrees() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("a-nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("nested.sh"), "echo nested\n").unwrap();
        fs::write(tempdir.path().join("z-parent.sh"), "echo parent\n").unwrap();

        let files = discover_files(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &DiscoveryOptions {
                max_files: Some(1),
                excluded_subtrees: vec![nested],
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path, PathBuf::from("z-parent.sh"));
    }

    #[test]
    fn explicit_files_in_other_languages_are_skipped() {
        let tempdir = tempdir().unwrap();
        let python = tempdir.path().join("tool.py");
        let python_shebang = tempdir.path().join("tool");
        let ruby_versioned = tempdir.path().join("task");
        let shell_shebang_py = tempdir.path().join("wrapper.py");
        fs::write(&python, "print(1)\n").unwrap();
        fs::write(&python_shebang, "#!/usr/bin/env python3.12\nprint(1)\n").unwrap();
        fs::write(&ruby_versioned, "#!/usr/bin/ruby2\nputs 1\n").unwrap();
        fs::write(&shell_shebang_py, "#!/bin/sh\nexec python3 \"$@\"\n").unwrap();

        let result = discover_files_with_status(
            &[
                python.clone(),
                python_shebang.clone(),
                ruby_versioned.clone(),
                shell_shebang_py.clone(),
            ],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();

        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.display_path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("wrapper.py")]
        );
        let mut skipped = result
            .skipped
            .iter()
            .map(|input| input.path.clone())
            .collect::<Vec<_>>();
        skipped.sort();
        assert_eq!(
            skipped,
            vec![
                PathBuf::from("task"),
                PathBuf::from("tool"),
                PathBuf::from("tool.py")
            ]
        );
    }

    #[test]
    fn classify_explicit_input_uses_path_and_contents() {
        let workflow = Path::new(".github/workflows/ci.yml");
        assert_eq!(
            classify_explicit_input(workflow, b"on: push\n"),
            ExplicitInputKind::Process(FileKind::Embedded)
        );
        assert!(matches!(
            classify_explicit_input(Path::new("config.yaml"), b"a: b\n"),
            ExplicitInputKind::Skip(_)
        ));
        assert!(matches!(
            classify_explicit_input(Path::new("script"), b"#!/usr/bin/env node\n"),
            ExplicitInputKind::Skip(_)
        ));
        assert_eq!(
            classify_explicit_input(Path::new("script"), b"echo hi\n"),
            ExplicitInputKind::Process(FileKind::Shell)
        );
        assert_eq!(
            classify_explicit_input(Path::new("run.bash"), b"#!/usr/bin/env python\n"),
            ExplicitInputKind::Process(FileKind::Shell)
        );
    }

    #[test]
    fn ignored_ancestor_directories_do_not_hide_inputs() {
        let tempdir = tempdir().unwrap();
        let project = tempdir.path().join(".cache/bazel/vendor/proj");
        fs::create_dir_all(project.join("vendor")).unwrap();
        fs::write(project.join("main.sh"), "echo ok\n").unwrap();
        fs::write(project.join("vendor/dep.sh"), "echo dep\n").unwrap();

        for parallel in [false, true] {
            let files = discover_files(
                std::slice::from_ref(&project),
                &project,
                &DiscoveryOptions {
                    parallel,
                    ..DiscoveryOptions::default()
                },
            )
            .unwrap();
            assert_eq!(
                files
                    .iter()
                    .map(|file| file.display_path.clone())
                    .collect::<Vec<_>>(),
                vec![PathBuf::from("main.sh")],
                "parallel={parallel}"
            );
        }

        let explicit = discover_files(
            &[project.join("vendor/dep.sh")],
            &project,
            &DiscoveryOptions::default(),
        )
        .unwrap();
        assert_eq!(explicit.len(), 1);

        let forced = discover_files(
            &[project.join("vendor/dep.sh")],
            &project,
            &DiscoveryOptions {
                force_exclude: true,
                ..DiscoveryOptions::default()
            },
        )
        .unwrap();
        assert!(forced.is_empty());
    }

    #[test]
    fn missing_inputs_are_reported_without_aborting() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "echo ok\n").unwrap();

        let result = discover_files_with_status(
            &[PathBuf::from("missing.sh"), PathBuf::from("ok.sh")],
            tempdir.path(),
            &DiscoveryOptions::default(),
        )
        .unwrap();

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].path, PathBuf::from("missing.sh"));
        assert!(result.errors[0].message.starts_with("cannot access path"));
    }
}
