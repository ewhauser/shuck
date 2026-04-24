use std::collections::{BTreeMap, HashMap};
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
use shuck_ast::TextSize;
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_extract::{EmbeddedScript, ExtractedDialect, HostLineStart, extract_all};
use shuck_indexer::Indexer;
use shuck_linter::{
    AmbientShellOptions, Applicability, CompiledPerFileIgnoreList, LinterSettings, PerFileIgnore,
    Rule, RuleSelector, RuleSet, ShellCheckCodeMap, ShellDialect, SuppressionIndex,
    add_ignores_to_path, first_statement_line, parse_directives,
};
use shuck_parser::{
    Error as ParseError,
    parser::{ParseResult, Parser},
};

use crate::ExitStatus;
use crate::args::{CheckCommand, FileSelectionArgs, PatternRuleSelectorPair, RuleSelectionArgs};
use crate::cache::resolve_cache_root;
use crate::commands::check_output::{
    DisplayPosition, DisplaySpan, DisplayedApplicability, DisplayedDiagnostic,
    DisplayedDiagnosticKind, DisplayedEdit, DisplayedFix, print_report_to,
};
use crate::commands::project_runner::{
    PendingProjectFile, ProjectRunRequest, prepare_project_runs,
    prepare_project_runs_with_cache_key,
};
use crate::config::{
    ConfigArguments, LintConfig, discovered_config_path_for_root, load_project_config,
    resolve_project_root_for_input,
};
use crate::discover::{
    DEFAULT_IGNORED_DIR_NAMES, DiscoveredFile, DiscoveryOptions, FileKind, ProjectRoot,
    normalize_path,
};

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
    per_file_ignores: Vec<EffectivePerFileIgnore>,
    rule_options: EffectiveRuleOptions,
    embedded_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectivePerFileIgnore {
    pattern: String,
    rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveRuleOptions {
    c001_treat_indirect_expansion_targets_as_used: bool,
    c063_report_unreached_nested_definitions: bool,
}

impl EffectiveCheckSettings {
    fn new(
        enabled_rules: RuleSet,
        per_file_ignores: &[PerFileIgnore],
        rule_options: &shuck_linter::LinterRuleOptions,
        embedded_enabled: bool,
    ) -> Self {
        let mut enabled_rules = enabled_rules
            .iter()
            .map(|rule| rule.code().to_owned())
            .collect::<Vec<_>>();
        enabled_rules.sort();

        let mut per_file_ignores = per_file_ignores
            .iter()
            .map(|ignore| {
                let mut rules = ignore
                    .rules()
                    .iter()
                    .map(|rule| rule.code().to_owned())
                    .collect::<Vec<_>>();
                rules.sort();
                EffectivePerFileIgnore {
                    pattern: ignore.pattern().to_owned(),
                    rules,
                }
            })
            .collect::<Vec<_>>();
        per_file_ignores.sort_by(|left, right| left.pattern.cmp(&right.pattern));

        Self {
            enabled_rules,
            per_file_ignores,
            rule_options: EffectiveRuleOptions::new(rule_options),
            embedded_enabled,
        }
    }
}

impl CacheKey for EffectiveCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-check-settings");
        self.enabled_rules.cache_key(state);
        self.per_file_ignores.cache_key(state);
        self.rule_options.cache_key(state);
        self.embedded_enabled.cache_key(state);
    }
}

impl CacheKey for EffectivePerFileIgnore {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.pattern.cache_key(state);
        self.rules.cache_key(state);
    }
}

impl EffectiveRuleOptions {
    fn new(rule_options: &shuck_linter::LinterRuleOptions) -> Self {
        Self {
            c001_treat_indirect_expansion_targets_as_used: rule_options
                .c001
                .treat_indirect_expansion_targets_as_used,
            c063_report_unreached_nested_definitions: rule_options
                .c063
                .report_unreached_nested_definitions,
        }
    }
}

impl CacheKey for EffectiveRuleOptions {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-rule-options");
        self.c001_treat_indirect_expansion_targets_as_used
            .cache_key(state);
        self.c063_report_unreached_nested_definitions
            .cache_key(state);
    }
}

#[derive(Debug, Clone)]
struct ResolvedCheckSettings {
    linter_settings: LinterSettings,
    fixable_rules: RuleSet,
    effective: EffectiveCheckSettings,
    embedded_enabled: bool,
}

impl CacheKey for ResolvedCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.effective.cache_key(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckCacheSettings {
    effective: EffectiveCheckSettings,
    analyzed_paths: Vec<PathBuf>,
}

impl CheckCacheSettings {
    fn new(settings: &ResolvedCheckSettings, files: &[DiscoveredFile]) -> Self {
        Self {
            effective: settings.effective.clone(),
            analyzed_paths: analyzed_shell_relative_paths(files),
        }
    }
}

impl CacheKey for CheckCacheSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"check-cache-settings");
        self.effective.cache_key(state);
        self.analyzed_paths.cache_key(state);
    }
}

