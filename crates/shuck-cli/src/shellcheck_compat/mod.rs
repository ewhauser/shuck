mod config;
mod optional;
mod render;

use std::collections::{BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, Rule, RuleSet, Severity, ShellCheckCodeMap, ShellCheckLevel, ShellDialect,
    rule_metadata,
};
use shuck_parser::parser::Parser;
use shuck_semantic::SourcePathResolver;
use shuck_semantic::SourceRefKind;

use self::config::{CompatConfig, load_config, resolve_config_override};
use self::optional::{
    OPTIONAL_CHECKS, OptionalCheck, OptionalCheckBehavior, compat_default_disabled_rules,
    find_optional_check, supported_optional_checks,
};
use self::render::{
    print_error_help, print_list_optional, print_report, print_version, usage_text,
};
use crate::shellcheck_runtime::{ShellCheckFix, ShellCheckReplacement};
use crate::stdin::read_from_stdin;

const COMPAT_ENV_VAR: &str = "SHUCK_SHELLCHECK_COMPAT";
const DEFAULT_WIKI_LINK_COUNT: usize = 3;
const EXTENDED_ANALYSIS_RULES: &[Rule] = &[
    Rule::UndefinedVariable,
    Rule::UntrackedSourceFile,
    Rule::OverwrittenFunction,
    Rule::FunctionCalledWithoutArgs,
    Rule::FunctionReferencesUnsetParam,
    Rule::UncheckedDirectoryChangeInFunction,
    Rule::LocalCrossReference,
    Rule::UnreachableAfterExit,
    Rule::UnusedHeredoc,
];

pub fn should_activate(argv: &[OsString]) -> bool {
    env_truthy(COMPAT_ENV_VAR)
        || argv
            .first()
            .and_then(shellcheck_basename)
            .is_some_and(|name| name == "shellcheck")
}

pub fn run(argv: Vec<OsString>) -> ExitCode {
    match run_inner(&argv) {
        Ok(status) => status,
        Err(err) => {
            let _ = print_error_help(&err.message, err.show_help);
            ExitCode::from(err.exit_code)
        }
    }
}

fn run_inner(argv: &[OsString]) -> Result<ExitCode, CompatCliError> {
    let cli = parse_args(argv)?;
    if cli.show_help {
        print!("{}", usage_text());
        return Ok(ExitCode::from(0));
    }
    if cli.show_version {
        print_version();
        return Ok(ExitCode::from(0));
    }
    if cli.list_optional {
        print_list_optional(OPTIONAL_CHECKS);
        return Ok(ExitCode::from(0));
    }

    let cwd = std::env::current_dir().map_err(|err| {
        CompatCliError::runtime(2, format!("could not read current directory: {err}"))
    })?;
    let config_path = resolve_config_override(&cwd, &cli.config_override)?;
    let config = match config_path.as_deref() {
        Some(path) => load_config(path)?,
        None => CompatConfig::default(),
    };
    let options = resolve_options(cli, config, &cwd)?;
    if options.files.is_empty() {
        return Err(CompatCliError::usage(3, "No files specified."));
    }

    let report = analyze_files(&options, &cwd)?;
    print_report(&report, &options)?;

    let mut stderr = io::stderr().lock();
    for error in &report.file_errors {
        let _ = writeln!(stderr, "{}: {}", error.path, error.message);
    }

    let exit = if !report.file_errors.is_empty() {
        2
    } else if report.diagnostics.is_empty() {
        0
    } else {
        1
    };
    Ok(ExitCode::from(exit))
}

#[derive(Debug, Clone)]
struct CompatCliError {
    exit_code: u8,
    message: String,
    show_help: bool,
}

impl CompatCliError {
    fn usage(exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            show_help: true,
        }
    }

    fn runtime(exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            show_help: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatLevel {
    Style,
    Info,
    Warning,
    Error,
}

impl CompatLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Style => "style",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    fn gcc_label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info | Self::Style => "note",
        }
    }

    const fn sort_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Style => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatColorMode {
    Auto,
    Always,
    Never,
}

impl FromStr for CompatColorMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatFormat {
    Checkstyle,
    Diff,
    Gcc,
    Json,
    Json1,
    Quiet,
    Tty,
}

