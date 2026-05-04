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

    fn from_options(options: &ClientOptions) -> Self {
        Self {
            fix_all: options.fix_all.unwrap_or(true),
            unsafe_fixes: options.unsafe_fixes.unwrap_or(false),
            show_syntax_errors: options.show_syntax_errors.unwrap_or(false),
        }
    }
}

impl ShuckSettings {
    pub(crate) fn resolve(
        file_path: Option<&Path>,
        workspace_roots: &[PathBuf],
        options: &ClientOptions,
    ) -> Self {
        let mut config = ShuckConfig::default();

        let project_root = file_path.map(|file_path| {
            let fallback_root = containing_workspace_root(file_path, workspace_roots)
                .or_else(|| file_path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            let project_root = resolve_project_root_for_file(file_path, &fallback_root, true)
                .unwrap_or(fallback_root.clone());

            config = load_project_config(&project_root, &ConfigArguments::default())
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        "Failed to load shuck config for {}: {error}",
                        project_root.display()
                    );
                    ShuckConfig::default()
                });
            project_root
        });
        apply_config_overrides(&mut config, options.to_config_overrides());
        let config_root = project_root
            .clone()
            .unwrap_or_else(|| workspace_roots.first().cloned().unwrap_or_else(|| PathBuf::from(".")));

        Self {
            linter: linter_settings_for_config(&config_root, file_path, &config),
            formatter: formatter_settings_for_config(&config),
            fixable_rules: fixable_rules_for_config(&config),
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

fn linter_settings_for_config(
    project_root: &Path,
    file_path: Option<&Path>,
    config: &ShuckConfig,
) -> LinterSettings {
    let mut rules = LinterSettings::default_rules();
    rules = apply_selector_list(rules, config.lint.select.as_deref());
    rules = rules.union(&selectors_to_rule_set(
        config.lint.extend_select.as_deref().unwrap_or(&[]),
    ));
    rules = rules.subtract(&selectors_to_rule_set(
        config.lint.ignore.as_deref().unwrap_or(&[]),
    ));

    let per_file_ignores = per_file_ignores_for_config(config);
    let compiled_per_file_ignores =
        CompiledPerFileIgnoreList::resolve(project_root.to_path_buf(), per_file_ignores)
            .unwrap_or_default();
    let shell = file_path
        .and_then(|file_path| {
            CompiledPerFileShellList::from_config(project_root.to_path_buf(), config)
                .shell_for_path(file_path)
        })
        .unwrap_or(LinterShellDialect::Unknown);

    LinterSettings {
        rules,
        shell,
        per_file_ignores: Arc::new(compiled_per_file_ignores),
        rule_options: linter_rule_options_for_lint_config(&config.lint),
        ..LinterSettings::default()
    }
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

fn apply_selector_list(rules: RuleSet, selectors: Option<&[String]>) -> RuleSet {
    selectors.map_or(rules, selectors_to_rule_set)
}

fn fixable_rules_for_config(config: &ShuckConfig) -> RuleSet {
    let fixable = parse_optional_selector_list(config.lint.fixable.as_deref(), "lint.fixable");
    let extend_fixable =
        parse_selector_list(config.lint.extend_fixable.as_deref(), "lint.extend-fixable");
    let unfixable = parse_selector_list(config.lint.unfixable.as_deref(), "lint.unfixable");

    apply_rule_selector_layer(
        RuleSet::all(),
        fixable.as_deref(),
        &extend_fixable,
        &unfixable,
    )
}

fn selectors_to_rule_set(selectors: &[String]) -> RuleSet {
    selectors_to_rule_set_from_parsed(&parse_selectors(selectors).unwrap_or_default())
}

fn selectors_to_rule_set_from_parsed(selectors: &[RuleSelector]) -> RuleSet {
    selectors
        .iter()
        .fold(RuleSet::EMPTY, |rules, selector| rules.union(&selector.into_rule_set()))
}

fn per_file_ignores_for_config(config: &ShuckConfig) -> Vec<PerFileIgnore> {
    let mut per_file_ignores = Vec::new();
    if let Some(entries) = config.lint.per_file_ignores.as_ref() {
        per_file_ignores.extend(parse_per_file_ignore_map(entries));
    }
    if let Some(entries) = config.lint.extend_per_file_ignores.as_ref() {
        per_file_ignores.extend(parse_per_file_ignore_map(entries));
    }
    per_file_ignores
}

fn parse_per_file_ignore_map(
    entries: &BTreeMap<String, Vec<String>>,
) -> impl Iterator<Item = PerFileIgnore> + '_ {
    entries.iter().filter_map(|(pattern, selectors)| {
        parse_selectors(selectors).ok().map(|parsed| {
            PerFileIgnore::new(pattern.clone(), selectors_to_rule_set_from_parsed(&parsed))
        })
    })
}

fn parse_selectors(selectors: &[String]) -> anyhow::Result<Vec<RuleSelector>> {
    selectors
        .iter()
        .map(|selector| RuleSelector::from_str(selector).map_err(anyhow::Error::new))
        .collect()
}

fn parse_optional_selector_list(
    selectors: Option<&[String]>,
    scope: &str,
) -> Option<Vec<RuleSelector>> {
    selectors.map(|selectors| match parse_selectors(selectors) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!("Ignoring invalid {scope} selectors: {error}");
            Vec::new()
        }
    })
}

