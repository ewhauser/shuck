//! Command-line argument types and parsing helpers for the `shuck` CLI.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::error::ErrorKind;
use clap::{
    Args as ClapArgs, ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
};
use shuck_formatter::{IndentStyle, ShellDialect};
use shuck_linter::RuleSelector;

use crate::config::{ConfigArgumentParser, ConfigArguments, SingleConfigArgument};
use crate::format_settings::FormatSettingsPatch;

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());
const EXPERIMENTAL_ENV_VAR: &str = "SHUCK_EXPERIMENTAL";

/// Shell dialect override accepted by `shuck format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatDialectArg {
    /// Detect the dialect from the source and file path when possible.
    Auto,
    /// Parse and format as Bash.
    Bash,
    /// Parse and format as a POSIX-style shell.
    Posix,
    /// Parse and format as mksh.
    Mksh,
    /// Parse and format as zsh.
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

/// Indentation styles accepted by `shuck format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatIndentStyleArg {
    /// Indent with tab characters.
    Tab,
    /// Indent with spaces.
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

/// Output formats supported by `shuck check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CheckOutputFormatArg {
    /// Emit one diagnostic per line.
    Concise,
    /// Emit rich human-readable diagnostics.
    Full,
    /// Emit a JSON array of diagnostics.
    Json,
    /// Emit one JSON object per line.
    JsonLines,
    /// Emit JUnit XML.
    Junit,
    /// Emit grouped human-readable diagnostics.
    Grouped,
    /// Emit GitHub Actions workflow commands.
    Github,
    /// Emit GitLab code quality output.
    Gitlab,
    /// Emit Reviewdog RDJSON.
    Rdjson,
    /// Emit SARIF.
    Sarif,
}

/// Color preference for terminal output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TerminalColor {
    /// Display colors if the output goes to an interactive terminal.
    Auto,
    /// Always display colors.
    Always,
    /// Never display colors.
    Never,
}

/// Managed shell names accepted by `shuck run`, `shuck install`, and `shuck shell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ManagedShellArg {
    /// GNU bash.
    Bash,
    /// The gbash runtime.
    Gbash,
    /// The Bashkit runtime.
    Bashkit,
    /// Z shell.
    Zsh,
    /// Debian Almquist shell.
    Dash,
    /// MirBSD Korn shell.
    Mksh,
    /// BusyBox shell wrapper (Linux only).
    Busybox,
}

impl From<ManagedShellArg> for shuck_run::Shell {
    fn from(value: ManagedShellArg) -> Self {
        match value {
            ManagedShellArg::Bash => Self::Bash,
            ManagedShellArg::Gbash => Self::Gbash,
            ManagedShellArg::Bashkit => Self::Bashkit,
            ManagedShellArg::Zsh => Self::Zsh,
            ManagedShellArg::Dash => Self::Dash,
            ManagedShellArg::Mksh => Self::Mksh,
            ManagedShellArg::Busybox => Self::Busybox,
        }
    }
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
    /// Either a path to a TOML configuration file (`shuck.toml`), or a TOML
    /// `<KEY> = <VALUE>` pair (such as you might find in a `shuck.toml`
    /// configuration file) overriding a specific configuration option.
    /// Overrides of individual settings using this option always take
    /// precedence over all configuration files, including configuration files
    /// that were also specified using `--config`.
    #[arg(
        long,
        action = clap::ArgAction::Append,
        value_name = "CONFIG_OPTION",
        value_parser = ConfigArgumentParser,
        global = true,
        help_heading = "Global options"
    )]
    config: Vec<SingleConfigArgument>,
    /// Ignore all configuration files.
    #[arg(long, global = true, help_heading = "Global options")]
    isolated: bool,
    /// Control when colored output is used.
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        global = true,
        help_heading = "Global options"
    )]
    color: Option<TerminalColor>,
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
    /// Lint shell files and supported embedded shell scripts.
    Check(Box<CheckCommand>),
    /// Start the language server over stdio.
    Server(ServerCommand),
    /// Run a shell script with a managed interpreter.
    Run(RunCommand),
    /// Pre-install a managed shell interpreter or list available versions.
    Install(InstallCommand),
    /// Spawn a shell session using a managed interpreter.
    Shell(ShellCommand),
    #[command(hide = true)]
    Format(FormatCommand),
    /// Remove shuck cache entries for the provided paths' projects.
    Clean(CleanCommand),
}

