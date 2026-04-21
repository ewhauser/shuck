use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::{ffi::OsStr, io::IsTerminal};

use anyhow::{Result, anyhow};
use notify::{RecursiveMode, Watcher, recommended_watcher};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_indexer::Indexer;
use shuck_linter::{
    Applicability, LinterSettings, ShellCheckCodeMap, ShellDialect, SuppressionIndex,
    add_ignores_to_path, first_statement_line, parse_directives,
};
use shuck_parser::{
    Error as ParseError,
    parser::{ParseResult, Parser},
};

use crate::ExitStatus;
use crate::args::{CheckCommand, FileSelectionArgs};
use crate::cache::resolve_cache_root;
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind, print_report_to,
};
use crate::commands::project_runner::{PendingProjectFile, prepare_project_runs};
use crate::config::{
    ConfigArguments, discovered_config_path_for_root, resolve_project_root_for_input,
};
use crate::discover::{DEFAULT_IGNORED_DIR_NAMES, DiscoveryOptions, normalize_path};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
    diagnostics: Vec<DisplayedDiagnostic>,
    cache_hits: usize,
    cache_misses: usize,
    fixes_applied: usize,
}

impl CheckReport {
    fn exit_status(&self, exit_zero: bool, exit_non_zero_on_fix: bool) -> ExitStatus {
        if exit_non_zero_on_fix && self.fixes_applied > 0 {
            return ExitStatus::Failure;
        }
        diagnostics_exit_status(&self.diagnostics, exit_zero)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct AddIgnoreReport {
    diagnostics: Vec<DisplayedDiagnostic>,
    directives_added: usize,
}

impl AddIgnoreReport {
    fn exit_status(&self, exit_zero: bool) -> ExitStatus {
        diagnostics_exit_status(&self.diagnostics, exit_zero)
    }
}

fn diagnostics_exit_status(diagnostics: &[DisplayedDiagnostic], exit_zero: bool) -> ExitStatus {
    let has_fatal = diagnostics.iter().any(|d| match &d.kind {
        DisplayedDiagnosticKind::ParseError => true,
        DisplayedDiagnosticKind::Lint { severity, .. } => severity == "error",
    });
    if has_fatal {
        return ExitStatus::Failure;
    }
    if diagnostics.is_empty() || exit_zero {
        ExitStatus::Success
    } else {
        ExitStatus::Failure
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveCheckSettings {
    enabled_rules: Vec<String>,
}

impl Default for EffectiveCheckSettings {
    fn default() -> Self {
        let mut enabled_rules = LinterSettings::default()
            .rules
            .iter()
            .map(|rule| rule.code().to_owned())
            .collect::<Vec<_>>();
        enabled_rules.sort();

        Self { enabled_rules }
    }
}

impl CacheKey for EffectiveCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-check-settings");
        self.enabled_rules.cache_key(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum CheckCacheData {
    Success(Vec<CachedLintDiagnostic>),
    ParseError(ParseCacheFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ParseCacheFailure {
    message: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintDiagnostic {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    code: String,
    severity: String,
    message: String,
}

impl CachedLintDiagnostic {
    fn from_diagnostic(diagnostic: &shuck_linter::Diagnostic) -> Self {
        Self {
            start_line: diagnostic.span.start.line,
            start_column: diagnostic.span.start.column,
            end_line: diagnostic.span.end.line,
            end_column: diagnostic.span.end.column,
            code: diagnostic.code().to_owned(),
            severity: diagnostic.severity.as_str().to_owned(),
            message: diagnostic.message.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct FileCheckResult {
    file: crate::discover::DiscoveredFile,
    file_key: shuck_cache::FileCacheKey,
    cache_data: CheckCacheData,
    diagnostics: Vec<DisplayedDiagnostic>,
    fixes_applied: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchTarget {
    watch_path: PathBuf,
    watch_paths: Vec<PathBuf>,
    recursive: bool,
    match_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct WatchPath {
    resolved_path: PathBuf,
    canonical_path: PathBuf,
}

impl WatchTarget {
    fn recursive(path: PathBuf) -> Self {
        Self {
            watch_path: path.clone(),
            watch_paths: vec![path.clone()],
            recursive: true,
            match_paths: vec![path],
        }
    }

    fn file(path: PathBuf) -> Self {
        let watch_path = path.parent().unwrap_or(&path).to_path_buf();
        Self {
            watch_path: watch_path.clone(),
            watch_paths: vec![watch_path],
            recursive: false,
            match_paths: vec![path],
        }
    }

    fn recursive_mode(&self) -> RecursiveMode {
        if self.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        }
    }

    fn matches_event_path(&self, path: &Path) -> bool {
        if self.recursive {
            self.match_paths
                .iter()
                .any(|match_path| path.starts_with(match_path))
        } else {
            self.match_paths.iter().any(|match_path| match_path == path)
        }
    }

    fn add_match_path(&mut self, path: PathBuf) {
        self.match_paths.push(path);
        self.match_paths.sort();
        self.match_paths.dedup();
    }

    fn add_watch_path(&mut self, path: PathBuf) {
        self.watch_paths.push(path);
        self.watch_paths.sort();
        self.watch_paths.dedup();
    }

    fn merge(&mut self, other: WatchTarget) {
        debug_assert_eq!(self.watch_path, other.watch_path);
        debug_assert_eq!(self.recursive, other.recursive);

        self.watch_paths.extend(other.watch_paths);
        self.watch_paths.sort();
        self.watch_paths.dedup();
        self.match_paths.extend(other.match_paths);
        self.match_paths.sort();
        self.match_paths.dedup();
    }

    fn covers(&self, other: &WatchTarget) -> bool {
        if !self.recursive {
            return false;
        }

        other
            .match_paths
            .iter()
            .all(|path| self.matches_event_path(path))
    }
}

pub(crate) fn check(
    args: CheckCommand,
    config_arguments: &ConfigArguments,
    cache_dir: Option<&Path>,
) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let cache_root = resolve_cache_root(&cwd, cache_dir)?;
    if args.watch {
        return watch_check(&args, config_arguments, &cwd, &cache_root);
    }

    if let Some(raw_reason) = args.add_ignore.as_deref() {
        if raw_reason.contains(['\n', '\r']) {
            return Err(anyhow!(
                "--add-ignore <reason> cannot contain newline characters"
            ));
        }

        let report = run_add_ignore_with_cwd(
            &args,
            config_arguments,
            &cwd,
            &cache_root,
            (!raw_reason.is_empty()).then_some(raw_reason),
        )?;
        if report.directives_added > 0 {
            let s = if report.directives_added == 1 {
                ""
            } else {
                "s"
            };
            eprintln!(
                "Added {} shuck ignore directive{s}.",
                report.directives_added
            );
        }
        print_diagnostics(&report.diagnostics, args.output_format)?;
        return Ok(report.exit_status(args.exit_zero));
    }

    let report = run_check_with_cwd(&args, config_arguments, &cwd, &cache_root)?;
    print_report(&report, args.output_format)?;
    Ok(report.exit_status(args.exit_zero, args.exit_non_zero_on_fix))
}

fn watch_check(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
) -> Result<ExitStatus> {
    let watch_targets = collect_watch_targets(&args.paths, config_arguments, cwd)?;
    let (tx, rx) = channel();
    let mut watcher = recommended_watcher(tx)?;
    for target in &watch_targets {
        for watch_path in &target.watch_paths {
            watcher.watch(watch_path, target.recursive_mode())?;
        }
    }

    clear_screen()?;
    print_watch_banner("Starting linter in watch mode...")?;
    let report = run_check_with_cwd(args, config_arguments, cwd, cache_root)?;
    print_report(&report, args.output_format)?;

    loop {
        wait_for_watch_rerun(&rx, cache_root, &watch_targets)?;

        clear_screen()?;
        print_watch_banner("File change detected...")?;
        let report = run_check_with_cwd(args, config_arguments, cwd, cache_root)?;
        print_report(&report, args.output_format)?;
    }
}

fn print_report(
    report: &CheckReport,
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<()> {
    print_diagnostics(&report.diagnostics, output_format)
}

fn print_diagnostics(
    diagnostics: &[DisplayedDiagnostic],
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<()> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    print_report_to(
        &mut stdout,
        diagnostics,
        output_format,
        colored::control::SHOULD_COLORIZE.should_colorize(),
    )?;
    Ok(())
}

fn should_clear_screen(stdout_is_terminal: bool) -> bool {
    stdout_is_terminal
}

fn clear_screen() -> Result<()> {
    if !should_clear_screen(io::stdout().is_terminal()) {
        return Ok(());
    }
    clearscreen::clear()?;
    Ok(())
}

fn print_watch_banner(message: &str) -> Result<()> {
    let mut stderr = BufWriter::new(io::stderr().lock());
    writeln!(stderr, "{message}")?;
    stderr.flush()?;
    Ok(())
}

fn effective_check_inputs(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    }
}

fn collect_watch_targets(
    paths: &[PathBuf],
    config_arguments: &ConfigArguments,
    cwd: &Path,
) -> Result<Vec<WatchTarget>> {
    let inputs = effective_check_inputs(paths);
    let mut targets = Vec::new();
    for input in inputs {
        let resolved_input = if input.is_absolute() {
            normalize_path(&input)
        } else {
            normalize_path(&cwd.join(&input))
        };
        let metadata = fs::metadata(&resolved_input)?;
        let canonical_input = fs::canonicalize(&resolved_input).map_err(anyhow::Error::from)?;

        let mut target = if metadata.is_dir() {
            WatchTarget::recursive(resolved_input.clone())
        } else {
            WatchTarget::file(resolved_input.clone())
        };
        if metadata.is_dir() {
            target.add_watch_path(canonical_input.clone());
        } else if let Some(parent) = canonical_input.parent() {
            target.add_watch_path(parent.to_path_buf());
        }
        target.add_match_path(canonical_input);
        targets.push(target);

        if let Some(config_path) = watch_config_target(config_arguments, cwd, &resolved_input)? {
            let canonical_config_parent =
                config_path.canonical_path.parent().map(Path::to_path_buf);
            let mut target = WatchTarget::file(config_path.resolved_path);
            target.add_match_path(config_path.canonical_path);
            if let Some(parent) = canonical_config_parent {
                target.add_watch_path(parent.to_path_buf());
            }
            targets.push(target);
        }
    }

    targets.sort_by(|left, right| {
        left.watch_path
            .components()
            .count()
            .cmp(&right.watch_path.components().count())
            .then_with(|| right.recursive.cmp(&left.recursive))
            .then_with(|| left.watch_path.cmp(&right.watch_path))
    });

    let mut deduped = Vec::new();
    for target in targets {
        if let Some(existing) = deduped.iter_mut().find(|existing: &&mut WatchTarget| {
            existing.watch_path == target.watch_path && existing.recursive == target.recursive
        }) {
            existing.merge(target);
            continue;
        }

        if deduped
            .iter()
            .any(|existing: &WatchTarget| existing.covers(&target))
        {
            continue;
        }

        if target.recursive {
            deduped.retain(|existing| !target.covers(existing));
        }

        deduped.push(target);
    }

    Ok(deduped)
}

fn watch_config_target(
    config_arguments: &ConfigArguments,
    cwd: &Path,
    resolved_input: &Path,
) -> Result<Option<WatchPath>> {
    if let Some(explicit_config) = config_arguments.explicit_config_file() {
        let resolved_config = if explicit_config.is_absolute() {
            normalize_path(explicit_config)
        } else {
            normalize_path(&cwd.join(explicit_config))
        };

        return Ok(Some(WatchPath {
            canonical_path: fs::canonicalize(&resolved_config).map_err(anyhow::Error::from)?,
            resolved_path: resolved_config,
        }));
    }

    if !config_arguments.use_config_roots() {
        return Ok(None);
    }

    let project_root = resolve_project_root_for_input(resolved_input, true)?;
    let Some(config_path) = discovered_config_path_for_root(&project_root)? else {
        return Ok(None);
    };

    let resolved_path = normalize_path(&config_path);
    Ok(Some(WatchPath {
        canonical_path: fs::canonicalize(&resolved_path).map_err(anyhow::Error::from)?,
        resolved_path,
    }))
}

fn wait_for_watch_rerun(
    rx: &Receiver<notify::Result<notify::Event>>,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> Result<()> {
    loop {
        let event = match rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => return Err(error.into()),
            Err(error) => return Err(error.into()),
        };

        if drain_watch_batch(event, rx, cache_root, watch_targets)? {
            return Ok(());
        }
    }
}

fn drain_watch_batch(
    first_event: notify::Event,
    rx: &Receiver<notify::Result<notify::Event>>,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> Result<bool> {
    let mut should_rerun = watch_event_requires_rerun(&first_event, cache_root, watch_targets);

    loop {
        match rx.try_recv() {
            Ok(Ok(event)) => {
                should_rerun |= watch_event_requires_rerun(&event, cache_root, watch_targets);
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(TryRecvError::Empty) => return Ok(should_rerun),
            Err(TryRecvError::Disconnected) => {
                return Err(anyhow!("watch channel disconnected"));
            }
        }
    }
}

fn watch_event_requires_rerun(
    event: &notify::Event,
    cache_root: &Path,
    watch_targets: &[WatchTarget],
) -> bool {
    if event.kind.is_access() || event.kind.is_other() {
        return false;
    }

    if event.need_rescan() {
        return true;
    }

    event
        .paths
        .iter()
        .map(|path| normalize_path(path))
        .filter(|path| !watch_event_path_is_ignored(path, cache_root))
        .any(|path| {
            watch_targets
                .iter()
                .any(|target| target.matches_event_path(&path))
        })
}

fn watch_event_path_is_ignored(path: &Path, cache_root: &Path) -> bool {
    path.starts_with(cache_root)
        || path.components().any(|component| {
            let std::path::Component::Normal(part) = component else {
                return false;
            };
            DEFAULT_IGNORED_DIR_NAMES
                .iter()
                .any(|name| part == OsStr::new(name))
        })
}

fn run_check_with_cwd(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
) -> Result<CheckReport> {
    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let fix_applicability = requested_fix_applicability(args);
    let settings = EffectiveCheckSettings::default();
    let runs = prepare_project_runs::<CheckCacheData, EffectiveCheckSettings, _>(
        &args.paths,
        cwd,
        &DiscoveryOptions {
            exclude_patterns: args.file_selection.exclude.clone(),
            extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
            respect_gitignore: args.respect_gitignore(),
            force_exclude: args.force_exclude(),
            parallel: true,
            cache_root: Some(cache_root.to_path_buf()),
            use_config_roots: config_arguments.use_config_roots(),
        },
        cache_root,
        args.no_cache || fix_applicability.is_some(),
        b"project-cache-key",
        |_| Ok(settings.clone()),
    )?;
    let base_linter_settings = LinterSettings::default();
    let shellcheck_map = ShellCheckCodeMap::default();

    let mut report = CheckReport::default();

    for mut run in runs {
        let analyzed_paths = run
            .files
            .iter()
            .map(|file| file.absolute_path.clone())
            .collect::<Vec<_>>();
        let pending = run.take_pending_files(|file, cached| {
            report.cache_hits += 1;
            match cached {
                CheckCacheData::Success(diagnostics) => {
                    let source = (include_source && !diagnostics.is_empty())
                        .then(|| read_shared_source(&file.absolute_path))
                        .transpose()?;
                    push_cached_lint_diagnostics(
                        &mut report,
                        &file.display_path,
                        &diagnostics,
                        source,
                    );
                }
                CheckCacheData::ParseError(error) => {
                    let source = include_source
                        .then(|| read_shared_source(&file.absolute_path))
                        .transpose()?;
                    report.diagnostics.push(DisplayedDiagnostic {
                        path: file.display_path,
                        span: DisplaySpan::point(error.line, error.column),
                        message: error.message,
                        kind: DisplayedDiagnosticKind::ParseError,
                        source,
                    });
                }
            }
            Ok(())
        })?;

        let results = pending
            .into_par_iter()
            .map(|pending| {
                analyze_file(
                    pending,
                    &base_linter_settings
                        .clone()
                        .with_analyzed_paths(analyzed_paths.clone()),
                    &shellcheck_map,
                    include_source,
                    fix_applicability,
                )
            })
            .collect::<Vec<_>>();

        for result in results {
            let result = result?;
            report.fixes_applied += result.fixes_applied;
            report.diagnostics.extend(result.diagnostics);
            if let Some(cache) = run.cache.as_mut() {
                cache.insert(
                    result.file.relative_path.clone(),
                    result.file_key,
                    result.cache_data,
                );
            }
            report.cache_misses += 1;
        }

        run.persist_cache()?;
    }

    report.diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.span.start.line.cmp(&right.span.start.line))
            .then(left.span.start.column.cmp(&right.span.start.column))
            .then(left.message.cmp(&right.message))
    });

    Ok(report)
}

fn run_add_ignore_with_cwd(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
    reason: Option<&str>,
) -> Result<AddIgnoreReport> {
    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let settings = EffectiveCheckSettings::default();
    let runs = prepare_project_runs::<CheckCacheData, EffectiveCheckSettings, _>(
        &args.paths,
        cwd,
        &DiscoveryOptions {
            exclude_patterns: args.file_selection.exclude.clone(),
            extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
            respect_gitignore: args.respect_gitignore(),
            force_exclude: args.force_exclude(),
            parallel: false,
            cache_root: Some(cache_root.to_path_buf()),
            use_config_roots: config_arguments.use_config_roots(),
        },
        cache_root,
        true,
        b"project-cache-key",
        |_| Ok(settings.clone()),
    )?;
    let base_linter_settings = LinterSettings::default();

    let mut report = AddIgnoreReport::default();

    for run in runs {
        let analyzed_paths = run
            .files
            .iter()
            .map(|file| file.absolute_path.clone())
            .collect::<Vec<_>>();
        let linter_settings = base_linter_settings
            .clone()
            .with_analyzed_paths(analyzed_paths);

        for file in run.files {
            let result = add_ignores_to_path(&file.absolute_path, &linter_settings, reason)?;
            report.directives_added += result.directives_added;
            if result.parse_error.is_none() && result.diagnostics.is_empty() {
                continue;
            }

            let source = include_source
                .then(|| read_shared_source(&file.absolute_path))
                .transpose()?;
            if let Some(error) = result.parse_error {
                report.diagnostics.push(DisplayedDiagnostic {
                    path: file.display_path.clone(),
                    span: DisplaySpan::point(error.line, error.column),
                    message: error.message,
                    kind: DisplayedDiagnosticKind::ParseError,
                    source: source.clone(),
                });
            }
            push_lint_diagnostics(
                &mut report.diagnostics,
                &file.display_path,
                &result.diagnostics,
                source,
            );
        }
    }

    report.diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.span.start.line.cmp(&right.span.start.line))
            .then(left.span.start.column.cmp(&right.span.start.column))
            .then(left.message.cmp(&right.message))
    });

    Ok(report)
}

pub(crate) fn benchmark_check_paths(
    cwd: &Path,
    paths: &[PathBuf],
    output_format: crate::args::CheckOutputFormatArg,
) -> Result<usize> {
    let report = run_check_with_cwd(
        &CheckCommand {
            fix: false,
            unsafe_fixes: false,
            add_ignore: None,
            no_cache: true,
            output_format,
            watch: false,
            paths: paths.to_vec(),
            file_selection: FileSelectionArgs::default(),
            exit_zero: false,
            exit_non_zero_on_fix: false,
        },
        &ConfigArguments::default(),
        cwd,
        &cwd.join("cache"),
    )?;

    Ok(report.diagnostics.len())
}

fn analyze_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
    fix_applicability: Option<Applicability>,
) -> Result<FileCheckResult> {
    let mut source = read_shared_source(&pending.file.absolute_path)?;
    let inferred_shell = ShellDialect::infer(&source, Some(&pending.file.absolute_path));
    let parse_dialect = match inferred_shell {
        ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh => {
            shuck_parser::ShellDialect::Posix
        }
        ShellDialect::Mksh => shuck_parser::ShellDialect::Mksh,
        ShellDialect::Zsh => shuck_parser::ShellDialect::Zsh,
        ShellDialect::Unknown | ShellDialect::Bash => shuck_parser::ShellDialect::Bash,
    };

    let linter_settings = base_linter_settings.clone().with_shell(inferred_shell);
    let mut parse_result = Parser::with_dialect(&source, parse_dialect).parse();
    let mut diagnostics = collect_lint_diagnostics(
        &pending,
        &source,
        &parse_result,
        &linter_settings,
        shellcheck_map,
    );
    let mut fixes_applied = 0;

    if let Some(applicability) = fix_applicability {
        let applied = shuck_linter::apply_fixes(&source, &diagnostics, applicability);
        if applied.fixes_applied > 0 {
            source = Arc::<str>::from(applied.code);
            fs::write(&pending.file.absolute_path, &*source)?;
            parse_result = Parser::with_dialect(&source, parse_dialect).parse();
            diagnostics = collect_lint_diagnostics(
                &pending,
                &source,
                &parse_result,
                &linter_settings,
                shellcheck_map,
            );
            fixes_applied = applied.fixes_applied;
        }
    }

    let (cache_data, diagnostics) = if parse_result.is_err() && diagnostics.is_empty() {
        let ParseError::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        (
            CheckCacheData::ParseError(ParseCacheFailure {
                message: message.clone(),
                line,
                column,
            }),
            vec![DisplayedDiagnostic {
                path: pending.file.display_path.clone(),
                span: DisplaySpan::point(line, column),
                message,
                kind: DisplayedDiagnosticKind::ParseError,
                source: include_source.then_some(source.clone()),
            }],
        )
    } else {
        display_lint_diagnostics(&pending, &source, &diagnostics, include_source)
    };

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data,
        diagnostics,
        fixes_applied,
    })
}

fn collect_lint_diagnostics(
    pending: &PendingProjectFile,
    source: &Arc<str>,
    parse_result: &ParseResult,
    linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
) -> Vec<shuck_linter::Diagnostic> {
    let indexer = Indexer::new(source, parse_result);
    let directives = parse_directives(
        source,
        &parse_result.file,
        indexer.comment_index(),
        shellcheck_map,
    );
    let suppression_index = (!directives.is_empty()).then(|| {
        SuppressionIndex::new(
            &directives,
            &parse_result.file,
            first_statement_line(&parse_result.file).unwrap_or(u32::MAX),
        )
    });
    shuck_linter::lint_file_at_path_with_parse_result(
        parse_result,
        source,
        &indexer,
        linter_settings,
        suppression_index.as_ref(),
        Some(&pending.file.absolute_path),
    )
}

fn display_lint_diagnostics(
    pending: &PendingProjectFile,
    source: &Arc<str>,
    diagnostics: &[shuck_linter::Diagnostic],
    include_source: bool,
) -> (CheckCacheData, Vec<DisplayedDiagnostic>) {
    let diagnostic_source = (!diagnostics.is_empty() && include_source).then(|| source.clone());

    (
        CheckCacheData::Success(
            diagnostics
                .iter()
                .map(CachedLintDiagnostic::from_diagnostic)
                .collect(),
        ),
        diagnostics
            .iter()
            .map(|diagnostic| DisplayedDiagnostic {
                path: pending.file.display_path.clone(),
                span: DisplaySpan::new(
                    DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                    DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
                ),
                message: diagnostic.message.clone(),
                kind: DisplayedDiagnosticKind::Lint {
                    code: diagnostic.code().to_owned(),
                    severity: diagnostic.severity.as_str().to_owned(),
                },
                source: diagnostic_source.clone(),
            })
            .collect(),
    )
}

fn requested_fix_applicability(args: &CheckCommand) -> Option<Applicability> {
    if args.unsafe_fixes {
        Some(Applicability::Unsafe)
    } else if args.fix {
        Some(Applicability::Safe)
    } else {
        None
    }
}

fn push_cached_lint_diagnostics(
    report: &mut CheckReport,
    path: &Path,
    diagnostics: &[CachedLintDiagnostic],
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        report.diagnostics.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.start_line, diagnostic.start_column),
                DisplayPosition::new(diagnostic.end_line, diagnostic.end_column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity.clone(),
            },
            source: source.clone(),
        });
    }
}