fn analyzed_shell_relative_paths(files: &[DiscoveredFile]) -> Vec<PathBuf> {
    let mut paths = files
        .iter()
        .filter(|file| file.kind == FileKind::Shell)
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

#[derive(Debug, Clone, Default)]
struct RuleSelectionLayer {
    select: Option<Vec<RuleSelector>>,
    ignore: Vec<RuleSelector>,
    extend_select: Vec<RuleSelector>,
    per_file_ignores: Option<Vec<PerFileIgnoreSpec>>,
    extend_per_file_ignores: Vec<PerFileIgnoreSpec>,
    fixable: Option<Vec<RuleSelector>>,
    unfixable: Vec<RuleSelector>,
    extend_fixable: Vec<RuleSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PerFileIgnoreSpec {
    pattern: String,
    selectors: Vec<RuleSelector>,
}

impl PerFileIgnoreSpec {
    fn into_ignore(self) -> PerFileIgnore {
        let rules = self
            .selectors
            .into_iter()
            .fold(RuleSet::EMPTY, |rules, selector| {
                rules.union(&selector.into_rule_set())
            });
        PerFileIgnore::new(self.pattern, rules)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CheckCacheData {
    diagnostics: Vec<CachedDisplayedDiagnostic>,
}

impl CheckCacheData {
    fn from_displayed(diagnostics: &[DisplayedDiagnostic]) -> Self {
        Self {
            diagnostics: diagnostics
                .iter()
                .map(CachedDisplayedDiagnostic::from_displayed)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum CachedDisplayedDiagnosticKind {
    ParseError,
    Lint { code: String, severity: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedDisplayedDiagnostic {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    message: String,
    kind: CachedDisplayedDiagnosticKind,
    fix: Option<CachedLintFix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum CachedApplicability {
    Safe,
    Unsafe,
}

impl From<Applicability> for CachedApplicability {
    fn from(value: Applicability) -> Self {
        match value {
            Applicability::Safe => Self::Safe,
            Applicability::Unsafe => Self::Unsafe,
        }
    }
}

impl From<CachedApplicability> for DisplayedApplicability {
    fn from(value: CachedApplicability) -> Self {
        match value {
            CachedApplicability::Safe => Self::Safe,
            CachedApplicability::Unsafe => Self::Unsafe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintFix {
    applicability: CachedApplicability,
    message: Option<String>,
    edits: Vec<CachedLintEdit>,
}

impl CachedLintFix {
    fn from_displayed(fix: &DisplayedFix) -> Self {
        Self {
            applicability: match fix.applicability {
                DisplayedApplicability::Safe => CachedApplicability::Safe,
                DisplayedApplicability::Unsafe => CachedApplicability::Unsafe,
            },
            message: fix.message.clone(),
            edits: fix
                .edits
                .iter()
                .map(CachedLintEdit::from_displayed)
                .collect(),
        }
    }

    fn to_displayed(&self) -> DisplayedFix {
        DisplayedFix {
            applicability: self.applicability.into(),
            message: self.message.clone(),
            edits: self
                .edits
                .iter()
                .map(CachedLintEdit::to_displayed)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedLintEdit {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    content: String,
}

impl CachedLintEdit {
    fn from_displayed(edit: &DisplayedEdit) -> Self {
        Self {
            start_line: edit.location.line,
            start_column: edit.location.column,
            end_line: edit.end_location.line,
            end_column: edit.end_location.column,
            content: edit.content.clone(),
        }
    }

    fn to_displayed(&self) -> DisplayedEdit {
        DisplayedEdit {
            location: DisplayPosition::new(self.start_line, self.start_column),
            end_location: DisplayPosition::new(self.end_line, self.end_column),
            content: self.content.clone(),
        }
    }
}

impl CachedDisplayedDiagnostic {
    fn from_displayed(diagnostic: &DisplayedDiagnostic) -> Self {
        Self {
            start_line: diagnostic.span.start.line,
            start_column: diagnostic.span.start.column,
            end_line: diagnostic.span.end.line,
            end_column: diagnostic.span.end.column,
            message: diagnostic.message.clone(),
            kind: match &diagnostic.kind {
                DisplayedDiagnosticKind::ParseError => CachedDisplayedDiagnosticKind::ParseError,
                DisplayedDiagnosticKind::Lint { code, severity } => {
                    CachedDisplayedDiagnosticKind::Lint {
                        code: code.clone(),
                        severity: severity.clone(),
                    }
                }
            },
            fix: diagnostic.fix.as_ref().map(CachedLintFix::from_displayed),
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

fn resolve_project_check_settings(
    project_root: &ProjectRoot,
    config_arguments: &ConfigArguments,
    cli_rule_selection: &RuleSelectionArgs,
) -> Result<ResolvedCheckSettings> {
    let config = load_project_config(&project_root.storage_root, config_arguments)?;
    let layers = [
        parse_lint_config_layer(&config.lint)?,
        parse_cli_rule_selection_layer(cli_rule_selection),
    ];
    let rule_options = linter_rule_options_for_lint_config(&config.lint);

    let mut enabled_rules = LinterSettings::default_rules();
    let mut fixable_rules = RuleSet::all();
    let mut per_file_ignores = Vec::new();

    for layer in layers {
        enabled_rules = apply_rule_selector_layer(
            enabled_rules,
            layer.select.as_deref(),
            &layer.extend_select,
            &layer.ignore,
        );
        fixable_rules = apply_rule_selector_layer(
            fixable_rules,
            layer.fixable.as_deref(),
            &layer.extend_fixable,
            &layer.unfixable,
        );
        per_file_ignores = apply_per_file_ignore_layer(
            per_file_ignores,
            layer.per_file_ignores,
            layer.extend_per_file_ignores,
        );
    }

    let compiled_per_file_ignores = CompiledPerFileIgnoreList::resolve(
        project_root.canonical_root.clone(),
        per_file_ignores.clone(),
    )?;
    let embedded_enabled = config.check.embedded.unwrap_or(true);
    let effective = EffectiveCheckSettings::new(
        enabled_rules,
        &per_file_ignores,
        &rule_options,
        embedded_enabled,
    );

    Ok(ResolvedCheckSettings {
        linter_settings: LinterSettings {
            rules: enabled_rules,
            per_file_ignores: Arc::new(compiled_per_file_ignores),
            rule_options,
            ..LinterSettings::default()
        },
        fixable_rules,
        effective,
        embedded_enabled,
    })
}

fn linter_rule_options_for_lint_config(lint: &LintConfig) -> shuck_linter::LinterRuleOptions {
    let mut rule_options = shuck_linter::LinterRuleOptions::default();
    if let Some(value) = lint
        .rule_options
        .as_ref()
        .and_then(|options| options.c001.as_ref())
        .and_then(|c001| c001.treat_indirect_expansion_targets_as_used)
    {
        rule_options.c001.treat_indirect_expansion_targets_as_used = value;
    }
    if let Some(value) = lint
        .rule_options
        .as_ref()
        .and_then(|options| options.c063.as_ref())
        .and_then(|c063| c063.report_unreached_nested_definitions)
    {
        rule_options.c063.report_unreached_nested_definitions = value;
    }

    rule_options
}

fn parse_lint_config_layer(lint: &LintConfig) -> Result<RuleSelectionLayer> {
    Ok(RuleSelectionLayer {
        select: lint
            .select
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.select"))
            .transpose()?,
        ignore: lint
            .ignore
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.ignore"))
            .transpose()?
            .unwrap_or_default(),
        extend_select: lint
            .extend_select
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.extend-select"))
            .transpose()?
            .unwrap_or_default(),
        per_file_ignores: lint
            .per_file_ignores
            .as_ref()
            .map(|patterns| parse_per_file_ignore_map(patterns, "lint.per-file-ignores"))
            .transpose()?,
        extend_per_file_ignores: lint
            .extend_per_file_ignores
            .as_ref()
            .map(|patterns| parse_per_file_ignore_map(patterns, "lint.extend-per-file-ignores"))
            .transpose()?
            .unwrap_or_default(),
        fixable: lint
            .fixable
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.fixable"))
            .transpose()?,
        unfixable: lint
            .unfixable
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.unfixable"))
            .transpose()?
            .unwrap_or_default(),
        extend_fixable: lint
            .extend_fixable
            .as_ref()
            .map(|selectors| parse_rule_selectors(selectors, "lint.extend-fixable"))
            .transpose()?
            .unwrap_or_default(),
    })
}

fn parse_cli_rule_selection_layer(args: &RuleSelectionArgs) -> RuleSelectionLayer {
    RuleSelectionLayer {
        select: args.select.clone(),
        ignore: args.ignore.clone(),
        extend_select: args.extend_select.clone(),
        per_file_ignores: args
            .per_file_ignores
            .as_ref()
            .map(|pairs| group_per_file_ignore_pairs(pairs)),
        extend_per_file_ignores: group_per_file_ignore_pairs(&args.extend_per_file_ignores),
        fixable: args.fixable.clone(),
        unfixable: args.unfixable.clone(),
        extend_fixable: args.extend_fixable.clone(),
    }
}

fn parse_rule_selectors(selectors: &[String], scope: &str) -> Result<Vec<RuleSelector>> {
    selectors
        .iter()
        .map(|selector| {
            let selector = selector.trim();
            if selector.is_empty() {
                return Err(anyhow!(
                    "invalid {scope} selector: selector cannot be empty"
                ));
            }

            selector
                .parse::<RuleSelector>()
                .map_err(|err| anyhow!("invalid {scope} selector `{selector}`: {err}"))
        })
        .collect()
}

fn parse_per_file_ignore_map(
    patterns: &BTreeMap<String, Vec<String>>,
    scope: &str,
) -> Result<Vec<PerFileIgnoreSpec>> {
    patterns
        .iter()
        .map(|(pattern, selectors)| {
            Ok(PerFileIgnoreSpec {
                pattern: pattern.clone(),
                selectors: parse_rule_selectors(selectors, scope)?,
            })
        })
        .collect()
}

fn group_per_file_ignore_pairs(pairs: &[PatternRuleSelectorPair]) -> Vec<PerFileIgnoreSpec> {
    let mut grouped = BTreeMap::<String, Vec<RuleSelector>>::new();
    for pair in pairs {
        grouped
            .entry(pair.pattern.clone())
            .or_default()
            .push(pair.selector.clone());
    }

    grouped
        .into_iter()
        .map(|(pattern, selectors)| PerFileIgnoreSpec { pattern, selectors })
        .collect()
}

fn apply_rule_selector_layer(
    current: RuleSet,
    select: Option<&[RuleSelector]>,
    extend_select: &[RuleSelector],
    ignore: &[RuleSelector],
) -> RuleSet {
    let mut specificities = select
        .into_iter()
        .flatten()
        .chain(extend_select.iter())
        .chain(ignore.iter())
        .map(selector_specificity)
        .collect::<Vec<_>>();
    specificities.sort_unstable();
    specificities.dedup();

    let mut updates = HashMap::<Rule, bool>::new();
    for specificity in specificities {
        for selector in select
            .into_iter()
            .flatten()
            .chain(extend_select.iter())
            .filter(|selector| selector_specificity(selector) == specificity)
        {
            for rule in selector.into_rule_set().iter() {
                updates.insert(rule, true);
            }
        }
        for selector in ignore
            .iter()
            .filter(|selector| selector_specificity(selector) == specificity)
        {
            for rule in selector.into_rule_set().iter() {
                updates.insert(rule, false);
            }
        }
    }

    if select.is_some() {
        updates
            .into_iter()
            .filter_map(|(rule, enabled)| enabled.then_some(rule))
            .collect()
    } else {
        let mut rules = current;
        for (rule, enabled) in updates {
            rules.set(rule, enabled);
        }
        rules
    }
}

fn selector_specificity(selector: &RuleSelector) -> usize {
    match selector {
        RuleSelector::All => 0,
        RuleSelector::Category(_) => 1,
        RuleSelector::Prefix(prefix) => 2 + prefix.len(),
        RuleSelector::Rule(_) => usize::MAX,
    }
}

fn apply_per_file_ignore_layer(
    current: Vec<PerFileIgnore>,
    per_file_ignores: Option<Vec<PerFileIgnoreSpec>>,
    extend_per_file_ignores: Vec<PerFileIgnoreSpec>,
) -> Vec<PerFileIgnore> {
    let mut per_file_ignores = per_file_ignores
        .map(|per_file_ignores| {
            per_file_ignores
                .into_iter()
                .map(PerFileIgnoreSpec::into_ignore)
                .collect()
        })
        .unwrap_or(current);
    per_file_ignores.extend(
        extend_per_file_ignores
            .into_iter()
            .map(PerFileIgnoreSpec::into_ignore),
    );
    per_file_ignores
}
fn run_check_with_cwd(
    args: &CheckCommand,
    config_arguments: &ConfigArguments,
    cwd: &Path,
    cache_root: &Path,
) -> Result<CheckReport> {
    let include_source = matches!(args.output_format, crate::args::CheckOutputFormatArg::Full);
    let fix_applicability = requested_fix_applicability(args);
    let mut runs = prepare_project_runs_with_cache_key::<
        CheckCacheData,
        ResolvedCheckSettings,
        CheckCacheSettings,
        _,
        _,
    >(
        ProjectRunRequest {
            inputs: &args.paths,
            cwd,
            discovery_options: &DiscoveryOptions {
                exclude_patterns: args.file_selection.exclude.clone(),
                extend_exclude_patterns: args.file_selection.extend_exclude.clone(),
                respect_gitignore: args.respect_gitignore(),
                force_exclude: args.force_exclude(),
                parallel: true,
                cache_root: Some(cache_root.to_path_buf()),
                use_config_roots: config_arguments.use_config_roots(),
            },
            cache_root,
            no_cache: args.no_cache || fix_applicability.is_some(),
            cache_tag: b"project-cache-key",
        },
        |project_root| {
            resolve_project_check_settings(project_root, config_arguments, &args.rule_selection)
        },
        |_, files, settings| Ok(CheckCacheSettings::new(settings, files)),
    )?;
    let shellcheck_map = ShellCheckCodeMap::default();

    let mut report = CheckReport::default();

    for run in &mut runs {
        if !run.settings.embedded_enabled {
            run.files.retain(|file| file.kind == FileKind::Shell);
        }
    }

    for mut run in runs {
        let project_settings = run.settings.clone();
        let analyzed_paths = LinterSettings::analyzed_path_set(
            run.files
                .iter()
                .filter(|file| file.kind == FileKind::Shell)
                .map(|file| file.absolute_path.clone()),
        );
        let linter_settings = project_settings
            .linter_settings
            .clone()
            .with_analyzed_path_set(analyzed_paths);
        let pending = run.take_pending_files(|file, cached| {
            report.cache_hits += 1;
            let source = (include_source && !cached.diagnostics.is_empty())
                .then(|| read_shared_source(&file.absolute_path))
                .transpose()?;
            push_cached_diagnostics(
                &mut report,
                &file.display_path,
                &file.relative_path,
                &file.absolute_path,
                &cached.diagnostics,
                source,
            );
            Ok(())
        })?;

        let results = pending
            .into_par_iter()
            .map(|pending| {
                analyze_file(
                    pending,
                    &linter_settings,
                    &shellcheck_map,
                    include_source,
                    fix_applicability,
                    &project_settings.fixable_rules,
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
    let mut runs = prepare_project_runs::<CheckCacheData, ResolvedCheckSettings, _>(
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
        |project_root| {
            resolve_project_check_settings(project_root, config_arguments, &args.rule_selection)
        },
    )?;

    let mut report = AddIgnoreReport::default();

    for run in &mut runs {
        run.files.retain(|file| file.kind == FileKind::Shell);
    }

    for run in runs {
        let analyzed_paths = run
            .files
            .iter()
            .map(|file| file.absolute_path.clone())
            .collect::<Vec<_>>();
        let linter_settings = run
            .settings
            .linter_settings
            .clone()
            .with_analyzed_paths(analyzed_paths);

        for file in run.files {
            let result = add_ignores_to_path(&file.absolute_path, &linter_settings, reason)?;
            report.directives_added += result.directives_added;
            if result.parse_error.is_none() && result.diagnostics.is_empty() {
                continue;
            }

            let raw_source = read_shared_source(&file.absolute_path)?;
            let source = include_source.then_some(raw_source.clone());
            if let Some(error) = result.parse_error {
                report.diagnostics.push(display_parse_error(
                    &file.display_path,
                    &file.relative_path,
                    &file.absolute_path,
                    error.line,
                    error.column,
                    error.message,
                    source.clone(),
                ));
            }
            push_lint_diagnostics(
                &mut report.diagnostics,
                &file.display_path,
                &file.relative_path,
                &file.absolute_path,
                &result.diagnostics,
                &raw_source,
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
            rule_selection: RuleSelectionArgs::default(),
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
    fixable_rules: &RuleSet,
) -> Result<FileCheckResult> {
    match pending.file.kind {
        FileKind::Shell => analyze_shell_file(
            pending,
            base_linter_settings,
            shellcheck_map,
            include_source,
            fix_applicability,
            fixable_rules,
        ),
        FileKind::Embedded => analyze_embedded_file(
            pending,
            base_linter_settings,
            shellcheck_map,
            include_source,
        ),
    }
}

fn analyze_shell_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
    fix_applicability: Option<Applicability>,
    fixable_rules: &RuleSet,
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
        &pending.file.absolute_path,
    );
    let mut fixes_applied = 0;

    if let Some(applicability) = fix_applicability {
        let fixable_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| fixable_rules.contains(diagnostic.rule))
            .cloned()
            .collect::<Vec<_>>();
        let applied = shuck_linter::apply_fixes(&source, &fixable_diagnostics, applicability);
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
                &pending.file.absolute_path,
            );
            fixes_applied = applied.fixes_applied;
        }
    }

    let diagnostics = if parse_result.is_err() && diagnostics.is_empty() {
        let ParseError::Parse {
            message,
            line,
            column,
        } = parse_result.strict_error();
        vec![display_parse_error(
            &pending.file.display_path,
            &pending.file.relative_path,
            &pending.file.absolute_path,
            line,
            column,
            message,
            include_source.then_some(source.clone()),
        )]
    } else {
        display_lint_diagnostics(&pending, &source, &diagnostics, include_source)
    };
    let cache_data = CheckCacheData::from_displayed(&diagnostics);

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data,
        diagnostics,
        fixes_applied,
    })
}

fn analyze_embedded_file(
    pending: PendingProjectFile,
    base_linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    include_source: bool,
) -> Result<FileCheckResult> {
    let host_source = read_shared_source(&pending.file.absolute_path)?;
    let host_display_source = include_source.then_some(host_source.clone());
    let extracted = match extract_all(&pending.file.absolute_path, host_source.as_ref()) {
        Ok(extracted) => extracted,
        Err(err) => {
            let diagnostics = vec![display_parse_error(
                &pending.file.display_path,
                &pending.file.relative_path,
                &pending.file.absolute_path,
                1,
                1,
                err.to_string(),
                host_display_source,
            )];
            return Ok(FileCheckResult {
                file: pending.file,
                file_key: pending.file_key,
                cache_data: CheckCacheData::from_displayed(&diagnostics),
                diagnostics,
                fixes_applied: 0,
            });
        }
    };

    let mut displayed = Vec::new();

    for embedded in extracted.into_iter().filter(embedded_supported_dialect) {
        let Some((shell_dialect, parse_dialect)) = embedded_dialects(embedded.dialect) else {
            continue;
        };

        let snippet_source: Arc<str> = Arc::from(embedded.source.clone());
        let parse_result = Parser::with_dialect(&snippet_source, parse_dialect).parse();
        let linter_settings = base_linter_settings
            .clone()
            .with_shell(shell_dialect)
            .with_ambient_shell_options(AmbientShellOptions {
                errexit: embedded.implicit_flags.errexit,
                pipefail: embedded.implicit_flags.pipefail,
            });
        let diagnostics = collect_lint_diagnostics(
            &pending,
            &snippet_source,
            &parse_result,
            &linter_settings,
            shellcheck_map,
            &pending.file.absolute_path,
        )
        .into_iter()
        .filter(|diagnostic| embedded_rule_allowed(diagnostic.rule))
        .collect::<Vec<_>>();

        if parse_result.is_err() && diagnostics.is_empty() {
            let ParseError::Parse {
                message,
                line,
                column,
            } = parse_result.strict_error();
            displayed.push(remap_embedded_parse_error(
                &pending,
                &embedded,
                line,
                column,
                prefixed_embedded_message(&embedded, &message),
                host_display_source.clone(),
            ));
            continue;
        }

        displayed.extend(remap_embedded_lint_diagnostics(
            &pending,
            &embedded,
            &diagnostics,
            host_display_source.clone(),
        ));
    }

    Ok(FileCheckResult {
        file: pending.file,
        file_key: pending.file_key,
        cache_data: CheckCacheData::from_displayed(&displayed),
        diagnostics: displayed,
        fixes_applied: 0,
    })
}

fn collect_lint_diagnostics(
    _pending: &PendingProjectFile,
    source: &Arc<str>,
    parse_result: &ParseResult,
    linter_settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
    source_path: &Path,
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
    shuck_linter::lint_file(
        parse_result,
        source,
        &indexer,
        linter_settings,
        suppression_index.as_ref(),
        Some(source_path),
    )
}

fn display_lint_diagnostics(
    pending: &PendingProjectFile,
    source: &Arc<str>,
    diagnostics: &[shuck_linter::Diagnostic],
    include_source: bool,
) -> Vec<DisplayedDiagnostic> {
    let diagnostic_source = (!diagnostics.is_empty() && include_source).then(|| source.clone());

    diagnostics
        .iter()
        .map(|diagnostic| DisplayedDiagnostic {
            path: pending.file.display_path.clone(),
            relative_path: pending.file.relative_path.clone(),
            absolute_path: pending.file.absolute_path.clone(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: displayed_fix_from_diagnostic(diagnostic, source),
            source: diagnostic_source.clone(),
        })
        .collect()
}

fn display_parse_error(
    display_path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    line: usize,
    column: usize,
    message: String,
    source: Option<Arc<str>>,
) -> DisplayedDiagnostic {
    DisplayedDiagnostic {
        path: display_path.to_path_buf(),
        relative_path: relative_path.to_path_buf(),
        absolute_path: absolute_path.to_path_buf(),
        span: DisplaySpan::point(line, column),
        message,
        kind: DisplayedDiagnosticKind::ParseError,
        fix: None,
        source,
    }
}

fn displayed_fix_from_diagnostic(
    diagnostic: &shuck_linter::Diagnostic,
    source: &str,
) -> Option<DisplayedFix> {
    let fix = diagnostic.fix.as_ref()?;
    let line_index = shuck_indexer::LineIndex::new(source);

    Some(DisplayedFix {
        applicability: match fix.applicability() {
            Applicability::Safe => DisplayedApplicability::Safe,
            Applicability::Unsafe => DisplayedApplicability::Unsafe,
        },
        message: diagnostic.fix_title.clone(),
        edits: fix
            .edits()
            .iter()
            .map(|edit| displayed_edit_from_fix(edit, &line_index, source))
            .collect(),
    })
}

fn displayed_edit_from_fix(
    edit: &shuck_linter::Edit,
    line_index: &shuck_indexer::LineIndex,
    source: &str,
) -> DisplayedEdit {
    let range = edit.range();
    let start_offset = floor_char_boundary(source, usize::from(range.start()));
    let end_offset = ceil_char_boundary(source, usize::from(range.end()));

    DisplayedEdit {
        location: display_position_at_offset(source, line_index, start_offset),
        end_location: display_position_at_offset(source, line_index, end_offset),
        content: edit.content().to_owned(),
    }
}

fn display_position_at_offset(
    source: &str,
    line_index: &shuck_indexer::LineIndex,
    target_offset: usize,
) -> DisplayPosition {
    let line = line_index.line_number(TextSize::new(target_offset as u32));
    let line_start = line_index
        .line_start(line)
        .map(usize::from)
        .unwrap_or_default();

    DisplayPosition::new(line, source[line_start..target_offset].chars().count() + 1)
}

fn floor_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset < source.len() && !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
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

fn push_cached_diagnostics(
    report: &mut CheckReport,
    path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    diagnostics: &[CachedDisplayedDiagnostic],
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        report.diagnostics.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            relative_path: relative_path.to_path_buf(),
            absolute_path: absolute_path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.start_line, diagnostic.start_column),
                DisplayPosition::new(diagnostic.end_line, diagnostic.end_column),
            ),
            message: diagnostic.message.clone(),
            kind: match &diagnostic.kind {
                CachedDisplayedDiagnosticKind::ParseError => DisplayedDiagnosticKind::ParseError,
                CachedDisplayedDiagnosticKind::Lint { code, severity } => {
                    DisplayedDiagnosticKind::Lint {
                        code: code.clone(),
                        severity: severity.clone(),
                    }
                }
            },
            fix: diagnostic.fix.as_ref().map(CachedLintFix::to_displayed),
            source: source.clone(),
        });
    }
}

fn push_lint_diagnostics(
    displayed: &mut Vec<DisplayedDiagnostic>,
    path: &Path,
    relative_path: &Path,
    absolute_path: &Path,
    diagnostics: &[shuck_linter::Diagnostic],
    raw_source: &Arc<str>,
    source: Option<Arc<str>>,
) {
    for diagnostic in diagnostics {
        displayed.push(DisplayedDiagnostic {
            path: path.to_path_buf(),
            relative_path: relative_path.to_path_buf(),
            absolute_path: absolute_path.to_path_buf(),
            span: DisplaySpan::new(
                DisplayPosition::new(diagnostic.span.start.line, diagnostic.span.start.column),
                DisplayPosition::new(diagnostic.span.end.line, diagnostic.span.end.column),
            ),
            message: diagnostic.message.clone(),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: displayed_fix_from_diagnostic(diagnostic, raw_source),
            source: source.clone(),
        });
    }
}

fn embedded_supported_dialect(embedded: &EmbeddedScript) -> bool {
    !matches!(embedded.dialect, ExtractedDialect::Unsupported)
}

fn embedded_dialects(
    dialect: ExtractedDialect,
) -> Option<(ShellDialect, shuck_parser::ShellDialect)> {
    match dialect {
        ExtractedDialect::Bash => Some((ShellDialect::Bash, shuck_parser::ShellDialect::Bash)),
        ExtractedDialect::Sh => Some((ShellDialect::Sh, shuck_parser::ShellDialect::Posix)),
        ExtractedDialect::Unsupported => None,
    }
}

fn embedded_rule_allowed(rule: Rule) -> bool {
    !matches!(
        rule,
        Rule::NonAbsoluteShebang
            | Rule::IndentedShebang
            | Rule::SpaceAfterHashBang
            | Rule::ShebangNotOnFirstLine
            | Rule::MissingShebangLine
            | Rule::DuplicateShebangFlag
            | Rule::DynamicSourcePath
            | Rule::UntrackedSourceFile
    )
}

fn remap_embedded_lint_diagnostics(
    pending: &PendingProjectFile,
    embedded: &EmbeddedScript,
    diagnostics: &[shuck_linter::Diagnostic],
    source: Option<Arc<str>>,
) -> Vec<DisplayedDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| DisplayedDiagnostic {
            path: pending.file.display_path.clone(),
            relative_path: pending.file.relative_path.clone(),
            absolute_path: pending.file.absolute_path.clone(),
            span: remap_embedded_span(
                embedded,
                diagnostic.span.start.line,
                diagnostic.span.start.column,
                diagnostic.span.end.line,
                diagnostic.span.end.column,
            ),
            message: prefixed_embedded_message(embedded, &diagnostic.message),
            kind: DisplayedDiagnosticKind::Lint {
                code: diagnostic.code().to_owned(),
                severity: diagnostic.severity.as_str().to_owned(),
            },
            fix: None,
            source: source.clone(),
        })
        .collect()
}

fn remap_embedded_parse_error(
    pending: &PendingProjectFile,
    embedded: &EmbeddedScript,
    line: usize,
    column: usize,
    message: String,
    source: Option<Arc<str>>,
) -> DisplayedDiagnostic {
    let position = remap_embedded_position(embedded, line, column);
    DisplayedDiagnostic {
        path: pending.file.display_path.clone(),
        relative_path: pending.file.relative_path.clone(),
        absolute_path: pending.file.absolute_path.clone(),
        span: DisplaySpan::point(position.line, position.column),
        message,
        kind: DisplayedDiagnosticKind::ParseError,
        fix: None,
        source,
    }
}

fn remap_embedded_span(
    embedded: &EmbeddedScript,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
) -> DisplaySpan {
    DisplaySpan::new(
        remap_embedded_position(embedded, start_line, start_column),
        remap_embedded_position(embedded, end_line, end_column),
    )
}

fn remap_embedded_position(
    embedded: &EmbeddedScript,
    line: usize,
    column: usize,
) -> DisplayPosition {
    let snippet_line = line.max(1);
    let host_line_start = embedded
        .host_line_starts
        .get(snippet_line.saturating_sub(1))
        .copied()
        .unwrap_or(HostLineStart {
            line: embedded.host_start_line + snippet_line.saturating_sub(1),
            column: embedded.host_start_column,
        });
    remap_embedded_column(embedded, snippet_line, host_line_start, column)
}

fn remap_embedded_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    host_line_start: HostLineStart,
    snippet_column: usize,
) -> DisplayPosition {
    let decoded_column = remap_placeholder_column(embedded, snippet_line, snippet_column);
    remap_decoded_yaml_column(embedded, snippet_line, host_line_start, decoded_column)
}

fn remap_placeholder_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    snippet_column: usize,
) -> usize {
    let local_column = snippet_column.saturating_sub(1);
    let mut cumulative_delta = 0isize;

    for placeholder in &embedded.placeholders {
        let (placeholder_line, placeholder_column) =
            source_line_column_for_offset(&embedded.source, placeholder.substituted_span.start);
        if placeholder_line != snippet_line {
            continue;
        }

        let substituted_start = placeholder_column.saturating_sub(1);
        let substituted_len = span_char_len(&embedded.source, &placeholder.substituted_span);
        let substituted_end = substituted_start + substituted_len;
        let host_len = placeholder.original.chars().count();

        if local_column >= substituted_end {
            cumulative_delta += host_len as isize - substituted_len as isize;
            continue;
        }

        if local_column >= substituted_start {
            let decoded_start = substituted_start as isize + cumulative_delta;
            let relative = local_column - substituted_start;
            let mapped = decoded_start + relative.min(host_len.saturating_sub(1)) as isize;
            return mapped.max(0) as usize + 1;
        }
    }

    (local_column as isize + cumulative_delta).max(0) as usize + 1
}

fn remap_decoded_yaml_column(
    embedded: &EmbeddedScript,
    snippet_line: usize,
    host_line_start: HostLineStart,
    decoded_column: usize,
) -> DisplayPosition {
    let mut segment = host_line_start;
    let mut segment_column = 1usize;

    for mapping in embedded
        .host_column_mappings
        .iter()
        .filter(|mapping| mapping.line == snippet_line && mapping.column <= decoded_column)
    {
        segment = HostLineStart {
            line: mapping.host_line,
            column: mapping.host_column,
        };
        segment_column = mapping.column;
    }

    DisplayPosition::new(
        segment.line,
        segment.column + decoded_column.saturating_sub(segment_column),
    )
}

fn source_line_column_for_offset(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;

    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }

    (line, column)
}

fn span_char_len(source: &str, span: &std::ops::Range<usize>) -> usize {
    source
        .get(span.clone())
        .map_or(0, |value| value.chars().count())
}

fn prefixed_embedded_message(embedded: &EmbeddedScript, message: &str) -> String {
    format!("{}: {message}", embedded.label)
}

fn read_shared_source(path: &Path) -> Result<Arc<str>> {
    Ok(Arc::<str>::from(fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use notify::event::{CreateKind, EventAttributes, ModifyKind, RemoveKind, RenameMode};
    use shuck_extract::{EmbeddedFormat, ImplicitShellFlags};
    use shuck_linter::{Category, Rule, RuleSelector};
    use tempfile::tempdir;

    use super::*;
    use crate::args::{CheckOutputFormatArg, PatternRuleSelectorPair, RuleSelectionArgs};
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
                kind: FileKind::Shell,
            },
            file_key: shuck_cache::FileCacheKey::from_path(path).unwrap(),
        }
    }

    fn cache_root(cwd: &Path) -> PathBuf {
        cwd.join("cache")
    }

    fn diagnostic_paths(path: &str) -> (PathBuf, PathBuf, PathBuf) {
        let display = PathBuf::from(path);
        let relative = PathBuf::from(path);
        let absolute = PathBuf::from(format!("/tmp/{path}"));
        (display, relative, absolute)
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
            rule_selection: RuleSelectionArgs::default(),
            file_selection: FileSelectionArgs::default(),
            exit_zero: false,
            exit_non_zero_on_fix: false,
        }
    }

    fn check_args(no_cache: bool) -> CheckCommand {
        check_args_with_format(no_cache, CheckOutputFormatArg::Full)
    }

    fn lint_displayed_diagnostic(
        path: &str,
        span: DisplaySpan,
        message: &str,
        code: &str,
        severity: &str,
    ) -> DisplayedDiagnostic {
        let (path, relative_path, absolute_path) = diagnostic_paths(path);
        DisplayedDiagnostic {
            path,
            relative_path,
            absolute_path,
            span,
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: code.to_owned(),
                severity: severity.to_owned(),
            },
            fix: None,
            source: None,
        }
    }

    fn parse_displayed_diagnostic(
        path: &str,
        span: DisplaySpan,
        message: &str,
    ) -> DisplayedDiagnostic {
        let (path, relative_path, absolute_path) = diagnostic_paths(path);
        DisplayedDiagnostic {
            path,
            relative_path,
            absolute_path,
            span,
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::ParseError,
            fix: None,
            source: None,
        }
    }

    fn diagnostic_codes(report: &CheckReport) -> Vec<String> {
        report
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match &diagnostic.kind {
                DisplayedDiagnosticKind::Lint { code, .. } => Some(code.clone()),
                DisplayedDiagnosticKind::ParseError => None,
            })
            .collect()
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
    fn checks_embedded_github_actions_workflows() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::UnusedAssignment.code().to_owned()]
        );
        assert_eq!(
            report.diagnostics[0].path,
            PathBuf::from(".github/workflows/ci.yml")
        );
        assert_eq!(report.diagnostics[0].span.start.line, 7);
        assert_eq!(report.diagnostics[0].span.start.column, 11);
        assert!(
            report.diagnostics[0]
                .message
                .starts_with("jobs.test.steps[0].run:")
        );
        assert!(
            report.diagnostics[0]
                .source
                .as_deref()
                .is_some_and(|source| source.contains("on: push"))
        );
    }

    #[test]
    fn skips_default_windows_shell_steps() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  windows:
    runs-on: windows-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn can_disable_embedded_workflow_checks_in_config() {
        let tempdir = tempdir().unwrap();
        let workflows = tempdir.path().join(".github/workflows");
        fs::create_dir_all(&workflows).unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[check]\nembedded = false\n",
        )
        .unwrap();
        fs::write(
            workflows.join("ci.yml"),
            r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: |
          unused=1
          echo ok
"#,
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn remaps_embedded_columns_on_later_lines() {
        let embedded = EmbeddedScript {
            source: "echo hi\necho bye\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![
                HostLineStart { line: 7, column: 9 },
                HostLineStart { line: 8, column: 9 },
                HostLineStart { line: 9, column: 9 },
            ],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 2, 5);
        assert_eq!(position.line, 8);
        assert_eq!(position.column, 13);
    }

    #[test]
    fn remaps_columns_after_placeholder_expansion_on_the_same_line() {
        let embedded = EmbeddedScript {
            source: "echo ${_SHUCK_GHA_1}$FOO\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![HostLineStart { line: 7, column: 9 }],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: vec![shuck_extract::PlaceholderMapping {
                name: "_SHUCK_GHA_1".to_owned(),
                original: "${{ github.ref }}".to_owned(),
                expression: "github.ref".to_owned(),
                taint: shuck_extract::ExpressionTaint::Trusted,
                substituted_span: 5..20,
                host_span: 5..22,
            }],
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 21);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 31);
    }

    #[test]
    fn remaps_columns_after_non_ascii_placeholder_expansion() {
        let embedded = EmbeddedScript {
            source: "echo ${_SHUCK_GHA_1}$FOO\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 9,
            host_line_starts: vec![HostLineStart { line: 7, column: 9 }],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: vec![shuck_extract::PlaceholderMapping {
                name: "_SHUCK_GHA_1".to_owned(),
                original: "${{ github.refé }}".to_owned(),
                expression: "github.refé".to_owned(),
                taint: shuck_extract::ExpressionTaint::Trusted,
                substituted_span: 5..20,
                host_span: 5..24,
            }],
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 21);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 32);
    }

    #[test]
    fn remaps_positions_for_escaped_yaml_newlines() {
        let embedded = EmbeddedScript {
            source: "echo hi\nif true\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 15,
            host_line_starts: vec![
                HostLineStart {
                    line: 7,
                    column: 15,
                },
                HostLineStart {
                    line: 7,
                    column: 24,
                },
                HostLineStart {
                    line: 7,
                    column: 33,
                },
            ],
            host_column_mappings: Vec::new(),
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 2, 1);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 24);
    }

    #[test]
    fn remaps_columns_after_non_newline_yaml_escapes() {
        let embedded = EmbeddedScript {
            source: "echo\tif true\n".to_owned(),
            host_offset: 0,
            host_start_line: 7,
            host_start_column: 15,
            host_line_starts: vec![HostLineStart {
                line: 7,
                column: 15,
            }],
            host_column_mappings: vec![shuck_extract::HostColumnMapping {
                line: 1,
                column: 6,
                host_line: 7,
                host_column: 21,
            }],
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 6);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 21);
    }

    #[test]
    fn remaps_columns_after_folded_double_quoted_yaml_newlines() {
        let embedded = EmbeddedScript {
            source: "echo ok ; unused=1\n".to_owned(),
            host_offset: 0,
            host_start_line: 6,
            host_start_column: 15,
            host_line_starts: vec![HostLineStart {
                line: 6,
                column: 15,
            }],
            host_column_mappings: vec![shuck_extract::HostColumnMapping {
                line: 1,
                column: 9,
                host_line: 7,
                host_column: 13,
            }],
            dialect: ExtractedDialect::Bash,
            label: "jobs.test.steps[0].run".to_owned(),
            format: EmbeddedFormat::GitHubActions,
            placeholders: Vec::new(),
            implicit_flags: ImplicitShellFlags::default(),
        };

        let position = remap_embedded_position(&embedded, 1, 9);
        assert_eq!(position.line, 7);
        assert_eq!(position.column, 13);
    }

    #[test]
    fn exit_zero_suppresses_only_non_fatal_diagnostics() {
        let warning = lint_displayed_diagnostic(
            "warn.sh",
            DisplaySpan::point(1, 1),
            "lint",
            "C001",
            "warning",
        );
        let error_lint =
            lint_displayed_diagnostic("err.sh", DisplaySpan::point(1, 1), "lint", "C035", "error");
        let parse = parse_displayed_diagnostic("broken.sh", DisplaySpan::point(1, 1), "parse");

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
    fn select_replaces_default_rules() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/sh\nunused=1\nread\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            select: Some(vec![RuleSelector::Rule(Rule::BareRead)]),
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::BareRead.code().to_owned()]
        );
    }

    #[test]
    fn style_rules_are_disabled_by_default() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/bash\nprintf '%s\\n' x &;\n",
        )
        .unwrap();

        let report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn extend_select_can_reenable_s074() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/bash\nprintf '%s\\n' x &;\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            extend_select: vec![RuleSelector::Rule(Rule::AmpersandSemicolon)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::AmpersandSemicolon.code().to_owned()]
        );
    }

    #[test]
    fn extend_select_category_reenables_style_rules() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/bash\nprintf '%s\\n' x &;\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            extend_select: vec![RuleSelector::Category(Category::Style)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::AmpersandSemicolon.code().to_owned()]
        );
    }

    #[test]
    fn select_all_includes_style_rules() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/bash\nprintf '%s\\n' x &;\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            select: Some(vec![RuleSelector::All]),
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::AmpersandSemicolon.code().to_owned()]
        );
    }

    #[test]
    fn extend_select_adds_on_top_of_config_selection() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/sh\nunused=1\nread\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            extend_select: vec![RuleSelector::Rule(Rule::BareRead)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        let mut codes = diagnostic_codes(&report);
        codes.sort();
        assert_eq!(
            codes,
            vec![
                Rule::UnusedAssignment.code().to_owned(),
                Rule::BareRead.code().to_owned(),
            ]
        );
    }

    #[test]
    fn ignore_can_trigger_parse_error_fallback() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("broken.sh"),
            "#!/bin/sh\nif true; then\n  :\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            ignore: vec![RuleSelector::Rule(Rule::MissingFi)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        assert!(matches!(
            report.diagnostics[0].kind,
            DisplayedDiagnosticKind::ParseError
        ));
    }

    #[test]
    fn per_file_ignores_suppress_matching_rules_only() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("ignored.sh"),
            "#!/bin/bash\nunused=1\necho ok\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("kept.sh"),
            "#!/bin/bash\nunused=1\necho ok\n",
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            per_file_ignores: Some(vec![PatternRuleSelectorPair {
                pattern: "ignored.sh".to_owned(),
                selector: RuleSelector::Rule(Rule::UnusedAssignment),
            }]),
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].path, PathBuf::from("kept.sh"));
        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::UnusedAssignment.code().to_owned()]
        );
    }

    #[test]
    fn rejects_empty_rule_selectors() {
        let error = parse_rule_selectors(&[String::new()], "lint.select").unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid lint.select selector: selector cannot be empty"
        );
    }

    #[test]
    fn unfixable_rules_prevent_fix_application() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("warn.sh");
        let source = "#!/bin/bash\nprintf '%s\\n' x &;\n";
        fs::write(&script, source).unwrap();

        let mut args = check_args(true);
        args.fix = true;
        args.rule_selection = RuleSelectionArgs {
            extend_select: vec![RuleSelector::Rule(Rule::AmpersandSemicolon)],
            unfixable: vec![RuleSelector::Rule(Rule::AmpersandSemicolon)],
            ..RuleSelectionArgs::default()
        };

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        assert_eq!(report.fixes_applied, 0);
        assert_eq!(fs::read_to_string(script).unwrap(), source);
        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::AmpersandSemicolon.code().to_owned()]
        );
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
            &RuleSet::all(),
        )
        .unwrap();

        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.cache_data.diagnostics.len(), 1);
        assert!(matches!(
            result.cache_data.diagnostics[0].kind,
            CachedDisplayedDiagnosticKind::ParseError
        ));
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
    fn cache_key_includes_analyzed_path_set() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("main.sh"),
            "#!/bin/sh\n. ./helper.sh\nprintf '%s\\n' \"$from_helper\"\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("helper.sh"),
            "#!/bin/sh\nfrom_helper=ok\n",
        )
        .unwrap();

        let mut narrow_args = check_args(false);
        narrow_args.paths = vec![PathBuf::from("main.sh")];
        narrow_args.rule_selection.select =
            Some(vec![RuleSelector::Rule(Rule::UntrackedSourceFile)]);

        let mut broad_args = narrow_args.clone();
        broad_args.paths = vec![PathBuf::from("main.sh"), PathBuf::from("helper.sh")];

        let narrow = run_check_with_cwd(
            &narrow_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(narrow.cache_hits, 0);
        assert_eq!(narrow.cache_misses, 1);
        assert_eq!(diagnostic_codes(&narrow), vec!["C003"]);

        let broad = run_check_with_cwd(
            &broad_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(broad.cache_hits, 0);
        assert_eq!(broad.cache_misses, 2);
        assert!(
            broad.diagnostics.is_empty(),
            "{:?}",
            diagnostic_codes(&broad)
        );

        let broad_again = run_check_with_cwd(
            &broad_args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(broad_again.cache_hits, 2);
        assert_eq!(broad_again.cache_misses, 0);
        assert!(
            broad_again.diagnostics.is_empty(),
            "{:?}",
            diagnostic_codes(&broad_again)
        );
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
    fn invalidates_cache_when_rule_options_change() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(
            &script,
            "#!/bin/bash\ntarget=ok\nname=target\nprintf '%s\\n' \"${!name}\"\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert_eq!(first.diagnostics.len(), 1);
        assert!(first.diagnostics[0].message.contains("target"));

        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C001']\n\n[lint.rule-options.c001]\ntreat-indirect-expansion-targets-as-used = true\n",
        )
        .unwrap();

        let second = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(second.cache_hits, 0);
        assert_eq!(second.cache_misses, 1);
        assert!(second.diagnostics.is_empty());
    }

    #[test]
    fn invalidates_cache_when_c063_rule_options_change() {
        let tempdir = tempdir().unwrap();
        let script = tempdir.path().join("script.sh");
        fs::write(&script, "#!/bin/bash\nouter() {\n  inner() { :; }\n}\n").unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C063']\n",
        )
        .unwrap();

        let first = run_check_with_cwd(
            &check_args(false),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();
        assert_eq!(first.cache_hits, 0);
        assert_eq!(first.cache_misses, 1);
        assert!(first.diagnostics.is_empty());

        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['C063']\n\n[lint.rule-options.c063]\nreport-unreached-nested-definitions = true\n",
        )
        .unwrap();

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
                source: Some(Arc::<str>::from("echo ok\nvalue=$foo\nprintf '%s' $bar\n")),
                ..lint_displayed_diagnostic(
                    "script.sh",
                    DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
                    "example message",
                    "C014",
                    "warning",
                )
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
            diagnostics: vec![parse_displayed_diagnostic(
                "script.sh",
                DisplaySpan::point(2, 7),
                "unterminated construct",
            )],
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
            &path,
        );
        let diagnostics = display_lint_diagnostics(&pending, &source, &diagnostics, true);

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
