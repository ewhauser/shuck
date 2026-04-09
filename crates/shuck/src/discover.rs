use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{DirEntry, ParallelVisitor, WalkBuilder, WalkState};

use crate::config::{resolve_project_root_for_file, resolve_project_root_for_input};

pub(crate) const DEFAULT_IGNORED_DIR_NAMES: &[&str] = &[
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

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct ProjectRoot {
    pub storage_root: PathBuf,
    pub canonical_root: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DiscoveredFile {
    pub display_path: PathBuf,
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub project_root: ProjectRoot,
}

#[derive(Debug, Default)]
pub(crate) struct DiscoveryOptions {
    pub exclude_patterns: Vec<String>,
    pub respect_gitignore: bool,
    pub force_exclude: bool,
    pub parallel: bool,
    pub cache_root: Option<PathBuf>,
}

pub(crate) fn discover_files(
    inputs: &[PathBuf],
    cwd: &Path,
    options: &DiscoveryOptions,
) -> Result<Vec<DiscoveredFile>> {
    let exclude_matcher = ExcludeMatcher::new(&options.exclude_patterns)?;
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

    let mut files = BTreeMap::new();
    for input in resolved_inputs {
        let input = normalize_path(&input);

        if should_ignore_path(&input, options.cache_root.as_deref()) {
            continue;
        }

        let storage_root = resolve_project_root_for_input(&input)
            .with_context(|| format!("resolve project root for {}", input.display()))?;
        let canonical_root = fs::canonicalize(&storage_root)
            .with_context(|| format!("canonicalize {}", storage_root.display()))?;
        let project_root = ProjectRoot {
            storage_root,
            canonical_root,
        };

        collect_input(
            &input,
            cwd,
            &project_root,
            &exclude_matcher,
            &mut explicit_ignore_cache,
            options,
            &mut files,
        )?;
    }

    Ok(files.into_values().collect())
}

fn collect_input(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    explicit_ignore_cache: &mut ExplicitIgnoreCache,
    options: &DiscoveryOptions,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let metadata = fs::metadata(input).with_context(|| format!("stat {}", input.display()))?;
    if metadata.is_dir() {
        if options.force_exclude && exclude_matcher.matches(input, cwd) {
            return Ok(());
        }
        collect_directory(input, cwd, project_root, exclude_matcher, options, files)?;
    } else if metadata.is_file() && is_shell_script(input)? {
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
        add_file(input, cwd, &fallback_start, project_root, files)?;
    }

    Ok(())
}

fn collect_directory(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    options: &DiscoveryOptions,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let mut builder = WalkBuilder::new(input);
    configure_walk_builder(
        &mut builder,
        options.respect_gitignore,
        options.cache_root.clone(),
    );

    if options.parallel {
        return collect_directory_parallel(
            input,
            cwd,
            project_root,
            exclude_matcher,
            &mut builder,
            files,
        );
    }

    for entry in builder.build() {
        let entry = entry.with_context(|| format!("walk {}", input.display()))?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if should_ignore_path(path, options.cache_root.as_deref())
            || exclude_matcher.matches(path, cwd)
            || !is_shell_script(path)?
        {
            continue;
        }

        add_file(path, cwd, input, project_root, files)?;
    }

    Ok(())
}

fn collect_directory_parallel(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    exclude_matcher: &ExcludeMatcher,
    builder: &mut WalkBuilder,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
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

    if let Some(error) = state.error.into_inner().unwrap() {
        return Err(error);
    }

    let mut matched_paths = state.matched_paths.into_inner().unwrap();
    matched_paths.sort();
    for matched_path in matched_paths {
        add_file(&matched_path, cwd, input, project_root, files)?;
    }

    Ok(())
}

fn configure_walk_builder(
    builder: &mut WalkBuilder,
    respect_gitignore: bool,
    cache_root: Option<PathBuf>,
) {
    builder.hidden(false);
    builder.parents(respect_gitignore);
    builder.ignore(respect_gitignore);
    builder.git_ignore(respect_gitignore);
    builder.git_global(respect_gitignore);
    builder.git_exclude(respect_gitignore);
    builder.require_git(false);
    builder.filter_entry(move |entry| !is_ignored_directory(entry, cache_root.as_deref()));
}

fn is_ignored_directory(entry: &DirEntry, cache_root: Option<&Path>) -> bool {
    entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
        && (DEFAULT_IGNORED_DIR_NAMES
            .iter()
            .any(|name| entry.file_name() == OsStr::new(name))
            || path_matches_cache_root(entry.path(), cache_root))
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

        Ok(!self
            .matchers
            .get(&key)
            .unwrap()
            .matched_path_or_any_parents(path, false)
            .is_ignore())
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

fn should_ignore_path(path: &Path, cache_root: Option<&Path>) -> bool {
    path_has_default_ignored_component(path) || path_matches_cache_root(path, cache_root)
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
    matched_paths: Mutex<Vec<PathBuf>>,
    error: Mutex<Option<anyhow::Error>>,
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
    local_paths: Vec<PathBuf>,
    local_error: Option<anyhow::Error>,
}

impl<'a> ignore::ParallelVisitorBuilder<'a> for ShellFilesVisitorBuilder<'a> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 'a> {
        Box::new(ShellFilesVisitor {
            input: self.input,
            cwd: self.cwd,
            exclude_matcher: self.exclude_matcher,
            state: self.state,
            local_paths: Vec::new(),
            local_error: None,
        })
    }
}

impl ParallelVisitor for ShellFilesVisitor<'_> {
    fn visit(&mut self, result: std::result::Result<DirEntry, ignore::Error>) -> WalkState {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                self.local_error =
                    Some(anyhow!(err).context(format!("walk {}", self.input.display())));
                return WalkState::Quit;
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

        match is_shell_script(path) {
            Ok(true) => self.local_paths.push(entry.into_path()),
            Ok(false) => {}
            Err(err) => {
                self.local_error = Some(err);
                return WalkState::Quit;
            }
        }

        WalkState::Continue
    }
}

impl Drop for ShellFilesVisitor<'_> {
    fn drop(&mut self) {
        if !self.local_paths.is_empty() {
            self.state
                .matched_paths
                .lock()
                .unwrap()
                .append(&mut self.local_paths);
        }

        if let Some(error) = self.local_error.take() {
            let mut state_error = self.state.error.lock().unwrap();
            if state_error.is_none() {
                *state_error = Some(error);
            }
        }
    }
}

