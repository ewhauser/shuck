use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use shuck_cache::{CacheKey, CacheKeyHasher};
#[cfg(test)]
pub(super) use shuck_config::resolve::parse_rule_selectors;
use shuck_config::resolve::{
    ParsedOverride, PerFileIgnoreSpec, ResolvePolicy, ResolvedZshPluginSettings,
    RuleSelectionLayer, SettingsOverride, SettingsResolver, ZshLoadSpec, ZshPluginEntrypointSpec,
    ZshPluginLayer,
};
use shuck_config::{ConfigArguments, load_project_config};
pub(super) use shuck_linter::{CompiledPerFileShellList, PerFileShell};
use shuck_linter::{
    LinterSettings, PerFileIgnore, ResolvedAmbientContracts, RuleSelector, RuleSet,
};

use crate::args::{
    PatternFrameworkNameTriple, PatternPathPair, PatternRuleSelectorPair, PatternShellPair,
    RuleSelectionArgs, ZshPluginArgs,
};
use crate::discover::ProjectRoot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EffectiveCheckSettings {
    enabled_rules: Vec<String>,
    per_file_ignores: Vec<EffectivePerFileIgnore>,
    per_file_shell: Vec<EffectivePerFileShell>,
    rule_options: EffectiveRuleOptions,
    ambient_contracts: EffectiveAmbientContractSettings,
    zsh_plugins: EffectiveZshPluginSettings,
    source_paths: Vec<String>,
    lint_sources: bool,
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
    s078_allowed_shells: Vec<String>,
    s079_allowed_forms: Vec<String>,
    s079_allowed_paths: Vec<String>,
    s080_max_lines: usize,
    s080_count: String,
    s081_ignore_shebang_only_files: bool,
    s082_kinds: Vec<String>,
    s082_require_owner: bool,
    s082_require_message: bool,
    s083_require_for: u8,
    s083_long_function_line_threshold: usize,
    s084_require_globals: bool,
    s084_require_arguments: bool,
    s084_require_outputs: bool,
    s084_require_returns: bool,
    s085_non_trivial_line_threshold: usize,
    s085_non_trivial_function_count: usize,
    s085_main_name: String,
    c158_treat_readonly_as_documented: bool,
    c158_treat_export_as_intentional: bool,
    c159_allow_conditional_init: bool,
    c160_allowed_anchors: Vec<String>,
    c161_ignore_after_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveAmbientContractSettings {
    well_known_ids: Vec<String>,
    custom_descriptors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct EffectiveZshPluginSettings {
    resolution_enabled: bool,
    roots: Vec<(String, String)>,
    plugin_loads: Vec<EffectiveZshPluginLoad>,
    theme_loads: Vec<EffectiveZshPluginLoad>,
    entrypoints: Vec<EffectiveZshPluginEntrypoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveZshPluginLoad {
    pattern: String,
    framework: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveZshPluginEntrypoint {
    pattern: String,
    paths: Vec<String>,
}

impl EffectiveCheckSettings {
    #[allow(clippy::too_many_arguments)]
    fn new(
        enabled_rules: RuleSet,
        per_file_ignores: &[PerFileIgnore],
        per_file_shell: &[PerFileShell],
        rule_options: &shuck_linter::LinterRuleOptions,
        ambient_contracts: &ResolvedAmbientContracts,
        zsh_plugins: &ResolvedZshPluginSettings,
        source_paths: Vec<String>,
        lint_sources: bool,
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
                pattern: shell.pattern().to_owned(),
                shell: shell.shell().as_str().to_owned(),
            })
            .collect::<Vec<_>>();

        Self {
            enabled_rules,
            per_file_ignores,
            per_file_shell,
            rule_options: EffectiveRuleOptions::new(rule_options),
            ambient_contracts: EffectiveAmbientContractSettings::new(ambient_contracts),
            zsh_plugins: EffectiveZshPluginSettings::new(zsh_plugins),
            source_paths,
            lint_sources,
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
        self.ambient_contracts.cache_key(state);
        self.zsh_plugins.cache_key(state);
        self.source_paths.cache_key(state);
        self.lint_sources.cache_key(state);
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
            s078_allowed_shells: rule_options.s078.allowed_shells.clone(),
            s079_allowed_forms: rule_options.s079.allowed_forms.clone(),
            s079_allowed_paths: rule_options.s079.allowed_paths.clone(),
            s080_max_lines: rule_options.s080.max_lines,
            s080_count: rule_options.s080.count.clone(),
            s081_ignore_shebang_only_files: rule_options.s081.ignore_shebang_only_files,
            s082_kinds: rule_options.s082.kinds.clone(),
            s082_require_owner: rule_options.s082.require_owner,
            s082_require_message: rule_options.s082.require_message,
            s083_require_for: s083_require_for_cache_value(rule_options.s083.require_for),
            s083_long_function_line_threshold: rule_options.s083.long_function_line_threshold,
            s084_require_globals: rule_options.s084.require_globals,
            s084_require_arguments: rule_options.s084.require_arguments,
            s084_require_outputs: rule_options.s084.require_outputs,
            s084_require_returns: rule_options.s084.require_returns,
            s085_non_trivial_line_threshold: rule_options.s085.non_trivial_line_threshold,
            s085_non_trivial_function_count: rule_options.s085.non_trivial_function_count,
            s085_main_name: rule_options.s085.main_name.clone(),
            c158_treat_readonly_as_documented: rule_options.c158.treat_readonly_as_documented,
            c158_treat_export_as_intentional: rule_options.c158.treat_export_as_intentional,
            c159_allow_conditional_init: rule_options.c159.allow_conditional_init,
            c160_allowed_anchors: rule_options.c160.allowed_anchors.clone(),
            c161_ignore_after_source: rule_options.c161.ignore_after_source,
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
        self.s078_allowed_shells.cache_key(state);
        self.s079_allowed_forms.cache_key(state);
        self.s079_allowed_paths.cache_key(state);
        self.s080_max_lines.cache_key(state);
        self.s080_count.cache_key(state);
        self.s081_ignore_shebang_only_files.cache_key(state);
        self.s082_kinds.cache_key(state);
        self.s082_require_owner.cache_key(state);
        self.s082_require_message.cache_key(state);
        self.s083_require_for.cache_key(state);
        self.s083_long_function_line_threshold.cache_key(state);
        self.s084_require_globals.cache_key(state);
        self.s084_require_arguments.cache_key(state);
        self.s084_require_outputs.cache_key(state);
        self.s084_require_returns.cache_key(state);
        self.s085_non_trivial_line_threshold.cache_key(state);
        self.s085_non_trivial_function_count.cache_key(state);
        self.s085_main_name.cache_key(state);
        self.c158_treat_readonly_as_documented.cache_key(state);
        self.c158_treat_export_as_intentional.cache_key(state);
        self.c159_allow_conditional_init.cache_key(state);
        self.c160_allowed_anchors.cache_key(state);
        self.c161_ignore_after_source.cache_key(state);
    }
}

fn s083_require_for_cache_value(value: shuck_linter::S083FunctionDocRequirement) -> u8 {
    match value {
        shuck_linter::S083FunctionDocRequirement::All => 0,
        shuck_linter::S083FunctionDocRequirement::Exported => 1,
        shuck_linter::S083FunctionDocRequirement::Long => 2,
        shuck_linter::S083FunctionDocRequirement::Parameterized => 3,
    }
}

impl EffectiveAmbientContractSettings {
    fn new(ambient_contracts: &ResolvedAmbientContracts) -> Self {
        let effective = ambient_contracts.effective();
        Self {
            well_known_ids: effective.well_known_ids,
            custom_descriptors: effective.custom_descriptors,
        }
    }
}

impl CacheKey for EffectiveAmbientContractSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-ambient-contracts");
        self.well_known_ids.cache_key(state);
        self.custom_descriptors.cache_key(state);
    }
}

impl EffectiveZshPluginSettings {
    fn new(settings: &ResolvedZshPluginSettings) -> Self {
        let mut roots = settings
            .roots()
            .iter()
            .map(|(framework, path)| (framework.clone(), path.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
        roots.sort_by(|left, right| left.0.cmp(&right.0));

        let effective_loads = |loads: &[shuck_config::resolve::CompiledZshLoad]| {
            let mut loads = loads
                .iter()
                .map(|load| EffectiveZshPluginLoad {
                    pattern: load.pattern().to_owned(),
                    framework: load.framework().to_owned(),
                    name: load.name().to_owned(),
                })
                .collect::<Vec<_>>();
            loads.sort_by(|left, right| {
                left.pattern
                    .cmp(&right.pattern)
                    .then(left.framework.cmp(&right.framework))
                    .then(left.name.cmp(&right.name))
            });
            loads
        };

        let mut entrypoints = settings
            .entrypoints()
            .iter()
            .map(|entrypoint| EffectiveZshPluginEntrypoint {
                pattern: entrypoint.pattern().to_owned(),
                paths: entrypoint
                    .paths()
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            })
            .collect::<Vec<_>>();
        entrypoints.sort_by(|left, right| left.pattern.cmp(&right.pattern));

        Self {
            resolution_enabled: settings.is_enabled(),
            roots,
            plugin_loads: effective_loads(settings.plugin_loads()),
            theme_loads: effective_loads(settings.theme_loads()),
            entrypoints,
        }
    }
}

impl CacheKey for EffectiveZshPluginSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-zsh-plugin-settings");
        self.resolution_enabled.cache_key(state);
        for (framework, path) in &self.roots {
            framework.cache_key(state);
            path.cache_key(state);
        }
        self.plugin_loads.cache_key(state);
        self.theme_loads.cache_key(state);
        self.entrypoints.cache_key(state);
    }
}

impl CacheKey for EffectiveZshPluginLoad {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.pattern.cache_key(state);
        self.framework.cache_key(state);
        self.name.cache_key(state);
    }
}

