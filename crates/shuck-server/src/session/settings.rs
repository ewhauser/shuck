use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use globset::{Glob, GlobMatcher};
use shuck_config::{
    ConfigArguments, FormatSettingsPatch, LintConfig, ShuckConfig, apply_config_overrides,
    load_project_config, resolve_project_root_for_file,
};
use shuck_formatter::{ShellDialect as FormatDialect, ShellFormatOptions};
use shuck_linter::{
    CompiledPerFileIgnoreList, LinterRuleOptions, LinterSettings, PerFileIgnore, RuleSelector,
    RuleSet, ShellDialect as LinterShellDialect,
};

use crate::session::ClientOptions;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientSettings {
    fix_all: bool,
    unsafe_fixes: bool,
    show_syntax_errors: bool,
}

#[derive(Clone, Debug)]
pub struct ShuckSettings {
    linter: LinterSettings,
    formatter: ShellFormatOptions,
    fixable_rules: RuleSet,
    project_root: Option<PathBuf>,
}

pub struct GlobalClientSettings {
    options: ClientOptions,
    #[allow(dead_code)]
    client: crate::Client,
}

impl ClientSettings {
    pub(crate) fn fix_all(&self) -> bool {
        self.fix_all
    }

    pub(crate) fn unsafe_fixes(&self) -> bool {
        self.unsafe_fixes
    }

    pub(crate) fn show_syntax_errors(&self) -> bool {
        self.show_syntax_errors
    }

    pub(crate) fn from_options(options: &ClientOptions) -> Self {
        Self::from_layered_options(&[options])
    }

    pub(crate) fn from_layered_options(option_layers: &[&ClientOptions]) -> Self {
        let mut fix_all = None;
        let mut unsafe_fixes = None;
        let mut show_syntax_errors = None;

        for options in option_layers {
            if options.fix_all.is_some() {
                fix_all = options.fix_all;
            }
            if options.unsafe_fixes.is_some() {
                unsafe_fixes = options.unsafe_fixes;
            }
            if options.show_syntax_errors.is_some() {
                show_syntax_errors = options.show_syntax_errors;
            }
        }

        Self {
            fix_all: fix_all.unwrap_or(true),
            unsafe_fixes: unsafe_fixes.unwrap_or(false),
            show_syntax_errors: show_syntax_errors.unwrap_or(false),
        }
    }
}

impl ShuckSettings {
    pub(crate) fn resolve(
        file_path: Option<&Path>,
        workspace_roots: &[PathBuf],
        option_layers: &[&ClientOptions],
    ) -> Self {
        let mut project_config = ShuckConfig::default();

        let project_root = file_path.map(|file_path| {
            let fallback_root = containing_workspace_root(file_path, workspace_roots)
                .or_else(|| file_path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            let project_root = resolve_project_root_for_file(file_path, &fallback_root, true)
                .unwrap_or(fallback_root.clone());

            project_config = load_project_config(&project_root, &ConfigArguments::default())
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        "Failed to load shuck config for {}: {error}",
                        project_root.display()
                    );
                    ShuckConfig::default()
                });
            project_root
        });
        let config_root = project_root.clone().unwrap_or_else(|| {
            workspace_roots
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("."))
        });

        Self {
            linter: linter_settings_for_layers(
                &config_root,
                file_path,
                &project_config,
                option_layers,
            ),
            formatter: formatter_settings_for_layers(&project_config, option_layers),
            fixable_rules: fixable_rules_for_layers(&project_config, option_layers),
            project_root,
        }
    }

    pub(crate) fn linter(&self) -> &LinterSettings {
        &self.linter
    }

    pub(crate) fn formatter(&self) -> &ShellFormatOptions {
        &self.formatter
    }

    pub(crate) fn fixable_rules(&self) -> RuleSet {
        self.fixable_rules
    }

    pub(crate) fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }
}

impl Default for ShuckSettings {
    fn default() -> Self {
        Self {
            linter: LinterSettings::default(),
            formatter: ShellFormatOptions::default(),
            fixable_rules: RuleSet::all(),
            project_root: None,
        }
    }
}

impl GlobalClientSettings {
    pub(super) fn new(options: ClientOptions, client: crate::Client) -> Self {
        Self { options, client }
    }

    pub(super) fn to_settings_arc(&self) -> Arc<ClientSettings> {
        Arc::new(ClientSettings::from_options(&self.options))
    }

    pub(crate) fn options(&self) -> &ClientOptions {
        &self.options
    }