#[derive(Debug, Subcommand)]
enum ExperimentalCommand {
    /// Lint shell files and supported embedded shell scripts.
    Check(Box<CheckCommand>),
    /// Start the language server over stdio.
    Server(ServerCommand),
    /// Run a shell script with a managed interpreter.
    Run(RunCommand),
    /// Pre-install a managed shell interpreter or list available versions.
    Install(InstallCommand),
    /// Spawn a shell session using a managed interpreter.
    Shell(ShellCommand),
    /// Format shell files.
    Format(FormatCommand),
    /// Remove shuck cache entries for the provided paths' projects.
    Clean(CleanCommand),
}

/// Parsed top-level arguments for the `shuck` command.
#[derive(Debug, Clone)]
pub struct Args {
    /// Override for the cache root directory.
    pub cache_dir: Option<PathBuf>,
    pub(crate) config: ConfigArguments,
    pub(crate) color: Option<TerminalColor>,
    /// The subcommand selected by the user.
    pub command: Command,
}

impl Args {
    /// Parse arguments from the current process and exit on invalid input.
    pub fn parse() -> Self {
        Self::try_parse().unwrap_or_else(|err| err.exit())
    }

    /// Parse arguments from the current process without exiting on errors.
    pub fn try_parse() -> Result<Self, clap::Error> {
        Self::try_parse_from(std::env::args_os())
    }

    /// Parse arguments from an arbitrary iterator of command-line values.
    pub fn try_parse_from<I, T>(itr: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        if experimental_enabled() {
            let parsed = parse_with_color::<ExperimentalCli, _, _>(itr)?;
            Self::from_experimental(parsed)
        } else {
            let parsed = parse_with_color::<StableCli, _, _>(itr)?;
            Self::from_stable(parsed)
        }
    }
}

impl Args {
    fn from_stable(value: StableCli) -> Result<Self, clap::Error> {
        let StableCli { global, command } = value;
        let GlobalArgs {
            cache_dir,
            config,
            isolated,
            color,
        } = global;
        let command = match command {
            StableCommand::Check(command) => Command::Check(command),
            StableCommand::Server(command) => Command::Server(command),
            StableCommand::Run(command) => Command::Run(command),
            StableCommand::Install(command) => Command::Install(command),
            StableCommand::Shell(command) => Command::Shell(command),
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
            cache_dir,
            config: ConfigArguments::from_cli(config, isolated)?,
            color,
            command,
        })
    }

    fn from_experimental(value: ExperimentalCli) -> Result<Self, clap::Error> {
        let ExperimentalCli { global, command } = value;
        let GlobalArgs {
            cache_dir,
            config,
            isolated,
            color,
        } = global;
        let command = match command {
            ExperimentalCommand::Check(command) => Command::Check(command),
            ExperimentalCommand::Server(command) => Command::Server(command),
            ExperimentalCommand::Run(command) => Command::Run(command),
            ExperimentalCommand::Install(command) => Command::Install(command),
            ExperimentalCommand::Shell(command) => Command::Shell(command),
            ExperimentalCommand::Format(command) => Command::Format(command),
            ExperimentalCommand::Clean(command) => Command::Clean(command),
        };

        Ok(Self {
            cache_dir,
            config: ConfigArguments::from_cli(config, isolated)?,
            color,
            command,
        })
    }
}

/// Supported `shuck` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Lint shell files and supported embedded shell scripts.
    Check(Box<CheckCommand>),
    /// Start the language server over stdio.
    Server(ServerCommand),
    /// Run a shell script with a managed interpreter.
    Run(RunCommand),
    /// Pre-install a managed shell interpreter or list available versions.
    Install(InstallCommand),
    /// Spawn a shell session using a managed interpreter.
    Shell(ShellCommand),
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

/// Arguments for `shuck server`.
#[derive(Debug, Clone, Default, ClapArgs)]
pub struct ServerCommand {}

/// Arguments for `shuck check`.
#[derive(Debug, Clone, ClapArgs)]
pub struct CheckCommand {
    /// Apply safe fixes.
    #[arg(long)]
    pub fix: bool,
    /// Apply unsafe fixes.
    #[arg(long = "unsafe-fixes")]
    pub unsafe_fixes: bool,
    /// Enable automatic additions of shuck ignore directives to failing lines.
    /// Optionally provide a reason to append after the codes.
    #[arg(
        long = "add-ignore",
        value_name = "REASON",
        default_missing_value = "",
        num_args = 0..=1,
        require_equals = true,
        conflicts_with = "fix",
        conflicts_with = "unsafe_fixes",
    )]
    pub add_ignore: Option<String>,
    /// Output serialization format for violations.
    /// The default serialization format is "full".
    #[arg(
        long = "output-format",
        value_enum,
        env = "SHUCK_OUTPUT_FORMAT",
        default_value_t = CheckOutputFormatArg::Full
    )]
    pub output_format: CheckOutputFormatArg,
    /// Run in watch mode by re-running whenever files change.
    #[arg(short = 'w', long, conflicts_with = "add_ignore")]
    pub watch: bool,
    /// Files or directories to check.
    pub paths: Vec<PathBuf>,
    /// Rule selection and suppression settings.
    #[command(flatten)]
    pub rule_selection: RuleSelectionArgs,
    /// File discovery and exclusion settings.
    #[command(flatten)]
    pub file_selection: FileSelectionArgs,
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

