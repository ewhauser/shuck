use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::GitignoreBuilder;
use ignore::{DirEntry, WalkBuilder};

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
}

pub(crate) fn discover_files(
    inputs: &[PathBuf],
    cwd: &Path,
    options: &DiscoveryOptions,
) -> Result<Vec<DiscoveredFile>> {
    let exclude_matcher = ExcludeMatcher::new(&options.exclude_patterns)?;

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

        if path_has_default_ignored_component(&input) {
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
    options: &DiscoveryOptions,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let metadata = fs::metadata(input).with_context(|| format!("stat {}", input.display()))?;
    if metadata.is_dir() {
        if options.force_exclude && exclude_matcher.matches(input, cwd) {
            return Ok(());
        }
        collect_directory(
            input,
            cwd,
            project_root,
            exclude_matcher,
            options.respect_gitignore,
            files,
        )?;
    } else if metadata.is_file() && is_shell_script(input)? {
        if options.force_exclude {
            if exclude_matcher.matches(input, cwd) {
                return Ok(());
            }
            if !is_allowed_by_gitignore(input, cwd, options.respect_gitignore)? {
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
    respect_gitignore: bool,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let mut builder = WalkBuilder::new(input);
    configure_walk_builder(&mut builder, respect_gitignore);

    for entry in builder.build() {
        let entry = entry.with_context(|| format!("walk {}", input.display()))?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if exclude_matcher.matches(path, cwd) || !is_shell_script(path)? {
            continue;
        }

        add_file(path, cwd, input, project_root, files)?;
    }

    Ok(())
}

fn configure_walk_builder(builder: &mut WalkBuilder, respect_gitignore: bool) {
    builder.hidden(false);
    builder.parents(respect_gitignore);
    builder.ignore(respect_gitignore);
    builder.git_ignore(respect_gitignore);
    builder.git_global(respect_gitignore);
    builder.git_exclude(respect_gitignore);
    builder.require_git(false);
    builder.filter_entry(|entry| !is_default_ignored_directory(entry));
}

fn is_default_ignored_directory(entry: &DirEntry) -> bool {
    entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
        && DEFAULT_IGNORED_DIR_NAMES
            .iter()
            .any(|name| entry.file_name() == OsStr::new(name))
}

fn is_allowed_by_gitignore(path: &Path, cwd: &Path, respect_gitignore: bool) -> Result<bool> {
    if !respect_gitignore || !path.starts_with(cwd) {
        return Ok(true);
    }

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

    let gitignore = builder.build()?;
    Ok(!gitignore
        .matched_path_or_any_parents(path, path.is_dir())
        .is_ignore())
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
        .map_err(|_| {
            anyhow!(
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

    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(infer_shebang_dialect(&bytes).is_some())
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