    pub(crate) fn update_options(&mut self, options: ClientOptions) {
        self.options = options;
    }
}

fn containing_workspace_root(path: &Path, workspace_roots: &[PathBuf]) -> Option<PathBuf> {
    workspace_roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
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
        PerFileIgnore::new(
            self.pattern,
            selectors_to_rule_set_from_parsed(&self.selectors),
        )
    }
}

fn lint_layers<'a>(
    project_config: &'a ShuckConfig,
    option_layers: &'a [&'a ClientOptions],
) -> impl Iterator<Item = RuleSelectionLayer> + 'a {
    std::iter::once(parse_lint_config_layer(&project_config.lint)).chain(option_layers.iter().map(
        |options| {
            options
                .lint
                .as_ref()
                .map(parse_lint_config_layer)
                .unwrap_or_default()
        },
    ))
}

fn linter_settings_for_layers(
    project_root: &Path,
    file_path: Option<&Path>,
    project_config: &ShuckConfig,
    option_layers: &[&ClientOptions],
) -> LinterSettings {
    let mut rules = LinterSettings::default_rules();
    let mut per_file_ignores = Vec::new();
    let mut per_file_shell = Vec::new();
    let mut rule_options = LinterRuleOptions::default();

    for layer in lint_layers(project_config, option_layers) {
        rules = apply_rule_selector_layer(
            rules,
            layer.select.as_deref(),
            &layer.extend_select,
            &layer.ignore,
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
    for lint in std::iter::once(&project_config.lint).chain(
        option_layers
            .iter()
            .filter_map(|options| options.lint.as_ref()),
    ) {
        apply_linter_rule_options_for_lint_config(&mut rule_options, lint);
    }

    let compiled_per_file_ignores =
        CompiledPerFileIgnoreList::resolve(project_root.to_path_buf(), per_file_ignores)
            .unwrap_or_default();
    let shell = file_path
        .and_then(|file_path| {
            CompiledPerFileShellList::resolve(project_root.to_path_buf(), per_file_shell)
                .unwrap_or_else(|error| {
                    tracing::warn!("Failed to compile per-file shell settings: {error}");
                    CompiledPerFileShellList::default()
                })
                .shell_for_path(file_path)
        })
        .unwrap_or(LinterShellDialect::Unknown);

    LinterSettings {
        rules,
        shell,
        per_file_ignores: Arc::new(compiled_per_file_ignores),
        rule_options,
        ..LinterSettings::default()
    }
}

fn formatter_settings_for_layers(
    project_config: &ShuckConfig,
    option_layers: &[&ClientOptions],
) -> ShellFormatOptions {
    let mut config = ShuckConfig {
        format: project_config.format.clone(),
        ..ShuckConfig::default()
    };
    for options in option_layers {
        apply_config_overrides(&mut config, options.to_config_overrides());
    }

    formatter_settings_for_config(&config)
}

fn formatter_settings_for_config(config: &ShuckConfig) -> ShellFormatOptions {
    let patch = config.format.to_patch().unwrap_or(FormatSettingsPatch {
        ..FormatSettingsPatch::default()
    });
    let mut options = ShellFormatOptions::default();
    if let Some(indent_style) = patch.indent_style {
        options = options.with_indent_style(indent_style);
    }
    if let Some(indent_width) = patch.indent_width {
        options = options.with_indent_width(indent_width);
    }
    if let Some(binary_next_line) = patch.binary_next_line {
        options = options.with_binary_next_line(binary_next_line);
    }
    if let Some(switch_case_indent) = patch.switch_case_indent {
        options = options.with_switch_case_indent(switch_case_indent);
    }
    if let Some(space_redirects) = patch.space_redirects {
        options = options.with_space_redirects(space_redirects);
    }
    if let Some(keep_padding) = patch.keep_padding {
        options = options.with_keep_padding(keep_padding);
    }
    if let Some(function_next_line) = patch.function_next_line {
        options = options.with_function_next_line(function_next_line);
    }
    if let Some(never_split) = patch.never_split {
        options = options.with_never_split(never_split);
    }
    if let Some(simplify) = patch.simplify {
        options = options.with_simplify(simplify);
    }
    if let Some(minify) = patch.minify {
        options = options.with_minify(minify);
    }
    if let Some(dialect) = patch.dialect {
        options = options.with_dialect(match dialect {
            FormatDialect::Auto => FormatDialect::Auto,
            FormatDialect::Bash => FormatDialect::Bash,
            FormatDialect::Posix => FormatDialect::Posix,
            FormatDialect::Mksh => FormatDialect::Mksh,
            FormatDialect::Zsh => FormatDialect::Zsh,
        });
    }
    options
}

fn fixable_rules_for_layers(
    project_config: &ShuckConfig,
    option_layers: &[&ClientOptions],
) -> RuleSet {
    let mut fixable_rules = RuleSet::all();

    for layer in lint_layers(project_config, option_layers) {
        fixable_rules = apply_rule_selector_layer(
            fixable_rules,
            layer.fixable.as_deref(),
            &layer.extend_fixable,
            &layer.unfixable,
        );
    }

    fixable_rules
}

fn selectors_to_rule_set_from_parsed(selectors: &[RuleSelector]) -> RuleSet {
    selectors.iter().fold(RuleSet::EMPTY, |rules, selector| {
        rules.union(&selector.into_rule_set())
    })
}

fn parse_lint_config_layer(lint: &LintConfig) -> RuleSelectionLayer {
    RuleSelectionLayer {
        select: parse_selector_list(lint.select.as_deref(), "lint.select"),
        ignore: parse_selector_list(lint.ignore.as_deref(), "lint.ignore").unwrap_or_default(),
        extend_select: parse_selector_list(lint.extend_select.as_deref(), "lint.extend-select")
            .unwrap_or_default(),
        per_file_ignores: lint
            .per_file_ignores
            .as_ref()
            .map(|entries| parse_per_file_ignore_specs(entries, "lint.per-file-ignores")),
        extend_per_file_ignores: lint
            .extend_per_file_ignores
            .as_ref()
            .map(|entries| parse_per_file_ignore_specs(entries, "lint.extend-per-file-ignores"))
            .unwrap_or_default(),
        per_file_shell: lint
            .per_file_shell
            .as_ref()
            .map(|entries| parse_per_file_shell_map(entries, "lint.per-file-shell")),
        extend_per_file_shell: lint
            .extend_per_file_shell
            .as_ref()
            .map(|entries| parse_per_file_shell_map(entries, "lint.extend-per-file-shell"))
            .unwrap_or_default(),
        fixable: parse_selector_list(lint.fixable.as_deref(), "lint.fixable"),
        unfixable: parse_selector_list(lint.unfixable.as_deref(), "lint.unfixable")
            .unwrap_or_default(),
        extend_fixable: parse_selector_list(lint.extend_fixable.as_deref(), "lint.extend-fixable")
            .unwrap_or_default(),
    }
}

fn parse_per_file_ignore_specs(
    entries: &BTreeMap<String, Vec<String>>,
    scope: &str,
) -> Vec<PerFileIgnoreSpec> {
    entries
        .iter()
        .filter_map(|(pattern, selectors)| match parse_selectors(selectors) {
            Ok(parsed) => Some(PerFileIgnoreSpec {
                pattern: pattern.clone(),
                selectors: parsed,
            }),
            Err(error) => {
                tracing::warn!("Ignoring invalid {scope} entry for {pattern:?}: {error}");
                None
            }
        })
        .collect()
}

fn parse_selectors(selectors: &[String]) -> anyhow::Result<Vec<RuleSelector>> {
    selectors
        .iter()
        .map(|selector| RuleSelector::from_str(selector).map_err(anyhow::Error::new))
        .collect()
}

fn parse_selector_list(selectors: Option<&[String]>, scope: &str) -> Option<Vec<RuleSelector>> {
    let selectors = selectors?;
    match parse_selectors(selectors) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            tracing::warn!("Ignoring invalid {scope} selectors: {error}");
            None
        }
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

    let mut updates = std::collections::HashMap::<shuck_linter::Rule, bool>::new();
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

fn linter_rule_options_for_lint_config(lint: &LintConfig) -> LinterRuleOptions {
    let mut rule_options = LinterRuleOptions::default();
    apply_linter_rule_options_for_lint_config(&mut rule_options, lint);
    rule_options
}

fn apply_linter_rule_options_for_lint_config(
    rule_options: &mut LinterRuleOptions,
    lint: &LintConfig,
) {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PerFileShell {
    pattern: String,
    shell: LinterShellDialect,
}

#[derive(Debug, Clone, Default)]
struct CompiledPerFileShellList {
    project_root: PathBuf,
    entries: Vec<CompiledPerFileShell>,
}

#[derive(Debug, Clone)]
struct CompiledPerFileShell {
    basename_matcher: GlobMatcher,
    relative_matcher: GlobMatcher,
    absolute_matcher: GlobMatcher,
    negated: bool,
    shell: LinterShellDialect,
}

impl CompiledPerFileShellList {
    fn resolve(project_root: PathBuf, per_file_shell: Vec<PerFileShell>) -> anyhow::Result<Self> {
        let entries = per_file_shell
            .into_iter()
            .map(|per_file_shell| {
                let mut pattern = per_file_shell.pattern;
                let negated = pattern.starts_with('!');
                if negated {
                    pattern.drain(..1);
                }

                let basename_matcher = Glob::new(&pattern)
                    .map_err(|err| anyhow::anyhow!("invalid glob {pattern:?}: {err}"))?
                    .compile_matcher();
                let relative_matcher = Glob::new(&pattern)
                    .map_err(|err| anyhow::anyhow!("invalid glob {pattern:?}: {err}"))?
                    .compile_matcher();
                let absolute_matcher = Glob::new(&pattern)
                    .map_err(|err| anyhow::anyhow!("invalid glob {pattern:?}: {err}"))?
                    .compile_matcher();

                Ok(CompiledPerFileShell {
                    basename_matcher,
                    relative_matcher,
                    absolute_matcher,
                    negated,
                    shell: per_file_shell.shell,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(Self {
            project_root,
            entries,
        })
    }

    fn shell_for_path(&self, path: &Path) -> Option<LinterShellDialect> {
        let relative_path = path.strip_prefix(&self.project_root).unwrap_or(path);
        let file_name = relative_path.file_name().or_else(|| path.file_name())?;

        self.entries.iter().fold(None, |shell, entry| {
            let matches = entry.basename_matcher.is_match(file_name)
                || entry.relative_matcher.is_match(relative_path)
                || matches_absolute_path(&entry.absolute_matcher, path);
            let applies = if entry.negated { !matches } else { matches };

            if applies { Some(entry.shell) } else { shell }
        })
    }
}

fn parse_per_file_shell_map(entries: &BTreeMap<String, String>, scope: &str) -> Vec<PerFileShell> {
    entries
        .iter()
        .filter_map(|(pattern, shell_name)| {
            let shell = LinterShellDialect::from_name(shell_name);
            if shell == LinterShellDialect::Unknown {
                tracing::warn!("Ignoring invalid {scope} entry for {pattern:?}: {shell_name:?}");
                return None;
            }

            Some(PerFileShell {
                pattern: pattern.clone(),
                shell,
            })
        })
        .collect()
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

fn matches_absolute_path(matcher: &GlobMatcher, path: &Path) -> bool {
    matcher.is_match(path)
        || matcher.is_match(normalize_path(path))
        || slash_normalized_match_path(path)
            .as_deref()
            .is_some_and(|normalized| matcher.is_match(normalized))
        || normalized_absolute_match_path(path)
            .as_ref()
            .is_some_and(|normalized| {
                matcher.is_match(normalized)
                    || slash_normalized_match_path(normalized)
                        .as_deref()
                        .is_some_and(|slash_normalized| matcher.is_match(slash_normalized))
            })
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

fn normalized_absolute_match_path(path: &Path) -> Option<PathBuf> {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn client_settings_default_to_safe_editor_behavior() {
        let settings = ClientSettings::from_options(&ClientOptions::default());
        assert!(settings.fix_all());
        assert!(!settings.unsafe_fixes());
        assert!(!settings.show_syntax_errors());
    }

    #[test]
    fn shuck_settings_load_rule_selection_from_config() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(settings.project_root(), Some(tempdir.path()));
        assert!(
            settings
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(settings.linter().rules.len(), 1);
    }

    #[test]
    fn shuck_settings_merge_extended_per_file_ignores() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nper-file-ignores = { '*.sh' = ['C001'] }\nextend-per-file-ignores = { '*.sh' = ['C006'] }\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        let ignored = settings.linter().per_file_ignores.ignored_rules(&file_path);
        assert!(ignored.contains(shuck_linter::Rule::UnusedAssignment));
        assert!(ignored.contains(shuck_linter::Rule::UndefinedVariable));
    }

    #[test]
    fn shuck_settings_apply_per_file_shell_override() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nper-file-shell = { '*.sh' = 'zsh' }\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "echo ${(%):-%x}\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(settings.linter().shell, LinterShellDialect::Zsh);
    }

    #[test]
    fn shuck_settings_honor_unfixable_rule_config() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nunfixable = ['C001']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert!(
            !settings
                .fixable_rules()
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
    }

    #[test]
    fn shuck_settings_apply_client_overrides_without_a_file_path() {
        let options = ClientOptions {
            lint: Some(shuck_config::LintConfig {
                select: Some(vec!["C001".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            format: Some(shuck_config::FormatConfig {
                indent_style: Some("space".to_owned()),
                indent_width: Some(2),
                ..shuck_config::FormatConfig::default()
            }),
            ..ClientOptions::default()
        };
        let settings = ShuckSettings::resolve(None, &[], &[&options]);

        assert!(
            settings
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(settings.linter().rules.len(), 1);
        assert_eq!(
            settings.formatter().indent_style(),
            shuck_formatter::IndentStyle::Space
        );
        assert_eq!(settings.formatter().indent_width(), 2);
    }

    #[test]
    fn invalid_selectors_leave_default_rules_enabled() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nselect = ['C001', 'oops']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(settings.linter().rules, LinterSettings::default_rules());
    }

    #[test]
    fn invalid_ignore_selectors_do_not_clear_valid_rule_selection() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nselect = ['C001']\nignore = ['oops']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert!(
            settings
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(settings.linter().rules.len(), 1);
    }

    #[test]
    fn invalid_fixable_selectors_leave_fixable_rules_enabled() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nfixable = ['oops']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(settings.fixable_rules(), RuleSet::all());
    }

    #[test]
    fn later_select_replaces_earlier_extend_select() {
        let client_options = ClientOptions {
            lint: Some(shuck_config::LintConfig {
                select: Some(vec!["C001".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nextend-select = ['C006']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );

        assert!(
            settings
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert!(
            !settings
                .linter()
                .rules
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
        assert_eq!(settings.linter().rules.len(), 1);
    }

    #[test]
    fn later_fixable_replaces_earlier_extend_fixable() {
        let client_options = ClientOptions {
            lint: Some(shuck_config::LintConfig {
                fixable: Some(vec!["C001".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nextend-fixable = ['C006']\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );

        assert!(
            settings
                .fixable_rules()
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert!(
            !settings
                .fixable_rules()
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
    }

    #[test]
    fn later_per_file_shell_replaces_earlier_extend_per_file_shell() {
        let client_options = ClientOptions {
            lint: Some(shuck_config::LintConfig {
                per_file_shell: Some(BTreeMap::from([(
                    "portable.sh".to_owned(),
                    "sh".to_owned(),
                )])),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nextend-per-file-shell = { '*.sh' = 'bash' }\n",
        )
        .expect("config should be written");

        let portable_path = tempdir.path().join("portable.sh");
        let other_path = tempdir.path().join("other.sh");
        std::fs::write(&portable_path, "source helper.sh\n").expect("source should be written");
        std::fs::write(&other_path, "source helper.sh\n").expect("source should be written");

        let portable_settings = ShuckSettings::resolve(
            Some(&portable_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );
        let other_settings = ShuckSettings::resolve(
            Some(&other_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );

        assert_eq!(portable_settings.linter().shell, LinterShellDialect::Sh);
        assert_eq!(other_settings.linter().shell, LinterShellDialect::Unknown);
    }

    #[test]
    fn later_per_file_ignores_replace_earlier_extend_per_file_ignores() {
        let client_options = ClientOptions {
            lint: Some(shuck_config::LintConfig {
                per_file_ignores: Some(BTreeMap::from([(
                    "portable.sh".to_owned(),
                    vec!["C006".to_owned()],
                )])),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        };
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint]\nextend-per-file-ignores = { '*.sh' = ['C001'] }\n",
        )
        .expect("config should be written");

        let portable_path = tempdir.path().join("portable.sh");
        let other_path = tempdir.path().join("other.sh");
        std::fs::write(&portable_path, "foo=1\n").expect("source should be written");
        std::fs::write(&other_path, "foo=1\n").expect("source should be written");

        let portable_settings = ShuckSettings::resolve(
            Some(&portable_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );
        let other_settings = ShuckSettings::resolve(
            Some(&other_path),
            &[tempdir.path().to_path_buf()],
            &[&client_options],
        );

        let portable_ignored = portable_settings
            .linter()
            .per_file_ignores
            .ignored_rules(&portable_path);
        let other_ignored = other_settings
            .linter()
            .per_file_ignores
            .ignored_rules(&other_path);

        assert!(portable_ignored.contains(shuck_linter::Rule::UndefinedVariable));
        assert!(!portable_ignored.contains(shuck_linter::Rule::UnusedAssignment));
        assert!(other_ignored.is_empty());
    }
}