impl FromStr for CompatFormat {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "checkstyle" => Ok(Self::Checkstyle),
            "diff" => Ok(Self::Diff),
            "gcc" => Ok(Self::Gcc),
            "json" => Ok(Self::Json),
            "json1" => Ok(Self::Json1),
            "quiet" => Ok(Self::Quiet),
            "tty" => Ok(Self::Tty),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompatSeverityThreshold {
    Style,
    Info,
    Warning,
    Error,
}

impl CompatSeverityThreshold {
    fn allows(self, level: CompatLevel) -> bool {
        let level = match level {
            CompatLevel::Style => Self::Style,
            CompatLevel::Info => Self::Info,
            CompatLevel::Warning => Self::Warning,
            CompatLevel::Error => Self::Error,
        };
        level >= self
    }
}

impl FromStr for CompatSeverityThreshold {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "style" => Ok(Self::Style),
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CliConfigOverride {
    Search,
    Ignore,
    Explicit(PathBuf),
}

#[derive(Debug, Clone)]
struct ParsedCompatCli {
    show_help: bool,
    show_version: bool,
    list_optional: bool,
    check_sourced: bool,
    color: Option<CompatColorMode>,
    include_codes: Vec<String>,
    exclude_codes: Vec<String>,
    extended_analysis: Option<bool>,
    format: Option<CompatFormat>,
    enable_checks: Vec<String>,
    source_paths: Vec<String>,
    shell: Option<String>,
    severity: Option<CompatSeverityThreshold>,
    wiki_link_count: Option<usize>,
    external_sources: bool,
    files: Vec<PathBuf>,
    config_override: CliConfigOverride,
}

impl Default for ParsedCompatCli {
    fn default() -> Self {
        Self {
            show_help: false,
            show_version: false,
            list_optional: false,
            check_sourced: false,
            color: None,
            include_codes: Vec::new(),
            exclude_codes: Vec::new(),
            extended_analysis: None,
            format: None,
            enable_checks: Vec::new(),
            source_paths: Vec::new(),
            shell: None,
            severity: None,
            wiki_link_count: None,
            external_sources: false,
            files: Vec::new(),
            config_override: CliConfigOverride::Search,
        }
    }
}

#[derive(Debug, Clone)]
struct CompatOptions {
    color: CompatColorMode,
    format: CompatFormat,
    severity: CompatSeverityThreshold,
    wiki_link_count: usize,
    shell: Option<String>,
    check_sourced: bool,
    external_sources: bool,
    source_paths: Vec<String>,
    files: Vec<PathBuf>,
    enabled_rules: RuleSet,
    report_environment_style_names: bool,
    shellcheck_map: ShellCheckCodeMap,
}

#[derive(Debug, Clone, Copy, Default)]
struct CompatOptionalState {
    report_environment_style_names: bool,
}

impl CompatOptionalState {
    fn enable(&mut self, check: &OptionalCheck) {
        if matches!(
            check.behavior,
            OptionalCheckBehavior::ReportEnvironmentStyleNames
        ) {
            self.report_environment_style_names = true;
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedCompatSelection {
    rules: RuleSet,
    optional_state: CompatOptionalState,
}

#[derive(Debug, Clone)]
struct FileAccessError {
    path: String,
    message: String,
}

#[derive(Debug, Clone)]
struct CompatDiagnostic {
    file: String,
    line: usize,
    end_line: usize,
    column: usize,
    end_column: usize,
    level: CompatLevel,
    code: u32,
    message: String,
    fix: Option<ShellCheckFix>,
    source: Option<Arc<str>>,
}

#[derive(Debug, Clone, Default)]
struct CompatReport {
    diagnostics: Vec<CompatDiagnostic>,
    file_errors: Vec<FileAccessError>,
}

fn parse_args(argv: &[OsString]) -> Result<ParsedCompatCli, CompatCliError> {
    let mut cli = ParsedCompatCli::default();
    let mut args = argv.iter().skip(1).peekable();
    let mut positional_only = false;

    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if positional_only {
            cli.files.push(PathBuf::from(arg));
            continue;
        }

        if text == "--" {
            positional_only = true;
            continue;
        }

        if !text.starts_with('-') || text == "-" {
            cli.files.push(PathBuf::from(arg));
            continue;
        }

        if let Some(rest) = text.strip_prefix("--") {
            parse_long_option(&mut cli, rest, &mut args)?;
            continue;
        }

        parse_short_option(&mut cli, &text[1..], &mut args)?;
    }

    Ok(cli)
}

fn parse_long_option<'a, I>(
    cli: &mut ParsedCompatCli,
    raw: &str,
    args: &mut std::iter::Peekable<I>,
) -> Result<(), CompatCliError>
where
    I: Iterator<Item = &'a OsString>,
{
    let (name, inline) = match raw.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (raw, None),
    };

