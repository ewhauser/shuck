use std::path::{Path, PathBuf};
use std::sync::Arc;

use shuck_config::resolve::{
    ResolvePolicy, ResolvedFormatSettings, ResolvedLintSettings, ResolvedZshPluginSettings,
    SettingsOverride, SettingsResolver,
};
use shuck_config::{
    ConfigArguments, FormatExclusions, ShuckConfig, load_project_config,
    resolve_project_root_for_file,
};
use shuck_formatter::ShellFormatOptions;
use shuck_linter::{LinterSettings, RuleSet};

use crate::session::{
    ClientOptions, CompletionFeatureOptions, RenameFeatureOptions, WorkspaceSymbolFeatureOptions,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientSettings {
    fix_all: bool,
    unsafe_fixes: bool,
    show_syntax_errors: bool,
    completion: CompletionFeatureOptions,
    rename: RenameFeatureOptions,
}

#[derive(Clone, Debug)]
pub struct ShuckSettings {
    linter: LinterSettings,
    zsh_plugins: Arc<ResolvedZshPluginSettings>,
    formatter: ShellFormatOptions,
    format_dialect_error: Option<String>,
    format_exclusions: FormatExclusions,
    fixable_rules: RuleSet,
    project_root: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsResolveContext {
    config_root: PathBuf,
    project_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedProjectSettings {
    lint: ResolvedLintSettings,
    format: ResolvedFormatSettings,
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

    pub(crate) fn completion(&self) -> CompletionFeatureOptions {
        self.completion
    }

    pub(crate) fn rename(&self) -> RenameFeatureOptions {
        self.rename
    }

    pub(crate) fn from_options(options: &ClientOptions) -> Self {
        Self::from_layered_options(&[options])
    }

    pub(crate) fn from_layered_options(option_layers: &[&ClientOptions]) -> Self {
        let mut fix_all = None;
        let mut unsafe_fixes = None;
        let mut show_syntax_errors = None;
        let mut completion = CompletionFeatureOptions::default();
        let mut rename = RenameFeatureOptions::default();

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
            completion = options.server.completion_layered_over(completion);
            rename = options.server.rename_layered_over(rename);
        }

        Self {
            fix_all: fix_all.unwrap_or(true),
            unsafe_fixes: unsafe_fixes.unwrap_or(false),
            show_syntax_errors: show_syntax_errors.unwrap_or(false),
            completion,
            rename,
        }
    }
}

impl ShuckSettings {
    pub(crate) fn resolve(
        file_path: Option<&Path>,
        workspace_roots: &[PathBuf],
        option_layers: &[&ClientOptions],
    ) -> Self {
        let context = file_path
            .map(|file_path| SettingsResolveContext::for_file(file_path, workspace_roots))
            .unwrap_or_else(|| SettingsResolveContext::without_file(workspace_roots));
        ResolvedProjectSettings::resolve(&context, option_layers).for_file(file_path)
    }

    pub(crate) fn linter(&self) -> &LinterSettings {
        &self.linter
    }

    pub(crate) fn zsh_plugins(&self) -> &ResolvedZshPluginSettings {
        &self.zsh_plugins
    }

    pub(crate) fn formatter(&self) -> &ShellFormatOptions {
        &self.formatter
    }

    pub(crate) fn format_dialect_error(&self) -> Option<&str> {
        self.format_dialect_error.as_deref()
    }

    pub(crate) fn is_format_excluded(&self, path: Option<&Path>) -> bool {
        path.is_some_and(|path| self.format_exclusions.is_excluded(path))
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
        let lint = ResolvedLintSettings::default();
        Self {
            linter: lint.linter,
            zsh_plugins: lint.zsh_plugins,
            formatter: ShellFormatOptions::default(),
            format_dialect_error: None,
            format_exclusions: FormatExclusions::default(),
            fixable_rules: lint.fixable_rules,
            project_root: None,
        }
    }
}

impl SettingsResolveContext {
    pub(crate) fn for_file(file_path: &Path, workspace_roots: &[PathBuf]) -> Self {
        let fallback_root = containing_workspace_root(file_path, workspace_roots)
            .or_else(|| file_path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let project_root = resolve_project_root_for_file(file_path, &fallback_root, true)
            .unwrap_or(fallback_root.clone());

        Self {
            config_root: project_root.clone(),
            project_root: Some(project_root),
        }
    }

    pub(crate) fn without_file(workspace_roots: &[PathBuf]) -> Self {
        Self {
            config_root: workspace_roots
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from(".")),
            project_root: None,
        }
    }

    pub(crate) fn config_root(&self) -> &Path {
        &self.config_root
    }
}

impl ResolvedProjectSettings {
    pub(crate) fn resolve(
        context: &SettingsResolveContext,
        option_layers: &[&ClientOptions],
    ) -> Self {
        let project_config = context
            .project_root
            .as_ref()
            .map(|project_root| {
                load_project_config(project_root, &ConfigArguments::default()).unwrap_or_else(
                    |error| {
                        tracing::warn!(
                            "Failed to load shuck config for {}: {error}",
                            project_root.display()
                        );
                        ShuckConfig::default()
                    },
                )
            })
            .unwrap_or_default();
        let overrides = option_layers
            .iter()
            .map(|options| SettingsOverride::Config(Box::new(options.to_config_overrides())))
            .collect::<Vec<_>>();
        let mut resolver = SettingsResolver::new(
            context.config_root(),
            &project_config,
            ResolvePolicy::Lenient,
        )
        .with_overrides(&overrides);
        let lint = resolver.resolve_lint().unwrap_or_else(|error| {
            tracing::warn!("Failed to resolve shuck lint settings: {error:#}");
            ResolvedLintSettings::default()
        });
        let format = resolver.resolve_format().unwrap_or_else(|error| {
            tracing::warn!("Failed to resolve shuck format settings: {error:#}");
            ResolvedFormatSettings::default()
        });
        for warning in resolver.warnings() {
            tracing::warn!("Ignoring invalid shuck setting: {warning}");
        }

        Self {
            lint,
            format,
            project_root: context.project_root.clone(),
        }
    }

    pub(crate) fn for_file(&self, file_path: Option<&Path>) -> ShuckSettings {
        let (formatter, format_dialect_error) = match file_path {
            Some(file_path) => match self.format.options_for_path(file_path) {
                Ok(options) => (options, None),
                Err(error) => (self.format.options.clone(), Some(error.to_string())),
            },
            None => (self.format.options.clone(), None),
        };
        ShuckSettings {
            linter: self.lint.linter_for_path(file_path),
            zsh_plugins: Arc::clone(&self.lint.zsh_plugins),
            formatter,
            format_dialect_error,
            format_exclusions: self.format.exclusions.clone(),
            fixable_rules: self.lint.fixable_rules,
            project_root: self.project_root.clone(),
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

    pub(crate) fn workspace_symbol_options(&self) -> WorkspaceSymbolFeatureOptions {
        self.options.server.workspace_symbols
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shuck_formatter::ShellDialect as FormatDialect;
    use shuck_linter::ShellDialect as LinterShellDialect;

    use super::*;

    #[test]
    fn client_settings_default_to_safe_editor_behavior() {
        let settings = ClientSettings::from_options(&ClientOptions::default());
        assert!(settings.fix_all());
        assert!(!settings.unsafe_fixes());
        assert!(!settings.show_syntax_errors());
        assert!(settings.rename().allow_cross_file);

        let options = serde_json::from_value(serde_json::json!({
            "server": {
                "rename": {
                    "allowCrossFile": false
                }
            }
        }))
        .expect("rename opt-out should deserialize");
        assert!(
            !ClientSettings::from_options(&options)
                .rename()
                .allow_cross_file
        );
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
    fn shared_per_file_shell_applies_to_linter_and_formatter() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[per-file-shell]\n'dot_z*' = 'zsh'\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("dot_zshenv");
        std::fs::write(&file_path, "(($+commands[vivid]))\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(settings.linter().shell, LinterShellDialect::Zsh);
        assert_eq!(settings.formatter().dialect(), FormatDialect::Zsh);
        assert_eq!(settings.format_dialect_error(), None);
    }

    #[test]
    fn shared_per_file_shell_conflicts_block_formatter_settings() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[per-file-shell]\n'*' = 'bash'\n'dot_z*' = 'zsh'\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("dot_zshenv");
        std::fs::write(&file_path, "(($+commands[vivid]))\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert!(
            settings
                .format_dialect_error()
                .is_some_and(|error| error.contains("conflicting per-file shell mappings"))
        );
    }

    #[test]
    fn shuck_settings_apply_s085_rule_options() {
        let options = ClientOptions::default();
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        std::fs::write(
            tempdir.path().join(".shuck.toml"),
            "[lint.rule-options.s085]\nnon-trivial-line-threshold = 20\nnon-trivial-function-count = 3\nmain-name = 'run'\n",
        )
        .expect("config should be written");

        let file_path = tempdir.path().join("script.sh");
        std::fs::write(&file_path, "run() { :; }\n").expect("source should be written");

        let settings = ShuckSettings::resolve(
            Some(&file_path),
            &[tempdir.path().to_path_buf()],
            &[&options],
        );

        assert_eq!(
            settings
                .linter()
                .rule_options
                .s085
                .non_trivial_line_threshold,
            20
        );
        assert_eq!(
            settings
                .linter()
                .rule_options
                .s085
                .non_trivial_function_count,
            3
        );
        assert_eq!(settings.linter().rule_options.s085.main_name, "run");
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