impl CheckCommand {
    /// Whether standard ignore files such as `.gitignore` should be respected.
    pub fn respect_gitignore(&self) -> bool {
        self.file_selection.respect_gitignore()
    }

    /// Whether excludes should also apply to explicitly passed paths.
    pub fn force_exclude(&self) -> bool {
        self.file_selection.force_exclude()
    }
}

/// Arguments for `shuck run`.
#[derive(Debug, Clone, ClapArgs)]
pub struct RunCommand {
    /// Shell interpreter name (`bash`, `gbash`, `bashkit`, `zsh`, `dash`, `mksh`, or Linux-only `busybox`).
    #[arg(short = 's', long, value_enum)]
    pub shell: Option<ManagedShellArg>,
    /// Version constraint (for example `5.2`, `>=5.1,<6`, or `latest`).
    #[arg(short = 'V', long = "shell-version", value_name = "CONSTRAINT")]
    pub shell_version: Option<String>,
    /// Use the system-installed interpreter instead of a managed one.
    #[arg(long)]
    pub system: bool,
    /// Resolve and print the interpreter path without executing.
    #[arg(long)]
    pub dry_run: bool,
    /// Show resolution and download progress.
    #[arg(short = 'v', long)]
    pub verbose: bool,
    /// Evaluate a command string instead of running a script file.
    #[arg(
        short = 'c',
        long = "command",
        value_name = "COMMAND",
        conflicts_with = "script"
    )]
    pub command: Option<String>,
    /// Script path to execute, or `-` to read from stdin.
    pub script: Option<PathBuf>,
    /// Arguments passed through to the script or command.
    #[arg(last = true, value_name = "ARGS")]
    pub script_args: Vec<OsString>,
}

/// Arguments for `shuck install`.
#[derive(Debug, Clone, ClapArgs)]
pub struct InstallCommand {
    /// Show available shells and versions instead of installing anything.
    #[arg(long)]
    pub list: bool,
    /// Force a fresh registry fetch even if the local registry cache is still fresh.
    #[arg(long)]
    pub refresh: bool,
    /// Shell interpreter name (`bash`, `gbash`, `bashkit`, `zsh`, `dash`, `mksh`, or Linux-only `busybox`).
    #[arg(required_unless_present = "list", value_enum)]
    pub shell: Option<ManagedShellArg>,
    /// Version constraint to install.
    #[arg(required_unless_present = "list")]
    pub version: Option<String>,
}

/// Arguments for `shuck shell`.
#[derive(Debug, Clone, ClapArgs)]
pub struct ShellCommand {
    /// Shell interpreter name (`bash`, `gbash`, `bashkit`, `zsh`, `dash`, `mksh`, or Linux-only `busybox`).
    #[arg(short = 's', long, value_enum)]
    pub shell: Option<ManagedShellArg>,
    /// Version constraint (for example `5.2`, `>=5.1,<6`, or `latest`).
    #[arg(short = 'V', long = "shell-version", value_name = "CONSTRAINT")]
    pub shell_version: Option<String>,
    /// Use the system-installed interpreter instead of a managed one.
    #[arg(long)]
    pub system: bool,
    /// Show resolution and download progress.
    #[arg(short = 'v', long)]
    pub verbose: bool,
}

/// A `<pattern>:<rule-selector>` mapping from the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternRuleSelectorPair {
    /// Glob-style file pattern.
    pub pattern: String,
    /// Rule selector applied to matching files.
    pub selector: RuleSelector,
}

impl std::str::FromStr for PatternRuleSelectorPair {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (pattern, selector) = value
            .rsplit_once(':')
            .ok_or_else(|| "expected <FilePattern>:<RuleCode>".to_owned())?;
        let pattern = pattern.trim();
        let selector = selector.trim();

        if pattern.is_empty() || selector.is_empty() {
            return Err("expected <FilePattern>:<RuleCode>".to_owned());
        }

        Ok(Self {
            pattern: pattern.to_owned(),
            selector: parse_cli_rule_selector(selector)?,
        })
    }
}

