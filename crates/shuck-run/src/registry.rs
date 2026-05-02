use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use tempfile::NamedTempFile;
use url::Url;

use crate::download::fetch_url_to_path;
use crate::{AvailableShell, Environment, Shell, Version, VersionConstraint};

const REGISTRY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const REGISTRY_SCHEMA_VERSION: u64 = 2;
const ROOT_KIND: &str = "shuck.shells.index";
const SHELL_KIND: &str = "shuck.shells.versions";
const RELEASE_KIND: &str = "shuck.shells.release";

#[derive(Debug)]
pub(crate) struct RegistryIndex {
    pub(crate) shells: BTreeMap<String, RegistryShell>,
}

#[derive(Debug)]
pub(crate) struct RegistryShell {
    versions: BTreeMap<String, CachedDocumentRef>,
}

#[derive(Debug, Clone)]
struct CachedDocumentRef {
    remote_url: Url,
    cache_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RegistryArtifact {
    pub(crate) url: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Deserialize)]
struct RootDocument {
    version: u64,
    kind: String,
    shells: BTreeMap<String, RootShellDocument>,
}

#[derive(Debug, Deserialize)]
struct RootShellDocument {
    versions_url: String,
}

#[derive(Debug, Deserialize)]
struct ShellDocument {
    version: u64,
    kind: String,
    shell: String,
    versions: BTreeMap<String, ShellVersionDocument>,
}

#[derive(Debug, Deserialize)]
struct ShellVersionDocument {
    manifest_url: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseDocument {
    version: u64,
    kind: String,
    shell: String,
    release: String,
    platforms: BTreeMap<String, RegistryArtifact>,
}

impl CachedDocumentRef {
    fn root(environment: &Environment) -> Result<Self> {
        let mut remote_url = Url::parse(&environment.registry_url)
            .with_context(|| format!("parse registry URL `{}`", environment.registry_url))?;
        if !remote_url.path().ends_with(".json") {
            let mut path = remote_url.path().to_owned();
            if !path.ends_with('/') {
                path.push('/');
            }
            remote_url.set_path(&path);
            remote_url = remote_url
                .join("index.json")
                .context("resolve registry root index URL")?;
        }

        Ok(Self {
            remote_url,
            cache_path: environment.shells_root.join("index.json"),
        })
    }