    match name {
        "help" => cli.show_help = true,
        "version" => cli.show_version = true,
        "check-sourced" => cli.check_sourced = true,
        "list-optional" => cli.list_optional = true,
        "norc" => cli.config_override = CliConfigOverride::Ignore,
        "external-sources" => cli.external_sources = true,
        "color" => {
            let value = inline
                .or_else(|| optional_color_value(args))
                .unwrap_or_else(|| "always".to_owned());
            cli.color =
                Some(value.parse().map_err(|_| {
                    CompatCliError::usage(4, "color expects auto, always, or never")
                })?);
        }
        "include" => {
            cli.include_codes
                .extend(parse_code_list(&required_value(name, inline, args)?));
        }
        "exclude" => {
            cli.exclude_codes
                .extend(parse_code_list(&required_value(name, inline, args)?));
        }
        "extended-analysis" => {
            let value = required_value(name, inline, args)?;
            cli.extended_analysis = Some(parse_bool(&value).ok_or_else(|| {
                CompatCliError::usage(4, "extended-analysis expects a boolean value")
            })?);
        }
        "format" => {
            cli.format = Some(required_value(name, inline, args)?.parse().map_err(|_| {
                CompatCliError::usage(
                    4,
                    "format expects checkstyle, diff, gcc, json, json1, quiet, or tty",
                )
            })?);
        }
        "rcfile" => {
            let value = required_value(name, inline, args)?;
            cli.config_override = CliConfigOverride::Explicit(PathBuf::from(value));
        }
        "enable" => {
            cli.enable_checks
                .extend(parse_optional_check_list(&required_value(
                    name, inline, args,
                )?));
        }
        "source-path" => {
            cli.source_paths
                .extend(parse_source_path_list(&required_value(name, inline, args)?));
        }
        "shell" => {
            cli.shell = Some(required_value(name, inline, args)?);
        }
        "severity" => {
            cli.severity = Some(required_value(name, inline, args)?.parse().map_err(|_| {
                CompatCliError::usage(4, "severity expects error, warning, info, or style")
            })?);
        }
        "wiki-link-count" => {
            let value = required_value(name, inline, args)?;
            cli.wiki_link_count = Some(value.parse().map_err(|_| {
                CompatCliError::usage(4, "wiki-link-count expects a non-negative integer")
            })?);
        }
        _ => {
            return Err(CompatCliError::usage(
                3,
                format!("unrecognized option `--{name}`"),
            ));
        }
    }