/// A `<pattern>:<shell>` mapping from the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternShellPair {
    /// Glob-style file pattern.
    pub pattern: String,
    /// Shell dialect applied to matching files.
    pub shell: shuck_linter::ShellDialect,
}

impl std::str::FromStr for PatternShellPair {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (pattern, shell) = value
            .rsplit_once(':')
            .ok_or_else(|| "expected <FilePattern>:<Shell>".to_owned())?;
        let pattern = pattern.trim();
        let shell = shell.trim();

        if pattern.is_empty() || shell.is_empty() {
            return Err("expected <FilePattern>:<Shell>".to_owned());
        }

        let shell = shuck_linter::ShellDialect::from_name(shell);
        if shell == shuck_linter::ShellDialect::Unknown {
            return Err(
                "expected shell dialect to be one of sh, bash, dash, ksh, mksh, zsh".to_owned(),
            );
        }

        Ok(Self {
            pattern: pattern.to_owned(),
            shell,
        })
    }
}

fn parse_cli_rule_selector(value: &str) -> Result<RuleSelector, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("rule selector cannot be empty".to_owned());
    }

    value.parse::<RuleSelector>().map_err(|err| err.to_string())
}

/// Rule-selection flags shared by `shuck check`.
#[derive(Debug, Clone, Default, ClapArgs)]
pub struct RuleSelectionArgs {
    /// Comma-separated list of rule selectors to enable (for example `google`, `C`, or `C001`; or ALL to enable all rules).
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub select: Option<Vec<RuleSelector>>,
    /// Comma-separated list of rule selectors to disable.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub ignore: Vec<RuleSelector>,
    /// Like --select, but adds additional rule selectors on top of those already specified.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub extend_select: Vec<RuleSelector>,
    /// List of mappings from file pattern to code to exclude.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "PER_FILE_IGNORES",
        help_heading = "Rule selection"
    )]
    pub per_file_ignores: Option<Vec<PatternRuleSelectorPair>>,
    /// Like `--per-file-ignores`, but adds additional ignores on top of those already specified.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "EXTEND_PER_FILE_IGNORES",
        help_heading = "Rule selection"
    )]
    pub extend_per_file_ignores: Vec<PatternRuleSelectorPair>,
    /// List of mappings from file pattern to shell dialect.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "PER_FILE_SHELL",
        help_heading = "Rule selection"
    )]
    pub per_file_shell: Option<Vec<PatternShellPair>>,
    /// Like `--per-file-shell`, but adds additional shell mappings on top of those already specified.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "EXTEND_PER_FILE_SHELL",
        help_heading = "Rule selection"
    )]
    pub extend_per_file_shell: Vec<PatternShellPair>,
    /// List of rule selectors to treat as eligible for fix. Only applicable when fix itself is enabled (e.g., via `--fix`).
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub fixable: Option<Vec<RuleSelector>>,
    /// List of rule selectors to treat as ineligible for fix. Only applicable when fix itself is enabled (e.g., via `--fix`).
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub unfixable: Vec<RuleSelector>,
    /// Like --fixable, but adds additional rule selectors on top of those already specified.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_cli_rule_selector,
        value_name = "RULE_CODE",
        help_heading = "Rule selection",
        hide_possible_values = true
    )]
    pub extend_fixable: Vec<RuleSelector>,
}

fn parse_with_color<Cli, I, T>(itr: I) -> Result<Cli, clap::Error>
where
    Cli: CommandFactory + FromArgMatches,
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = itr.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut command = Cli::command().color(command_color_choice(&args));
    let matches = command.try_get_matches_from_mut(args)?;
    Cli::from_arg_matches(&matches)
}

fn command_color_choice(args: &[OsString]) -> ColorChoice {
    match preparse_color(args) {
        Some(ColorChoice::Always) => ColorChoice::Always,
        Some(ColorChoice::Never) => ColorChoice::Never,
        Some(ColorChoice::Auto) | None => {
            if std::env::var_os("FORCE_COLOR").is_some_and(|value| !value.is_empty()) {
                ColorChoice::Always
            } else {
                ColorChoice::Auto
            }
        }
    }
}

fn preparse_color(args: &[OsString]) -> Option<ColorChoice> {
    let mut expect_value = false;
    let mut color = None;

    for argument in args.iter().skip(1) {
        if expect_value {
            let value = argument.to_string_lossy();
            color = value.parse().ok();
            expect_value = false;
            continue;
        }

        let argument = argument.to_string_lossy();
        if argument == "--" {
            break;
        }
        if argument == "--color" {
            expect_value = true;
            continue;
        }
        if let Some(value) = argument.strip_prefix("--color=") {
            color = value.parse().ok();
        }
    }

    color
}

