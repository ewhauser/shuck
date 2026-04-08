use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};
use shuck_cache::{legacy_cache_dir, read_project_root_from_cache_file};

use crate::ExitStatus;
use crate::args::CleanCommand;
use crate::cache::resolve_cache_root;
use crate::config::resolve_project_root_for_input;

pub(crate) fn clean(args: CleanCommand, cache_dir: Option<&Path>) -> Result<ExitStatus> {
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
        let root = resolve_project_root_for_input(&input)?;
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

fn remove_shared_cache_entries(
    cache_root: &Path,
    canonical_roots: &BTreeSet<PathBuf>,
) -> Result<()> {
    let version_dirs = match fs::read_dir(cache_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };

    for version_dir in version_dirs {
        let version_dir = version_dir?;
        let version_dir_path = version_dir.path();
        if !version_dir.file_type()?.is_dir() {
            continue;
        }

        for entry in fs::read_dir(&version_dir_path)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }

            let Some(project_root) = read_project_root_from_cache_file(&path)? else {
                continue;
            };
            if canonical_roots.contains(&project_root) {
                fs::remove_file(&path)?;
            }
        }

        remove_dir_if_empty(&version_dir_path)?;
    }

    remove_dir_if_empty(cache_root)?;
    Ok(())
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
