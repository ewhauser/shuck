#![warn(missing_docs)]

//! Library entrypoints for the `shuck` CLI.
//!
//! This crate primarily exists so the command-line binary, tests, and benchmarks can share the
//! same argument parsing and command execution code. Most users should invoke the `shuck` binary
//! directly rather than depend on this library API.

/// Command-line argument types and parsing helpers for the `shuck` executable.
pub mod args;

mod cache;
mod commands;
mod config;
mod discover;
mod format_settings;
#[doc(hidden)]
pub mod shellcheck_compat;
#[doc(hidden)]
pub mod shellcheck_runtime;
mod stdin;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::args::{Args, Command, FormatCommand, TerminalColor};
use crate::config::ConfigArguments;

/// Exit status returned by [`run`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    /// The command completed successfully without reporting failures.
    Success,
    /// The command completed, but reported lint or formatting failures.
    Failure,
    /// The command failed due to invalid input, I/O, or another runtime error.
    Error,
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        match status {
            ExitStatus::Success => ExitCode::from(0),
            ExitStatus::Failure => ExitCode::from(1),
            ExitStatus::Error => ExitCode::from(2),
        }
    }
}

/// Run a parsed `shuck` command and return the resulting process status.
pub fn run(args: Args) -> Result<ExitStatus> {
    let Args {
        cache_dir,
        config,
        color,
        command,
    } = args;

    if let Some(color_override) = colored_override(color, std::env::var_os("FORCE_COLOR")) {
        colored::control::set_override(color_override);
    }

    match command {
        Command::Check(command) => commands::check::check(command, &config, cache_dir.as_deref()),
        Command::Format(command) => format(command, &config, cache_dir.as_deref()),
        Command::Clean(command) => commands::clean::clean(command, &config, cache_dir.as_deref()),
    }
}

#[doc(hidden)]
pub fn benchmark_check_paths(
    cwd: &Path,
    paths: &[PathBuf],
    output_format: args::CheckOutputFormatArg,
) -> Result<usize> {
    commands::check::benchmark_check_paths(cwd, paths, output_format)
}

fn format(
    mut args: FormatCommand,
    config_arguments: &ConfigArguments,
    cache_dir: Option<&Path>,
) -> Result<ExitStatus> {
    let stdin = is_stdin(&args.files, args.stdin_filename.as_deref());
    args.files = resolve_default_files(args.files, stdin);

    if stdin {
        commands::format_stdin::format_stdin(args, config_arguments)
    } else {
        commands::format::format(args, config_arguments, cache_dir)
    }
}

fn is_stdin(files: &[PathBuf], stdin_filename: Option<&Path>) -> bool {
    if stdin_filename.is_some() {
        return true;
    }

    matches!(files, [file] if file == Path::new("-"))
}

fn resolve_default_files(files: Vec<PathBuf>, is_stdin: bool) -> Vec<PathBuf> {
    if files.is_empty() {
        if is_stdin {
            vec![PathBuf::from("-")]
        } else {
            vec![PathBuf::from(".")]
        }
    } else {
        files
    }
}

fn colored_override(
    color: Option<TerminalColor>,
    env_force_color: Option<std::ffi::OsString>,
) -> Option<bool> {
    match color {
        Some(TerminalColor::Always) => Some(true),
        Some(TerminalColor::Never) => Some(false),
        Some(TerminalColor::Auto) | None => {
            env_force_color.map(|force_color| !force_color.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_color_env_is_respected() {
        assert_eq!(colored_override(None, Some("1".into())), Some(true));
    }

    #[test]
    fn cli_color_overrides_force_color_env() {
        assert_eq!(
            colored_override(Some(TerminalColor::Never), Some("1".into())),
            Some(false)
        );
        assert_eq!(
            colored_override(Some(TerminalColor::Always), None),
            Some(true)
        );
    }
}
