pub mod args;

mod commands;
mod config;
mod discover;
mod stdin;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::args::{Args, Command, FormatCommand};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    Success,
    Failure,
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

pub fn run(args: Args) -> Result<ExitStatus> {
    match args.command {
        Command::Check(command) => commands::check::check(command),
        Command::Format(command) => format(command),
        Command::Clean(command) => commands::clean::clean(command),
    }
}

fn format(mut args: FormatCommand) -> Result<ExitStatus> {
    let stdin = is_stdin(&args.files, args.stdin_filename.as_deref());
    args.files = resolve_default_files(args.files, stdin);

    if stdin {
        commands::format_stdin::format_stdin(args)
    } else {
        commands::format::format(args)
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