pub(crate) fn add_file(
    path: &Path,
    cwd: &Path,
    fallback_start: &Path,
    default_project_root: &ProjectRoot,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let absolute_path =
        fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    let storage_root = resolve_project_root_for_file(path, fallback_start)
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
        });

    Ok(())
}

pub(crate) fn display_path(path: &Path, cwd: &Path) -> PathBuf {
    path.strip_prefix(cwd)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| normalize_path(path))
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

pub(crate) fn is_shell_script(path: &Path) -> Result<bool> {
    if let Some("sh" | "bash" | "zsh" | "ksh" | "dash" | "mksh") = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        return Ok(true);
    }

    let bytes = read_shebang_prefix(path)?;
    Ok(infer_shebang_dialect(&bytes).is_some())
}

fn read_shebang_prefix(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = BufReader::new(file).take(SHEBANG_SNIFF_LIMIT_BYTES);
    let mut bytes = Vec::new();
    reader
        .read_until(b'\n', &mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(bytes)
}

fn infer_shebang_dialect(src: &[u8]) -> Option<&'static str> {
    let first_line = src.split(|byte| *byte == b'\n').next()?;
    let line = std::str::from_utf8(first_line).ok()?.trim();
    let line = line.strip_prefix("#!")?.trim();

    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let interpreter = if Path::new(first).file_name()?.to_str()? == "env" {
        parts.next()?
    } else {
        Path::new(first).file_name()?.to_str()?
    };

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
    fn new(patterns: &[String]) -> Result<Self> {
        if patterns.is_empty() {
            return Ok(Self { set: None });
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
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
    fn ignores_large_extensionless_non_shell_file() {
        let tempdir = tempdir().unwrap();
        let file = tempdir.path().join("blob");
        fs::write(&file, vec![b'x'; (SHEBANG_SNIFF_LIMIT_BYTES as usize) * 2]).unwrap();

        assert!(!is_shell_script(&file).unwrap());
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
}
