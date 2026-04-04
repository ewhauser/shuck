use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "shuck")]
#[command(about = "Shell checker CLI for shuck")]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse shell files and report syntax failures.
    Check(CheckCommand),
    /// Remove shuck caches under the provided paths.
    Clean(CleanCommand),
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CheckCommand {
    /// Apply safe fixes.
    #[arg(long)]
    pub fix: bool,
    /// Apply unsafe fixes.
    #[arg(long = "unsafe-fixes")]
    pub unsafe_fixes: bool,
    /// Disable cache reads and writes.
    #[arg(long = "no-cache")]
    pub no_cache: bool,
    /// Files or directories to check.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CleanCommand {
    /// Files or directories whose project caches should be removed.
    pub paths: Vec<PathBuf>,
}
