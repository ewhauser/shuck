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
    /// Format shell files.
    Format(FormatCommand),
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
pub struct FormatCommand {
    /// List of files or directories to format, or `-` to read from stdin.
    pub files: Vec<PathBuf>,
    /// Avoid writing any formatted files back; instead, exit non-zero if any files would change.
    #[arg(long)]
    pub check: bool,
    /// Avoid writing any formatted files back; instead, print a diff for each changed file.
    #[arg(long)]
    pub diff: bool,
    /// Disable cache reads and writes.
    #[arg(long = "no-cache")]
    pub no_cache: bool,
    /// The name of the file when reading the source from stdin.
    #[arg(long)]
    pub stdin_filename: Option<PathBuf>,
    /// Omit files or directories matching the provided glob patterns.
    #[arg(long, value_delimiter = ',', value_name = "GLOB")]
    pub exclude: Vec<String>,
    /// Respect file exclusions via `.gitignore` and other standard ignore files.
    #[arg(long, overrides_with = "no_respect_gitignore")]
    pub(crate) respect_gitignore: bool,
    #[arg(long, overrides_with = "respect_gitignore", hide = true)]
    pub(crate) no_respect_gitignore: bool,
    /// Enforce exclusions even for paths passed directly on the command line.
    #[arg(long, overrides_with = "no_force_exclude")]
    pub(crate) force_exclude: bool,
    #[arg(long, overrides_with = "force_exclude", hide = true)]
    pub(crate) no_force_exclude: bool,
}

impl FormatCommand {
    pub fn respect_gitignore(&self) -> bool {
        match (self.respect_gitignore, self.no_respect_gitignore) {
            (false, false) | (true, false) => true,
            (false, true) => false,
            (true, true) => unreachable!("clap should make this impossible"),
        }
    }

    pub fn force_exclude(&self) -> bool {
        match (self.force_exclude, self.no_force_exclude) {
            (false, false) | (false, true) => false,
            (true, false) => true,
            (true, true) => unreachable!("clap should make this impossible"),
        }
    }
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CleanCommand {
    /// Files or directories whose project caches should be removed.
    pub paths: Vec<PathBuf>,
}
