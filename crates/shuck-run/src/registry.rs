use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::download::fetch_url_to_path;
use crate::{AvailableShell, Environment, Shell, Version, VersionConstraint};

const REGISTRY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryIndex {
    #[serde(rename = "version")]
    _version: u64,
    pub(crate) shells: BTreeMap<String, RegistryShell>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryShell {
    pub(crate) versions: BTreeMap<String, RegistryVersion>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryVersion {
    pub(crate) platforms: BTreeMap<String, RegistryArtifact>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryArtifact {
    pub(crate) url: String,
    pub(crate) sha256: String,
}

pub(crate) fn available_shells(
    registry: &RegistryIndex,
    shell: Option<Shell>,
) -> Vec<AvailableShell> {
    let shells = shell.into_iter().collect::<Vec<_>>();
    let names = if shells.is_empty() {
        registry
            .shells
            .keys()
            .filter_map(|name| Shell::from_name(name))
            .collect::<Vec<_>>()
    } else {
        shells
    };

    let mut available = names
        .into_iter()
        .filter_map(|shell| {
            registry.shells.get(shell.as_str()).map(|entry| {
                let mut versions = entry
                    .versions
                    .keys()
                    .filter_map(|version| Version::parse(version).ok())
                    .collect::<Vec<_>>();
                versions.sort();
                versions.reverse();
                AvailableShell { shell, versions }
            })
        })
        .collect::<Vec<_>>();
    available.sort_by_key(|entry| entry.shell);
    available
}

pub(crate) fn select_version_for_platform(
    registry: &RegistryIndex,
    shell: Shell,
    constraint: &VersionConstraint,
    platform: &str,
) -> Result<Version> {
    let shell_entry = shell_entry(registry, shell)?;
    let mut versions = parsed_versions(shell_entry)?;
    versions.sort();

    let mut matched_constraint = false;
    for version in versions.into_iter().rev() {
        if !constraint.matches(&version) {
            continue;
        }
        matched_constraint = true;
        if shell_entry
            .versions
            .get(version.as_str())
            .is_some_and(|entry| entry.platforms.contains_key(platform))
        {
            return Ok(version);
        }
    }

    if matched_constraint {
        Err(anyhow!(
            "{shell} {} does not have a prebuilt binary for {platform}.",
            constraint.describe()
        ))
    } else {
        Err(version_unavailable_error(shell, constraint))
    }
}

pub(crate) fn load_registry(
    environment: &Environment,
    refresh: bool,
    verbose: bool,
) -> Result<RegistryIndex> {
    fs::create_dir_all(&environment.shells_root)
        .with_context(|| format!("create {}", environment.shells_root.display()))?;
    let index_path = environment.shells_root.join("index.json");

    let should_refresh =
        refresh || !index_path.exists() || index_is_stale(&index_path).unwrap_or(true);
    if should_refresh {
        match refresh_registry_index(environment, &index_path, verbose) {
            Ok(registry) => return Ok(registry),
            Err(_err) if index_path.exists() && !refresh => {}
            Err(err) => return Err(err),
        }
    }

    read_registry_index(&index_path)
}

fn index_is_stale(path: &Path) -> Result<bool> {
    let modified = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .modified()
        .with_context(|| format!("read mtime for {}", path.display()))?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Ok(age > REGISTRY_MAX_AGE)
}

fn refresh_registry_index(
    environment: &Environment,
    index_path: &Path,
    verbose: bool,
) -> Result<RegistryIndex> {
    let temp_file = NamedTempFile::new_in(&environment.shells_root)
        .with_context(|| format!("create temp registry in {}", environment.shells_root.display()))?;
    fetch_url_to_path(&environment.registry_url, temp_file.path(), verbose)
        .with_context(|| format!("fetch {}", environment.registry_url))?;
    let registry = read_registry_index(temp_file.path())?;
    match temp_file.persist(index_path) {
        Ok(_) => Ok(registry),
        Err(err) => Err(err.error).with_context(|| format!("replace {}", index_path.display())),
    }
}

fn read_registry_index(path: &Path) -> Result<RegistryIndex> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("parse {}", path.display()))
}

fn shell_entry(registry: &RegistryIndex, shell: Shell) -> Result<&RegistryShell> {
    registry.shells.get(shell.as_str()).ok_or_else(|| {
        anyhow!(
            "{} is not available. Run `shuck install --list` to see available shells.",
            shell
        )
    })
}

fn parsed_versions(shell_entry: &RegistryShell) -> Result<Vec<Version>> {
    shell_entry
        .versions
        .keys()
        .map(|version| Version::parse(version))
        .collect()
}

fn version_unavailable_error(shell: Shell, constraint: &VersionConstraint) -> anyhow::Error {
    anyhow!(
        "{shell} {} is not available. Run `shuck install --list {shell}` to see available versions.",
        constraint.describe()
    )
}