fn parse_selector_list(selectors: Option<&[String]>, scope: &str) -> Vec<RuleSelector> {
    parse_optional_selector_list(selectors, scope).unwrap_or_default()
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

fn linter_rule_options_for_lint_config(lint: &LintConfig) -> LinterRuleOptions {
    let mut rule_options = LinterRuleOptions::default();
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
    fn from_config(project_root: PathBuf, config: &ShuckConfig) -> Self {
        let mut per_file_shell = Vec::new();
        if let Some(entries) = config.lint.per_file_shell.as_ref() {
            per_file_shell.extend(parse_per_file_shell_map(entries, "lint.per-file-shell"));
        }
        if let Some(entries) = config.lint.extend_per_file_shell.as_ref() {
            per_file_shell.extend(parse_per_file_shell_map(
                entries,
                "lint.extend-per-file-shell",
            ));
        }

        match Self::resolve(project_root, per_file_shell) {
            Ok(compiled) => compiled,
            Err(error) => {
                tracing::warn!("Failed to compile per-file shell settings: {error}");
                Self::default()
            }
        }
    }

    fn resolve(
        project_root: PathBuf,
        per_file_shell: Vec<PerFileShell>,
    ) -> anyhow::Result<Self> {
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

fn parse_per_file_shell_map(
    entries: &BTreeMap<String, String>,
    scope: &str,
) -> Vec<PerFileShell> {
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
            &ClientOptions::default(),
        );

        assert_eq!(settings.project_root(), Some(tempdir.path()));
        assert!(settings.linter().rules.contains(shuck_linter::Rule::UnusedAssignment));
        assert_eq!(settings.linter().rules.len(), 1);
    }

    #[test]
    fn shuck_settings_merge_extended_per_file_ignores() {
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
            &ClientOptions::default(),
        );

        let ignored = settings.linter().per_file_ignores.ignored_rules(&file_path);
        assert!(ignored.contains(shuck_linter::Rule::UnusedAssignment));
        assert!(ignored.contains(shuck_linter::Rule::UndefinedVariable));
    }

    #[test]
    fn shuck_settings_apply_per_file_shell_override() {
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
            &ClientOptions::default(),
        );

        assert_eq!(settings.linter().shell, LinterShellDialect::Zsh);
    }

    #[test]
    fn shuck_settings_honor_unfixable_rule_config() {
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
            &ClientOptions::default(),
        );

        assert!(!settings.fixable_rules().contains(shuck_linter::Rule::UnusedAssignment));
    }

    #[test]
    fn shuck_settings_apply_client_overrides_without_a_file_path() {
        let settings = ShuckSettings::resolve(
            None,
            &[],
            &ClientOptions {
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
            },
        );

        assert!(settings.linter().rules.contains(shuck_linter::Rule::UnusedAssignment));
        assert_eq!(settings.linter().rules.len(), 1);
        assert_eq!(
            settings.formatter().indent_style(),
            shuck_formatter::IndentStyle::Space
        );
        assert_eq!(settings.formatter().indent_width(), 2);
    }
}
