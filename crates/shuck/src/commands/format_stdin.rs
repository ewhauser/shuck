use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use shuck_formatter::{FormatError, FormattedSource, FormatterSettings, format_source};

use crate::ExitStatus;
use crate::args::FormatCommand;
use crate::commands::format::{FormatMode, unified_diff, write_parse_error_line};
use crate::stdin::read_from_stdin;

pub(crate) fn format_stdin(args: FormatCommand) -> Result<ExitStatus> {
    let mode = FormatMode::from_cli(&args);
    let source = read_from_stdin()?;
    let path = args.stdin_filename.as_deref();
    let display_path = display_path(path);

    match format_source(&source, path, &FormatterSettings::default()) {
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
    }
}

fn display_path(path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("<stdin>"))
}
