use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::GitignoreBuilder;
use ignore::{DirEntry, WalkBuilder};

use crate::args::FormatCommand;
use crate::config::resolve_project_root_for_input;
use crate::discover::{
    DEFAULT_IGNORED_DIR_NAMES, DiscoveredFile, ProjectRoot, add_file, is_shell_script,
    normalize_path,
};

pub(crate) fn resolve_format_files(
    args: &FormatCommand,
    cwd: &Path,
) -> Result<Vec<DiscoveredFile>> {
    let exclude_matcher = ExcludeMatcher::new(&args.exclude)?;
    let respect_gitignore = args.respect_gitignore();
    let force_exclude = args.force_exclude();
    let inputs = args
        .files
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            }
        })
        .collect::<Vec<_>>();

    let mut files = BTreeMap::new();
    for input in inputs {
        let input = normalize_path(&input);
        let metadata = fs::metadata(&input).with_context(|| format!("stat {}", input.display()))?;

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

        if metadata.is_dir() {
            if force_exclude && exclude_matcher.matches(&input, cwd) {
                continue;
            }
            collect_directory(
                &input,
                cwd,
                &project_root,
                &exclude_matcher,
                respect_gitignore,
                &mut files,
            )?;
        } else if metadata.is_file() && is_shell_script(&input)? {
            if force_exclude {
                if exclude_matcher.matches(&input, cwd) {
                    continue;
                }
                if !is_allowed_by_gitignore(&input, cwd, respect_gitignore)? {
                    continue;
                }
            }

            let fallback_start = input
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            add_file(&input, cwd, &fallback_start, &project_root, &mut files)?;
        }
    }

    Ok(files.into_values().collect())
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

fn is_default_ignored_directory(entry: &DirEntry) -> bool {
    entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir())
        && DEFAULT_IGNORED_DIR_NAMES
            .iter()
            .any(|name| entry.file_name() == OsStr::new(name))
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
                .map_err(|err| anyhow!("invalid --exclude pattern `{pattern}`: {err}"))?;
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