/// File-discovery and exclusion flags shared by multiple commands.
#[derive(Debug, Clone, Default, ClapArgs)]
pub struct FileSelectionArgs {
    /// List of paths, used to omit files and/or directories from analysis.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "FILE_PATTERN",
        help_heading = "File selection"
    )]
    pub exclude: Vec<String>,
    /// Like --exclude, but adds additional files and directories on top of those already excluded.
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "FILE_PATTERN",
        help_heading = "File selection"
    )]
    pub extend_exclude: Vec<String>,
    /// Respect file exclusions via `.gitignore` and other standard ignore files.
    /// Use `--no-respect-gitignore` to disable.
    #[arg(
        long,
        overrides_with = "no_respect_gitignore",
        help_heading = "File selection"
    )]
    pub(crate) respect_gitignore: bool,
    #[arg(long, overrides_with = "respect_gitignore", hide = true)]
    pub(crate) no_respect_gitignore: bool,
    /// Enforce exclusions, even for paths passed to shuck directly on the command-line.
    /// Use `--no-force-exclude` to disable.
    #[arg(
        long,
        overrides_with = "no_force_exclude",
        help_heading = "File selection"
    )]
    pub(crate) force_exclude: bool,
    #[arg(long, overrides_with = "force_exclude", hide = true)]
    pub(crate) no_force_exclude: bool,
}

impl FileSelectionArgs {
    /// Resolve the effective `respect_gitignore` setting after CLI overrides.
    pub fn respect_gitignore(&self) -> bool {
        resolve_bool_flag(self.respect_gitignore, self.no_respect_gitignore, true)
    }

    /// Resolve the effective `force_exclude` setting after CLI overrides.
    pub fn force_exclude(&self) -> bool {
        resolve_bool_flag(self.force_exclude, self.no_force_exclude, false)
    }
}