impl CacheKey for EffectiveZshPluginEntrypoint {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.pattern.cache_key(state);
        self.paths.cache_key(state);
    }
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedCheckSettings {
    pub(super) linter_settings: LinterSettings,
    pub(super) per_file_shell: Arc<CompiledPerFileShellList>,
    pub(super) zsh_plugins: Arc<ResolvedZshPluginSettings>,
    pub(super) fixable_rules: RuleSet,
    /// Extra directories searched when resolving `# shuck: source=` directive
    /// targets, after the annotating file's own directory.
    pub(super) source_paths: Vec<String>,
    /// Whether `lint=true` source directives lint the resolved target.
    pub(super) lint_sources: bool,
    pub(super) effective: EffectiveCheckSettings,
    pub(super) embedded_enabled: bool,
}

impl CacheKey for ResolvedCheckSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.effective.cache_key(state);
    }
}
pub(super) fn resolve_project_check_settings(
    project_root: &ProjectRoot,
    config_arguments: &ConfigArguments,
    cli_rule_selection: &RuleSelectionArgs,
    cli_zsh_plugins: &ZshPluginArgs,
) -> Result<ResolvedCheckSettings> {
    let config = load_project_config(&project_root.storage_root, config_arguments)?;
    let overrides = [SettingsOverride::Parsed(Box::new(ParsedOverride {
        rules: parse_cli_rule_selection_layer(cli_rule_selection),
        zsh_plugins: parse_cli_zsh_plugin_layer(cli_zsh_plugins),
    }))];
    let resolved =
        SettingsResolver::new(&project_root.canonical_root, &config, ResolvePolicy::Strict)
            .with_overrides(&overrides)
            .resolve_lint()?;

    let effective = EffectiveCheckSettings::new(
        resolved.linter.rules,
        &resolved.per_file_ignores,
        &resolved.per_file_shell,
        &resolved.linter.rule_options,
        resolved.linter.ambient_contracts.as_ref(),
        &resolved.zsh_plugins,
        resolved.source_paths.clone(),
        resolved.lint_sources,
        resolved.embedded,
    );

    Ok(ResolvedCheckSettings {
        linter_settings: resolved.linter,
        per_file_shell: resolved.compiled_per_file_shell,
        zsh_plugins: resolved.zsh_plugins,
        fixable_rules: resolved.fixable_rules,
        source_paths: resolved.source_paths,
        lint_sources: resolved.lint_sources,
        effective,
        embedded_enabled: resolved.embedded,
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

fn parse_cli_zsh_plugin_layer(args: &ZshPluginArgs) -> ZshPluginLayer {
    ZshPluginLayer {
        resolution: args.resolution(),
        roots: args.zsh_plugin_root.as_ref().map(|roots| {
            roots
                .iter()
                .map(|root| (root.framework.clone(), root.path.clone()))
                .collect()
        }),
        extend_roots: args
            .extend_zsh_plugin_root
            .iter()
            .map(|root| (root.framework.clone(), root.path.clone()))
            .collect(),
        plugin_loads: args
            .zsh_plugin
            .as_ref()
            .map(|loads| loads.iter().map(zsh_load_spec_from_cli).collect()),
        extend_plugin_loads: args
            .extend_zsh_plugin
            .iter()
            .map(zsh_load_spec_from_cli)
            .collect(),
        theme_loads: args
            .zsh_theme
            .as_ref()
            .map(|loads| loads.iter().map(zsh_load_spec_from_cli).collect()),
        extend_theme_loads: args
            .extend_zsh_theme
            .iter()
            .map(zsh_load_spec_from_cli)
            .collect(),
        entrypoints: args
            .zsh_plugin_entrypoint
            .as_ref()
            .map(|loads| loads.iter().map(zsh_entrypoint_spec_from_cli).collect()),
        extend_entrypoints: args
            .extend_zsh_plugin_entrypoint
            .iter()
            .map(zsh_entrypoint_spec_from_cli)
            .collect(),
    }
}

fn zsh_load_spec_from_cli(load: &PatternFrameworkNameTriple) -> ZshLoadSpec {
    ZshLoadSpec {
        pattern: load.pattern.clone(),
        framework: load.framework.clone(),
        name: load.name.clone(),
    }
}

fn zsh_entrypoint_spec_from_cli(load: &PatternPathPair) -> ZshPluginEntrypointSpec {
    ZshPluginEntrypointSpec {
        pattern: load.pattern.clone(),
        paths: vec![load.path.clone()],
    }
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
    PerFileShell::new(pair.pattern.clone(), pair.shell)
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
        AmbientContractActivation, AmbientContractConfig, AmbientContractEffects,
        AmbientContractSpec, Category, LinterSettings, NamedGroup, Rule, RuleSelector, RuleSet,
        ShellCheckCodeMap, ShellDialect,
    };
    use shuck_parser::parser::Parser;
    use shuck_semantic::FileContract;
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
    use crate::discover::{FileKind, ProjectRoot, normalize_path};
    use shuck_config::resolve::resolve_ambient_contracts;
    use shuck_config::{
        ConfigArguments, LintContractActivationConfig, LintContractActivationTypeConfig,
        LintContractConsumesConfig, LintContractWhenConfig, LintContractsConfig,
        LintCustomContractConfig,
    };

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
            "#!/bin/bash\n# Exercises style-rule selection.\nprintf '%s\\n' x &;\n",
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
            "#!/bin/bash\n# Exercises style-rule selection.\nprintf '%s\\n' x &;\n",
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
            "#!/bin/bash\n# Exercises style-rule selection.\nprintf '%s\\n' x &;\n",
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
            "#!/bin/bash\n# Exercises style-rule selection.\nprintf '%s\\n' x &;\n",
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
    fn config_select_accepts_named_groups() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nselect = ['google']\n",
        )
        .unwrap();

        let settings = resolve_project_check_settings(
            &ProjectRoot {
                storage_root: tempdir.path().to_path_buf(),
                canonical_root: fs::canonicalize(tempdir.path()).unwrap(),
            },
            &ConfigArguments::default(),
            &RuleSelectionArgs::default(),
            &ZshPluginArgs::default(),
        )
        .unwrap();

        assert_eq!(
            settings.linter_settings.rules,
            NamedGroup::Google.rule_set()
        );
    }

    #[test]
    fn config_extend_select_accepts_named_groups() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nextend-select = ['google']\n",
        )
        .unwrap();

        let settings = resolve_project_check_settings(
            &ProjectRoot {
                storage_root: tempdir.path().to_path_buf(),
                canonical_root: fs::canonicalize(tempdir.path()).unwrap(),
            },
            &ConfigArguments::default(),
            &RuleSelectionArgs::default(),
            &ZshPluginArgs::default(),
        )
        .unwrap();

        assert_eq!(
            settings.linter_settings.rules,
            LinterSettings::default_rules().union(&NamedGroup::Google.rule_set())
        );
    }

    #[test]
    fn config_rejects_unknown_s080_count_modes() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint.rule-options.s080]\ncount = 'non-comment-nonblank'\n",
        )
        .unwrap();

        let err = resolve_project_check_settings(
            &ProjectRoot {
                storage_root: tempdir.path().to_path_buf(),
                canonical_root: fs::canonicalize(tempdir.path()).unwrap(),
            },
            &ConfigArguments::default(),
            &RuleSelectionArgs::default(),
            &ZshPluginArgs::default(),
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("unsupported `[lint.rule-options.s080].count` value"));
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
    fn effective_ambient_contract_settings_change_when_contract_effects_change() {
        let request_contracts = |prefix: &str| LintContractsConfig {
            well_known: Some(false),
            disabled: None,
            custom: Some(vec![LintCustomContractConfig {
                id: "custom-tmux".to_owned(),
                replaces: None,
                when: LintContractWhenConfig::Activation(LintContractActivationConfig {
                    activation_type: LintContractActivationTypeConfig::ZshPlugin,
                    framework: Some("oh-my-zsh".to_owned()),
                    plugin: Some("tmux".to_owned()),
                    theme: None,
                }),
                files: Some(vec!["**/.zshrc".to_owned()]),
                reads: None,
                consumes: Some(LintContractConsumesConfig {
                    names: None,
                    prefixes: Some(vec![prefix.to_owned()]),
                    all: None,
                }),
                provides: None,
                functions: None,
            }]),
        };

        let first =
            resolve_ambient_contracts(Path::new("/workspace"), Some(&request_contracts("TMUX_A_")))
                .unwrap();
        let second =
            resolve_ambient_contracts(Path::new("/workspace"), Some(&request_contracts("TMUX_B_")))
                .unwrap();

        assert_ne!(
            EffectiveAmbientContractSettings::new(&first),
            EffectiveAmbientContractSettings::new(&second)
        );
    }

    #[test]
    fn cli_and_language_server_resolve_identical_linter_settings() {
        let tempdir = tempdir().unwrap();
        let root = fs::canonicalize(tempdir.path()).unwrap();
        fs::write(
            root.join(".shuck.toml"),
            r#"
[per-file-shell]
"*.zsh" = "zsh"
"portable.sh" = "dash"

[lint]
select = ["C", "S080", "S082", "C158"]
ignore = ["C006"]
extend-select = ["S085"]
per-file-ignores = { "vendor/*.sh" = ["C001"] }
extend-per-file-ignores = { "portable.sh" = ["C063"] }

[lint.rule-options.c001]
treat-indirect-expansion-targets-as-used = true

[lint.rule-options.s080]
max-lines = 120
count = "non-comment-non-blank"

[lint.rule-options.s082]
kinds = ["TODO"]
require-owner = true

[lint.rule-options.s085]
non-trivial-line-threshold = 20
main-name = "run"

[lint.rule-options.c158]
treat-export-as-intentional = true

[lint.contracts]
well-known = false

[[lint.contracts.custom]]
id = "project-env"
when = "always"
files = ["**/*.sh"]
provides = { variables = ["PROJECT_HOME"] }
consumes = { prefixes = ["PROJECT_"] }
"#,
        )
        .unwrap();

        let cli = resolve_project_check_settings(
            &ProjectRoot {
                storage_root: root.clone(),
                canonical_root: root.clone(),
            },
            &ConfigArguments::default(),
            &RuleSelectionArgs::default(),
            &ZshPluginArgs::default(),
        )
        .unwrap();

        for file in ["portable.sh", "vendor/tool.sh", "prompt.zsh", "other.bash"] {
            let path = root.join(file);
            let mut cli_linter = cli.linter_settings.clone();
            cli_linter.shell = cli
                .per_file_shell
                .shell_for_path(&path)
                .unwrap_or(ShellDialect::Unknown);
            let lsp_linter = shuck_server::resolve_linter_settings_for_file(
                &path,
                std::slice::from_ref(&root),
                &shuck_server::ClientOptions::default(),
            );

            assert_eq!(cli_linter, lsp_linter, "settings diverged for {file}");
        }

        let portable = root.join("portable.sh");
        let lsp_linter = shuck_server::resolve_linter_settings_for_file(
            &portable,
            std::slice::from_ref(&root),
            &shuck_server::ClientOptions::default(),
        );
        assert_eq!(lsp_linter.shell, ShellDialect::Dash);
        assert!(
            lsp_linter
                .rule_options
                .c001
                .treat_indirect_expansion_targets_as_used
        );
        assert_eq!(lsp_linter.rule_options.s080.max_lines, 120);
        assert!(lsp_linter.rule_options.s082.require_owner);
        assert!(lsp_linter.rule_options.c158.treat_export_as_intentional);
        assert_ne!(
            lsp_linter.ambient_contracts.as_ref(),
            &ResolvedAmbientContracts::default()
        );
        assert!(
            lsp_linter
                .per_file_ignores
                .ignored_rules(&root.join("vendor/tool.sh"))
                .contains(Rule::UnusedAssignment)
        );
        assert!(!lsp_linter.rules.contains(Rule::UndefinedVariable));
    }

    #[test]
    fn parses_named_group_rule_selectors() {
        assert_eq!(
            parse_rule_selectors(&["google".to_owned()], "lint.select").unwrap(),
            vec![RuleSelector::Named(NamedGroup::Google)]
        );
    }

    #[test]
    fn select_google_matches_config_extend_select_on_google_only_fixture() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nextend-select = ['google']\n",
        )
        .unwrap();
        fs::write(
            tempdir.path().join("script.sh"),
            "#!/bin/sh\nunused=1\necho $1\n",
        )
        .unwrap();

        let config_report = run_check_with_cwd(
            &check_args(true),
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        let mut args = check_args(true);
        args.rule_selection = RuleSelectionArgs {
            select: Some(vec![RuleSelector::Named(NamedGroup::Google)]),
            ..RuleSelectionArgs::default()
        };
        let cli_report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .unwrap();

        let mut config_codes = diagnostic_codes(&config_report);
        let mut cli_codes = diagnostic_codes(&cli_report);
        config_codes.sort();
        cli_codes.sort();

        assert_eq!(config_codes, cli_codes);
        assert_eq!(
            cli_codes,
            vec![
                Rule::UnusedAssignment.code().to_owned(),
                Rule::UnquotedExpansion.code().to_owned(),
            ]
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