fn push_lint_diagnostics(
    displayed: &mut Vec<DisplayedDiagnostic>,
    path: &Path,
    diagnostics: &[shuck_linter::Diagnostic],
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        displayed.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            source: source.clone(),
        });
    }
}

fn read_shared_source(path: &Path) -> Result<Arc<str>> {
    Ok(Arc::<str>::from(fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use notify::event::{CreateKind, EventAttributes, ModifyKind, RemoveKind, RenameMode};
    use tempfile::tempdir;

    use super::*;
    use crate::args::CheckOutputFormatArg;
    use crate::config::ConfigArguments;

    fn pending_project_file(path: &Path, project_root: &Path) -> PendingProjectFile {
        PendingProjectFile {
            file: crate::discover::DiscoveredFile {
                display_path: path.strip_prefix(project_root).unwrap().to_path_buf(),
                absolute_path: path.to_path_buf(),
                relative_path: path.strip_prefix(project_root).unwrap().to_path_buf(),
                project_root: crate::discover::ProjectRoot {
                    storage_root: project_root.to_path_buf(),
                    canonical_root: fs::canonicalize(project_root).unwrap(),
                },
            },
            file_key: shuck_cache::FileCacheKey::from_path(path).unwrap(),
        }
    }

    fn cache_root(cwd: &Path) -> PathBuf {
        cwd.join("cache")
    }

    fn match_paths(canonical: &Path, resolved: &Path) -> Vec<PathBuf> {
        let mut paths = vec![canonical.to_path_buf(), normalize_path(resolved)];
        paths.sort();
        paths.dedup();
        paths
    }

    fn watch_paths(canonical: &Path, resolved: &Path) -> Vec<PathBuf> {
        let mut paths = vec![canonical.to_path_buf(), normalize_path(resolved)];
        paths.sort();
        paths.dedup();
        paths
    }

    fn make_file_read_only(path: &Path) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn check_args_with_format(no_cache: bool, output_format: CheckOutputFormatArg) -> CheckCommand {
        CheckCommand {
            fix: false,
            unsafe_fixes: false,
            add_ignore: None,
            no_cache,
            output_format,
            watch: false,
            paths: Vec::new(),
            file_selection: FileSelectionArgs::default(),
            exit_zero: false,
            exit_non_zero_on_fix: false,
        }
    }

    fn check_args(no_cache: bool) -> CheckCommand {
        check_args_with_format(no_cache, CheckOutputFormatArg::Full)
    }

    #[test]
    fn reports_parse_errors() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.exit_status(false, false), ExitStatus::Failure);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
    }

    #[test]
    fn exit_zero_suppresses_only_non_fatal_diagnostics() {
        let warning = DisplayedDiagnostic {
            path: PathBuf::from("warn.sh"),
            span: DisplaySpan::point(1, 1),
            message: "lint".to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: "C001".to_owned(),
                severity: "warning".to_owned(),
            },
            source: None,
        };
        let error_lint = DisplayedDiagnostic {
            path: PathBuf::from("err.sh"),
            span: DisplaySpan::point(1, 1),
            message: "lint".to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: "C035".to_owned(),
                severity: "error".to_owned(),
            },
            source: None,
        };
        let parse = DisplayedDiagnostic {
            path: PathBuf::from("broken.sh"),
            span: DisplaySpan::point(1, 1),
            message: "parse".to_owned(),
            kind: DisplayedDiagnosticKind::ParseError,
            source: None,
        };

        let warning_only = CheckReport {
            diagnostics: vec![warning.clone()],
            ..CheckReport::default()
        };
        assert_eq!(warning_only.exit_status(false, false), ExitStatus::Failure);
        assert_eq!(warning_only.exit_status(true, false), ExitStatus::Success);

        let with_error_lint = CheckReport {
            diagnostics: vec![warning.clone(), error_lint],
            ..CheckReport::default()
        };
        assert_eq!(
            with_error_lint.exit_status(true, false),
            ExitStatus::Failure
        );

        let with_parse_error = CheckReport {
            diagnostics: vec![warning, parse],
            ..CheckReport::default()
        };
        assert_eq!(
            with_parse_error.exit_status(true, false),
            ExitStatus::Failure
        );
    }

    #[test]
    fn exit_non_zero_on_fix_fires_when_fixes_applied() {
        let report = CheckReport {
            fixes_applied: 1,
            ..CheckReport::default()
        };
        assert_eq!(report.exit_status(false, false), ExitStatus::Success);
        assert_eq!(report.exit_status(false, true), ExitStatus::Failure);
        assert_eq!(report.exit_status(true, true), ExitStatus::Failure);
    }

    #[test]
    fn reports_missing_fi_as_c035_lint() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("broken.sh"),
            "#!/bin/sh\nif true; then\n  :\n",
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        match &report.diagnostics[0].kind {
            DisplayedDiagnosticKind::Lint { code, .. } => assert_eq!(code, "C035"),
            other => panic!("expected lint diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn reports_missing_fi_as_parse_error_when_parse_rule_is_disabled() {
        let tempdir = tempdir().unwrap();
        let broken_path = tempdir.path().join("broken.sh");
        fs::write(&broken_path, "#!/bin/sh\nif true; then\n  :\n").unwrap();

        let result = analyze_file(
            pending_project_file(&broken_path, tempdir.path()),
            &LinterSettings::for_rule(shuck_linter::Rule::UnusedAssignment)
                .with_analyzed_paths([broken_path.clone()]),
            &ShellCheckCodeMap::default(),
            false,
            None,
        )
        .unwrap();

        assert_eq!(result.diagnostics.len(), 1);
        assert!(matches!(result.cache_data, CheckCacheData::ParseError(_)));
        match &result.diagnostics[0].kind {
            DisplayedDiagnosticKind::ParseError => {}
            other => panic!("expected parse error, got {other:?}"),
        }
        assert!(result.diagnostics[0].message.contains("expected 'fi'"));
    }

    #[test]
    fn reuses_cached_results() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.cache_misses, 0);
    }

    #[test]
    fn invalidates_cache_when_file_changes() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(&script, "#!/bin/bash\necho ok\n").unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);

        fs::write(&script, "#!/bin/bash\nif true\n").unwrap();
        make_file_read_only(&script);

        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert_eq!(second.diagnostics.len(), 1);
    }

    #[test]
    fn no_cache_does_not_write_cache_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn infers_shell_from_extension_for_local_rule() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("posix.sh"), "local foo=bar\n").unwrap();
        fs::write(tempdir.path().join("bashy.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let c014 = report
            .diagnostics
            .iter()
            .filter(|diagnostic| matches!(&diagnostic.kind, DisplayedDiagnosticKind::Lint { code, .. } if code == "C014"))
            .collect::<Vec<_>>();

        assert_eq!(c014.len(), 1);
        assert_eq!(c014[0].path, PathBuf::from("bashy.bash"));
    }

    #[test]
    fn mixes_cache_hits_and_misses_in_a_single_run() {
        let tempdir = tempdir().unwrap();
        let first = tempdir.path().join("first.sh");
        let second = tempdir.path().join("second.sh");
        fs::write(&first, "#!/bin/bash\necho ok\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho ok\n").unwrap();

        let cache_root = cache_root(tempdir.path());
        let initial = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();
        assert_eq!(initial.cache_hits, 0);
        assert_eq!(initial.cache_misses, 2);

        fs::write(&second, "#!/bin/bash\nif true\n").unwrap();

        let rerun = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();
        assert_eq!(rerun.cache_hits, 1);
        assert_eq!(rerun.cache_misses, 1);
        assert_eq!(rerun.diagnostics.len(), 1);
        assert_eq!(rerun.diagnostics[0].path, PathBuf::from("second.sh"));
    }

    #[test]
    fn sorts_diagnostics_deterministically_after_parallel_run() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("z.sh"), "#!/bin/bash\nif true\n").unwrap();
        fs::write(tempdir.path().join("a.bash"), "local foo=bar\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let paths = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();

        let mut sorted_paths = paths.clone();
        sorted_paths.sort();
        assert_eq!(paths, sorted_paths);
        assert_eq!(paths.first(), Some(&PathBuf::from("a.bash")));
        assert_eq!(paths.last(), Some(&PathBuf::from("z.sh")));
    }

    #[test]
    fn duplicate_explicit_file_and_directory_inputs_are_deduplicated() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("dup.sh"), "#!/bin/bash\nif true\n").unwrap();

        let args = CheckCommand {
            paths: vec![PathBuf::from("."), PathBuf::from("dup.sh")],
            ..check_args(true)
        };
        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.cache_hits, 0);
        assert_eq!(report.cache_misses, 1);
        assert_eq!(report.diagnostics.len(), 1);
    }

    #[test]
    fn skips_a_configured_cache_directory_inside_the_walked_tree() {
        let tempdir = tempdir().unwrap();
        let cache_root = tempdir.path().join("custom-cache");
        fs::create_dir_all(&cache_root).unwrap();
        fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();
        fs::write(cache_root.join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

        let report = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root,
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
        assert!(!tempdir.path().join(".shuck_cache").exists());
    }

    #[test]
    fn report_output_includes_ansi_styles_when_enabled() {
        colored::control::set_override(true);

        let report = CheckReport {
            diagnostics: vec![DisplayedDiagnostic {
                path: PathBuf::from("script.sh"),
                span: DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
                message: "example message".to_owned(),
                kind: DisplayedDiagnosticKind::Lint {
                    code: "C014".to_owned(),
                    severity: "warning".to_owned(),
                },
                source: Some(Arc::<str>::from("echo ok\nvalue=$foo\nprintf '%s' $bar\n")),
            }],
            cache_hits: 0,
            cache_misses: 0,
            fixes_applied: 0,
        };

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &report.diagnostics,
            CheckOutputFormatArg::Full,
            true,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\u{1b}["));
        assert!(output.contains("warning"));
        assert!(output.contains("C014"));

        colored::control::unset_override();
    }

    #[test]
    fn report_output_stays_plain_when_colors_are_disabled() {
        let report = CheckReport {
            diagnostics: vec![DisplayedDiagnostic {
                path: PathBuf::from("script.sh"),
                span: DisplaySpan::point(2, 7),
                message: "unterminated construct".to_owned(),
                kind: DisplayedDiagnosticKind::ParseError,
                source: None,
            }],
            cache_hits: 0,
            cache_misses: 0,
            fixes_applied: 0,
        };

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &report.diagnostics,
            CheckOutputFormatArg::Concise,
            false,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "script.sh:2:7: parse error unterminated construct\n"
        );
    }

    #[test]
    fn cached_diagnostics_retain_source_for_full_output() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("warn.sh"),
            "#!/bin/bash\nunused=1\necho ok\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args_with_format(false, CheckOutputFormatArg::Full),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        let second = run_check_with_cwd(
            &check_args_with_format(false, CheckOutputFormatArg::Full),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(first.cache_misses, 1);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(second.diagnostics.len(), 1);
        assert_eq!(
            second.diagnostics[0].source.as_deref(),
            Some("#!/bin/bash\nunused=1\necho ok\n")
        );
    }

    #[test]
    fn lint_diagnostics_share_the_original_source_arc_for_full_output() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("warn.sh");
        fs::write(&path, "#!/bin/bash\nunused=1\necho ok\n").unwrap();

        let pending = pending_project_file(&path, tempdir.path());
        let source = read_shared_source(&path).unwrap();
        let parse_result = Parser::with_dialect(&source, shuck_parser::ShellDialect::Bash).parse();

        let diagnostics = collect_lint_diagnostics(
            &pending,
            &source,
            &parse_result,
            &LinterSettings::default(),
            &ShellCheckCodeMap::default(),
        );
        let (_, diagnostics) = display_lint_diagnostics(&pending, &source, &diagnostics, true);

        let diagnostic_source = diagnostics[0]
            .source
            .as_ref()
            .expect("full output should retain source");
        assert!(Arc::ptr_eq(diagnostic_source, &source));
    }

    #[test]
    fn watch_event_filter_ignores_access_other_ignored_dirs_and_cache_paths() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![
            WatchTarget::recursive(PathBuf::from("/workspace/project")),
            WatchTarget::file(PathBuf::from("/workspace/config/shuck.toml")),
        ];

        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Access(notify::event::AccessKind::Any),
                paths: vec![PathBuf::from("script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Other,
                paths: vec![PathBuf::from("script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Create(CreateKind::File),
                paths: vec![PathBuf::from(".git/hooks/post-commit")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![cache_root.join("entry.bin")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(!watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/workspace/config/other.txt")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
    }

    #[test]
    fn watch_event_filter_triggers_on_create_modify_remove_rename_and_rescan() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![
            WatchTarget::recursive(PathBuf::from("/workspace/project")),
            WatchTarget::file(PathBuf::from("/workspace/config/shuck.toml")),
        ];

        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Create(CreateKind::File),
                paths: vec![PathBuf::from("/workspace/project/script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/workspace/config/shuck.toml")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Remove(RemoveKind::File),
                paths: vec![PathBuf::from("/workspace/project/script.sh")],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                paths: vec![
                    PathBuf::from("/tmp/tempfile"),
                    PathBuf::from("/workspace/config/shuck.toml"),
                ],
                attrs: EventAttributes::default(),
            },
            cache_root,
            &watch_targets,
        ));

        let mut attrs = EventAttributes::default();
        attrs.set_flag(notify::event::Flag::Rescan);
        assert!(watch_event_requires_rerun(
            &notify::Event {
                kind: notify::EventKind::Modify(ModifyKind::Any),
                paths: vec![],
                attrs,
            },
            cache_root,
            &watch_targets,
        ));
    }

    #[test]
    fn clear_screen_requires_terminal_stdout() {
        assert!(should_clear_screen(true));
        assert!(!should_clear_screen(false));
    }

    #[test]
    fn collect_watch_targets_stay_within_requested_scope_and_watch_config_files() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        let deeper = nested.join("deeper");
        fs::create_dir_all(&deeper).unwrap();
        fs::write(tempdir.path().join("shuck.toml"), "[format]\n").unwrap();
        let file = nested.join("script.sh");
        fs::write(&file, "#!/bin/bash\necho ok\n").unwrap();

        let default_targets =
            collect_watch_targets(&[], &ConfigArguments::default(), tempdir.path()).unwrap();
        assert_eq!(
            default_targets,
            vec![WatchTarget {
                watch_path: normalize_path(tempdir.path()),
                watch_paths: watch_paths(
                    &fs::canonicalize(tempdir.path()).unwrap(),
                    tempdir.path()
                ),
                recursive: true,
                match_paths: match_paths(
                    &fs::canonicalize(tempdir.path()).unwrap(),
                    tempdir.path()
                ),
            }]
        );

        let nested_targets = collect_watch_targets(
            &[PathBuf::from("nested"), PathBuf::from("nested/deeper")],
            &ConfigArguments::default(),
            tempdir.path(),
        )
        .unwrap();
        assert_eq!(
            nested_targets,
            vec![
                WatchTarget {
                    watch_path: normalize_path(tempdir.path()),
                    watch_paths: watch_paths(
                        &fs::canonicalize(tempdir.path()).unwrap(),
                        tempdir.path()
                    ),
                    recursive: false,
                    match_paths: match_paths(
                        &fs::canonicalize(tempdir.path().join("shuck.toml")).unwrap(),
                        &tempdir.path().join("shuck.toml"),
                    ),
                },
                WatchTarget {
                    watch_path: normalize_path(&nested),
                    watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                    recursive: true,
                    match_paths: match_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                },
            ]
        );

        let file_targets = collect_watch_targets(
            &[PathBuf::from("nested/script.sh")],
            &ConfigArguments::default(),
            tempdir.path(),
        )
        .unwrap();
        assert_eq!(
            file_targets,
            vec![
                WatchTarget {
                    watch_path: normalize_path(tempdir.path()),
                    watch_paths: watch_paths(
                        &fs::canonicalize(tempdir.path()).unwrap(),
                        tempdir.path()
                    ),
                    recursive: false,
                    match_paths: match_paths(
                        &fs::canonicalize(tempdir.path().join("shuck.toml")).unwrap(),
                        &tempdir.path().join("shuck.toml"),
                    ),
                },
                WatchTarget {
                    watch_path: normalize_path(&nested),
                    watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                    recursive: false,
                    match_paths: match_paths(&fs::canonicalize(&file).unwrap(), &file),
                },
            ]
        );
    }

    #[test]
    fn collect_watch_targets_merge_files_in_the_same_parent_directory() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        let first = nested.join("first.sh");
        let second = nested.join("second.sh");
        fs::write(&first, "#!/bin/bash\necho ok\n").unwrap();
        fs::write(&second, "#!/bin/bash\necho ok\n").unwrap();

        let targets = collect_watch_targets(
            &[
                PathBuf::from("nested/first.sh"),
                PathBuf::from("nested/second.sh"),
            ],
            &ConfigArguments::from_cli(Vec::new(), true).unwrap(),
            tempdir.path(),
        )
        .unwrap();

        assert_eq!(
            targets,
            vec![WatchTarget {
                watch_path: normalize_path(&nested),
                watch_paths: watch_paths(&fs::canonicalize(&nested).unwrap(), &nested),
                recursive: false,
                match_paths: {
                    let mut paths = vec![
                        fs::canonicalize(&first).unwrap(),
                        normalize_path(&first),
                        fs::canonicalize(&second).unwrap(),
                        normalize_path(&second),
                    ];
                    paths.sort();
                    paths.dedup();
                    paths
                },
            }]
        );
    }

    #[test]
    fn drain_watch_batch_coalesces_queued_events_before_rerunning() {
        let cache_root = Path::new("/tmp/shuck-cache");
        let watch_targets = vec![WatchTarget::recursive(PathBuf::from("/workspace/project"))];
        let (tx, rx) = channel();

        tx.send(Ok(notify::Event {
            kind: notify::EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![PathBuf::from("/workspace/project/ignored/.git/index")],
            attrs: EventAttributes::default(),
        }))
        .unwrap();

        let first = notify::Event {
            kind: notify::EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            paths: vec![PathBuf::from("/workspace/project/script.sh")],
            attrs: EventAttributes::default(),
        };

        assert!(drain_watch_batch(first, &rx, cache_root, &watch_targets).unwrap());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
}