/// Arguments for `shuck format`.
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
    /// File discovery and exclusion settings.
    #[command(flatten)]
    pub file_selection: FileSelectionArgs,
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

    /// Resolve the effective `binary-next-line` formatter option.
    pub fn binary_next_line(&self) -> Option<bool> {
        tri_state_bool(self.binary_next_line, self.no_binary_next_line)
    }

    /// Resolve the effective `switch-case-indent` formatter option.
    pub fn switch_case_indent(&self) -> Option<bool> {
        tri_state_bool(self.switch_case_indent, self.no_switch_case_indent)
    }

    /// Resolve the effective `space-redirects` formatter option.
    pub fn space_redirects(&self) -> Option<bool> {
        tri_state_bool(self.space_redirects, self.no_space_redirects)
    }

    /// Resolve the effective `keep-padding` formatter option.
    pub fn keep_padding(&self) -> Option<bool> {
        tri_state_bool(self.keep_padding, self.no_keep_padding)
    }

    /// Resolve the effective `function-next-line` formatter option.
    pub fn function_next_line(&self) -> Option<bool> {
        tri_state_bool(self.function_next_line, self.no_function_next_line)
    }

    /// Resolve the effective `never-split` formatter option.
    pub fn never_split(&self) -> Option<bool> {
        tri_state_bool(self.never_split, self.no_never_split)
    }

    /// Whether standard ignore files such as `.gitignore` should be respected.
    pub fn respect_gitignore(&self) -> bool {
        self.file_selection.respect_gitignore()
    }

    /// Whether excludes should also apply to explicitly passed paths.
    pub fn force_exclude(&self) -> bool {
        self.file_selection.force_exclude()
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

fn resolve_bool_flag(positive: bool, negative: bool, default: bool) -> bool {
    match (positive, negative) {
        (false, false) => default,
        (true, false) => true,
        (false, true) => false,
        // Clap's `overrides_with` on these paired flags keeps only the
        // last occurrence, so both booleans cannot remain set here.
        (true, true) => unreachable!("clap should make this impossible"),
    }
}

/// Arguments for `shuck clean`.
#[derive(Debug, Clone, ClapArgs)]
pub struct CleanCommand {
    /// Files or directories whose project caches should be removed.
    pub paths: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::builder::TypedValueParser;
    use shuck_linter::Rule;

    #[test]
    fn global_config_override_is_available_after_subcommand() {
        let command = StableCli::command();
        let override_argument = crate::config::ConfigArgumentParser
            .parse_ref(
                &command,
                None,
                std::ffi::OsStr::new("format.indent-width = 2"),
            )
            .unwrap();

        let args = Args::try_parse_from(["shuck", "check", "--config", "format.indent-width = 2"])
            .unwrap();

        assert_eq!(
            args.config,
            ConfigArguments::from_cli(vec![override_argument], false).unwrap()
        );
    }

    #[test]
    fn explicit_config_file_and_inline_override_both_parse_globally() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_path = tempdir.path().join("shuck.toml");
        std::fs::write(&config_path, "[format]\nfunction-next-line = false\n").unwrap();
        let command = StableCli::command();
        let override_argument = crate::config::ConfigArgumentParser
            .parse_ref(
                &command,
                None,
                std::ffi::OsStr::new("format.function-next-line = true"),
            )
            .unwrap();

        let args = Args::try_parse_from([
            "shuck",
            "--config",
            config_path.to_str().unwrap(),
            "--config",
            "format.function-next-line = true",
            "check",
        ])
        .unwrap();

        assert_eq!(
            args.config,
            ConfigArguments::from_cli(
                vec![
                    SingleConfigArgument::FilePath(config_path),
                    override_argument
                ],
                false,
            )
            .unwrap()
        );
    }

    #[test]
    fn global_color_can_be_parsed_before_subcommand() {
        let args = Args::try_parse_from(["shuck", "--color", "never", "check"]).unwrap();
        assert_eq!(args.color, Some(TerminalColor::Never));
    }

    #[test]
    fn preparse_color_uses_last_value() {
        assert_eq!(
            preparse_color(&[
                OsString::from("shuck"),
                OsString::from("--color=always"),
                OsString::from("--color"),
                OsString::from("never"),
            ]),
            Some(ColorChoice::Never)
        );
    }

    fn parse_check<I, T>(args: I) -> CheckCommand
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let parsed = StableCli::try_parse_from(args).unwrap();
        match Args::from_stable(parsed).unwrap().command {
            Command::Check(command) => *command,
            command => panic!("expected check command, got {command:?}"),
        }
    }

    fn parse_run<I, T>(args: I) -> RunCommand
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let parsed = StableCli::try_parse_from(args).unwrap();
        match Args::from_stable(parsed).unwrap().command {
            Command::Run(command) => command,
            command => panic!("expected run command, got {command:?}"),
        }
    }

    fn parse_install<I, T>(args: I) -> InstallCommand
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let parsed = StableCli::try_parse_from(args).unwrap();
        match Args::from_stable(parsed).unwrap().command {
            Command::Install(command) => command,
            command => panic!("expected install command, got {command:?}"),
        }
    }

    fn parse_shell<I, T>(args: I) -> ShellCommand
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let parsed = StableCli::try_parse_from(args).unwrap();
        match Args::from_stable(parsed).unwrap().command {
            Command::Shell(command) => command,
            command => panic!("expected shell command, got {command:?}"),
        }
    }

    #[test]
    fn parses_add_ignore_without_reason() {
        let command = parse_check(["shuck", "check", "--add-ignore"]);

        assert_eq!(command.add_ignore, Some(String::new()));
    }

    #[test]
    fn parses_add_ignore_with_reason() {
        let command = parse_check(["shuck", "check", "--add-ignore=legacy"]);

        assert_eq!(command.add_ignore.as_deref(), Some("legacy"));
    }

    #[test]
    fn parses_short_watch_flag() {
        let command = parse_check(["shuck", "check", "-w"]);

        assert!(command.watch);
    }

    #[test]
    fn parses_long_watch_flag() {
        let command = parse_check(["shuck", "check", "--watch"]);

        assert!(command.watch);
    }

    #[test]
    fn parses_all_check_output_formats() {
        for (raw, expected) in [
            ("concise", CheckOutputFormatArg::Concise),
            ("full", CheckOutputFormatArg::Full),
            ("json", CheckOutputFormatArg::Json),
            ("json-lines", CheckOutputFormatArg::JsonLines),
            ("junit", CheckOutputFormatArg::Junit),
            ("grouped", CheckOutputFormatArg::Grouped),
            ("github", CheckOutputFormatArg::Github),
            ("gitlab", CheckOutputFormatArg::Gitlab),
            ("rdjson", CheckOutputFormatArg::Rdjson),
            ("sarif", CheckOutputFormatArg::Sarif),
        ] {
            let command = parse_check(["shuck", "check", "--output-format", raw]);
            assert_eq!(command.output_format, expected, "failed to parse {raw}");
        }
    }

    #[test]
    fn parses_run_command_flags_and_passthrough_args() {
        let command = parse_run([
            "shuck",
            "run",
            "--shell",
            "bash",
            "--shell-version",
            "5.2",
            "--system",
            "--dry-run",
            "--verbose",
            "deploy.sh",
            "--",
            "--env",
            "staging",
        ]);

        assert_eq!(command.shell, Some(ManagedShellArg::Bash));
        assert_eq!(command.shell_version.as_deref(), Some("5.2"));
        assert!(command.system);
        assert!(command.dry_run);
        assert!(command.verbose);
        assert_eq!(
            command.script.as_deref(),
            Some(PathBuf::from("deploy.sh").as_path())
        );
        assert_eq!(
            command.script_args,
            vec![OsString::from("--env"), OsString::from("staging")]
        );
    }

    #[test]
    fn parses_run_command_string_mode() {
        let command = parse_run([
            "shuck", "run", "-s", "bash", "-c", "echo hi", "--", "one", "two",
        ]);

        assert_eq!(command.shell, Some(ManagedShellArg::Bash));
        assert_eq!(command.command.as_deref(), Some("echo hi"));
        assert!(command.script.is_none());
        assert_eq!(
            command.script_args,
            vec![OsString::from("one"), OsString::from("two")]
        );
    }

    #[test]
    fn parses_busybox_shell_variants() {
        let run = parse_run(["shuck", "run", "--shell", "busybox", "deploy.sh"]);
        assert_eq!(run.shell, Some(ManagedShellArg::Busybox));

        let install = parse_install(["shuck", "install", "busybox", "1.36"]);
        assert_eq!(install.shell, Some(ManagedShellArg::Busybox));

        let shell = parse_shell(["shuck", "shell", "--shell", "busybox"]);
        assert_eq!(shell.shell, Some(ManagedShellArg::Busybox));
    }

    #[test]
    fn parses_install_list_without_version() {
        let command = parse_install(["shuck", "install", "--list", "bash"]);
        assert!(command.list);
        assert_eq!(command.shell, Some(ManagedShellArg::Bash));
        assert!(command.version.is_none());
    }

    #[test]
    fn parses_shell_command_flags() {
        let command = parse_shell([
            "shuck",
            "shell",
            "--shell",
            "zsh",
            "--shell-version",
            "5.9",
            "--system",
            "--verbose",
        ]);

        assert_eq!(command.shell, Some(ManagedShellArg::Zsh));
        assert_eq!(command.shell_version.as_deref(), Some("5.9"));
        assert!(command.system);
        assert!(command.verbose);
    }

    #[test]
    fn parses_extended_managed_shell_names() {
        let run_command = parse_run(["shuck", "run", "--shell", "gbash", "-c", "echo hi"]);
        assert_eq!(run_command.shell, Some(ManagedShellArg::Gbash));

        let install_command = parse_install(["shuck", "install", "--list", "bashkit"]);
        assert_eq!(install_command.shell, Some(ManagedShellArg::Bashkit));
    }

    #[test]
    fn parses_rule_selection_flags() {
        let command = parse_check([
            "shuck",
            "check",
            "--select",
            "C001",
            "--select",
            "S,C002",
            "--ignore",
            "C003,C004",
            "--extend-select",
            "X",
            "--fixable",
            "ALL",
            "--unfixable",
            "C001",
            "--extend-fixable",
            "S074",
        ]);

        assert_eq!(
            command.rule_selection.select,
            Some(vec![
                RuleSelector::Rule(Rule::UnusedAssignment),
                RuleSelector::Category(shuck_linter::Category::Style),
                RuleSelector::Rule(Rule::DynamicSourcePath),
            ])
        );
        assert_eq!(
            command.rule_selection.ignore,
            vec![
                RuleSelector::Rule(Rule::UntrackedSourceFile),
                RuleSelector::Rule(Rule::UncheckedDirectoryChange),
            ]
        );
        assert_eq!(
            command.rule_selection.extend_select,
            vec![RuleSelector::Category(shuck_linter::Category::Portability)]
        );
        assert_eq!(
            command.rule_selection.fixable,
            Some(vec![RuleSelector::All])
        );
        assert_eq!(
            command.rule_selection.unfixable,
            vec![RuleSelector::Rule(Rule::UnusedAssignment)]
        );
        assert_eq!(
            command.rule_selection.extend_fixable,
            vec![RuleSelector::Rule(Rule::AmpersandSemicolon)]
        );
    }

    #[test]
    fn parses_named_rule_selection_flags() {
        let command = parse_check([
            "shuck",
            "check",
            "--select",
            "google",
            "--extend-select",
            "google",
            "--fixable",
            "google",
        ]);

        assert_eq!(
            command.rule_selection.select,
            Some(vec![RuleSelector::Named(shuck_linter::NamedGroup::Google)])
        );
        assert_eq!(
            command.rule_selection.extend_select,
            vec![RuleSelector::Named(shuck_linter::NamedGroup::Google)]
        );
        assert_eq!(
            command.rule_selection.fixable,
            Some(vec![RuleSelector::Named(shuck_linter::NamedGroup::Google)])
        );
    }

    #[test]
    fn parses_per_file_ignore_pairs() {
        let command = parse_check([
            "shuck",
            "check",
            "--per-file-ignores",
            "tests/*.sh:C001",
            "--extend-per-file-ignores",
            "!src/*.sh:S",
        ]);

        assert_eq!(
            command.rule_selection.per_file_ignores,
            Some(vec![PatternRuleSelectorPair {
                pattern: "tests/*.sh".to_owned(),
                selector: RuleSelector::Rule(Rule::UnusedAssignment),
            }])
        );
        assert_eq!(
            command.rule_selection.extend_per_file_ignores,
            vec![PatternRuleSelectorPair {
                pattern: "!src/*.sh".to_owned(),
                selector: RuleSelector::Category(shuck_linter::Category::Style),
            }]
        );
    }

    #[test]
    fn parses_named_per_file_ignore_pairs() {
        let command = parse_check(["shuck", "check", "--per-file-ignores", "tests/*.sh:google"]);

        assert_eq!(
            command.rule_selection.per_file_ignores,
            Some(vec![PatternRuleSelectorPair {
                pattern: "tests/*.sh".to_owned(),
                selector: RuleSelector::Named(shuck_linter::NamedGroup::Google),
            }])
        );
    }

    #[test]
    fn parses_per_file_ignore_pairs_with_colons_in_pattern() {
        let command = parse_check(["shuck", "check", "--per-file-ignores", r"C:\repo\*.sh:C001"]);

        assert_eq!(
            command.rule_selection.per_file_ignores,
            Some(vec![PatternRuleSelectorPair {
                pattern: r"C:\repo\*.sh".to_owned(),
                selector: RuleSelector::Rule(Rule::UnusedAssignment),
            }])
        );
    }

    #[test]
    fn parses_per_file_shell_pairs() {
        let command = parse_check([
            "shuck",
            "check",
            "--per-file-shell",
            "tests/*.sh:bash",
            "--extend-per-file-shell",
            "!src/*.sh:zsh",
        ]);

        assert_eq!(
            command.rule_selection.per_file_shell,
            Some(vec![PatternShellPair {
                pattern: "tests/*.sh".to_owned(),
                shell: shuck_linter::ShellDialect::Bash,
            }])
        );
        assert_eq!(
            command.rule_selection.extend_per_file_shell,
            vec![PatternShellPair {
                pattern: "!src/*.sh".to_owned(),
                shell: shuck_linter::ShellDialect::Zsh,
            }]
        );
    }

    #[test]
    fn rejects_empty_cli_rule_selectors() {
        let error = StableCli::try_parse_from(["shuck", "check", "--select", ""]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn rejects_empty_cli_rule_selectors_after_value_delimiter() {
        let error = StableCli::try_parse_from(["shuck", "check", "--select", "C001,"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn rejects_add_noqa_alias() {
        let error = StableCli::try_parse_from(["shuck", "check", "--add-noqa=legacy"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn rejects_add_ignore_with_fix_flags() {
        let error =
            StableCli::try_parse_from(["shuck", "check", "--add-ignore", "--fix"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn rejects_watch_with_add_ignore() {
        let error =
            StableCli::try_parse_from(["shuck", "check", "--watch", "--add-ignore"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn check_file_selection_negative_flags_override_positive_flags() {
        let args = Args::try_parse_from([
            "shuck",
            "check",
            "--respect-gitignore",
            "--no-respect-gitignore",
            "--force-exclude",
            "--no-force-exclude",
        ])
        .unwrap();

        let Command::Check(command) = args.command else {
            panic!("expected check command");
        };

        assert!(!command.respect_gitignore());
        assert!(!command.force_exclude());
    }

    #[test]
    fn check_file_selection_collects_exclude_and_extend_exclude_patterns() {
        let args = Args::try_parse_from([
            "shuck",
            "check",
            "--exclude",
            "base.sh",
            "--extend-exclude",
            "extra.sh",
        ])
        .unwrap();

        let Command::Check(command) = args.command else {
            panic!("expected check command");
        };

        assert_eq!(command.file_selection.exclude, vec!["base.sh"]);
        assert_eq!(command.file_selection.extend_exclude, vec!["extra.sh"]);
    }
}
