pub mod args;

mod commands;
mod config;
mod discover;

use std::process::ExitCode;

use anyhow::Result;

use crate::args::{Args, Command};

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
        Command::Clean(command) => commands::clean::clean(command),
    }
}