    Ok(())
}

fn parse_short_option<'a, I>(
    cli: &mut ParsedCompatCli,
    raw: &str,
    args: &mut std::iter::Peekable<I>,
) -> Result<(), CompatCliError>
where
    I: Iterator<Item = &'a OsString>,
{
    for (index, flag) in raw.char_indices() {
        let rest = &raw[index + flag.len_utf8()..];
        match flag {
            'a' => cli.check_sourced = true,
            'x' => cli.external_sources = true,
            'V' => cli.show_version = true,
            'C' => {
                let value = if let Some(rest) = rest.strip_prefix('=') {
                    Some(rest.to_owned())
                } else if rest.is_empty() {
                    optional_color_value(args)
                } else {
                    Some(rest.to_owned())
                }
                .unwrap_or_else(|| "always".to_owned());
                cli.color = Some(value.parse().map_err(|_| {
                    CompatCliError::usage(4, "color expects auto, always, or never")
                })?);
                break;
            }
            'i' => {
                cli.include_codes
                    .extend(parse_code_list(&value_after_short(flag, rest, args)?));
                break;
            }
            'e' => {
                cli.exclude_codes
                    .extend(parse_code_list(&value_after_short(flag, rest, args)?));
                break;
            }
            'f' => {
                cli.format = Some(value_after_short(flag, rest, args)?.parse().map_err(|_| {
                    CompatCliError::usage(
                        4,
                        "format expects checkstyle, diff, gcc, json, json1, quiet, or tty",
                    )
                })?);
                break;
            }
            'o' => {
                cli.enable_checks
                    .extend(parse_optional_check_list(&value_after_short(
                        flag, rest, args,
                    )?));
                break;
            }
            'P' => {
                cli.source_paths
                    .extend(parse_source_path_list(&value_after_short(
                        flag, rest, args,
                    )?));
                break;
            }
            's' => {
                cli.shell = Some(value_after_short(flag, rest, args)?);
                break;
            }
            'S' => {
                cli.severity =
                    Some(value_after_short(flag, rest, args)?.parse().map_err(|_| {
                        CompatCliError::usage(4, "severity expects error, warning, info, or style")
                    })?);
                break;
            }
            'W' => {
                cli.wiki_link_count =
                    Some(value_after_short(flag, rest, args)?.parse().map_err(|_| {
                        CompatCliError::usage(4, "wiki-link-count expects a non-negative integer")
                    })?);
                break;
            }
            _ => {
                return Err(CompatCliError::usage(
                    3,
                    format!("unrecognized option `-{flag}`"),
                ));
            }
        }
    }

    Ok(())
}

fn required_value<'a, I>(
    name: &str,
    inline: Option<String>,
    args: &mut std::iter::Peekable<I>,
) -> Result<String, CompatCliError>
where
    I: Iterator<Item = &'a OsString>,
{
    if let Some(value) = inline {
        return Ok(value);
    }
    args.next()
        .and_then(|value| value.to_str().map(ToOwned::to_owned))
        .ok_or_else(|| CompatCliError::usage(4, format!("option `--{name}` expects a value")))
}

fn value_after_short<'a, I>(
    flag: char,
    rest: &str,
    args: &mut std::iter::Peekable<I>,
) -> Result<String, CompatCliError>
where
    I: Iterator<Item = &'a OsString>,
{
    if let Some(rest) = rest.strip_prefix('=') {
        return Ok(rest.to_owned());
    }
    if !rest.is_empty() {
        return Ok(rest.to_owned());
    }
    args.next()
        .and_then(|value| value.to_str().map(ToOwned::to_owned))
        .ok_or_else(|| CompatCliError::usage(4, format!("option `-{flag}` expects a value")))
}

fn optional_color_value<'a, I>(args: &mut std::iter::Peekable<I>) -> Option<String>
where
    I: Iterator<Item = &'a OsString>,
{
    let next = args
        .peek()
        .and_then(|value| value.to_str())
        .filter(|value| CompatColorMode::from_str(value).is_ok())
        .map(ToOwned::to_owned);
    if next.is_some() {
        let _ = args.next();
    }
    next
}

fn resolve_options(
    cli: ParsedCompatCli,
    config: CompatConfig,
    cwd: &Path,
) -> Result<CompatOptions, CompatCliError> {
    let shell = cli.shell.clone().or(config.shell.clone());
    if let Some(shell_name) = shell.as_deref() {
        validate_shell(shell_name)?;
    }

    let shellcheck_map = ShellCheckCodeMap::default();
    let selection = resolve_rule_selection(&shellcheck_map, &cli, &config)?;

    let files = cli
        .files
        .into_iter()
        .map(|path| {
            if is_stdin_path(&path) {
                path
            } else {
                absolutize(cwd, &path)
            }
        })
        .collect::<Vec<_>>();
    Ok(CompatOptions {
        color: cli.color.or(config.color).unwrap_or(CompatColorMode::Auto),
        format: cli.format.or(config.format).unwrap_or(CompatFormat::Tty),
        severity: cli
            .severity
            .or(config.severity)
            .unwrap_or(CompatSeverityThreshold::Style),
        wiki_link_count: cli
            .wiki_link_count
            .or(config.wiki_link_count)
            .unwrap_or(DEFAULT_WIKI_LINK_COUNT),
        shell,
        check_sourced: cli.check_sourced || config.check_sourced.unwrap_or(false),
        external_sources: cli.external_sources || config.external_sources.unwrap_or(false),
        source_paths: {
            let mut values = config.source_paths;
            values.extend(cli.source_paths);
            values
        },
        files,
        enabled_rules: apply_extended_analysis(
            selection.rules,
            cli.extended_analysis
                .or(config.extended_analysis)
                .unwrap_or(true),
        ),
        report_environment_style_names: selection.optional_state.report_environment_style_names,
        shellcheck_map,
    })
}