    fn resolve_relative(&self, reference: &str) -> Result<Self> {
        let remote_url = self
            .remote_url
            .join(reference)
            .with_context(|| format!("resolve registry reference `{reference}`"))?;
        let cache_parent = self
            .cache_path
            .parent()
            .ok_or_else(|| anyhow!("invalid cache path {}", self.cache_path.display()))?;
        let cache_path = resolve_cache_path(cache_parent, reference)?;
        Ok(Self {
            remote_url,
            cache_path,
        })
    }
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

pub(crate) fn select_release_for_platform(
    registry: &RegistryIndex,
    shell: Shell,
    constraint: &VersionConstraint,
    platform: &str,
    refresh: bool,
    verbose: bool,
) -> Result<(Version, RegistryArtifact)> {
    let shell_entry = shell_entry(registry, shell)?;
    let mut versions = parsed_versions(shell_entry)?;
    versions.sort();

    let mut matched_constraint = false;
    for version in versions.into_iter().rev() {
        if !constraint.matches(&version) {
            continue;
        }
        matched_constraint = true;
        let manifest_ref = shell_entry.versions.get(version.as_str()).ok_or_else(|| {
            anyhow!(
                "missing release manifest reference for {shell} {}",
                version.as_str()
            )
        })?;
        let release = load_release_document(
            manifest_ref,
            shell.as_str(),
            version.as_str(),
            refresh,
            verbose,
        )?;
        if let Some(artifact) = release.platforms.get(platform) {
            return Ok((version, artifact.clone()));
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

    let root_ref = CachedDocumentRef::root(environment)?;
    let root = load_document(&root_ref, refresh, verbose, parse_root_document)?;

    let mut shells = BTreeMap::new();
    for (shell, entry) in root.shells {
        let shell_ref = root_ref.resolve_relative(&entry.versions_url)?;
        let shell_document = load_document(&shell_ref, refresh, verbose, |source| {
            parse_shell_document(source, &shell)
        })?;
        let versions = shell_document
            .versions
            .into_iter()
            .map(|(version, version_entry)| {
                let manifest_ref = shell_ref.resolve_relative(&version_entry.manifest_url)?;
                Ok((version, manifest_ref))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        shells.insert(shell, RegistryShell { versions });
    }

    Ok(RegistryIndex { shells })
}

fn resolve_cache_path(base: &Path, reference: &str) -> Result<PathBuf> {
    let mut resolved = base.to_path_buf();
    for component in Path::new(reference).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => resolved.push(segment),
            Component::ParentDir => {
                bail!("registry reference `{reference}` escapes the local cache root")
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("registry reference `{reference}` must be relative")
            }
        }
    }
    Ok(resolved)
}

fn load_release_document(
    manifest_ref: &CachedDocumentRef,
    expected_shell: &str,
    expected_version: &str,
    refresh: bool,
    verbose: bool,
) -> Result<ReleaseDocument> {
    load_document(manifest_ref, refresh, verbose, |source| {
        parse_release_document(source, expected_shell, expected_version)
    })
}

fn load_document<T, F>(
    document_ref: &CachedDocumentRef,
    refresh: bool,
    verbose: bool,
    parse: F,
) -> Result<T>
where
    F: Fn(&str) -> Result<T>,
{
    let should_refresh = refresh
        || !document_ref.cache_path.exists()
        || index_is_stale(&document_ref.cache_path).unwrap_or(true);
    if should_refresh {
        match refresh_document(document_ref, verbose, &parse) {
            Ok(document) => return Ok(document),
            Err(_err) if document_ref.cache_path.exists() && !refresh => {}
            Err(err) => return Err(err),
        }
    }

    match read_document(&document_ref.cache_path, &parse) {
        Ok(document) => Ok(document),
        Err(_err) if !refresh => refresh_document(document_ref, verbose, &parse),
        Err(err) => Err(err),
    }
}

fn refresh_document<T, F>(document_ref: &CachedDocumentRef, verbose: bool, parse: &F) -> Result<T>
where
    F: Fn(&str) -> Result<T>,
{
    let parent = document_ref
        .cache_path
        .parent()
        .ok_or_else(|| anyhow!("invalid cache path {}", document_ref.cache_path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let temp_file = NamedTempFile::new_in(parent)
        .with_context(|| format!("create temp registry document in {}", parent.display()))?;
    fetch_url_to_path(document_ref.remote_url.as_str(), temp_file.path(), verbose)
        .with_context(|| format!("fetch {}", document_ref.remote_url))?;
    let document = read_document(temp_file.path(), parse)?;
    match temp_file.persist(&document_ref.cache_path) {
        Ok(_) => Ok(document),
        Err(err) => {
            Err(err.error).with_context(|| format!("replace {}", document_ref.cache_path.display()))
        }
    }
}

fn read_document<T, F>(path: &Path, parse: &F) -> Result<T>
where
    F: Fn(&str) -> Result<T>,
{
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse(&source).with_context(|| format!("parse {}", path.display()))
}

fn parse_root_document(source: &str) -> Result<RootDocument> {
    let document: RootDocument =
        serde_json::from_str(source).context("decode root registry document")?;
    if document.version != REGISTRY_SCHEMA_VERSION {
        bail!(
            "root registry document version {} is unsupported",
            document.version
        );
    }
    if document.kind != ROOT_KIND {
        bail!(
            "root registry document kind `{}` is unsupported",
            document.kind
        );
    }
    Ok(document)
}

fn parse_shell_document(source: &str, expected_shell: &str) -> Result<ShellDocument> {
    let document: ShellDocument =
        serde_json::from_str(source).context("decode shell registry document")?;
    if document.version != REGISTRY_SCHEMA_VERSION {
        bail!(
            "shell registry document for `{expected_shell}` has unsupported version {}",
            document.version
        );
    }
    if document.kind != SHELL_KIND {
        bail!(
            "shell registry document for `{expected_shell}` has unsupported kind `{}`",
            document.kind
        );
    }
    if document.shell != expected_shell {
        bail!(
            "shell registry document expected `{expected_shell}`, found `{}`",
            document.shell
        );
    }
    Ok(document)
}

fn parse_release_document(
    source: &str,
    expected_shell: &str,
    expected_version: &str,
) -> Result<ReleaseDocument> {
    let document: ReleaseDocument =
        serde_json::from_str(source).context("decode release manifest")?;
    if document.version != REGISTRY_SCHEMA_VERSION {
        bail!(
            "release manifest for `{expected_shell}` `{expected_version}` has unsupported version {}",
            document.version
        );
    }
    if document.kind != RELEASE_KIND {
        bail!(
            "release manifest for `{expected_shell}` `{expected_version}` has unsupported kind `{}`",
            document.kind
        );
    }
    if document.shell != expected_shell {
        bail!(
            "release manifest expected shell `{expected_shell}`, found `{}`",
            document.shell
        );
    }
    if document.release != expected_version {
        bail!(
            "release manifest expected version `{expected_version}`, found `{}`",
            document.release
        );
    }
    Ok(document)
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
