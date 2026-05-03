use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use shuck_config::{
    ConfigArguments, resolve_project_root_for_file, resolve_project_root_for_input,
};
use shuck_formatter::{FormatError, FormattedSource, format_source, source_is_formatted};

use crate::ExitStatus;
use crate::args::FormatCommand;
use crate::commands::format::{FormatMode, unified_diff, write_parse_error_line};
use crate::format_settings::resolve_project_format_settings;
use crate::stdin::read_from_stdin;

pub(crate) fn format_stdin(
    args: FormatCommand,
    config_arguments: &ConfigArguments,
) -> Result<ExitStatus> {
    let mode = FormatMode::from_cli(&args);
    let source = read_from_stdin()?;
    let path = args.stdin_filename.as_deref();
    let display_path = display_path(path);
    let cwd = std::env::current_dir()?;
    let project_root = stdin_project_root(path, &cwd, config_arguments.use_config_roots())?;
    let options = resolve_project_format_settings(
        &project_root,
        config_arguments,
        args.format_settings_patch(),
    )?
    .to_shell_format_options();

    if matches!(mode, FormatMode::Check) {
        return match source_is_formatted(&source, path, &options) {
            Ok(true) => Ok(ExitStatus::Success),
            Ok(false) => Ok(ExitStatus::Failure),
            Err(FormatError::Parse {
                message,
                line,
                column,
            }) => {
                let mut stdout = io::stdout().lock();
                write_parse_error_line(&mut stdout, &display_path, line, column, &message)?;
                Ok(ExitStatus::Error)
            }
            Err(FormatError::Internal(message)) => Err(anyhow!(message)),
        };
    }

    match format_source(&source, path, &options) {
        Ok(FormattedSource::Unchanged) => {
            if mode.is_write() {
                let mut stdout = io::stdout().lock();
                stdout.write_all(source.as_bytes())?;
            }
            Ok(ExitStatus::Success)
        }
        Ok(FormattedSource::Formatted(formatted)) => match mode {
            FormatMode::Write => {
                let mut stdout = io::stdout().lock();
                stdout.write_all(formatted.as_bytes())?;
                Ok(ExitStatus::Success)
            }
            FormatMode::Check => Ok(ExitStatus::Failure),
            FormatMode::Diff => {
                let mut stdout = io::stdout().lock();
                write!(
                    &mut stdout,
                    "{}",
                    unified_diff(&display_path, &source, &formatted)
                )?;
                Ok(ExitStatus::Failure)
            }
        },
        Err(FormatError::Parse {
            message,
            line,
            column,
        }) => {
            let mut stdout = io::stdout().lock();
            write_parse_error_line(&mut stdout, &display_path, line, column, &message)?;
            Ok(ExitStatus::Error)
        }
        Err(FormatError::Internal(message)) => Err(anyhow!(message)),
    }
}

fn display_path(path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("<stdin>"))
}

fn stdin_project_root(path: Option<&Path>, cwd: &Path, use_config_roots: bool) -> Result<PathBuf> {
    match path {
        Some(path) => {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            Ok(resolve_project_root_for_file(&path, cwd, use_config_roots)?)
        }
        None => Ok(resolve_project_root_for_input(cwd, use_config_roots)?),
    }
}