fn resolve_rule_selection(
    shellcheck_map: &ShellCheckCodeMap,
    cli: &ParsedCompatCli,
    config: &CompatConfig,
) -> Result<ResolvedCompatSelection, CompatCliError> {
    let mut rules = compat_default_rules(shellcheck_map);
    let mut optional_state = CompatOptionalState::default();

    let mut requested_optional = config.enable_checks.clone();
    requested_optional.extend(cli.enable_checks.clone());
    if !requested_optional.is_empty() {
        for name in requested_optional {
            if name == "all" {
                for check in supported_optional_checks() {
                    rules = rules.union(&check.enabled_rule_set());
                    optional_state.enable(check);
                }
                continue;
            }

            let Some(check) = find_optional_check(&name) else {
                return Err(CompatCliError::usage(
                    4,
                    format!("unknown optional check `{name}`"),
                ));
            };
            if !check.supported {
                continue;
            }

            rules = rules.union(&check.enabled_rule_set());
            optional_state.enable(check);
        }
    }

    let include_codes = combined_codes(&config.include_codes, &cli.include_codes);
    let exclude_codes = combined_codes(&config.exclude_codes, &cli.exclude_codes);
    if !include_codes.is_empty() {
        rules = rules_for_codes(shellcheck_map, &include_codes)?;
    }
    if !exclude_codes.is_empty() {
        let excluded = rules_for_codes(shellcheck_map, &exclude_codes)?;
        rules = rules.subtract(&excluded);
    }

    Ok(ResolvedCompatSelection {
        rules,
        optional_state,
    })
}

fn compat_default_rules(shellcheck_map: &ShellCheckCodeMap) -> RuleSet {
    shellcheck_map
        .mappings()
        .map(|(_, rule)| rule)
        .collect::<RuleSet>()
        .subtract(&compat_default_disabled_rules())
}

fn combined_codes(config_codes: &[String], cli_codes: &[String]) -> Vec<String> {
    config_codes
        .iter()
        .chain(cli_codes)
        .cloned()
        .collect::<Vec<_>>()
}

fn rules_for_codes(
    shellcheck_map: &ShellCheckCodeMap,
    codes: &[String],
) -> Result<RuleSet, CompatCliError> {
    let mut rules = HashSet::new();
    for code in codes {
        let resolved = shellcheck_map.resolve_all(code);
        if resolved.is_empty() {
            if !is_valid_shellcheck_code(code) {
                return Err(CompatCliError::usage(
                    4,
                    format!(
                        "shellcheck code `{code}` is not implemented by this compatibility mode"
                    ),
                ));
            }
            continue;
        }
        rules.extend(resolved);
    }
    Ok(rules.into_iter().collect())
}

fn is_valid_shellcheck_code(code: &str) -> bool {
    code.strip_prefix("SC")
        .or_else(|| code.strip_prefix("sc"))
        .unwrap_or(code)
        .parse::<u32>()
        .is_ok()
}

fn apply_extended_analysis(mut rules: RuleSet, enabled: bool) -> RuleSet {
    if enabled {
        return rules;
    }

    let extended_rules = EXTENDED_ANALYSIS_RULES.iter().copied().collect::<RuleSet>();
    rules = rules.subtract(&extended_rules);
    rules
}

fn analyze_files(options: &CompatOptions, cwd: &Path) -> Result<CompatReport, CompatCliError> {
    let explicit_files = options
        .files
        .iter()
        .map(|path| canonicalize_or_clone(path))
        .collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    let mut diagnostics = Vec::new();
    let mut file_errors = Vec::new();

    for path in &options.files {
        analyze_one(
            path,
            cwd,
            options,
            &explicit_files,
            &mut visited,
            &mut diagnostics,
            &mut file_errors,
        )?;
    }

    diagnostics.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
            .then(left.end_line.cmp(&right.end_line))
            .then(left.end_column.cmp(&right.end_column))
            .then(left.level.sort_rank().cmp(&right.level.sort_rank()))
    });

    Ok(CompatReport {
        diagnostics,
        file_errors,
    })
}

