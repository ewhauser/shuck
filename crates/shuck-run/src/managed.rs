use std::fs::{self, File};
use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use crate::download::{FetchBudget, fetch_url_to_path};
use crate::environment::current_platform;
use crate::registry::{load_registry, select_release_for_platform};
use crate::system::detect_shell_version;
use crate::{
    Environment, ResolutionSource, ResolvedInterpreter, Shell, Version, VersionConstraint,
};

pub(crate) fn install_with_environment(
    environment: &Environment,
    shell: Shell,
    constraint: &VersionConstraint,
    verbose: bool,
    refresh_registry: bool,
) -> Result<ResolvedInterpreter> {
    shell.ensure_supported_on_current_platform()?;
    let platform = current_platform()?;

    // An exact pin that is already installed needs nothing from the registry, so it
    // keeps working offline and never pays for a registry refresh.
    if !refresh_registry
        && let VersionConstraint::Exact(version) = constraint
        && let Some(existing) = reusable_existing_install(
            shell,
            version,
            &install_binary_path(environment, shell, version, &platform),
        )
    {
        return Ok(existing);
    }

    let selection = load_registry(environment, refresh_registry, verbose).and_then(|registry| {
        select_release_for_platform(
            &registry,
            shell,
            constraint,
            &platform,
            refresh_registry,
            verbose,
        )
    });
    let (version, artifact) = match selection {
        Ok(selection) => selection,
        Err(err) => {
            // When the registry is unreachable, fall back to the newest installed
            // version that satisfies the constraint instead of failing outright.
            if !refresh_registry
                && let Some(existing) =
                    newest_installed_match(environment, shell, constraint, &platform)
            {
                return Ok(existing);
            }
            return Err(err);
        }
    };

    let install_dir = install_dir(environment, shell, &version, &platform);
    let binary_path = install_dir.join("bin").join(shell.as_str());
    if let Some(existing) = reusable_existing_install(shell, &version, &binary_path) {
        return Ok(existing);
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
    fetch_url_to_path(&artifact.url, archive.path(), FetchBudget::Archive, verbose)
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
        if let Some(existing) = reusable_existing_install(shell, &version, &binary_path) {
            return Ok(existing);
        }
        fs::remove_dir_all(&install_dir)
            .with_context(|| format!("remove partial install {}", install_dir.display()))?;
    }

    match fs::rename(&extracted_root, &install_dir) {
        Ok(()) => {
            if extracted_root == tempdir.path() {
                // The temp dir itself became the install; don't let its guard delete it.
                let _ = tempdir.keep();
            }
        }
        // Another process won the race; its install is validated below and our temp
        // dir is cleaned up when `tempdir` drops.
        Err(_) if install_dir.exists() => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "move {} to {}",
                    extracted_root.display(),
                    install_dir.display()
                )
            });
        }
    }

    let binary_path = install_dir.join("bin").join(shell.as_str());
    validate_existing_install(shell, &version, &binary_path)?
        .ok_or_else(|| anyhow!("installed {shell} is missing {}", binary_path.display()))
}

fn install_dir(
    environment: &Environment,
    shell: Shell,
    version: &Version,
    platform: &str,
) -> PathBuf {
    environment
        .shells_root
        .join(shell.as_str())
        .join(version.as_str())
        .join(platform)
}

fn install_binary_path(
    environment: &Environment,
    shell: Shell,
    version: &Version,
    platform: &str,
) -> PathBuf {
    install_dir(environment, shell, version, platform)
        .join("bin")
        .join(shell.as_str())
}

fn newest_installed_match(
    environment: &Environment,
    shell: Shell,
    constraint: &VersionConstraint,
    platform: &str,
) -> Option<ResolvedInterpreter> {
    let mut installed = fs::read_dir(environment.shells_root.join(shell.as_str()))
        .ok()?
        .flatten()
        .filter_map(|entry| Version::parse(&entry.file_name().to_string_lossy()).ok())
        .filter(|version| constraint.matches(version))
        .collect::<Vec<_>>();
    installed.sort();
    installed.into_iter().rev().find_map(|version| {
        let binary_path = install_binary_path(environment, shell, &version, platform);
        reusable_existing_install(shell, &version, &binary_path)
    })
}

