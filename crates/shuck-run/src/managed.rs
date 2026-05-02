use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use crate::download::fetch_url_to_path;
use crate::environment::current_platform;
use crate::registry::{load_registry, select_version};
use crate::system::detect_shell_version;
use crate::{Environment, ResolutionSource, ResolvedInterpreter, Shell, VersionConstraint};

pub(crate) fn install_with_environment(
    environment: &Environment,
    shell: Shell,
    constraint: &VersionConstraint,
    verbose: bool,
    refresh_registry: bool,
) -> Result<ResolvedInterpreter> {
    let registry = load_registry(environment, refresh_registry, verbose)?;
    let platform = current_platform()?;
    let version = select_version(&registry, shell, constraint)?;
    let artifact = registry
        .shells
        .get(shell.as_str())
        .and_then(|entry| entry.versions.get(version.as_str()))
        .and_then(|entry| entry.platforms.get(&platform))
        .ok_or_else(|| {
            anyhow!("{shell} {version} does not have a prebuilt binary for {platform}.")
        })?;

    let install_dir = environment
        .shells_root
        .join(shell.as_str())
        .join(version.as_str())
        .join(&platform);
    let binary_path = install_dir.join("bin").join(shell.as_str());
    if binary_path.exists() {
        let detected = detect_shell_version(shell, &binary_path)
            .with_context(|| format!("verify {}", binary_path.display()))?;
        if detected != version {
            bail!("installed {shell} reports version {detected}, expected {version}");
        }
        return Ok(ResolvedInterpreter {
            shell,
            version,
            path: binary_path,
            source: ResolutionSource::Managed,
        });
    }

    fs::create_dir_all(&environment.shells_root)
        .with_context(|| format!("create {}", environment.shells_root.display()))?;
    let archive = TempFileBuilder::new()
        .prefix("shuck-shell-")
        .suffix(".tar.gz")
        .tempfile_in(&environment.shells_root)
        .with_context(|| {
            format!(
                "create temp archive in {}",
                environment.shells_root.display()
            )
        })?;
    fetch_url_to_path(&artifact.url, archive.path(), verbose)
        .with_context(|| format!("download {shell} {version}"))?;
    verify_sha256(archive.path(), &artifact.sha256)
        .with_context(|| format!("verify checksum for {shell} {version}"))?;

    let tempdir = TempFileBuilder::new()
        .prefix("shuck-install-")
        .tempdir_in(&environment.shells_root)
        .with_context(|| {
            format!(
                "create temp install dir in {}",
                environment.shells_root.display()
            )
        })?;
    extract_archive(archive.path(), tempdir.path())
        .with_context(|| format!("extract {}", archive.path().display()))?;

    let extracted_root = locate_extracted_root(tempdir.path(), shell)
        .ok_or_else(|| anyhow!("archive did not contain bin/{}", shell.as_str()))?;
    fs::create_dir_all(
        install_dir
            .parent()
            .ok_or_else(|| anyhow!("invalid install directory {}", install_dir.display()))?,
    )?;

    if install_dir.exists() {
        return Ok(ResolvedInterpreter {
            shell,
            version,
            path: binary_path,
            source: ResolutionSource::Managed,
        });
    }

    if extracted_root == tempdir.path() {
        let temp_path = tempdir.keep();
        fs::rename(&temp_path, &install_dir).or_else(|err| {
            if install_dir.exists() {
                Ok(())
            } else {
                Err(err)
            }
        })?;
    } else {
        fs::rename(&extracted_root, &install_dir).or_else(|err| {
            if install_dir.exists() {
                Ok(())
            } else {
                Err(err)
            }
        })?;
    }

    let binary_path = install_dir.join("bin").join(shell.as_str());
    let detected = detect_shell_version(shell, &binary_path)
        .with_context(|| format!("verify {}", binary_path.display()))?;
    if detected != version {
        bail!("installed {shell} reports version {detected}, expected {version}");
    }

    Ok(ResolvedInterpreter {
        shell,
        version,
        path: binary_path,
        source: ResolutionSource::Managed,
    })
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("tar failed while extracting {}", archive.display());
    }
    Ok(())
}

fn locate_extracted_root(root: &Path, shell: Shell) -> Option<PathBuf> {
    let direct = root.join("bin").join(shell.as_str());
    if direct.exists() {
        return Some(root.to_path_buf());
    }

    fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("bin").join(shell.as_str()).exists())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let digest = Sha256::digest(bytes);
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected.trim().to_ascii_lowercase() {
        bail!(
            "Checksum mismatch. Expected {}, got {}. The archive may be corrupted.",
            expected,
            actual
        );
    }
    Ok(())
}
