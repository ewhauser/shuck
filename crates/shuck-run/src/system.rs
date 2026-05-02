use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

use crate::{ResolutionSource, ResolvedInterpreter, Shell, Version, VersionConstraint};

pub(crate) fn resolve_system(
    shell: Shell,
    constraint: &VersionConstraint,
) -> Result<ResolvedInterpreter> {
    let path = find_on_path(shell.as_str())
        .ok_or_else(|| anyhow!("{shell} not found on $PATH. Install it or remove --system."))?;
    resolve_system_at_path(shell, &path, constraint)
}

pub(crate) fn resolve_system_at_path(
    shell: Shell,
    path: &Path,
    constraint: &VersionConstraint,
) -> Result<ResolvedInterpreter> {
    let version = detect_shell_version(shell, path)
        .with_context(|| format!("detect system {shell} version at {}", path.display()))?;
    if !constraint.matches(&version) {
        bail!(
            "System {shell} is {}, but {} is required. Run `shuck install {shell} {}` to get a managed version.",
            version,
            constraint.describe(),
            constraint.describe()
        );
    }

    Ok(ResolvedInterpreter {
        shell,
        version,
        path: path.to_path_buf(),
        source: ResolutionSource::System,
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    find_on_path_in(std::env::var_os("PATH").as_deref(), name)
}

pub(crate) fn find_on_path_in(path_var: Option<&OsStr>, name: &str) -> Option<PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(path_var).find_map(|directory| {
        let candidate = directory.join(name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

#[cfg(unix)]
fn is_executable_file(candidate: &Path) -> bool {
    fs::metadata(candidate)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(candidate: &Path) -> bool {
    candidate.is_file()
}

pub(crate) fn detect_shell_version(shell: Shell, path: &Path) -> Result<Version> {
    let candidates = version_probe_commands(shell);
    let mut last_error = None;
    for args in candidates {
        let output = Command::new(path).args(args).output();
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stdout.trim().is_empty() {
                    stderr.trim().to_owned()
                } else if stderr.trim().is_empty() {
                    stdout.trim().to_owned()
                } else {
                    format!("{}\n{}", stdout.trim(), stderr.trim())
                };
                if let Some(version) = extract_version(shell, &combined) {
                    return Ok(version);
                }
                last_error = Some(anyhow!(
                    "could not parse version from `{}`",
                    combined.trim()
                ));
            }
            Err(err) => last_error = Some(err.into()),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("unable to detect shell version")))
}

fn version_probe_commands(shell: Shell) -> Vec<Vec<OsString>> {
    match shell {
        Shell::Bash | Shell::Zsh => vec![vec![OsString::from("--version")]],
        Shell::Dash => vec![
            vec![OsString::from("-V")],
            vec![OsString::from("--version")],
        ],
        Shell::Mksh => vec![
            vec![
                OsString::from("-c"),
                OsString::from("printf '%s' \"$KSH_VERSION\""),
            ],
            vec![OsString::from("-V")],
        ],
    }
}

fn extract_version(shell: Shell, text: &str) -> Option<Version> {
    if shell == Shell::Mksh
        && let Some(version) = extract_mksh_version(text)
    {
        return Version::parse(&version).ok();
    }

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if !ch.is_ascii_digit() {
            chars.next();
            continue;
        }

        let mut candidate = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_digit() || next == '.' || next.is_ascii_alphabetic() {
                candidate.push(next);
                chars.next();
            } else {
                break;
            }
        }

        if candidate.chars().any(|ch| ch.is_ascii_digit())
            && let Ok(version) = Version::parse(&candidate)
        {
            return Some(version);
        }
    }

    None
}

fn extract_mksh_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len().saturating_sub(1) {
        if bytes[index] == b'R' && bytes[index + 1].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || bytes[end].is_ascii_alphabetic())
            {
                end += 1;
            }
            return Some(text[index + 1..end].to_owned());
        }
    }
    None
}
