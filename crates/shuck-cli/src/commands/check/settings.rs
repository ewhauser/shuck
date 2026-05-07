use std::collections::{BTreeMap, HashMap};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use globset::{Glob, GlobMatcher};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_config::{
    ConfigArguments, LintConfig, ZshPluginEntrypointConfig, ZshPluginLoadConfig,
    ZshThemeLoadConfig, load_project_config,
};
use shuck_linter::{
    CompiledPerFileIgnoreList, LinterSettings, PerFileIgnore, Rule, RuleSelector, RuleSet,
    ShellDialect,
};
use shuck_semantic::{
    PluginFramework, PluginRequest, PluginRequestKind, PluginResolution, PluginResolver,
    layout_for_plugin_framework, zsh_plugin_framework_from_name, zsh_plugin_layouts,
};

use crate::args::{
    PatternFrameworkNameTriple, PatternPathPair, PatternRuleSelectorPair, PatternShellPair,
    RuleSelectionArgs, ZshPluginArgs,
};
use crate::discover::{ProjectRoot, normalize_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EffectiveCheckSettings {
    enabled_rules: Vec<String>,
    per_file_ignores: Vec<EffectivePerFileIgnore>,
    per_file_shell: Vec<EffectivePerFileShell>,
    rule_options: EffectiveRuleOptions,
    zsh_plugins: EffectiveZshPluginSettings,
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
    fn new(
        enabled_rules: RuleSet,
        per_file_ignores: &[PerFileIgnore],
        per_file_shell: &[PerFileShell],
        rule_options: &shuck_linter::LinterRuleOptions,
        zsh_plugins: &ResolvedZshPluginSettings,
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
            zsh_plugins: zsh_plugins.effective(),
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
        self.zsh_plugins.cache_key(state);
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

#[derive(Debug, Clone, Default)]
struct ZshPluginLayer {
    resolution: Option<bool>,
    roots: Option<BTreeMap<String, String>>,
    extend_roots: BTreeMap<String, String>,
    plugin_loads: Option<Vec<ZshPluginLoadSpec>>,
    extend_plugin_loads: Vec<ZshPluginLoadSpec>,
    theme_loads: Option<Vec<ZshThemeLoadSpec>>,
    extend_theme_loads: Vec<ZshThemeLoadSpec>,
    entrypoints: Option<Vec<ZshPluginEntrypointSpec>>,
    extend_entrypoints: Vec<ZshPluginEntrypointSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZshPluginLoadSpec {
    pattern: String,
    framework: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZshThemeLoadSpec {
    pattern: String,
    framework: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZshPluginEntrypointSpec {
    pattern: String,
    paths: Vec<String>,
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

#[derive(Debug, Clone)]
pub(super) struct ResolvedZshPluginSettings {
    enabled: bool,
    project_root: PathBuf,
    roots: BTreeMap<String, PathBuf>,
    plugin_loads: Vec<CompiledZshPluginLoad>,
    theme_loads: Vec<CompiledZshThemeLoad>,
    entrypoints: Vec<CompiledZshPluginEntrypoint>,
}

#[derive(Debug, Clone)]
struct CompiledZshPluginLoad {
    matcher: CompiledPathMatcher,
    framework: String,
    name: String,
}

#[derive(Debug, Clone)]
struct CompiledZshThemeLoad {
    matcher: CompiledPathMatcher,
    framework: String,
    name: String,
}

#[derive(Debug, Clone)]
struct CompiledZshPluginEntrypoint {
    matcher: CompiledPathMatcher,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct CompiledPathMatcher {
    pattern: String,
    basename_matcher: GlobMatcher,
    relative_matcher: GlobMatcher,
    absolute_matcher: GlobMatcher,
    negated: bool,
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

impl ResolvedZshPluginSettings {
    fn resolve(
        project_root: impl Into<PathBuf>,
        enabled: bool,
        roots: BTreeMap<String, String>,
        plugin_loads: Vec<ZshPluginLoadSpec>,
        theme_loads: Vec<ZshThemeLoadSpec>,
        entrypoints: Vec<ZshPluginEntrypointSpec>,
    ) -> Result<Self> {
        let project_root = project_root.into();
        if !enabled {
            return Ok(Self {
                enabled,
                project_root,
                roots: BTreeMap::new(),
                plugin_loads: Vec::new(),
                theme_loads: Vec::new(),
                entrypoints: Vec::new(),
            });
        }
        let roots = roots
            .into_iter()
            .map(|(framework, path)| {
                Ok((framework, normalize_zsh_plugin_path(&project_root, &path)?))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let plugin_loads = plugin_loads
            .into_iter()
            .map(|load| {
                Ok(CompiledZshPluginLoad {
                    matcher: CompiledPathMatcher::new(load.pattern)?,
                    framework: load.framework,
                    name: load.name,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let theme_loads = theme_loads
            .into_iter()
            .map(|load| {
                Ok(CompiledZshThemeLoad {
                    matcher: CompiledPathMatcher::new(load.pattern)?,
                    framework: load.framework,
                    name: load.name,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let entrypoints = entrypoints
            .into_iter()
            .map(|entrypoint| {
                Ok(CompiledZshPluginEntrypoint {
                    matcher: CompiledPathMatcher::new(entrypoint.pattern)?,
                    paths: entrypoint
                        .paths
                        .into_iter()
                        .map(|path| normalize_zsh_plugin_path(&project_root, &path))
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            enabled,
            project_root,
            roots,
            plugin_loads,
            theme_loads,
            entrypoints,
        })
    }

    fn effective(&self) -> EffectiveZshPluginSettings {
        let mut roots = self
            .roots
            .iter()
            .map(|(framework, path)| (framework.clone(), path.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
        roots.sort_by(|left, right| left.0.cmp(&right.0));

        let mut plugin_loads = self
            .plugin_loads
            .iter()
            .map(|load| EffectiveZshPluginLoad {
                pattern: load.matcher.pattern.clone(),
                framework: load.framework.clone(),
                name: load.name.clone(),
            })
            .collect::<Vec<_>>();
        plugin_loads.sort_by(|left, right| {
            left.pattern
                .cmp(&right.pattern)
                .then(left.framework.cmp(&right.framework))
                .then(left.name.cmp(&right.name))
        });

        let mut theme_loads = self
            .theme_loads
            .iter()
            .map(|load| EffectiveZshPluginLoad {
                pattern: load.matcher.pattern.clone(),
                framework: load.framework.clone(),
                name: load.name.clone(),
            })
            .collect::<Vec<_>>();
        theme_loads.sort_by(|left, right| {
            left.pattern
                .cmp(&right.pattern)
                .then(left.framework.cmp(&right.framework))
                .then(left.name.cmp(&right.name))
        });

        let mut entrypoints = self
            .entrypoints
            .iter()
            .map(|entrypoint| EffectiveZshPluginEntrypoint {
                pattern: entrypoint.matcher.pattern.clone(),
                paths: entrypoint
                    .paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            })
            .collect::<Vec<_>>();
        entrypoints.sort_by(|left, right| left.pattern.cmp(&right.pattern));

        EffectiveZshPluginSettings {
            resolution_enabled: self.enabled,
            roots,
            plugin_loads,
            theme_loads,
            entrypoints,
        }
    }
}

impl PluginResolver for ResolvedZshPluginSettings {
    fn additional_plugin_requests(&self, source_path: &Path) -> Vec<PluginRequest> {
        if !self.enabled {
            return Vec::new();
        }

        let mut requests = self
            .plugin_loads
            .iter()
            .filter(|load| load.matcher.matches(&self.project_root, source_path))
            .map(|load| PluginRequest {
                framework: plugin_framework_from_name(&load.framework),
                kind: PluginRequestKind::Plugin,
                name: load.name.clone(),
                span: shuck_ast::Span::new(),
                explicit: true,
                root_hint: None,
            })
            .collect::<Vec<_>>();
        requests.extend(
            self.theme_loads
                .iter()
                .filter(|load| load.matcher.matches(&self.project_root, source_path))
                .map(|load| PluginRequest {
                    framework: plugin_framework_from_name(&load.framework),
                    kind: PluginRequestKind::Theme,
                    name: load.name.clone(),
                    span: shuck_ast::Span::new(),
                    explicit: true,
                    root_hint: None,
                }),
        );
        requests.extend(
            self.entrypoints
                .iter()
                .filter(|entrypoint| entrypoint.matcher.matches(&self.project_root, source_path))
                .flat_map(|entrypoint| entrypoint.paths.iter())
                .map(|path| PluginRequest {
                    framework: PluginFramework::ExplicitFilesystem,
                    kind: PluginRequestKind::Entrypoint,
                    name: path.to_string_lossy().into_owned(),
                    span: shuck_ast::Span::new(),
                    explicit: true,
                    root_hint: None,
                }),
        );
        requests
    }

    fn resolve_source_path(&self, source_path: &Path, candidate: &str) -> Vec<PathBuf> {
        if !self.enabled {
            return Vec::new();
        }

        let mut paths = Vec::new();
        for layout in zsh_plugin_layouts() {
            for root in configured_plugin_roots(self, layout.root_keys()) {
                if let Some(suffix) = layout.resolve_source_suffix(root, source_path, candidate) {
                    paths.push(root.join(suffix));
                }
            }
        }
        paths
    }

    fn resolve_plugin_request(
        &self,
        _source_path: &Path,
        request: &PluginRequest,
    ) -> PluginResolution {
        if !self.enabled {
            return PluginResolution::default();
        }

        match request.kind {
            PluginRequestKind::Entrypoint => {
                let path = PathBuf::from(&request.name);
                PluginResolution {
                    entrypoints: vec![path],
                    file_entry_contracts: Vec::new(),
                }
            }
            PluginRequestKind::Plugin => {
                let Some(root) = request
                    .root_hint
                    .clone()
                    .or_else(|| plugin_root_for_request(self, request))
                else {
                    return PluginResolution::default();
                };
                let Some(layout) = layout_for_plugin_framework(&request.framework) else {
                    return PluginResolution::default();
                };
                let Some(path) =
                    layout.resolve_entrypoint(&root, PluginRequestKind::Plugin, &request.name)
                else {
                    return PluginResolution::default();
                };
                PluginResolution {
                    entrypoints: vec![path],
                    file_entry_contracts: Vec::new(),
                }
            }
            PluginRequestKind::Theme => {
                let Some(root) = request
                    .root_hint
                    .clone()
                    .or_else(|| plugin_root_for_request(self, request))
                else {
                    return PluginResolution::default();
                };
                let Some(layout) = layout_for_plugin_framework(&request.framework) else {
                    return PluginResolution::default();
                };
                let Some(path) =
                    layout.resolve_entrypoint(&root, PluginRequestKind::Theme, &request.name)
                else {
                    return PluginResolution::default();
                };
                PluginResolution {
                    entrypoints: vec![path],
                    file_entry_contracts: Vec::new(),
                }
            }
        }
    }
}

impl CompiledPathMatcher {
    fn new(pattern: String) -> Result<Self> {
        let mut matcher_pattern = pattern.clone();
        let negated = matcher_pattern.starts_with('!');
        if negated {
            matcher_pattern.drain(..1);
        }

        Ok(Self {
            pattern,
            basename_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            relative_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            absolute_matcher: Glob::new(&matcher_pattern)
                .map_err(|err| anyhow!("invalid glob {matcher_pattern:?}: {err}"))?
                .compile_matcher(),
            negated,
        })
    }

    fn matches(&self, project_root: &Path, path: &Path) -> bool {
        let relative_path = path.strip_prefix(project_root).unwrap_or(path);
        let Some(file_name) = relative_path.file_name().or_else(|| path.file_name()) else {
            return false;
        };
        let matches = self.basename_matcher.is_match(file_name)
            || self.relative_matcher.is_match(relative_path)
            || per_file_shell_absolute_match(&self.absolute_matcher, path);
        if self.negated { !matches } else { matches }
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

fn plugin_framework_from_name(name: &str) -> PluginFramework {
    zsh_plugin_framework_from_name(name)
}

fn plugin_root_for_request(
    settings: &ResolvedZshPluginSettings,
    request: &PluginRequest,
) -> Option<PathBuf> {
    if let Some(layout) = layout_for_plugin_framework(&request.framework) {
        return configured_plugin_roots(settings, layout.root_keys())
            .next()
            .cloned();
    }
    match &request.framework {
        PluginFramework::Other(other) => settings.roots.get(other.as_str()).cloned(),
        PluginFramework::ExplicitFilesystem => None,
        _ => None,
    }
}

fn configured_plugin_roots<'a>(
    settings: &'a ResolvedZshPluginSettings,
    keys: &'static [&'static str],
) -> impl Iterator<Item = &'a PathBuf> {
    keys.iter().filter_map(|key| settings.roots.get(*key))
}

fn normalize_zsh_plugin_path(project_root: &Path, value: &str) -> Result<PathBuf> {
    let expanded = expand_zsh_plugin_path(value)?;
    let path = PathBuf::from(expanded);
    let resolved = if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    };
    Ok(normalize_path(&resolved))
}

fn expand_zsh_plugin_path(value: &str) -> Result<String> {
    if value == "~" || value.starts_with("~/") {
        let home = home_dir_string()?;
        return Ok(if value == "~" {
            home
        } else {
            format!("{home}/{}", &value[2..])
        });
    }
    if value == "$HOME" || value.starts_with("$HOME/") {
        let home = home_dir_string()?;
        return Ok(if value == "$HOME" {
            home
        } else {
            format!("{home}/{}", &value["$HOME/".len()..])
        });
    }
    if value == "${HOME}" || value.starts_with("${HOME}/") {
        let home = home_dir_string()?;
        return Ok(if value == "${HOME}" {
            home
        } else {
            format!("{home}/{}", &value["${HOME}/".len()..])
        });
    }

    Ok(value.to_owned())
}

fn home_dir_string() -> Result<String> {
    env::var("HOME").map_err(|_| anyhow!("$HOME must be set to resolve zsh plugin paths"))
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
    cli_zsh_plugins: &ZshPluginArgs,
) -> Result<ResolvedCheckSettings> {
    let config = load_project_config(&project_root.storage_root, config_arguments)?;
    let rule_layers = [
        parse_lint_config_layer(&config.lint)?,
        parse_cli_rule_selection_layer(cli_rule_selection),
    ];
    let zsh_layers = [
        parse_lint_zsh_plugin_layer(&config.lint),
        parse_cli_zsh_plugin_layer(cli_zsh_plugins),
    ];
    let rule_options = linter_rule_options_for_lint_config(&config.lint);

    let mut enabled_rules = LinterSettings::default_rules();
    let mut fixable_rules = RuleSet::all();
    let mut per_file_ignores = Vec::new();
    let mut per_file_shell = Vec::new();
    let mut zsh_plugin_resolution_enabled = true;
    let mut zsh_plugin_roots = BTreeMap::new();
    let mut zsh_plugin_loads = Vec::new();
    let mut zsh_theme_loads = Vec::new();
    let mut zsh_entrypoints = Vec::new();

    for layer in rule_layers {
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
    for layer in zsh_layers {
        if let Some(enabled) = layer.resolution {
            zsh_plugin_resolution_enabled = enabled;
        }
        zsh_plugin_roots = apply_root_map_layer(zsh_plugin_roots, layer.roots, layer.extend_roots);
        zsh_plugin_loads = apply_extend_layer(
            zsh_plugin_loads,
            layer.plugin_loads,
            layer.extend_plugin_loads,
        );
        zsh_theme_loads =
            apply_extend_layer(zsh_theme_loads, layer.theme_loads, layer.extend_theme_loads);
        zsh_entrypoints =
            apply_extend_layer(zsh_entrypoints, layer.entrypoints, layer.extend_entrypoints);
    }

    let compiled_per_file_ignores = CompiledPerFileIgnoreList::resolve(
        project_root.canonical_root.clone(),
        per_file_ignores.clone(),
    )?;
    let compiled_per_file_shell = CompiledPerFileShellList::resolve(
        project_root.canonical_root.clone(),
        per_file_shell.clone(),
    )?;
    let resolved_zsh_plugins = ResolvedZshPluginSettings::resolve(
        project_root.canonical_root.clone(),
        zsh_plugin_resolution_enabled,
        zsh_plugin_roots,
        zsh_plugin_loads,
        zsh_theme_loads,
        zsh_entrypoints,
    )?;
    let embedded_enabled = config.check.embedded.unwrap_or(true);
    let effective = EffectiveCheckSettings::new(
        enabled_rules,
        &per_file_ignores,
        &per_file_shell,
        &rule_options,
        &resolved_zsh_plugins,
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
        zsh_plugins: Arc::new(resolved_zsh_plugins),
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

fn parse_lint_zsh_plugin_layer(lint: &LintConfig) -> ZshPluginLayer {
    let Some(plugins) = lint.zsh.as_ref().and_then(|zsh| zsh.plugins.as_ref()) else {
        return ZshPluginLayer::default();
    };

    ZshPluginLayer {
        resolution: plugins.resolution,
        roots: plugins.roots.clone(),
        extend_roots: BTreeMap::new(),
        plugin_loads: plugins
            .plugin_loads
            .as_ref()
            .map(|loads| loads.iter().map(zsh_plugin_load_spec_from_config).collect()),
        extend_plugin_loads: Vec::new(),
        theme_loads: plugins
            .theme_loads
            .as_ref()
            .map(|loads| loads.iter().map(zsh_theme_load_spec_from_config).collect()),
        extend_theme_loads: Vec::new(),
        entrypoints: plugins
            .entrypoints
            .as_ref()
            .map(|loads| loads.iter().map(zsh_entrypoint_spec_from_config).collect()),
        extend_entrypoints: Vec::new(),
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
            .map(|loads| loads.iter().map(zsh_plugin_load_spec_from_cli).collect()),
        extend_plugin_loads: args
            .extend_zsh_plugin
            .iter()
            .map(zsh_plugin_load_spec_from_cli)
            .collect(),
        theme_loads: args
            .zsh_theme
            .as_ref()
            .map(|loads| loads.iter().map(zsh_theme_load_spec_from_cli).collect()),
        extend_theme_loads: args
            .extend_zsh_theme
            .iter()
            .map(zsh_theme_load_spec_from_cli)
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

fn zsh_plugin_load_spec_from_config(load: &ZshPluginLoadConfig) -> ZshPluginLoadSpec {
    ZshPluginLoadSpec {
        pattern: load.pattern.clone(),
        framework: load.framework.clone(),
        name: load.name.clone(),
    }
}

fn zsh_theme_load_spec_from_config(load: &ZshThemeLoadConfig) -> ZshThemeLoadSpec {
    ZshThemeLoadSpec {
        pattern: load.pattern.clone(),
        framework: load.framework.clone(),
        name: load.name.clone(),
    }
}

fn zsh_entrypoint_spec_from_config(load: &ZshPluginEntrypointConfig) -> ZshPluginEntrypointSpec {
    ZshPluginEntrypointSpec {
        pattern: load.pattern.clone(),
        paths: load.paths.clone(),
    }
}

fn zsh_plugin_load_spec_from_cli(load: &PatternFrameworkNameTriple) -> ZshPluginLoadSpec {
    ZshPluginLoadSpec {
        pattern: load.pattern.clone(),
        framework: load.framework.clone(),
        name: load.name.clone(),
    }
}

fn zsh_theme_load_spec_from_cli(load: &PatternFrameworkNameTriple) -> ZshThemeLoadSpec {
    ZshThemeLoadSpec {
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

fn apply_root_map_layer(
    current: BTreeMap<String, String>,
    replace: Option<BTreeMap<String, String>>,
    extend: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = replace.unwrap_or(current);
    merged.extend(extend);
    merged
}

fn apply_extend_layer<T>(current: Vec<T>, replace: Option<Vec<T>>, extend: Vec<T>) -> Vec<T> {
    let mut merged = replace.unwrap_or(current);
    merged.extend(extend);
    merged
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
        RuleSelector::Named(_) => 1,
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
        Category, LinterSettings, NamedGroup, Rule, RuleSelector, RuleSet, ShellCheckCodeMap,
        ShellDialect,
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
    use crate::discover::{FileKind, ProjectRoot, normalize_path};
    use shuck_config::ConfigArguments;

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
    fn named_group_can_be_trimmed_by_exact_rule_ignore() {
        let rules = apply_rule_selector_layer(
            RuleSet::EMPTY,
            Some(&[RuleSelector::Named(NamedGroup::Google)]),
            &[],
            &[RuleSelector::Rule(Rule::UnusedAssignment)],
        );

        assert!(!rules.contains(Rule::UnusedAssignment));
        assert!(rules.contains(Rule::UnquotedExpansion));
    }

    #[test]
    fn named_group_can_be_trimmed_by_prefix_ignore() {
        let rules = apply_rule_selector_layer(
            RuleSet::EMPTY,
            Some(&[RuleSelector::Named(NamedGroup::Google)]),
            &[],
            &[RuleSelector::Prefix("S00".to_owned())],
        );

        assert!(!rules.contains(Rule::UnquotedExpansion));
        assert!(!rules.contains(Rule::ReadWithoutRaw));
        assert!(rules.contains(Rule::ExportCommandSubstitution));
        assert!(rules.contains(Rule::UnusedAssignment));
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
    fn disabled_zsh_plugin_resolution_ignores_invalid_plugin_settings() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            false,
            BTreeMap::from([("oh-my-zsh".to_owned(), "~/.oh-my-zsh".to_owned())]),
            vec![ZshPluginLoadSpec {
                pattern: "[".to_owned(),
                framework: "oh-my-zsh".to_owned(),
                name: "git".to_owned(),
            }],
            vec![ZshThemeLoadSpec {
                pattern: "[".to_owned(),
                framework: "oh-my-zsh".to_owned(),
                name: "agnoster".to_owned(),
            }],
            vec![ZshPluginEntrypointSpec {
                pattern: "[".to_owned(),
                paths: vec!["./vendor/prompt.plugin.zsh".to_owned()],
            }],
        )
        .unwrap();

        assert!(!settings.enabled);
        assert!(settings.roots.is_empty());
        assert!(settings.plugin_loads.is_empty());
        assert!(settings.theme_loads.is_empty());
        assert!(settings.entrypoints.is_empty());
    }

    #[test]
    fn oh_my_zsh_source_suffix_rewrites_framework_paths_only() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("oh-my-zsh".to_owned(), "/workspace/.oh-my-zsh".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/.oh-my-zsh/oh-my-zsh.sh"),
                "/opt/app/plugins/git/git.plugin.zsh",
            ),
            vec![PathBuf::from(
                "/workspace/.oh-my-zsh/plugins/git/git.plugin.zsh"
            )]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/script.zsh"),
                "/tmp/.oh-my-zsh/plugins/git/git.plugin.zsh",
            ),
            vec![PathBuf::from(
                "/workspace/.oh-my-zsh/plugins/git/git.plugin.zsh"
            )]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/script.zsh"),
                "/opt/app/plugins/git/git.plugin.zsh",
            ),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn oh_my_zsh_source_suffix_still_resolves_framework_entrypoint() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("oh-my-zsh".to_owned(), "/workspace/.oh-my-zsh".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/script.zsh"),
                "/not-installed/.oh-my-zsh/oh-my-zsh.sh",
            ),
            vec![PathBuf::from("/workspace/.oh-my-zsh/oh-my-zsh.sh")]
        );
    }

    #[test]
    fn zinit_source_suffix_rewrites_framework_bootstrap_paths_only() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zinit".to_owned(), "/workspace/zinit".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/script.zsh"),
                "/not-installed/.zinit/bin/zinit.zsh",
            ),
            vec![PathBuf::from("/workspace/zinit/zinit.zsh")]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/zinit/zinit.zsh"),
                "/opt/app/share/zinit-autoload.zsh",
            ),
            vec![PathBuf::from("/workspace/zinit/share/zinit-autoload.zsh")]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/script.zsh"),
                "/opt/app/share/zinit-autoload.zsh",
            ),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn zinit_plugin_root_resolves_bootstrap_source_paths() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zinit".to_owned(), "/workspace/zinit".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/app/.zshrc"),
                "/not-installed/.zinit/bin/zinit.zsh",
            ),
            vec![PathBuf::from("/workspace/zinit/zinit.zsh")]
        );
    }

    #[test]
    fn zinit_plugin_root_resolves_zi_alias_bootstrap_source_paths() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zi".to_owned(), "/workspace/zi".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/app/.zshrc"),
                "/not-installed/.zi/bin/zinit.zsh",
            ),
            vec![PathBuf::from("/workspace/zi/zinit.zsh")]
        );
    }

    #[test]
    fn zinit_plugin_root_does_not_use_oh_my_zsh_plugin_layout() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zinit".to_owned(), "/workspace/zinit".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let request = PluginRequest {
            framework: PluginFramework::Zinit,
            kind: PluginRequestKind::Plugin,
            name: "owner/repo".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: true,
            root_hint: None,
        };

        assert_eq!(
            settings.resolve_plugin_request(Path::new("/workspace/app/.zshrc"), &request),
            PluginResolution::default()
        );
    }

    #[test]
    fn prezto_plugin_root_resolves_module_entrypoint() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("prezto".to_owned(), "/workspace/prezto".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let request = PluginRequest {
            framework: PluginFramework::Prezto,
            kind: PluginRequestKind::Plugin,
            name: "editor".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: false,
            root_hint: None,
        };

        assert_eq!(
            settings.resolve_plugin_request(Path::new("/workspace/app/.zshrc"), &request),
            PluginResolution {
                entrypoints: vec![PathBuf::from("/workspace/prezto/modules/editor/init.zsh")],
                file_entry_contracts: Vec::new(),
            }
        );
    }

    #[test]
    fn prezto_theme_requests_do_not_use_oh_my_zsh_theme_layout() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("prezto".to_owned(), "/workspace/prezto".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let request = PluginRequest {
            framework: PluginFramework::Prezto,
            kind: PluginRequestKind::Theme,
            name: "sorin".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: false,
            root_hint: None,
        };

        assert_eq!(
            settings.resolve_plugin_request(Path::new("/workspace/app/.zshrc"), &request),
            PluginResolution::default()
        );
    }

    #[test]
    fn zdot_plugin_root_resolves_module_entrypoint() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zdot".to_owned(), "/workspace/zdot".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let request = PluginRequest {
            framework: PluginFramework::Zdot,
            kind: PluginRequestKind::Plugin,
            name: "fzf".to_owned(),
            span: shuck_ast::Span::new(),
            explicit: false,
            root_hint: None,
        };

        assert_eq!(
            settings.resolve_plugin_request(Path::new("/workspace/app/.zshrc"), &request),
            PluginResolution {
                entrypoints: vec![PathBuf::from("/workspace/zdot/modules/fzf/fzf.zsh")],
                file_entry_contracts: Vec::new(),
            }
        );
    }

    #[test]
    fn zdot_source_suffix_rewrites_framework_paths_only() {
        let settings = ResolvedZshPluginSettings::resolve(
            PathBuf::from("/workspace"),
            true,
            BTreeMap::from([("zdot".to_owned(), "/workspace/zdot".to_owned())]),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/zdot/zdot.zsh"),
                "/not-installed/zdot/core/hooks.zsh",
            ),
            vec![PathBuf::from("/workspace/zdot/core/hooks.zsh")]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/zdot/zdot.zsh"),
                "/not-installed/zdot/modules/fzf/fzf.zsh",
            ),
            vec![PathBuf::from("/workspace/zdot/modules/fzf/fzf.zsh")]
        );
        assert_eq!(
            settings.resolve_source_path(
                Path::new("/workspace/app/.zshrc"),
                "/tmp/other/core/hooks.zsh",
            ),
            Vec::<PathBuf>::new()
        );
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