fn validate_existing_install(
    shell: Shell,
    version: &Version,
    binary_path: &Path,
) -> Result<Option<ResolvedInterpreter>> {
    if !binary_path.exists() {
        return Ok(None);
    }

    let detected = detect_shell_version(shell, binary_path)
        .with_context(|| format!("verify {}", binary_path.display()))?;
    if &detected != version {
        bail!("installed {shell} reports version {detected}, expected {version}");
    }

    Ok(Some(ResolvedInterpreter {
        shell,
        version: version.clone(),
        path: binary_path.to_path_buf(),
        source: ResolutionSource::Managed,
    }))
}

fn reusable_existing_install(
    shell: Shell,
    version: &Version,
    binary_path: &Path,
) -> Option<ResolvedInterpreter> {
    validate_existing_install(shell, version, binary_path)
        .ok()
        .flatten()
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
    // Never restore the archive's owner or raw mode bits: when run as root that would
    // otherwise keep foreign uid/gid and setuid/setgid bits from the tarball.
    let output = Command::new("tar")
        .arg("--no-same-owner")
        .arg("--no-same-permissions")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("run tar")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "tar failed while extracting {}: {}",
            archive.display(),
            stderr.trim()
        );
    }
    clear_special_mode_bits(destination)
}

/// Strip setuid, setgid, and sticky bits from everything that was extracted.
#[cfg(unix)]
fn clear_special_mode_bits(root: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(root).with_context(|| format!("stat {}", root.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mode = metadata.permissions().mode();
    if mode & 0o7000 != 0 {
        fs::set_permissions(root, fs::Permissions::from_mode(mode & 0o777))
            .with_context(|| format!("reset permissions on {}", root.display()))?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            clear_special_mode_bits(&entry?.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn clear_special_mode_bits(_root: &Path) -> Result<()> {
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
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
        };
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
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

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use super::{extract_archive, verify_sha256};

    #[test]
    fn extraction_drops_setuid_and_setgid_bits() {
        let tempdir = tempfile::tempdir().unwrap();
        let source = tempdir.path().join("source");
        fs::create_dir_all(source.join("bin")).unwrap();
        let binary = source.join("bin/bash");
        fs::write(&binary, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o6755)).unwrap();
        assert_ne!(
            fs::metadata(&binary).unwrap().permissions().mode() & 0o4000,
            0
        );
        let archive = tempdir.path().join("shell.tar.gz");
        let status = Command::new("tar")
            .current_dir(&source)
            .arg("-czf")
            .arg(&archive)
            .arg("bin")
            .status()
            .unwrap();
        assert!(status.success());

        let destination = tempdir.path().join("out");
        fs::create_dir_all(&destination).unwrap();
        extract_archive(&archive, &destination).unwrap();

        let mode = fs::metadata(destination.join("bin/bash"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7000, 0, "mode {mode:o}");
        assert_ne!(
            mode & 0o100,
            0,
            "owner execute bit must survive, mode {mode:o}"
        );
    }

    #[test]
    fn sha256_is_computed_over_the_whole_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("data");
        // Larger than the read buffer so more than one chunk is hashed.
        fs::write(&path, vec![b'a'; 200 * 1024]).unwrap();
        verify_sha256(
            &path,
            "  F2D4C7A1E3C4AB8B0C26F1C6CD9D4B7A04C24C2D8A0E3D4D1A30C4A6C1E5F6A1 ",
        )
        .unwrap_err();
        let expected = {
            use sha2::{Digest, Sha256};
            Sha256::digest(vec![b'a'; 200 * 1024])
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        verify_sha256(&path, &expected.to_ascii_uppercase()).unwrap();
    }
}
