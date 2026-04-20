use std::ffi::OsString;
use std::path::PathBuf;

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::error::ErrorKind;
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use shuck_formatter::{IndentStyle, ShellDialect};

use crate::format_settings::FormatSettingsPatch;

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());
const EXPERIMENTAL_ENV_VAR: &str = "SHUCK_EXPERIMENTAL";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatDialectArg {
    Auto,
    Bash,
    Posix,
    Mksh,
    Zsh,
}

impl From<FormatDialectArg> for ShellDialect {
    fn from(value: FormatDialectArg) -> Self {
        match value {
            FormatDialectArg::Auto => Self::Auto,
            FormatDialectArg::Bash => Self::Bash,
            FormatDialectArg::Posix => Self::Posix,
            FormatDialectArg::Mksh => Self::Mksh,
            FormatDialectArg::Zsh => Self::Zsh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatIndentStyleArg {
    Tab,
    Space,
}

impl From<FormatIndentStyleArg> for IndentStyle {
    fn from(value: FormatIndentStyleArg) -> Self {
        match value {
            FormatIndentStyleArg::Tab => Self::Tab,
            FormatIndentStyleArg::Space => Self::Space,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CheckOutputFormatArg {
    Full,
    Concise,
}

#[derive(Debug, Parser)]
#[command(name = "shuck")]
#[command(about = "Shell checker CLI for shuck")]
#[command(styles = STYLES)]
struct StableCli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: StableCommand,
}

#[derive(Debug, Parser)]
#[command(name = "shuck")]
#[command(about = "Shell checker CLI for shuck")]
#[command(styles = STYLES)]
struct ExperimentalCli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: ExperimentalCommand,
}

#[derive(Debug, Clone, ClapArgs)]
struct GlobalArgs {
    /// Path to the cache directory.
    #[arg(
        long,
        env = "SHUCK_CACHE_DIR",
        global = true,
        value_name = "PATH",
        help_heading = "Miscellaneous"
    )]
    cache_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum StableCommand {
    /// Parse shell files and report syntax failures.
    Check(CheckCommand),
    #[command(hide = true)]
    Format(FormatCommand),
    /// Remove shuck cache entries for the provided paths' projects.
    Clean(CleanCommand),
}

#[derive(Debug, Subcommand)]
enum ExperimentalCommand {
    /// Parse shell files and report syntax failures.
    Check(CheckCommand),
    /// Format shell files.
    Format(FormatCommand),
    /// Remove shuck cache entries for the provided paths' projects.
    Clean(CleanCommand),
}

#[derive(Debug, Clone)]
pub struct Args {
    pub cache_dir: Option<PathBuf>,
    pub command: Command,
}

impl Args {
    pub fn parse() -> Self {
        Self::try_parse().unwrap_or_else(|err| err.exit())
    }

    pub fn try_parse() -> Result<Self, clap::Error> {
        Self::try_parse_from(std::env::args_os())
    }

    pub fn try_parse_from<I, T>(itr: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        if experimental_enabled() {
            ExperimentalCli::try_parse_from(itr).map(Into::into)
        } else {
            let parsed = StableCli::try_parse_from(itr)?;
            Self::from_stable(parsed)
        }
    }
}

impl Args {
    fn from_stable(value: StableCli) -> Result<Self, clap::Error> {
        let command = match value.command {
            StableCommand::Check(command) => Command::Check(command),
            StableCommand::Format(_) => {
                return Err(clap::Error::raw(
                    ErrorKind::InvalidSubcommand,
                    format!(
                        "the `format` subcommand is experimental; set {EXPERIMENTAL_ENV_VAR}=1 to enable it"
                    ),
                ));
            }
            StableCommand::Clean(command) => Command::Clean(command),
        };

        Ok(Self {
            cache_dir: value.global.cache_dir,
            command,
        })
    }
}

impl From<ExperimentalCli> for Args {
    fn from(value: ExperimentalCli) -> Self {
        let command = match value.command {
            ExperimentalCommand::Check(command) => Command::Check(command),
            ExperimentalCommand::Format(command) => Command::Format(command),
            ExperimentalCommand::Clean(command) => Command::Clean(command),
        };

        Self {
            cache_dir: value.global.cache_dir,
            command,
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Parse shell files and report syntax failures.
    Check(CheckCommand),
    /// Format shell files.
    Format(FormatCommand),
    /// Remove shuck cache entries for the provided paths' projects.
    Clean(CleanCommand),
}

fn experimental_enabled() -> bool {
    std::env::var_os(EXPERIMENTAL_ENV_VAR).is_some_and(|value| {
        !matches!(
            value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CheckCommand {
    /// Apply safe fixes.
    #[arg(long)]
    pub fix: bool,
    /// Apply unsafe fixes.
    #[arg(long = "unsafe-fixes")]
    pub unsafe_fixes: bool,
    /// Choose the text output format for reported diagnostics.
    #[arg(long = "output-format", value_enum, default_value_t = CheckOutputFormatArg::Full)]
    pub output_format: CheckOutputFormatArg,
    /// Files or directories to check.
    pub paths: Vec<PathBuf>,
    /// Disable cache reads and writes.
    #[arg(long = "no-cache", help_heading = "Miscellaneous")]
    pub no_cache: bool,
    /// Exit with status code "0", even upon detecting lint violations. Parse errors and error-severity diagnostics still fail.
    #[arg(short = 'e', long = "exit-zero", help_heading = "Miscellaneous")]
    pub exit_zero: bool,
    /// Exit with a non-zero status code if any files were modified via fix, even if no lint violations remain.
    #[arg(long = "exit-non-zero-on-fix", help_heading = "Miscellaneous")]
    pub exit_non_zero_on_fix: bool,
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
    /// Override the auto-discovered shell dialect used for parsing and formatting.
    #[arg(long, value_enum)]
    pub dialect: Option<FormatDialectArg>,
    /// Choose the indentation style.
    #[arg(long, value_enum)]
    pub indent_style: Option<FormatIndentStyleArg>,
    /// Set the indentation width for space indentation.
    #[arg(long, value_name = "WIDTH")]
    pub indent_width: Option<u8>,
    /// Put binary operators on the next line when breaking lists and pipelines.
    #[arg(long, overrides_with = "no_binary_next_line")]
    pub(crate) binary_next_line: bool,
    #[arg(
        long = "no-binary-next-line",
        overrides_with = "binary_next_line",
        hide = true
    )]
    pub(crate) no_binary_next_line: bool,
    /// Indent the bodies of `case` branches.
    #[arg(long, overrides_with = "no_switch_case_indent")]
    pub(crate) switch_case_indent: bool,
    #[arg(
        long = "no-switch-case-indent",
        overrides_with = "switch_case_indent",
        hide = true
    )]
    pub(crate) no_switch_case_indent: bool,
    /// Insert spaces around redirection operators and targets.
    #[arg(long, overrides_with = "no_space_redirects")]
    pub(crate) space_redirects: bool,
    #[arg(
        long = "no-space-redirects",
        overrides_with = "space_redirects",
        hide = true
    )]
    pub(crate) no_space_redirects: bool,
    /// Preserve source padding when it is safe to do so.
    #[arg(long, overrides_with = "no_keep_padding")]
    pub(crate) keep_padding: bool,
    #[arg(long = "no-keep-padding", overrides_with = "keep_padding", hide = true)]
    pub(crate) no_keep_padding: bool,
    /// Put function opening braces on the next line.
    #[arg(long, overrides_with = "no_function_next_line")]
    pub(crate) function_next_line: bool,
    #[arg(
        long = "no-function-next-line",
        overrides_with = "function_next_line",
        hide = true
    )]
    pub(crate) no_function_next_line: bool,
    /// Prefer compact layouts and avoid optional splitting.
    #[arg(long, overrides_with = "no_never_split")]
    pub(crate) never_split: bool,
    #[arg(long = "no-never-split", overrides_with = "never_split", hide = true)]
    pub(crate) no_never_split: bool,
    /// Apply safe simplifications before formatting.
    #[arg(long)]
    pub simplify: bool,
    /// Emit a compact minified form and drop comments.
    #[arg(long)]
    pub minify: bool,
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
    pub(crate) fn format_settings_patch(&self) -> FormatSettingsPatch {
        FormatSettingsPatch {
            dialect: self.dialect.map(Into::into),
            indent_style: self.indent_style.map(Into::into),
            indent_width: self.indent_width,
            binary_next_line: self.binary_next_line(),
            switch_case_indent: self.switch_case_indent(),
            space_redirects: self.space_redirects(),
            keep_padding: self.keep_padding(),
            function_next_line: self.function_next_line(),
            never_split: self.never_split(),
            simplify: self.simplify.then_some(true),
            minify: self.minify.then_some(true),
        }
    }

    pub fn binary_next_line(&self) -> Option<bool> {
        tri_state_bool(self.binary_next_line, self.no_binary_next_line)
    }

    pub fn switch_case_indent(&self) -> Option<bool> {
        tri_state_bool(self.switch_case_indent, self.no_switch_case_indent)
    }

    pub fn space_redirects(&self) -> Option<bool> {
        tri_state_bool(self.space_redirects, self.no_space_redirects)
    }

    pub fn keep_padding(&self) -> Option<bool> {
        tri_state_bool(self.keep_padding, self.no_keep_padding)
    }

    pub fn function_next_line(&self) -> Option<bool> {
        tri_state_bool(self.function_next_line, self.no_function_next_line)
    }

    pub fn never_split(&self) -> Option<bool> {
        tri_state_bool(self.never_split, self.no_never_split)
    }

    pub fn respect_gitignore(&self) -> bool {
        match (self.respect_gitignore, self.no_respect_gitignore) {
            (false, false) | (true, false) => true,
            (false, true) => false,
            // Clap's `overrides_with` on these paired flags keeps only the
            // last occurrence, so both booleans cannot remain set here.
            (true, true) => unreachable!("clap should make this impossible"),
        }
    }

    pub fn force_exclude(&self) -> bool {
        match (self.force_exclude, self.no_force_exclude) {
            (false, false) | (false, true) => false,
            (true, false) => true,
            // Clap's `overrides_with` on these paired flags keeps only the
            // last occurrence, so both booleans cannot remain set here.
            (true, true) => unreachable!("clap should make this impossible"),
        }
    }
}

fn tri_state_bool(positive: bool, negative: bool) -> Option<bool> {
    match (positive, negative) {
        (false, false) => None,
        (true, false) => Some(true),
        (false, true) => Some(false),
        // The caller wires every positive/negative flag pair with
        // `overrides_with`, so clap normalizes repeated input down to at most
        // one active boolean before we derive the tri-state value.
        (true, true) => unreachable!("clap should make this impossible"),
    }
}

#[derive(Debug, Clone, ClapArgs)]
pub struct CleanCommand {
    /// Files or directories whose project caches should be removed.
    pub paths: Vec<PathBuf>,
}
