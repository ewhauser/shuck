use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

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
        executable_names(name)
            .into_iter()
            .find_map(|candidate_name| {
                let candidate = directory.join(candidate_name);
                is_executable_file(&candidate).then_some(candidate)
            })
    })
}

#[cfg(unix)]
fn executable_names(name: &str) -> Vec<OsString> {
    vec![OsString::from(name)]
}

#[cfg(not(unix))]
fn executable_names(name: &str) -> Vec<OsString> {
    let mut names = vec![OsString::from(name)];
    if Path::new(name).extension().is_some() {
        return names;
    }

    let pathext =
        std::env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    for extension in pathext.to_string_lossy().split(';') {
        let extension = extension.trim();
        if extension.is_empty() {
            continue;
        }
        let mut candidate = OsString::from(name);
        candidate.push(extension);
        if !names.contains(&candidate) {
            names.push(candidate);
        }
    }

    names
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
        let output = retry_executable_file_busy(|| Command::new(path).args(&args).output());
        match output {
            Ok(output) => {
                if !output.status.success() {
                    last_error = Some(anyhow!(
                        "version probe failed with status {}",
                        output.status
                    ));
                    continue;
                }
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

fn retry_executable_file_busy<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    // Freshly written or replaced shell fixtures can briefly report ETXTBSY on Linux.
    // Retrying only that errno keeps real probe failures intact while smoothing over the
    // transient launch race in tests and during binary replacement.
    const RETRY_DELAYS: &[Duration] = &[
        Duration::from_millis(5),
        Duration::from_millis(10),
        Duration::from_millis(25),
        Duration::from_millis(50),
    ];

    let mut attempts = 0;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(err) if is_executable_file_busy(&err) && attempts < RETRY_DELAYS.len() => {
                thread::sleep(RETRY_DELAYS[attempts]);
                attempts += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(unix)]
fn is_executable_file_busy(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(26)
}

#[cfg(not(unix))]
fn is_executable_file_busy(_err: &std::io::Error) -> bool {
    false
}

fn version_probe_commands(shell: Shell) -> Vec<Vec<OsString>> {
    match shell {
        Shell::Bash | Shell::Bashkit | Shell::Zsh => vec![vec![OsString::from("--version")]],
        Shell::Gbash => vec![
            vec![OsString::from("--version")],
            vec![OsString::from("version")],
        ],
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
        Shell::Busybox => vec![vec![OsString::from("--help")]],
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;

    use super::retry_executable_file_busy;

    #[cfg(unix)]
    #[test]
    fn retries_executable_file_busy_errors() {
        let attempts = Cell::new(0);
        let result = retry_executable_file_busy(|| {
            let next = attempts.get();
            attempts.set(next + 1);
            if next < 2 {
                Err(io::Error::from_raw_os_error(26))
            } else {
                Ok("ok")
            }
        })
        .unwrap();

        assert_eq!(result, "ok");
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn does_not_retry_other_io_errors() {
        let attempts = Cell::new(0);
        let err = retry_executable_file_busy(|| -> io::Result<()> {
            attempts.set(attempts.get() + 1);
            Err(io::Error::from_raw_os_error(2))
        })
        .unwrap_err();

        assert_eq!(attempts.get(), 1);
        assert_eq!(err.raw_os_error(), Some(2));
    }
}
