use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use globset::{Glob, GlobMatcher};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_linter::{
    CompiledPerFileIgnoreList, LinterSettings, PerFileIgnore, Rule, RuleSelector, RuleSet,
    ShellDialect,
};

use crate::args::{PatternRuleSelectorPair, PatternShellPair, RuleSelectionArgs};
use crate::config::{ConfigArguments, LintConfig, load_project_config};
use crate::discover::{ProjectRoot, normalize_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EffectiveCheckSettings {
    enabled_rules: Vec<String>,
    per_file_ignores: Vec<EffectivePerFileIgnore>,
    per_file_shell: Vec<EffectivePerFileShell>,
    rule_options: EffectiveRuleOptions,
    embedded_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectivePerFileIgnore {
    pattern: String,
    rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectivePerFileShell {
    pattern: String,
    shell: String,
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
        per_file_shell: &[PerFileShell],
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

        let per_file_shell = per_file_shell
            .iter()
            .map(|shell| EffectivePerFileShell {
                pattern: shell.pattern.clone(),
                shell: shell_name(shell.shell).to_owned(),
            })
            .collect::<Vec<_>>();

        Self {
            enabled_rules,
            per_file_ignores,
            per_file_shell,
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
        self.per_file_shell.cache_key(state);
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

impl CacheKey for EffectivePerFileShell {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.pattern.cache_key(state);
        self.shell.cache_key(state);
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
pub(super) struct ResolvedCheckSettings {
    pub(super) linter_settings: LinterSettings,
    pub(super) per_file_shell: Arc<CompiledPerFileShellList>,
    pub(super) fixable_rules: RuleSet,
    pub(super) effective: EffectiveCheckSettings,
    pub(super) embedded_enabled: bool,
}

impl CacheKey for ResolvedCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.effective.cache_key(state);
    }
}
#[derive(Debug, Clone, Default)]
struct RuleSelectionLayer {
    select: Option<Vec<RuleSelector>>,
    ignore: Vec<RuleSelector>,
    extend_select: Vec<RuleSelector>,
    per_file_ignores: Option<Vec<PerFileIgnoreSpec>>,
    extend_per_file_ignores: Vec<PerFileIgnoreSpec>,
    per_file_shell: Option<Vec<PerFileShell>>,
    extend_per_file_shell: Vec<PerFileShell>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PerFileShell {
    pub(super) pattern: String,
    pub(super) shell: ShellDialect,
}

#[derive(Debug, Clone)]
pub(super) struct CompiledPerFileShellList {
    project_root: PathBuf,
    entries: Vec<CompiledPerFileShell>,
}

#[derive(Debug, Clone)]
struct CompiledPerFileShell {
    basename_matcher: GlobMatcher,
    relative_matcher: GlobMatcher,
    absolute_matcher: GlobMatcher,
    negated: bool,
    shell: ShellDialect,
}

impl CompiledPerFileShellList {
    pub(super) fn resolve(
        project_root: impl Into<PathBuf>,
        per_file_shell: impl IntoIterator<Item = PerFileShell>,
    ) -> Result<Self> {
        let entries = per_file_shell
            .into_iter()
            .map(|per_file_shell| {
                let mut pattern = per_file_shell.pattern;
                let negated = pattern.starts_with('!');
                if negated {
                    pattern.drain(..1);
                }

                Ok(CompiledPerFileShell {
                    basename_matcher: Glob::new(&pattern)
                        .map_err(|err| anyhow!("invalid glob {pattern:?}: {err}"))?
                        .compile_matcher(),
                    relative_matcher: Glob::new(&pattern)
                        .map_err(|err| anyhow!("invalid glob {pattern:?}: {err}"))?
                        .compile_matcher(),
                    absolute_matcher: Glob::new(&pattern)
                        .map_err(|err| anyhow!("invalid glob {pattern:?}: {err}"))?
                        .compile_matcher(),
                    negated,
                    shell: per_file_shell.shell,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            project_root: project_root.into(),
            entries,
        })
    }

    pub(super) fn shell_for_path(&self, path: &Path) -> Option<ShellDialect> {
        let relative_path = path.strip_prefix(&self.project_root).unwrap_or(path);
        let file_name = relative_path.file_name().or_else(|| path.file_name())?;

        self.entries.iter().fold(None, |shell, entry| {
            let matches = entry.basename_matcher.is_match(file_name)
                || entry.relative_matcher.is_match(relative_path)
                || per_file_shell_absolute_match(&entry.absolute_matcher, path);
            let applies = if entry.negated { !matches } else { matches };

            if applies { Some(entry.shell) } else { shell }
        })
    }
}

fn per_file_shell_absolute_match(matcher: &GlobMatcher, path: &Path) -> bool {
    matcher.is_match(path)
        || matcher.is_match(normalize_path(path))
        || slash_normalized_match_path(path)
            .as_deref()
            .is_some_and(|normalized| matcher.is_match(normalized))
        || normalized_absolute_shell_match_path(path)
            .as_ref()
            .is_some_and(|normalized| {
                matcher.is_match(normalized)
                    || slash_normalized_match_path(normalized)
                        .as_deref()
                        .is_some_and(|slash_normalized| matcher.is_match(slash_normalized))
            })
}

fn normalized_absolute_shell_match_path(path: &Path) -> Option<PathBuf> {
    let path = path.to_string_lossy();

    if let Some(stripped) = path.strip_prefix(r"\\?\UNC\") {
        return Some(PathBuf::from(format!(r"\\{stripped}")));
    }

    path.strip_prefix(r"\\?\").map(PathBuf::from)
}

fn slash_normalized_match_path(path: &Path) -> Option<PathBuf> {
    let path = path.to_string_lossy();
    path.contains('\\')
        .then(|| PathBuf::from(path.replace('\\', "/")))
}
pub(super) fn resolve_project_check_settings(
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
    let mut per_file_shell = Vec::new();

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
        per_file_shell = apply_per_file_shell_layer(
            per_file_shell,
            layer.per_file_shell,
            layer.extend_per_file_shell,
        );
    }

    let compiled_per_file_ignores = CompiledPerFileIgnoreList::resolve(
        project_root.canonical_root.clone(),
        per_file_ignores.clone(),
    )?;
    let compiled_per_file_shell = CompiledPerFileShellList::resolve(
        project_root.canonical_root.clone(),
        per_file_shell.clone(),
    )?;
    let embedded_enabled = config.check.embedded.unwrap_or(true);
    let effective = EffectiveCheckSettings::new(
        enabled_rules,
        &per_file_ignores,
        &per_file_shell,
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
        per_file_shell: Arc::new(compiled_per_file_shell),
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
        per_file_shell: lint
            .per_file_shell
            .as_ref()
            .map(|patterns| parse_per_file_shell_map(patterns, "lint.per-file-shell"))
            .transpose()?,
        extend_per_file_shell: lint
            .extend_per_file_shell
            .as_ref()
            .map(|patterns| parse_per_file_shell_map(patterns, "lint.extend-per-file-shell"))
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
        per_file_shell: args
            .per_file_shell
            .as_ref()
            .map(|pairs| pairs.iter().map(per_file_shell_from_pair).collect()),
        extend_per_file_shell: args
            .extend_per_file_shell
            .iter()
            .map(per_file_shell_from_pair)
            .collect(),
        fixable: args.fixable.clone(),
        unfixable: args.unfixable.clone(),
        extend_fixable: args.extend_fixable.clone(),
    }
}

pub(super) fn parse_rule_selectors(selectors: &[String], scope: &str) -> Result<Vec<RuleSelector>> {
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

fn parse_per_file_shell_map(
    patterns: &BTreeMap<String, String>,
    scope: &str,
) -> Result<Vec<PerFileShell>> {
    patterns
        .iter()
        .map(|(pattern, shell)| {
            let shell = parse_shell_dialect(shell)
                .map_err(|err| anyhow!("invalid {scope} shell `{shell}`: {err}"))?;
            Ok(PerFileShell {
                pattern: pattern.clone(),
                shell,
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

fn per_file_shell_from_pair(pair: &PatternShellPair) -> PerFileShell {
    PerFileShell {
        pattern: pair.pattern.clone(),
        shell: pair.shell,
    }
}

fn parse_shell_dialect(value: &str) -> Result<ShellDialect> {
    let shell = ShellDialect::from_name(value);
    if shell == ShellDialect::Unknown {
        return Err(anyhow!(
            "expected shell dialect to be one of sh, bash, dash, ksh, mksh, zsh"
        ));
    }

    Ok(shell)
}

fn shell_name(shell: ShellDialect) -> &'static str {
    match shell {
        ShellDialect::Unknown => "unknown",
        ShellDialect::Sh => "sh",
        ShellDialect::Bash => "bash",
        ShellDialect::Dash => "dash",
        ShellDialect::Ksh => "ksh",
        ShellDialect::Mksh => "mksh",
        ShellDialect::Zsh => "zsh",
    }
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

fn apply_per_file_shell_layer(
    current: Vec<PerFileShell>,
    per_file_shell: Option<Vec<PerFileShell>>,
    extend_per_file_shell: Vec<PerFileShell>,
) -> Vec<PerFileShell> {
    let mut per_file_shell = per_file_shell.unwrap_or(current);
    per_file_shell.extend(extend_per_file_shell);
    per_file_shell
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::mpsc::{TryRecvError, channel};

    use notify::event::{CreateKind, EventAttributes, ModifyKind, RemoveKind, RenameMode};
    use shuck_extract::{
        EmbeddedFormat, EmbeddedScript, ExtractedDialect, HostLineStart, ImplicitShellFlags,
    };
    use shuck_linter::{
        Category, LinterSettings, Rule, RuleSelector, RuleSet, ShellCheckCodeMap, ShellDialect,
    };
    use shuck_parser::parser::Parser;
    use tempfile::tempdir;

    use super::*;
    use crate::ExitStatus;
    use crate::args::{
        CheckCommand, CheckOutputFormatArg, FileSelectionArgs, PatternRuleSelectorPair,
        PatternShellPair, RuleSelectionArgs,
    };
    use crate::commands::check::add_ignore::run_add_ignore_with_cwd;
    use crate::commands::check::analyze::{
        analyze_file, collect_lint_diagnostics, read_shared_source,
    };
    use crate::commands::check::cache::CachedDisplayedDiagnosticKind;
    use crate::commands::check::display::display_lint_diagnostics;
    use crate::commands::check::embedded::remap_embedded_position;
    use crate::commands::check::run::run_check_with_cwd;
    use crate::commands::check::settings::{
        CompiledPerFileShellList, PerFileShell, parse_rule_selectors,
    };
    use crate::commands::check::test_support::*;
    use crate::commands::check::watch::{
        WatchTarget, collect_watch_targets, drain_watch_batch, should_clear_screen,
        watch_event_requires_rerun,
    };
    use crate::commands::check::{CheckReport, diagnostics_exit_status};
    use crate::commands::check_output::{
        DisplayPosition, DisplaySpan, DisplayedDiagnostic, DisplayedDiagnosticKind, print_report_to,
    };
    use crate::commands::project_runner::PendingProjectFile;
    use crate::config::ConfigArguments;
    use crate::discover::{FileKind, normalize_path};

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
    fn per_file_shell_overrides_inferred_shell_for_matching_files() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("bashy.sh"), "source helper.sh\n").unwrap();
        fs::write(tempdir.path().join("portable.sh"), "source helper.sh\n").unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            select: Some(vec![RuleSelector::Rule(Rule::SourceBuiltinInSh)]),
            per_file_shell: Some(vec![PatternShellPair {
                pattern: "bashy.sh".to_owned(),
                shell: ShellDialect::Bash,
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
        assert_eq!(report.diagnostics[0].path, PathBuf::from("portable.sh"));
        assert_eq!(
            diagnostic_codes(&report),
            vec![Rule::SourceBuiltinInSh.code().to_owned()]
        );
    }

    #[test]
    fn lint_config_per_file_shell_overrides_inferred_shell() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("bashy.sh"), "source helper.sh\n").unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['X031']\nper-file-shell = { 'bashy.sh' = 'bash' }\n",
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
    fn per_file_shell_matches_normalized_windows_verbatim_paths() {
        let path = Path::new(r"\\?\C:\repo\nested\script.sh");
        let per_file_shell = CompiledPerFileShellList::resolve(
            PathBuf::from(r"C:\repo"),
            [PerFileShell {
                pattern: "C:/repo/nested/*.sh".to_owned(),
                shell: ShellDialect::Bash,
            }],
        )
        .unwrap();

        assert_eq!(
            per_file_shell.shell_for_path(path),
            Some(ShellDialect::Bash)
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
}
