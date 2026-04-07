use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use walkdir::WalkDir;

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

pub(crate) fn discover_files(inputs: &[PathBuf], cwd: &Path) -> Result<Vec<DiscoveredFile>> {
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
        let storage_root = resolve_project_root_for_input(&input)
            .with_context(|| format!("resolve project root for {}", input.display()))?;
        let canonical_root = fs::canonicalize(&storage_root)
            .with_context(|| format!("canonicalize {}", storage_root.display()))?;
        let project_root = ProjectRoot {
            storage_root,
            canonical_root,
        };

        collect_input(&input, cwd, &project_root, &mut files)?;
    }

    Ok(files.into_values().collect())
}

fn collect_input(
    input: &Path,
    cwd: &Path,
    project_root: &ProjectRoot,
    files: &mut BTreeMap<PathBuf, DiscoveredFile>,
) -> Result<()> {
    let metadata = fs::metadata(input).with_context(|| format!("stat {}", input.display()))?;
    if metadata.is_dir() {
        let fallback_start = input.to_path_buf();
        let mut entries = WalkDir::new(input).into_iter();
        while let Some(entry) = entries.next() {
            let entry = entry.with_context(|| format!("walk {}", input.display()))?;
            let path = entry.path();
            if entry.file_type().is_dir()
                && path != input
                && DEFAULT_IGNORED_DIR_NAMES
                    .iter()
                    .any(|name| entry.file_name() == std::ffi::OsStr::new(name))
            {
                entries.skip_current_dir();
                continue;
            }

            if !entry.file_type().is_file() || !is_shell_script(path)? {
                continue;
            }

            add_file(path, cwd, &fallback_start, project_root, files)?;
        }
    } else if metadata.is_file() && is_shell_script(input)? {
        let fallback_start = input
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        add_file(input, cwd, &fallback_start, project_root, files)?;
    }

    Ok(())
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