#[allow(clippy::too_many_arguments)]
fn analyze_one(
    path: &Path,
    cwd: &Path,
    options: &CompatOptions,
    explicit_files: &BTreeSet<PathBuf>,
    visited: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<CompatDiagnostic>,
    file_errors: &mut Vec<FileAccessError>,
) -> Result<(), CompatCliError> {
    let canonical = canonicalize_or_clone(path);
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }

    let source =
        if is_stdin_path(path) {
            Arc::<str>::from(read_from_stdin().map_err(|err| {
                CompatCliError::runtime(2, format!("could not read stdin: {err}"))
            })?)
        } else {
            match fs::read_to_string(path) {
                Ok(source) => Arc::<str>::from(source),
                Err(err) => {
                    file_errors.push(FileAccessError {
                        path: display_path(path, cwd),
                        message: format!("could not read file: {err}"),
                    });
                    return Ok(());
                }
            }
        };

    let (initial, initial_resolved_paths) = lint_with_context(
        path,
        &source,
        options,
        explicit_files.iter().cloned().collect(),
    )?;
    let final_explicit = if options.external_sources {
        explicit_files
            .iter()
            .cloned()
            .chain(initial_resolved_paths.iter().cloned())
            .collect::<BTreeSet<_>>()
    } else {
        explicit_files.clone()
    };

    let (analysis, resolved_paths) =
        if final_explicit != explicit_files.iter().cloned().collect::<BTreeSet<_>>() {
            lint_with_context(path, &source, options, final_explicit.clone())?
        } else {
            (initial, initial_resolved_paths)
        };

    diagnostics.extend(analysis.into_iter().filter_map(|diagnostic| {
        map_diagnostic(
            path,
            cwd,
            diagnostic,
            source.clone(),
            &options.shellcheck_map,
            options.severity,
        )
    }));

    if options.check_sourced {
        for sourced in resolved_paths {
            analyze_one(
                &sourced,
                cwd,
                options,
                &final_explicit,
                visited,
                diagnostics,
                file_errors,
            )?;
        }
    }

    Ok(())
}

fn lint_with_context(
    path: &Path,
    source: &Arc<str>,
    options: &CompatOptions,
    explicit_paths: BTreeSet<PathBuf>,
) -> Result<(Vec<shuck_linter::Diagnostic>, BTreeSet<PathBuf>), CompatCliError> {
    let shell = options
        .shell
        .as_deref()
        .map(parse_shell_override)
        .transpose()?
        .unwrap_or_else(|| ShellDialect::infer(source, Some(path)));

    let parse_result = Parser::with_profile(source, shell.shell_profile()).parse();
    let indexer = Indexer::new(source, &parse_result);
    let explicit = explicit_paths.into_iter().collect::<Vec<_>>();
    let resolver = CompatSourceResolver {
        cwd: canonicalize_or_clone(&std::env::current_dir().map_err(|err| {
            CompatCliError::runtime(2, format!("could not read current directory: {err}"))
        })?),
        source_paths: options.source_paths.clone(),
    };
    let mut rule_options = shuck_linter::LinterRuleOptions::default();
    rule_options.c063.report_unreached_nested_definitions = true;
    let settings = LinterSettings {
        rules: options.enabled_rules,
        severity_overrides: Default::default(),
        shell,
        ambient_shell_options: Default::default(),
        analyzed_paths: Some(Arc::new(explicit.into_iter().collect())),
        per_file_ignores: Default::default(),
        report_environment_style_names: options.report_environment_style_names,
        resolve_source_closure: options.external_sources,
        rule_options,
    };

    let diagnostics =
        shuck_linter::lint_file_at_path_with_resolver_and_parse_result_with_comment_directives(
            &parse_result,
            source,
            &indexer,
            &settings,
            &options.shellcheck_map,
            Some(path),
            Some(&resolver),
        );
    let analysis = shuck_linter::analyze_file_at_path_with_resolver(
        &parse_result.file,
        source,
        &indexer,
        &settings,
        None,
        Some(path),
        Some(&resolver),
    );

    let resolved_paths = analysis
        .semantic
        .source_refs()
        .iter()
        .flat_map(|source_ref| resolve_source_ref_paths(path, source_ref, &resolver))
        .collect::<BTreeSet<_>>();

    Ok((diagnostics, resolved_paths))
}

