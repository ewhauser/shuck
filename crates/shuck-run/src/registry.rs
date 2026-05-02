use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

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

pub(crate) fn select_version(
    registry: &RegistryIndex,
    shell: Shell,
    constraint: &VersionConstraint,
) -> Result<Version> {
    let shell_entry = registry.shells.get(shell.as_str()).ok_or_else(|| {
        anyhow!(
            "{} is not available. Run `shuck install --list` to see available shells.",
            shell
        )
    })?;

    let mut versions = shell_entry
        .versions
        .keys()
        .map(|version| Version::parse(version))
        .collect::<Result<Vec<_>>>()?;
    versions.sort();

    versions
        .into_iter()
        .rev()
        .find(|version| constraint.matches(version))
        .ok_or_else(|| {
            anyhow!(
                "{shell} {} is not available. Run `shuck install --list {shell}` to see available versions.",
                constraint.describe()
            )
        })
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
        match fetch_url_to_path(&environment.registry_url, &index_path, verbose) {
            Ok(()) => {}
            Err(_err) if index_path.exists() && !refresh => {}
            Err(err) => {
                return Err(err).with_context(|| format!("fetch {}", environment.registry_url));
            }
        }
    }

    let source = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("parse {}", index_path.display()))
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
