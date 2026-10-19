use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};
use shuck_cache::{legacy_cache_dir, read_project_root_from_cache_file};
use shuck_config::{ConfigArguments, resolve_project_root_for_input};

use crate::ExitStatus;
use crate::args::CleanCommand;
use crate::cache::{is_cache_file_name, is_cache_version_dir_name, resolve_cache_root};

pub(crate) fn clean(
    args: CleanCommand,
    config_arguments: &ConfigArguments,
    cache_dir: Option<&Path>,
) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let cache_root = resolve_cache_root(&cwd, cache_dir)?;
    let inputs = if args.paths.is_empty() {
        vec![cwd.clone()]
    } else {
        args.paths
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    cwd.join(path)
                }
            })
            .collect::<Vec<PathBuf>>()
    };

    let mut roots = BTreeSet::new();
    let mut canonical_roots = BTreeSet::new();
    for input in inputs {
        let root = resolve_project_root_for_input(&input, config_arguments.use_config_roots())?;
        let canonical_root =
            fs::canonicalize(&root).with_context(|| format!("canonicalize {}", root.display()))?;
        canonical_roots.insert(canonical_root);
        roots.insert(root);
    }

    remove_shared_cache_entries(&cache_root, &canonical_roots)?;

    for root in roots {
        match fs::remove_dir_all(legacy_cache_dir(&root)) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }

    let mut stdout = BufWriter::new(io::stdout().lock());
    writeln!(stdout, "cache cleared")?;

    Ok(ExitStatus::Success)
}

/// Removes cache files belonging to `canonical_roots` from the shared cache root.
///
/// The cache root may be user-provided (`--cache-dir`, `SHUCK_CACHE_DIR`) and
/// can contain unrelated data, so only `<version>/<sha256>.bin` entries are
/// considered. Version directories are removed once empty, and the root only
/// when this pass emptied it. Entries that cannot be read are skipped with a
/// warning rather than aborting the clean.
fn remove_shared_cache_entries(
    cache_root: &Path,
    canonical_roots: &BTreeSet<PathBuf>,
) -> Result<()> {
    let version_dirs = match fs::read_dir(cache_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };

    let mut removed_any = false;
    for version_dir in version_dirs {
        let Some(version_dir) = warn_on_error(cache_root, version_dir) else {
            continue;
        };
        let version_dir_path = version_dir.path();
        let is_version_dir = version_dir
            .file_name()
            .to_str()
            .is_some_and(is_cache_version_dir_name)
            && version_dir
                .file_type()
                .is_ok_and(|file_type| file_type.is_dir());
        if !is_version_dir {
            continue;
        }

        let Some(entries) = warn_on_error(&version_dir_path, fs::read_dir(&version_dir_path))
        else {
            continue;
        };
        for entry in entries {
            let Some(entry) = warn_on_error(&version_dir_path, entry) else {
                continue;
            };
            let path = entry.path();
            let is_cache_file = entry.file_name().to_str().is_some_and(is_cache_file_name)
                && entry.file_type().is_ok_and(|file_type| file_type.is_file());
            if !is_cache_file {
                continue;
            }

            let Some(project_root) =
                warn_on_error(&path, read_project_root_from_cache_file(&path)).flatten()
            else {
                continue;
            };
            if canonical_roots.contains(&project_root)
                && warn_on_error(&path, fs::remove_file(&path)).is_some()
            {
                removed_any = true;
            }
        }

        warn_on_error(&version_dir_path, remove_dir_if_empty(&version_dir_path));
    }

    if removed_any {
        warn_on_error(cache_root, remove_dir_if_empty(cache_root));
    }
    Ok(())
}

fn warn_on_error<T>(path: &Path, result: io::Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) => {
            eprintln!("warning: skipping {}: {err}", path.display());
            None
        }
    }
}

fn remove_dir_if_empty(path: &Path) -> io::Result<()> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    if entries.next().is_none() {
        fs::remove_dir(path)?;
    }

    Ok(())
}