fn resolve_source_ref_paths(
    source_path: &Path,
    source_ref: &shuck_semantic::SourceRef,
    resolver: &CompatSourceResolver,
) -> Vec<PathBuf> {
    match &source_ref.kind {
        SourceRefKind::DirectiveDevNull | SourceRefKind::Dynamic => Vec::new(),
        SourceRefKind::Literal(candidate) | SourceRefKind::Directive(candidate) => {
            resolve_candidate_paths(source_path, candidate, resolver)
        }
        SourceRefKind::SingleVariableStaticTail { .. } => Vec::new(),
    }
}

fn resolve_candidate_paths(
    source_path: &Path,
    candidate: &str,
    resolver: &CompatSourceResolver,
) -> Vec<PathBuf> {
    let candidate_path = PathBuf::from(candidate);
    if candidate_path.is_absolute() {
        return candidate_path
            .is_file()
            .then_some(candidate_path)
            .into_iter()
            .collect();
    }

    let mut resolved = Vec::new();
    if let Some(base_dir) = source_path.parent() {
        let direct = base_dir.join(&candidate_path);
        if direct.is_file() {
            resolved.push(direct);
        }
    }
    resolved.extend(resolver.resolve_candidate_paths(source_path, candidate));
    resolved
}

struct CompatSourceResolver {
    cwd: PathBuf,
    source_paths: Vec<String>,
}

impl shuck_semantic::SourcePathResolver for CompatSourceResolver {
    fn resolve_candidate_paths(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        let mut resolved = Vec::new();
        for root in &self.source_paths {
            let root_path = if root == "SCRIPTDIR" {
                source_path.parent().unwrap_or(Path::new("")).to_path_buf()
            } else {
                let root_path = PathBuf::from(root);
                if root_path.is_absolute() {
                    root_path
                } else {
                    self.cwd.join(root_path)
                }
            };
            let candidate_path = root_path.join(candidate);
            if candidate_path.is_file() {
                resolved.push(candidate_path);
            }
        }
        resolved
    }
}

fn map_diagnostic(
    path: &Path,
    cwd: &Path,
    diagnostic: shuck_linter::Diagnostic,
    source: Arc<str>,
    shellcheck_map: &ShellCheckCodeMap,
    threshold: CompatSeverityThreshold,
) -> Option<CompatDiagnostic> {
    let code = shellcheck_map.code_for_rule(diagnostic.rule)?;
    let level = level_for_diagnostic(diagnostic.rule, diagnostic.severity, code);
    let fix = compat_fix_for_diagnostic(
        code,
        diagnostic.span.start.line,
        diagnostic.span.start.column,
        diagnostic.span.end.line,
        diagnostic.span.end.column,
    );
    threshold.allows(level).then(|| CompatDiagnostic {
        file: display_path(path, cwd),
        line: diagnostic.span.start.line,
        end_line: diagnostic.span.end.line,
        column: diagnostic.span.start.column,
        end_column: diagnostic.span.end.column,
        level,
        code,
        message: diagnostic.message,
        fix,
        source: Some(source),
    })
}

fn compat_fix_for_diagnostic(
    code: u32,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> Option<ShellCheckFix> {
    (code == 2086).then(|| ShellCheckFix {
        replacements: vec![
            ShellCheckReplacement {
                line: start_line,
                end_line: start_line,
                column: start_column,
                end_column: start_column,
                precedence: 7,
                insertion_point: "afterEnd".to_owned(),
                replacement: "\"".to_owned(),
            },
            ShellCheckReplacement {
                line: end_line,
                end_line,
                column: end_column,
                end_column,
                precedence: 7,
                insertion_point: "beforeStart".to_owned(),
                replacement: "\"".to_owned(),
            },
        ],
    })
}

fn level_for_diagnostic(rule: Rule, severity: Severity, shellcheck_code: u32) -> CompatLevel {
    if let Some(level) = rule_metadata(rule).and_then(|metadata| metadata.shellcheck_level) {
        return compat_level(level);
    }

    debug_assert!(
        false,
        "missing shellcheck_level metadata for mapped ShellCheck-compatible rule {}",
        rule.code()
    );
    legacy_level_for_diagnostic(rule, severity, shellcheck_code)
}

fn compat_level(level: ShellCheckLevel) -> CompatLevel {
    match level {
        ShellCheckLevel::Style => CompatLevel::Style,
        ShellCheckLevel::Info => CompatLevel::Info,
        ShellCheckLevel::Warning => CompatLevel::Warning,
        ShellCheckLevel::Error => CompatLevel::Error,
    }
}

fn legacy_level_for_diagnostic(
    rule: Rule,
    severity: Severity,
    shellcheck_code: u32,
) -> CompatLevel {
    match severity {
        Severity::Hint => CompatLevel::Style,
        Severity::Error | Severity::Warning => {
            if shellcheck_code < 2000 {
                CompatLevel::Error
            } else if matches!(rule.category(), shuck_linter::Category::Style) {
                CompatLevel::Info
            } else {
                CompatLevel::Warning
            }
        }
    }
}

fn parse_shell_override(value: &str) -> Result<ShellDialect, CompatCliError> {
    validate_shell(value)?;
    Ok(match value {
        "sh" | "dash" | "busybox" => ShellDialect::Sh,
        "bash" => ShellDialect::Bash,
        "ksh" => ShellDialect::Ksh,
        _ => ShellDialect::Sh,
    })
}

fn validate_shell(value: &str) -> Result<(), CompatCliError> {
    match value {
        "sh" | "bash" | "dash" | "ksh" | "busybox" => Ok(()),
        _ => Err(CompatCliError::usage(
            4,
            format!("unsupported shell `{value}`"),
        )),
    }
}

pub(super) fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(super) fn parse_code_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn parse_optional_check_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn parse_source_path_list(value: &str) -> Vec<String> {
    std::env::split_paths(OsStr::new(value))
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn display_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn canonicalize_or_clone(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_stdin_path(path: &Path) -> bool {
    path == Path::new("-")
}

fn absolutize(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn shellcheck_basename(value: &OsString) -> Option<String> {
    Path::new(value)
        .file_stem()
        .or_else(|| Path::new(value).file_name())
        .map(|value| value.to_string_lossy().into_owned())
}

fn env_truthy(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|value| {
        !matches!(
            value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

pub(super) fn use_color(mode: CompatColorMode) -> bool {
    match mode {
        CompatColorMode::Always => true,
        CompatColorMode::Never => false,
        CompatColorMode::Auto => io::stdout().is_terminal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_activation_treats_truthy_values_as_enabled() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(!parse_bool("off").unwrap());
    }

    #[test]
    fn parser_accepts_short_and_long_options() {
        let args = vec![
            OsString::from("shellcheck"),
            OsString::from("-a"),
            OsString::from("-C"),
            OsString::from("always"),
            OsString::from("--severity=warning"),
            OsString::from("script.sh"),
        ];

        let cli = parse_args(&args).unwrap();
        assert!(cli.check_sourced);
        assert_eq!(cli.color, Some(CompatColorMode::Always));
        assert_eq!(cli.severity, Some(CompatSeverityThreshold::Warning));
        assert_eq!(cli.files, vec![PathBuf::from("script.sh")]);
    }

    #[test]
    fn parser_rejects_unknown_options() {
        let args = vec![OsString::from("shellcheck"), OsString::from("--wat")];
        let err = parse_args(&args).unwrap_err();
        assert_eq!(err.exit_code, 3);
        assert!(err.message.contains("unrecognized option"));
    }

    #[test]
    fn compat_mode_reports_declaration_only_c001_targets() {
        let tempdir = tempfile::tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        std::fs::write(&script, "#!/bin/bash\nf(){\n  local cur\n}\nf\n").unwrap();
        let options = CompatOptions {
            color: CompatColorMode::Never,
            format: CompatFormat::Json1,
            severity: CompatSeverityThreshold::Style,
            wiki_link_count: 0,
            shell: Some("bash".to_owned()),
            check_sourced: false,
            external_sources: false,
            source_paths: Vec::new(),
            files: vec![script],
            enabled_rules: RuleSet::from_iter([Rule::UnusedAssignment]),
            report_environment_style_names: false,
            shellcheck_map: ShellCheckCodeMap::default(),
        };

        let report = analyze_files(&options, tempdir.path()).unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, 2034);
        assert_eq!(report.diagnostics[0].line, 3);
        assert_eq!(report.diagnostics[0].column, 9);
    }
}
