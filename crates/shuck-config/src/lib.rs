#![warn(missing_docs)]

//! Configuration loading and override handling for Shuck commands.
//!
//! This crate owns the TOML shapes used by `.shuck.toml`, command-line
//! `--config` overrides, project-root discovery, and the small metadata model
//! used to render configuration reference docs.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use clap::builder::{TypedValueParser, ValueParserFactory};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use serde::Deserialize;
use shuck_formatter::{IndentStyle, ShellDialect};
use shuck_run::RunConfig;

const CONFIG_FILENAMES: [&str; 2] = [".shuck.toml", "shuck.toml"];
/// Error shown when users try to set formatter dialect in a config file.
pub const CONFIG_DIALECT_UNSUPPORTED_ERROR: &str = "`[format].dialect` is not supported; formatter dialect is auto-discovered from the file name or shebang. Use `--dialect` for a per-run override";
const CONFIG_OVERRIDE_ROOT_KEYS: &[&str] = &["check", "format", "lint", "run"];
const CONFIG_OVERRIDE_CHECK_KEYS: &[&str] = &["embedded"];
const CONFIG_OVERRIDE_FORMAT_KEYS: &[&str] = &[
    "dialect",
    "indent-style",
    "indent-width",
    "binary-next-line",
    "switch-case-indent",
    "space-redirects",
    "keep-padding",
    "function-next-line",
    "never-split",
];
const CONFIG_OVERRIDE_LINT_KEYS: &[&str] = &[
    "select",
    "ignore",
    "extend-select",
    "per-file-ignores",
    "extend-per-file-ignores",
    "per-file-shell",
    "extend-per-file-shell",
    "fixable",
    "unfixable",
    "extend-fixable",
    "rule-options",
    "contracts",
    "zsh",
];
const CONFIG_OVERRIDE_LINT_CONTRACT_KEYS: &[&str] = &["well-known", "disabled", "custom"];
const CONFIG_OVERRIDE_LINT_ZSH_KEYS: &[&str] = &["plugins"];
const CONFIG_OVERRIDE_LINT_ZSH_PLUGIN_KEYS: &[&str] = &[
    "resolution",
    "roots",
    "plugin-loads",
    "theme-loads",
    "entrypoints",
];
const CONFIG_OVERRIDE_LINT_RULE_OPTION_KEYS: &[&str] = &[
    "c001", "c063", "s078", "s079", "s080", "s081", "s084", "s085", "c158", "c159", "c160", "c161",
    "c162",
];
const CONFIG_OVERRIDE_C001_RULE_OPTION_KEYS: &[&str] =
    &["treat-indirect-expansion-targets-as-used"];
const CONFIG_OVERRIDE_C063_RULE_OPTION_KEYS: &[&str] = &["report-unreached-nested-definitions"];
const CONFIG_OVERRIDE_S078_RULE_OPTION_KEYS: &[&str] = &["allowed-shells"];
const CONFIG_OVERRIDE_S079_RULE_OPTION_KEYS: &[&str] = &["allowed-forms", "allowed-paths"];
const CONFIG_OVERRIDE_S080_RULE_OPTION_KEYS: &[&str] = &["max-lines", "count"];
const CONFIG_OVERRIDE_S080_COUNT_VALUES: &[&str] = &["physical", "non-comment-non-blank"];
const CONFIG_OVERRIDE_S081_RULE_OPTION_KEYS: &[&str] = &["ignore-shebang-only-files"];
const CONFIG_OVERRIDE_S084_RULE_OPTION_KEYS: &[&str] = &[
    "require-globals",
    "require-arguments",
    "require-outputs",
    "require-returns",
];
const CONFIG_OVERRIDE_S085_RULE_OPTION_KEYS: &[&str] = &[
    "non-trivial-line-threshold",
    "non-trivial-function-count",
    "main-name",
];
const CONFIG_OVERRIDE_C158_RULE_OPTION_KEYS: &[&str] = &[
    "treat-readonly-as-documented",
    "treat-export-as-intentional",
];
const CONFIG_OVERRIDE_C159_RULE_OPTION_KEYS: &[&str] = &["allow-conditional-init"];
const CONFIG_OVERRIDE_C160_RULE_OPTION_KEYS: &[&str] = &["allowed-anchors"];
const CONFIG_OVERRIDE_C161_RULE_OPTION_KEYS: &[&str] = &["ignore-after-source"];
const CONFIG_OVERRIDE_C162_RULE_OPTION_KEYS: &[&str] = &["treat-as-masking"];
const CONFIG_OVERRIDE_RUN_KEYS: &[&str] = &["shell", "shell-version", "shells"];
const CONFIG_OVERRIDE_RUN_SHELL_NAMES: &[&str] =
    &["bash", "gbash", "bashkit", "zsh", "dash", "mksh", "busybox"];

/// Top-level Shuck configuration loaded from project config files.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ShuckConfig {
    /// File discovery and embedded-script checking options.
    pub check: CheckConfig,
    /// Shell formatting options.
    pub format: FormatConfig,
    /// Linter rule selection, contracts, and analysis options.
    pub lint: LintConfig,
    /// Runtime shell resolution options for `shuck run`.
    pub run: RunConfig,
}

/// Configuration for file-level checking behavior.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CheckConfig {
    /// Whether to lint embedded shell snippets in supported host files.
    pub embedded: Option<bool>,
}

/// Configuration for shell formatting behavior.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FormatConfig {
    /// Deprecated formatter dialect value retained only to reject config-file use clearly.
    pub dialect: Option<toml::Value>,
    /// Requested indentation style, such as `tab` or `space`.
    pub indent_style: Option<String>,
    /// Requested indentation width.
    pub indent_width: Option<u8>,
    /// Whether binary operators should begin continuation lines.
    pub binary_next_line: Option<bool>,
    /// Whether `case` arms should receive an extra indentation level.
    pub switch_case_indent: Option<bool>,
    /// Whether redirection operators should be surrounded by spaces.
    pub space_redirects: Option<bool>,
    /// Whether existing horizontal padding should be preserved.
    pub keep_padding: Option<bool>,
    /// Whether function bodies should start on the next line.
    pub function_next_line: Option<bool>,
    /// Whether the formatter should avoid splitting lines.
    pub never_split: Option<bool>,
}

/// Configuration for lint rule selection and analysis behavior.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LintConfig {
    /// Rule selectors that replace the default enabled rule set.
    pub select: Option<Vec<String>>,
    /// Rule selectors removed from the enabled rule set.
    pub ignore: Option<Vec<String>>,
    /// Rule selectors added to the enabled rule set.
    pub extend_select: Option<Vec<String>>,
    /// Per-file ignore selectors keyed by glob pattern.
    pub per_file_ignores: Option<BTreeMap<String, Vec<String>>>,
    /// Per-file ignore selectors merged into existing per-file ignores.
    pub extend_per_file_ignores: Option<BTreeMap<String, Vec<String>>>,
    /// Per-file shell dialect overrides keyed by glob pattern.
    pub per_file_shell: Option<BTreeMap<String, String>>,
    /// Per-file shell dialect overrides merged into existing overrides.
    pub extend_per_file_shell: Option<BTreeMap<String, String>>,
    /// Rule selectors that replace the set eligible for automatic fixes.
    pub fixable: Option<Vec<String>>,
    /// Rule selectors excluded from automatic fixes.
    pub unfixable: Option<Vec<String>>,
    /// Rule selectors added to the fixable set.
    pub extend_fixable: Option<Vec<String>>,
    /// Rule-specific option values.
    pub rule_options: Option<LintRuleOptionsConfig>,
    /// Ambient contract configuration for framework and plugin assumptions.
    pub contracts: Option<LintContractsConfig>,
    /// Zsh-specific analysis configuration.
    pub zsh: Option<LintZshConfig>,
}

/// Configuration for ambient contracts used during lint analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintContractsConfig {
    /// Whether built-in well-known contracts are enabled.
    pub well_known: Option<bool>,
    /// Contract identifiers that should be disabled.
    pub disabled: Option<Vec<String>>,
    /// User-defined contract entries.
    pub custom: Option<Vec<LintCustomContractConfig>>,
}

/// User-defined ambient contract configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintCustomContractConfig {
    /// Stable contract identifier.
    pub id: String,
    /// Contract identifiers replaced by this contract.
    pub replaces: Option<Vec<String>>,
    /// Activation condition for this contract.
    pub when: LintContractWhenConfig,
    /// Glob patterns where this contract applies.
    pub files: Option<Vec<String>>,
    /// Variables read by the contract before script execution.
    pub reads: Option<Vec<String>>,
    /// Variable names consumed by the contract.
    pub consumes: Option<LintContractConsumesConfig>,
    /// Variables or functions provided by the contract.
    pub provides: Option<LintContractProvidesConfig>,
    /// Function-specific contract entries.
    pub functions: Option<Vec<LintContractFunctionConfig>>,
}

/// Activation condition for a user-defined contract.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LintContractWhenConfig {
    /// Always activate using a literal label.
    Always(String),
    /// Activate when a configured framework, plugin, or theme is detected.
    Activation(LintContractActivationConfig),
}

/// Framework, plugin, or theme activation details for a contract.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintContractActivationConfig {
    #[serde(rename = "type")]
    /// Activation class.
    pub activation_type: LintContractActivationTypeConfig,
    /// Optional framework name.
    pub framework: Option<String>,
    /// Optional plugin name.
    pub plugin: Option<String>,
    /// Optional theme name.
    pub theme: Option<String>,
}

/// Supported contract activation classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum LintContractActivationTypeConfig {
    /// Activate for a zsh plugin.
    #[serde(rename = "zsh_plugin")]
    ZshPlugin,
    /// Activate for a zsh theme.
    #[serde(rename = "zsh_theme")]
    ZshTheme,
}

/// Variables consumed by an ambient contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintContractConsumesConfig {
    /// Exact variable names consumed by the contract.
    pub names: Option<Vec<String>>,
    /// Variable-name prefixes consumed by the contract.
    pub prefixes: Option<Vec<String>>,
    /// Whether the contract may consume any variable.
    pub all: Option<bool>,
}

/// Values provided by an ambient contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintContractProvidesConfig {
    /// Variable names provided by the contract.
    pub variables: Option<Vec<String>>,
    /// Function names provided by the contract.
    pub functions: Option<Vec<String>>,
}

/// Function-specific ambient contract entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintContractFunctionConfig {
    /// Function name.
    pub name: String,
    /// Variables read by the function.
    pub reads: Option<Vec<String>>,
    /// Variables set by the function.
    pub sets: Option<Vec<String>>,
}

/// Zsh-specific lint configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintZshConfig {
    /// Zsh plugin resolution settings.
    pub plugins: Option<ZshPluginsConfig>,
}

/// Configuration for zsh plugin and theme resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ZshPluginsConfig {
    /// Whether plugin resolution is enabled.
    pub resolution: Option<bool>,
    /// Named plugin root directories.
    pub roots: Option<BTreeMap<String, String>>,
    /// Patterns that identify plugin loads.
    pub plugin_loads: Option<Vec<ZshPluginLoadConfig>>,
    /// Patterns that identify theme loads.
    pub theme_loads: Option<Vec<ZshThemeLoadConfig>>,
    /// Patterns that provide plugin entrypoint paths.
    pub entrypoints: Option<Vec<ZshPluginEntrypointConfig>>,
}

/// A configured zsh plugin load pattern.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ZshPluginLoadConfig {
    /// Pattern matched against shell source.
    pub pattern: String,
    /// Framework name that owns the plugin.
    pub framework: String,
    /// Plugin name.
    pub name: String,
}

/// A configured zsh theme load pattern.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ZshThemeLoadConfig {
    /// Pattern matched against shell source.
    pub pattern: String,
    /// Framework name that owns the theme.
    pub framework: String,
    /// Theme name.
    pub name: String,
}

/// A configured zsh plugin entrypoint pattern.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ZshPluginEntrypointConfig {
    /// Pattern matched against shell source.
    pub pattern: String,
    /// Entrypoint paths associated with the pattern.
    pub paths: Vec<String>,
}

/// Rule-specific lint options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintRuleOptionsConfig {
    /// Options for rule C001.
    pub c001: Option<C001RuleOptionsConfig>,
    /// Options for rule C063.
    pub c063: Option<C063RuleOptionsConfig>,
    /// Options for rule S078.
    pub s078: Option<S078RuleOptionsConfig>,
    /// Options for rule S079.
    pub s079: Option<S079RuleOptionsConfig>,
    /// Options for rule S080.
    pub s080: Option<S080RuleOptionsConfig>,
    /// Options for rule S081.
    pub s081: Option<S081RuleOptionsConfig>,
    /// Options for rule S084.
    pub s084: Option<S084RuleOptionsConfig>,
    /// Options for rule S085.
    pub s085: Option<S085RuleOptionsConfig>,
    /// Options for rule C158.
    pub c158: Option<C158RuleOptionsConfig>,
    /// Options for rule C159.
    pub c159: Option<C159RuleOptionsConfig>,
    /// Options for rule C160.
    pub c160: Option<C160RuleOptionsConfig>,
    /// Options for rule C161.
    pub c161: Option<C161RuleOptionsConfig>,
    /// Options for rule C162.
    pub c162: Option<C162RuleOptionsConfig>,
}

impl LintRuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if let Some(c001) = overrides.c001 {
            self.c001.get_or_insert_default().apply_overrides(c001);
        }
        if let Some(c063) = overrides.c063 {
            self.c063.get_or_insert_default().apply_overrides(c063);
        }
        if let Some(s078) = overrides.s078 {
            self.s078.get_or_insert_default().apply_overrides(s078);
        }
        if let Some(s079) = overrides.s079 {
            self.s079.get_or_insert_default().apply_overrides(s079);
        }
        if let Some(s080) = overrides.s080 {
            self.s080.get_or_insert_default().apply_overrides(s080);
        }
        if let Some(s081) = overrides.s081 {
            self.s081.get_or_insert_default().apply_overrides(s081);
        }
        if let Some(s084) = overrides.s084 {
            self.s084.get_or_insert_default().apply_overrides(s084);
        }
        if let Some(s085) = overrides.s085 {
            self.s085.get_or_insert_default().apply_overrides(s085);
        }
        if let Some(c158) = overrides.c158 {
            self.c158.get_or_insert_default().apply_overrides(c158);
        }
        if let Some(c159) = overrides.c159 {
            self.c159.get_or_insert_default().apply_overrides(c159);
        }
        if let Some(c160) = overrides.c160 {
            self.c160.get_or_insert_default().apply_overrides(c160);
        }
        if let Some(c161) = overrides.c161 {
            self.c161.get_or_insert_default().apply_overrides(c161);
        }
        if let Some(c162) = overrides.c162 {
            self.c162.get_or_insert_default().apply_overrides(c162);
        }
    }
}

/// Options for rule C001.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C001RuleOptionsConfig {
    /// Whether indirect expansion target names should count as variable usage.
    pub treat_indirect_expansion_targets_as_used: Option<bool>,
}

impl C001RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.treat_indirect_expansion_targets_as_used.is_some() {
            self.treat_indirect_expansion_targets_as_used =
                overrides.treat_indirect_expansion_targets_as_used;
        }
    }
}

/// Options for rule C063.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C063RuleOptionsConfig {
    /// Whether nested function definitions in unreachable branches should be reported.
    pub report_unreached_nested_definitions: Option<bool>,
}

impl C063RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.report_unreached_nested_definitions.is_some() {
            self.report_unreached_nested_definitions =
                overrides.report_unreached_nested_definitions;
        }
    }
}

/// Options for rule S078.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S078RuleOptionsConfig {
    /// Shell names accepted in shebang interpreters.
    pub allowed_shells: Option<Vec<String>>,
}

impl S078RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.allowed_shells.is_some() {
            self.allowed_shells = overrides.allowed_shells;
        }
    }
}

/// Options for rule S079.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S079RuleOptionsConfig {
    /// Shebang invocation forms accepted by the policy.
    pub allowed_forms: Option<Vec<String>>,
    /// Exact shebang command strings accepted by the policy.
    pub allowed_paths: Option<Vec<String>>,
}

impl S079RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.allowed_forms.is_some() {
            self.allowed_forms = overrides.allowed_forms;
        }
        if overrides.allowed_paths.is_some() {
            self.allowed_paths = overrides.allowed_paths;
        }
    }
}

/// Options for rule S080.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S080RuleOptionsConfig {
    /// Maximum line count allowed by the policy.
    pub max_lines: Option<usize>,
    /// Line counting mode used by the policy.
    pub count: Option<String>,
}

impl S080RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.max_lines.is_some() {
            self.max_lines = overrides.max_lines;
        }
        if overrides.count.is_some() {
            self.count = overrides.count;
        }
    }
}

/// Options for rule S081.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S081RuleOptionsConfig {
    /// Whether files that contain only a shebang are exempt from the rule.
    pub ignore_shebang_only_files: Option<bool>,
}

impl S081RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.ignore_shebang_only_files.is_some() {
            self.ignore_shebang_only_files = overrides.ignore_shebang_only_files;
        }
    }
}

/// Options for rule S084.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S084RuleOptionsConfig {
    /// Whether comments should document global variables used by a function body.
    pub require_globals: Option<bool>,
    /// Whether comments should document positional parameters used by a function body.
    pub require_arguments: Option<bool>,
    /// Whether comments should document stdout output produced by a function body.
    pub require_outputs: Option<bool>,
    /// Whether comments should document explicit return statuses from a function body.
    pub require_returns: Option<bool>,
}

impl S084RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.require_globals.is_some() {
            self.require_globals = overrides.require_globals;
        }
        if overrides.require_arguments.is_some() {
            self.require_arguments = overrides.require_arguments;
        }
        if overrides.require_outputs.is_some() {
            self.require_outputs = overrides.require_outputs;
        }
        if overrides.require_returns.is_some() {
            self.require_returns = overrides.require_returns;
        }
    }
}

/// Options for rule S085.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct S085RuleOptionsConfig {
    /// Minimum source line count before the script is checked.
    pub non_trivial_line_threshold: Option<usize>,
    /// Minimum function definition count before the script is checked.
    pub non_trivial_function_count: Option<usize>,
    /// Expected entrypoint function name.
    pub main_name: Option<String>,
}

impl S085RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.non_trivial_line_threshold.is_some() {
            self.non_trivial_line_threshold = overrides.non_trivial_line_threshold;
        }
        if overrides.non_trivial_function_count.is_some() {
            self.non_trivial_function_count = overrides.non_trivial_function_count;
        }
        if overrides.main_name.is_some() {
            self.main_name = overrides.main_name;
        }
    }
}

/// Options for rule C158.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C158RuleOptionsConfig {
    /// Whether readonly declarations document intentional globals.
    pub treat_readonly_as_documented: Option<bool>,
    /// Whether exported assignments should be treated as intentional globals.
    pub treat_export_as_intentional: Option<bool>,
}

impl C158RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.treat_readonly_as_documented.is_some() {
            self.treat_readonly_as_documented = overrides.treat_readonly_as_documented;
        }
        if overrides.treat_export_as_intentional.is_some() {
            self.treat_export_as_intentional = overrides.treat_export_as_intentional;
        }
    }
}

/// Options for rule C159.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C159RuleOptionsConfig {
    /// Whether conditional initialization should suppress later mutable-global reports.
    pub allow_conditional_init: Option<bool>,
}

impl C159RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.allow_conditional_init.is_some() {
            self.allow_conditional_init = overrides.allow_conditional_init;
        }
    }
}

/// Options for rule C160.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C160RuleOptionsConfig {
    /// Variable anchors treated as safe roots for sourced relative paths.
    pub allowed_anchors: Option<Vec<String>>,
}

impl C160RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.allowed_anchors.is_some() {
            self.allowed_anchors = overrides.allowed_anchors;
        }
    }
}

/// Options for rule C161.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C161RuleOptionsConfig {
    /// Whether calls after a source command should be exempted.
    pub ignore_after_source: Option<bool>,
}

impl C161RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.ignore_after_source.is_some() {
            self.ignore_after_source = overrides.ignore_after_source;
        }
    }
}

/// Options for rule C162.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct C162RuleOptionsConfig {
    /// Declaration forms treated as masking command substitution returns.
    pub treat_as_masking: Option<Vec<String>>,
}

impl C162RuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if overrides.treat_as_masking.is_some() {
            self.treat_as_masking = overrides.treat_as_masking;
        }
    }
}

/// Partial formatter settings supplied by CLI flags or config files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormatSettingsPatch {
    /// Optional dialect override.
    pub dialect: Option<ShellDialect>,
    /// Optional indentation style override.
    pub indent_style: Option<IndentStyle>,
    /// Optional indentation width override.
    pub indent_width: Option<u8>,
    /// Optional binary-next-line override.
    pub binary_next_line: Option<bool>,
    /// Optional switch-case indentation override.
    pub switch_case_indent: Option<bool>,
    /// Optional spaced-redirection override.
    pub space_redirects: Option<bool>,
    /// Optional padding-preservation override.
    pub keep_padding: Option<bool>,
    /// Optional function-next-line override.
    pub function_next_line: Option<bool>,
    /// Optional never-split override.
    pub never_split: Option<bool>,
    /// Optional simplification override.
    pub simplify: Option<bool>,
    /// Optional minification override.
    pub minify: Option<bool>,
}

/// Documentation metadata for a configuration section.
#[derive(Debug, Clone, Copy)]
pub struct ConfigSectionMetadata {
    /// TOML key for the section.
    pub key: &'static str,
    /// Human-readable section description.
    pub docs: &'static str,
    /// Documented scalar fields in this section.
    pub fields: &'static [ConfigFieldMetadata],
    /// Nested subsections.
    pub sections: &'static [ConfigSectionMetadata],
}

/// Documentation metadata for a configuration field.
#[derive(Debug, Clone, Copy)]
pub struct ConfigFieldMetadata {
    /// TOML key for the field.
    pub key: &'static str,
    /// Human-readable field description.
    pub docs: &'static str,
    /// Display form of the default value.
    pub default: &'static str,
    /// Display form of the accepted value type.
    pub value_type: &'static str,
    /// Example TOML assignment.
    pub example: &'static str,
}

/// Return metadata used to render the configuration reference.
pub fn configuration_metadata() -> &'static [ConfigSectionMetadata] {
    &CONFIGURATION_METADATA
}

const CONFIGURATION_METADATA: [ConfigSectionMetadata; 3] = [
    ConfigSectionMetadata {
        key: "check",
        docs: "File-level analysis behavior for `shuck check`.",
        fields: &[ConfigFieldMetadata {
            key: "embedded",
            docs: "Lint supported embedded shell scripts in non-shell files, including GitHub Actions workflow `run` blocks and composite action steps.",
            default: "true",
            value_type: "bool",
            example: "embedded = false",
        }],
        sections: &[],
    },
    ConfigSectionMetadata {
        key: "lint",
        docs: "Rule selection, per-file ignores, fix eligibility, and rule-specific behavior for `shuck check`.",
        fields: &[
            ConfigFieldMetadata {
                key: "extend-fixable",
                docs: "Add rules to the set that can receive automatic fixes when `shuck check --fix` is enabled.",
                default: "[]",
                value_type: "list[selector]",
                example: r#"extend-fixable = ["S074"]"#,
            },
            ConfigFieldMetadata {
                key: "extend-per-file-ignores",
                docs: "Add more per-file ignores without replacing the existing `per-file-ignores` table.",
                default: "{}",
                value_type: "table[str, list[selector]]",
                example: r#"extend-per-file-ignores = { "vendor/**" = ["ALL"] }"#,
            },
            ConfigFieldMetadata {
                key: "extend-select",
                docs: "Enable additional rules or named selectors on top of the default or explicitly selected rule set.",
                default: "[]",
                value_type: "list[selector]",
                example: r#"extend-select = ["google"]"#,
            },
            ConfigFieldMetadata {
                key: "fixable",
                docs: "Replace the set of rules that can receive automatic fixes when fixes are available.",
                default: r#"["ALL"]"#,
                value_type: "list[selector]",
                example: r#"fixable = ["C", "S074"]"#,
            },
            ConfigFieldMetadata {
                key: "ignore",
                docs: "Remove rules from the active rule set.",
                default: "[]",
                value_type: "list[selector]",
                example: r#"ignore = ["S074"]"#,
            },
            ConfigFieldMetadata {
                key: "per-file-ignores",
                docs: "Ignore selected rules for files that match a glob pattern.",
                default: "{}",
                value_type: "table[str, list[selector]]",
                example: r#"per-file-ignores = { "scripts/*.sh" = ["S074"] }"#,
            },
            ConfigFieldMetadata {
                key: "select",
                docs: "Replace the default rule set with the selectors listed here, including named selectors such as `google`.",
                default: "all implemented non-style rules",
                value_type: "list[selector]",
                example: r#"select = ["google"]"#,
            },
            ConfigFieldMetadata {
                key: "unfixable",
                docs: "Prevent selected rules from receiving automatic fixes, even when `--fix` is enabled.",
                default: "[]",
                value_type: "list[selector]",
                example: r#"unfixable = ["C001"]"#,
            },
        ],
        sections: &[
            ConfigSectionMetadata {
                key: "contracts",
                docs: "Ambient contract settings for runtime behavior that cannot be recovered from the source graph alone.",
                fields: &[
                    ConfigFieldMetadata {
                        key: "well-known",
                        docs: "Enable or disable Shuck's built-in ambient contract registry.",
                        default: "true",
                        value_type: "bool",
                        example: "well-known = false",
                    },
                    ConfigFieldMetadata {
                        key: "disabled",
                        docs: "Disable built-in ambient contracts by exact ID, by group selector, or with `*` for every built-in contract.",
                        default: "[]",
                        value_type: "list[string]",
                        example: r#"disabled = ["zsh/oh-my-zsh", "runtime/github-actions/env"]"#,
                    },
                ],
                sections: &[ConfigSectionMetadata {
                    key: "custom",
                    docs: "User-authored ambient contracts layered on top of, or in place of, the built-in registry.",
                    fields: &[
                        ConfigFieldMetadata {
                            key: "id",
                            docs: "Stable identifier for the custom contract.",
                            default: "required",
                            value_type: "string",
                            example: r#"id = "github-actions-env""#,
                        },
                        ConfigFieldMetadata {
                            key: "replaces",
                            docs: "Built-in contract selectors that this custom contract replaces.",
                            default: "[]",
                            value_type: "list[string]",
                            example: r#"replaces = ["zsh/oh-my-zsh/plugin/tmux"]"#,
                        },
                        ConfigFieldMetadata {
                            key: "when",
                            docs: "Activation for the contract, either `\"always\"` or an object such as `{ type = \"zsh_plugin\", framework = \"oh-my-zsh\", plugin = \"tmux\" }`.",
                            default: "required",
                            value_type: "string | table",
                            example: r#"when = { type = "zsh_plugin", framework = "oh-my-zsh", plugin = "tmux" }"#,
                        },
                        ConfigFieldMetadata {
                            key: "files",
                            docs: "Optional file globs that limit where the contract applies.",
                            default: "[]",
                            value_type: "list[glob]",
                            example: r#"files = ["**/.zshrc"]"#,
                        },
                        ConfigFieldMetadata {
                            key: "reads",
                            docs: "Ambient names the activated runtime reads from the caller environment.",
                            default: "[]",
                            value_type: "list[name]",
                            example: r#"reads = ["GITHUB_ENV", "GITHUB_OUTPUT"]"#,
                        },
                        ConfigFieldMetadata {
                            key: "functions",
                            docs: "Provided function contracts with caller reads and caller-visible sets.",
                            default: "[]",
                            value_type: "list[{ name, reads?, sets? }]",
                            example: r#"functions = [{ name = "helper", reads = ["CALLER_VALUE"], sets = ["REPLY"] }]"#,
                        },
                    ],
                    sections: &[
                        ConfigSectionMetadata {
                            key: "consumes",
                            docs: "Assignments that stay live because external runtime behavior consumes them.",
                            fields: &[
                                ConfigFieldMetadata {
                                    key: "names",
                                    docs: "Exact names consumed by external runtime behavior.",
                                    default: "[]",
                                    value_type: "list[name]",
                                    example: r#"names = ["HISTFILE", "SAVEHIST"]"#,
                                },
                                ConfigFieldMetadata {
                                    key: "prefixes",
                                    docs: "Name prefixes consumed by external runtime behavior.",
                                    default: "[]",
                                    value_type: "list[prefix]",
                                    example: r#"prefixes = ["ZSH_TMUX_"]"#,
                                },
                                ConfigFieldMetadata {
                                    key: "all",
                                    docs: "Treat every assignment in the file as externally consumed.",
                                    default: "false",
                                    value_type: "bool",
                                    example: "all = true",
                                },
                            ],
                            sections: &[],
                        },
                        ConfigSectionMetadata {
                            key: "provides",
                            docs: "Bindings made available by the activated runtime at file entry or activation time.",
                            fields: &[
                                ConfigFieldMetadata {
                                    key: "variables",
                                    docs: "Variables provided by the contract.",
                                    default: "[]",
                                    value_type: "list[name]",
                                    example: r#"variables = ["reply", "REPLY"]"#,
                                },
                                ConfigFieldMetadata {
                                    key: "functions",
                                    docs: "Callable function names provided by the contract.",
                                    default: "[]",
                                    value_type: "list[name]",
                                    example: r#"functions = ["helper"]"#,
                                },
                            ],
                            sections: &[],
                        },
                    ],
                }],
            },
            ConfigSectionMetadata {
                key: "rule-options",
                docs: "Rule-specific behavior overrides for diagnostics that intentionally support more than one analysis mode.",
                fields: &[],
                sections: &[
                    ConfigSectionMetadata {
                        key: "c001",
                        docs: "Behavior overrides for `C001` unused assignment analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "treat-indirect-expansion-targets-as-used",
                            docs: "Treat scalar indirect-expansion targets such as `${!name}` as a use of the referenced target.",
                            default: "false",
                            value_type: "bool",
                            example: "treat-indirect-expansion-targets-as-used = true",
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c063",
                        docs: "Behavior overrides for `C063` overwritten and unreached function analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "report-unreached-nested-definitions",
                            docs: "Report nested function definitions when no reachable direct call reaches the enclosing function scope before it exits.",
                            default: "false",
                            value_type: "bool",
                            example: "report-unreached-nested-definitions = true",
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s078",
                        docs: "Behavior overrides for `S078` shebang shell policy.",
                        fields: &[ConfigFieldMetadata {
                            key: "allowed-shells",
                            docs: "Interpreter names accepted in shebangs for this project.",
                            default: r#"["bash"]"#,
                            value_type: "list[string]",
                            example: r#"allowed-shells = ["bash", "zsh"]"#,
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s079",
                        docs: "Behavior overrides for `S079` shebang invocation form policy.",
                        fields: &[
                            ConfigFieldMetadata {
                                key: "allowed-forms",
                                docs: "Shebang invocation forms accepted for this project. Supported values are `absolute-path` and `env-lookup`.",
                                default: r#"["env-lookup"]"#,
                                value_type: "list[string]",
                                example: r#"allowed-forms = ["env-lookup"]"#,
                            },
                            ConfigFieldMetadata {
                                key: "allowed-paths",
                                docs: "Exact shebang invocation strings accepted regardless of invocation form.",
                                default: r#"["/bin/bash", "/usr/bin/env bash"]"#,
                                value_type: "list[string]",
                                example: r#"allowed-paths = ["/usr/bin/env bash"]"#,
                            },
                        ],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s080",
                        docs: "Behavior overrides for `S080` script size policy.",
                        fields: &[
                            ConfigFieldMetadata {
                                key: "max-lines",
                                docs: "Maximum accepted line count for one script.",
                                default: "100",
                                value_type: "integer",
                                example: "max-lines = 100",
                            },
                            ConfigFieldMetadata {
                                key: "count",
                                docs: "Line-counting mode. Supported values are `physical` and `non-comment-non-blank`.",
                                default: r#""physical""#,
                                value_type: "string",
                                example: r#"count = "non-comment-non-blank""#,
                            },
                        ],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s081",
                        docs: "Behavior overrides for `S081` file description comments.",
                        fields: &[ConfigFieldMetadata {
                            key: "ignore-shebang-only-files",
                            docs: "Do not report scripts whose only content is a shebang line.",
                            default: "false",
                            value_type: "bool",
                            example: "ignore-shebang-only-files = true",
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s084",
                        docs: "Behavior overrides for `S084` function documentation content.",
                        fields: &[
                            ConfigFieldMetadata {
                                key: "require-globals",
                                docs: "Require a Globals section when a documented function reads or writes non-local variables.",
                                default: "true",
                                value_type: "bool",
                                example: "require-globals = false",
                            },
                            ConfigFieldMetadata {
                                key: "require-arguments",
                                docs: "Require an Arguments section when a documented function uses positional parameters.",
                                default: "true",
                                value_type: "bool",
                                example: "require-arguments = false",
                            },
                            ConfigFieldMetadata {
                                key: "require-outputs",
                                docs: "Require an Outputs section when a documented function writes with echo or printf.",
                                default: "true",
                                value_type: "bool",
                                example: "require-outputs = false",
                            },
                            ConfigFieldMetadata {
                                key: "require-returns",
                                docs: "Require a Returns section when a documented function has an explicit return code.",
                                default: "true",
                                value_type: "bool",
                                example: "require-returns = false",
                            },
                        ],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "s085",
                        docs: "Behavior overrides for `S085` missing main entrypoint analysis.",
                        fields: &[
                            ConfigFieldMetadata {
                                key: "non-trivial-line-threshold",
                                docs: "Minimum source line count before the script is checked.",
                                default: "30",
                                value_type: "int",
                                example: "non-trivial-line-threshold = 20",
                            },
                            ConfigFieldMetadata {
                                key: "non-trivial-function-count",
                                docs: "Minimum function definition count before the script is checked.",
                                default: "2",
                                value_type: "int",
                                example: "non-trivial-function-count = 3",
                            },
                            ConfigFieldMetadata {
                                key: "main-name",
                                docs: "Function name expected as the final top-level call in non-trivial scripts.",
                                default: r#""main""#,
                                value_type: "string",
                                example: r#"main-name = "run""#,
                            },
                        ],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c158",
                        docs: "Behavior overrides for `C158` implicit global assignment analysis.",
                        fields: &[
                            ConfigFieldMetadata {
                                key: "treat-readonly-as-documented",
                                docs: "Treat top-level readonly declarations as documented intentional globals.",
                                default: "true",
                                value_type: "bool",
                                example: "treat-readonly-as-documented = false",
                            },
                            ConfigFieldMetadata {
                                key: "treat-export-as-intentional",
                                docs: "Treat top-level exported bindings as intentional globals.",
                                default: "true",
                                value_type: "bool",
                                example: "treat-export-as-intentional = false",
                            },
                        ],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c159",
                        docs: "Behavior overrides for `C159` mutable global analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "allow-conditional-init",
                            docs: "Allow self-referential default initializers such as `name=${name:-value}` without treating them as global mutations.",
                            default: "true",
                            value_type: "bool",
                            example: "allow-conditional-init = false",
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c160",
                        docs: "Behavior overrides for `C160` unanchored source path analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "allowed-anchors",
                            docs: "Path prefix expressions accepted as script-directory anchors for source or dot commands.",
                            default: r#"["${BASH_SOURCE[0]%/*}", "$(dirname \"$0\")", "$(dirname \"${BASH_SOURCE[0]}\")"]"#,
                            value_type: "list[string]",
                            example: r#"allowed-anchors = ["$SCRIPT_DIR"]"#,
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c161",
                        docs: "Behavior overrides for `C161` function call ordering analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "ignore-after-source",
                            docs: "Ignore later calls after a source command because the sourced file may define functions.",
                            default: "true",
                            value_type: "bool",
                            example: "ignore-after-source = false",
                        }],
                        sections: &[],
                    },
                    ConfigSectionMetadata {
                        key: "c162",
                        docs: "Behavior overrides for `C162` extra masked-return declaration analysis.",
                        fields: &[ConfigFieldMetadata {
                            key: "treat-as-masking",
                            docs: "Declaration forms beyond the default masked-return set to report.",
                            default: r#"["readonly", "typeset"]"#,
                            value_type: "list[string]",
                            example: r#"treat-as-masking = ["readonly", "typeset"]"#,
                        }],
                        sections: &[],
                    },
                ],
            },
            ConfigSectionMetadata {
                key: "zsh",
                docs: "Zsh-specific lint behavior for `shuck check`.",
                fields: &[],
                sections: &[ConfigSectionMetadata {
                    key: "plugins",
                    docs: "Zsh plugin-resolution settings used to import real plugin entrypoints into semantic analysis.",
                    fields: &[
                        ConfigFieldMetadata {
                            key: "resolution",
                            docs: "Enable or disable zsh plugin resolution while leaving ordinary source closure enabled.",
                            default: "true",
                            value_type: "bool",
                            example: "resolution = false",
                        },
                        ConfigFieldMetadata {
                            key: "roots",
                            docs: "Fallback framework roots keyed by logical framework name.",
                            default: "{}",
                            value_type: "table[str, path]",
                            example: r#"roots = { oh-my-zsh = "~/.oh-my-zsh" }"#,
                        },
                        ConfigFieldMetadata {
                            key: "plugin-loads",
                            docs: "Additional logical plugin loads to attach to matching files.",
                            default: "[]",
                            value_type: "list[{ pattern, framework, name }]",
                            example: r#"plugin-loads = [{ pattern = "**/.zshrc", framework = "oh-my-zsh", name = "git" }]"#,
                        },
                        ConfigFieldMetadata {
                            key: "theme-loads",
                            docs: "Additional logical theme loads to attach to matching files.",
                            default: "[]",
                            value_type: "list[{ pattern, framework, name }]",
                            example: r#"theme-loads = [{ pattern = "**/.zshrc", framework = "oh-my-zsh", name = "agnoster" }]"#,
                        },
                        ConfigFieldMetadata {
                            key: "entrypoints",
                            docs: "Raw plugin entrypoint paths to import for matching files.",
                            default: "[]",
                            value_type: "list[{ pattern, paths }]",
                            example: r#"entrypoints = [{ pattern = "**/.zshrc", paths = ["./vendor/prompt/prompt.plugin.zsh"] }]"#,
                        },
                    ],
                    sections: &[],
                }],
            },
        ],
    },
    ConfigSectionMetadata {
        key: "run",
        docs: "Interpreter defaults and managed shell version pins for `shuck run` and related commands.",
        fields: &[
            ConfigFieldMetadata {
                key: "shell",
                docs: "Default managed shell to use when a script does not declare its own shell.",
                default: "none",
                value_type: "string",
                example: r#"shell = "bash""#,
            },
            ConfigFieldMetadata {
                key: "shell-version",
                docs: "Default version constraint to use when no script metadata or per-shell pin is more specific.",
                default: r#""latest""#,
                value_type: "string",
                example: r#"shell-version = "5.2""#,
            },
        ],
        sections: &[ConfigSectionMetadata {
            key: "shells",
            docs: "Per-shell version pins applied after the shell has been resolved for the current script.",
            fields: &[
                ConfigFieldMetadata {
                    key: "bash",
                    docs: "Version constraint for Bash scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"bash = "5.2""#,
                },
                ConfigFieldMetadata {
                    key: "gbash",
                    docs: "Version constraint for gbash scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"gbash = "0.0.32""#,
                },
                ConfigFieldMetadata {
                    key: "bashkit",
                    docs: "Version constraint for Bashkit scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"bashkit = "0.2.1""#,
                },
                ConfigFieldMetadata {
                    key: "zsh",
                    docs: "Version constraint for Zsh scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"zsh = "5.9""#,
                },
                ConfigFieldMetadata {
                    key: "dash",
                    docs: "Version constraint for Dash scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"dash = "0.5.12""#,
                },
                ConfigFieldMetadata {
                    key: "mksh",
                    docs: "Version constraint for mksh scripts.",
                    default: "none",
                    value_type: "string",
                    example: r#"mksh = "59c""#,
                },
                ConfigFieldMetadata {
                    key: "busybox",
                    docs: "Version constraint for BusyBox scripts on Linux hosts.",
                    default: "none",
                    value_type: "string",
                    example: r#"busybox = "1.36.1""#,
                },
            ],
            sections: &[],
        }],
    },
];

/// Resolved command-line configuration arguments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigArguments {
    isolated: bool,
    config_file: Option<PathBuf>,
    overrides: ShuckConfig,
}

impl ConfigArguments {
    /// Parse `--config` occurrences and `--isolated` into config-loading arguments.
    pub fn from_cli(
        config_options: Vec<SingleConfigArgument>,
        isolated: bool,
    ) -> std::result::Result<Self, clap::Error> {
        let mut config_file: Option<PathBuf> = None;
        let mut overrides = ShuckConfig::default();

        for option in config_options {
            match option {
                SingleConfigArgument::SettingsOverride(config_override) => {
                    overrides.apply_overrides(*config_override);
                }
                SingleConfigArgument::FilePath(path) => {
                    if isolated {
                        return Err(clap::Error::raw(
                            ErrorKind::ArgumentConflict,
                            format!(
                                "\
The argument `--config={}` cannot be used with `--isolated`

  tip: You cannot specify a configuration file and also specify `--isolated`,
       as `--isolated` causes shuck to ignore all configuration files.
       For more information, try `--help`.
",
                                path.display()
                            ),
                        ));
                    }

                    if let Some(existing) = &config_file {
                        return Err(clap::Error::raw(
                            ErrorKind::ArgumentConflict,
                            format!(
                                "\
You cannot specify more than one configuration file on the command line.

  tip: remove either `--config={}` or `--config={}`.
       For more information, try `--help`.
",
                                existing.display(),
                                path.display()
                            ),
                        ));
                    }

                    config_file = Some(path);
                }
            }
        }

        Ok(Self {
            isolated,
            config_file,
            overrides,
        })
    }

    /// Return whether project-root discovery should use config files as roots.
    pub fn use_config_roots(&self) -> bool {
        !self.isolated && self.config_file.is_none()
    }

    /// Return the explicitly requested config file, if one was provided.
    pub fn explicit_config_file(&self) -> Option<&Path> {
        self.config_file.as_deref()
    }
}

/// A single `--config` command-line value.
#[derive(Clone, Debug, PartialEq)]
pub enum SingleConfigArgument {
    /// Path to a config file.
    FilePath(PathBuf),
    /// Inline TOML override value.
    SettingsOverride(Box<ShuckConfig>),
}

/// Clap value parser for [`SingleConfigArgument`].
#[derive(Clone)]
pub struct ConfigArgumentParser;

impl ValueParserFactory for SingleConfigArgument {
    type Parser = ConfigArgumentParser;

    fn value_parser() -> Self::Parser {
        ConfigArgumentParser
    }
}

impl TypedValueParser for ConfigArgumentParser {
    type Value = SingleConfigArgument;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> std::result::Result<Self::Value, clap::Error> {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(SingleConfigArgument::FilePath(path));
        }

        let Some(value) = value.to_str() else {
            return Err(clap::Error::new(ErrorKind::InvalidUtf8));
        };

        parse_config_override(value)
            .map(Box::new)
            .map(SingleConfigArgument::SettingsOverride)
            .map_err(|detail| invalid_config_argument(cmd, arg, value, &detail))
    }
}

/// Resolve the project root for an input path.
///
/// When `use_config_roots` is true, parent directories are searched for
/// `.shuck.toml` or `shuck.toml` and the closest match becomes the root.
pub fn resolve_project_root_for_input(input: &Path, use_config_roots: bool) -> io::Result<PathBuf> {
    let base_dir = base_dir_for_input(input)?;
    if use_config_roots {
        Ok(find_config_root(&base_dir)?.unwrap_or(base_dir))
    } else {
        Ok(base_dir)
    }
}

/// Resolve the project root for a file path with a fallback starting directory.
///
/// This is used for inputs such as standard input where the logical file path
/// may differ from the current working directory.
pub fn resolve_project_root_for_file(
    file: &Path,
    fallback_start: &Path,
    use_config_roots: bool,
) -> io::Result<PathBuf> {
    let start = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback_start.to_path_buf());
    if use_config_roots {
        Ok(find_config_root(&start)?.unwrap_or_else(|| normalize_path(fallback_start)))
    } else {
        Ok(normalize_path(fallback_start))
    }
}

/// Load project configuration and apply command-line overrides.
pub fn load_project_config(
    project_root: &Path,
    config_arguments: &ConfigArguments,
) -> Result<ShuckConfig> {
    let mut config = if config_arguments.isolated {
        ShuckConfig::default()
    } else if let Some(config_path) = &config_arguments.config_file {
        load_config_file(config_path)?
    } else {
        let Some(config_path) = config_path_for_root(project_root)? else {
            return Ok(config_arguments.overrides.clone());
        };
        load_config_file(&config_path)?
    };

    config.apply_overrides(config_arguments.overrides.clone());
    Ok(config)
}

/// Apply override values to an already loaded config.
pub fn apply_config_overrides(config: &mut ShuckConfig, overrides: ShuckConfig) {
    config.apply_overrides(overrides);
}

impl FormatConfig {
    /// Convert formatter config-file values into a formatter settings patch.
    pub fn to_patch(&self) -> Result<FormatSettingsPatch> {
        if self.dialect.is_some() {
            return Err(anyhow!(CONFIG_DIALECT_UNSUPPORTED_ERROR));
        }

        Ok(FormatSettingsPatch {
            dialect: None,
            indent_style: self
                .indent_style
                .as_deref()
                .map(parse_config_indent_style)
                .transpose()?,
            indent_width: self.indent_width,
            binary_next_line: self.binary_next_line,
            switch_case_indent: self.switch_case_indent,
            space_redirects: self.space_redirects,
            keep_padding: self.keep_padding,
            function_next_line: self.function_next_line,
            never_split: self.never_split,
            simplify: None,
            minify: None,
        })
    }
}

impl ShuckConfig {
    fn apply_overrides(&mut self, overrides: ShuckConfig) {
        self.check.apply_overrides(overrides.check);
        self.format.apply_overrides(overrides.format);
        self.lint.apply_overrides(overrides.lint);
        apply_run_overrides(&mut self.run, overrides.run);
    }
}

impl CheckConfig {
    fn apply_overrides(&mut self, overrides: CheckConfig) {
        if overrides.embedded.is_some() {
            self.embedded = overrides.embedded;
        }
    }
}

impl FormatConfig {
    fn apply_overrides(&mut self, overrides: FormatConfig) {
        if overrides.dialect.is_some() {
            self.dialect = overrides.dialect;
        }
        if overrides.indent_style.is_some() {
            self.indent_style = overrides.indent_style;
        }
        if overrides.indent_width.is_some() {
            self.indent_width = overrides.indent_width;
        }
        if overrides.binary_next_line.is_some() {
            self.binary_next_line = overrides.binary_next_line;
        }
        if overrides.switch_case_indent.is_some() {
            self.switch_case_indent = overrides.switch_case_indent;
        }
        if overrides.space_redirects.is_some() {
            self.space_redirects = overrides.space_redirects;
        }
        if overrides.keep_padding.is_some() {
            self.keep_padding = overrides.keep_padding;
        }
        if overrides.function_next_line.is_some() {
            self.function_next_line = overrides.function_next_line;
        }
        if overrides.never_split.is_some() {
            self.never_split = overrides.never_split;
        }
    }
}

impl LintConfig {
    fn apply_overrides(&mut self, overrides: LintConfig) {
        if overrides.select.is_some() {
            self.select = overrides.select;
        }
        if overrides.ignore.is_some() {
            self.ignore = overrides.ignore;
        }
        if overrides.extend_select.is_some() {
            self.extend_select = overrides.extend_select;
        }
        if overrides.per_file_ignores.is_some() {
            self.per_file_ignores = overrides.per_file_ignores;
        }
        if overrides.extend_per_file_ignores.is_some() {
            self.extend_per_file_ignores = overrides.extend_per_file_ignores;
        }
        if overrides.per_file_shell.is_some() {
            self.per_file_shell = overrides.per_file_shell;
        }
        if overrides.extend_per_file_shell.is_some() {
            self.extend_per_file_shell = overrides.extend_per_file_shell;
        }
        if overrides.fixable.is_some() {
            self.fixable = overrides.fixable;
        }
        if overrides.unfixable.is_some() {
            self.unfixable = overrides.unfixable;
        }
        if overrides.extend_fixable.is_some() {
            self.extend_fixable = overrides.extend_fixable;
        }
        if let Some(rule_options) = overrides.rule_options {
            self.rule_options
                .get_or_insert_default()
                .apply_overrides(rule_options);
        }
        if let Some(contracts) = overrides.contracts {
            self.contracts
                .get_or_insert_default()
                .apply_overrides(contracts);
        }
        if let Some(zsh) = overrides.zsh {
            self.zsh.get_or_insert_default().apply_overrides(zsh);
        }
    }
}

impl LintContractsConfig {
    fn apply_overrides(&mut self, overrides: LintContractsConfig) {
        if overrides.well_known.is_some() {
            self.well_known = overrides.well_known;
        }
        if overrides.disabled.is_some() {
            self.disabled = overrides.disabled;
        }
        if let Some(custom) = overrides.custom {
            self.custom.get_or_insert_default().extend(custom);
        }
    }
}

impl LintZshConfig {
    fn apply_overrides(&mut self, overrides: LintZshConfig) {
        if let Some(plugins) = overrides.plugins {
            self.plugins
                .get_or_insert_default()
                .apply_overrides(plugins);
        }
    }
}

impl ZshPluginsConfig {
    fn apply_overrides(&mut self, overrides: ZshPluginsConfig) {
        if overrides.resolution.is_some() {
            self.resolution = overrides.resolution;
        }
        if let Some(roots) = overrides.roots {
            self.roots.get_or_insert_default().extend(roots);
        }
        if let Some(plugin_loads) = overrides.plugin_loads {
            self.plugin_loads
                .get_or_insert_default()
                .extend(plugin_loads);
        }
        if let Some(theme_loads) = overrides.theme_loads {
            self.theme_loads.get_or_insert_default().extend(theme_loads);
        }
        if let Some(entrypoints) = overrides.entrypoints {
            self.entrypoints.get_or_insert_default().extend(entrypoints);
        }
    }
}

fn load_config_file(config_path: &Path) -> Result<ShuckConfig> {
    let source = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    toml::from_str(&source).with_context(|| format!("parse {}", config_path.display()))
}

fn parse_config_override(value: &str) -> std::result::Result<ShuckConfig, String> {
    let table = toml::Table::from_str(value).map_err(|err| err.to_string())?;
    validate_override_table(&table)?;
    toml::from_str(value).map_err(|err: toml::de::Error| err.to_string())
}

fn validate_override_table(table: &toml::Table) -> std::result::Result<(), String> {
    for key in table.keys() {
        if !CONFIG_OVERRIDE_ROOT_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported config option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_ROOT_KEYS.join(", ")
            ));
        }
    }

    if let Some(format_value) = table.get("format") {
        let format = format_value
            .as_table()
            .ok_or_else(|| "`format` must be a TOML table".to_owned())?;
        for key in format.keys() {
            if !CONFIG_OVERRIDE_FORMAT_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "unsupported `[format]` option `{key}`; expected one of: {}",
                    CONFIG_OVERRIDE_FORMAT_KEYS.join(", ")
                ));
            }
        }
    }

    if let Some(check_value) = table.get("check") {
        let check = check_value
            .as_table()
            .ok_or_else(|| "`check` must be a TOML table".to_owned())?;
        for key in check.keys() {
            if !CONFIG_OVERRIDE_CHECK_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "unsupported `[check]` option `{key}`; expected one of: {}",
                    CONFIG_OVERRIDE_CHECK_KEYS.join(", ")
                ));
            }
        }
    }

    if let Some(lint_value) = table.get("lint") {
        let lint = lint_value
            .as_table()
            .ok_or_else(|| "`lint` must be a TOML table".to_owned())?;
        for key in lint.keys() {
            if !CONFIG_OVERRIDE_LINT_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "unsupported `[lint]` option `{key}`; expected one of: {}",
                    CONFIG_OVERRIDE_LINT_KEYS.join(", ")
                ));
            }
        }
        if let Some(rule_options_value) = lint.get("rule-options") {
            validate_lint_rule_options_override(rule_options_value)?;
        }
        if let Some(contracts_value) = lint.get("contracts") {
            validate_lint_contracts_override(contracts_value)?;
        }
        if let Some(zsh_value) = lint.get("zsh") {
            validate_lint_zsh_override(zsh_value)?;
        }
    }

    if let Some(run_value) = table.get("run") {
        validate_run_override(run_value)?;
    }

    Ok(())
}

fn validate_lint_contracts_override(value: &toml::Value) -> std::result::Result<(), String> {
    let contracts = value
        .as_table()
        .ok_or_else(|| "`[lint.contracts]` must be a TOML table".to_owned())?;
    for key in contracts.keys() {
        if !CONFIG_OVERRIDE_LINT_CONTRACT_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.contracts]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_LINT_CONTRACT_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_lint_zsh_override(value: &toml::Value) -> std::result::Result<(), String> {
    let zsh = value
        .as_table()
        .ok_or_else(|| "`[lint.zsh]` must be a TOML table".to_owned())?;
    for key in zsh.keys() {
        if !CONFIG_OVERRIDE_LINT_ZSH_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.zsh]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_LINT_ZSH_KEYS.join(", ")
            ));
        }
    }

    if let Some(plugins_value) = zsh.get("plugins") {
        validate_lint_zsh_plugins_override(plugins_value)?;
    }

    Ok(())
}

fn validate_lint_zsh_plugins_override(value: &toml::Value) -> std::result::Result<(), String> {
    let plugins = value
        .as_table()
        .ok_or_else(|| "`[lint.zsh.plugins]` must be a TOML table".to_owned())?;
    for key in plugins.keys() {
        if !CONFIG_OVERRIDE_LINT_ZSH_PLUGIN_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.zsh.plugins]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_LINT_ZSH_PLUGIN_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_run_override(value: &toml::Value) -> std::result::Result<(), String> {
    let run = value
        .as_table()
        .ok_or_else(|| "`run` must be a TOML table".to_owned())?;
    for key in run.keys() {
        if !CONFIG_OVERRIDE_RUN_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[run]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_RUN_KEYS.join(", ")
            ));
        }
    }

    if let Some(shells_value) = run.get("shells") {
        let shells = shells_value
            .as_table()
            .ok_or_else(|| "`[run.shells]` must be a TOML table".to_owned())?;
        for key in shells.keys() {
            if !CONFIG_OVERRIDE_RUN_SHELL_NAMES.contains(&key.as_str()) {
                return Err(format!(
                    "unsupported `[run.shells]` shell `{key}`; expected one of: {}",
                    CONFIG_OVERRIDE_RUN_SHELL_NAMES.join(", ")
                ));
            }
        }
    }

    Ok(())
}

fn apply_run_overrides(target: &mut RunConfig, overrides: RunConfig) {
    if overrides.shell.is_some() {
        target.shell = overrides.shell;
    }
    if overrides.shell_version.is_some() {
        target.shell_version = overrides.shell_version;
    }
    if !overrides.shells.is_empty() {
        target.shells = overrides.shells;
    }
}

fn validate_lint_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let rule_options = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options]` must be a TOML table".to_owned())?;
    for key in rule_options.keys() {
        if !CONFIG_OVERRIDE_LINT_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_LINT_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    if let Some(c001_value) = rule_options.get("c001") {
        validate_c001_rule_options_override(c001_value)?;
    }
    if let Some(c063_value) = rule_options.get("c063") {
        validate_c063_rule_options_override(c063_value)?;
    }
    if let Some(s078_value) = rule_options.get("s078") {
        validate_s078_rule_options_override(s078_value)?;
    }
    if let Some(s079_value) = rule_options.get("s079") {
        validate_s079_rule_options_override(s079_value)?;
    }
    if let Some(s080_value) = rule_options.get("s080") {
        validate_s080_rule_options_override(s080_value)?;
    }
    if let Some(s081_value) = rule_options.get("s081") {
        validate_s081_rule_options_override(s081_value)?;
    }
    if let Some(s084_value) = rule_options.get("s084") {
        validate_s084_rule_options_override(s084_value)?;
    }
    if let Some(s085_value) = rule_options.get("s085") {
        validate_s085_rule_options_override(s085_value)?;
    }
    if let Some(c158_value) = rule_options.get("c158") {
        validate_c158_rule_options_override(c158_value)?;
    }
    if let Some(c159_value) = rule_options.get("c159") {
        validate_c159_rule_options_override(c159_value)?;
    }
    if let Some(c160_value) = rule_options.get("c160") {
        validate_c160_rule_options_override(c160_value)?;
    }
    if let Some(c161_value) = rule_options.get("c161") {
        validate_c161_rule_options_override(c161_value)?;
    }
    if let Some(c162_value) = rule_options.get("c162") {
        validate_c162_rule_options_override(c162_value)?;
    }

    Ok(())
}

fn validate_c001_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c001 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c001]` must be a TOML table".to_owned())?;
    for key in c001.keys() {
        if !CONFIG_OVERRIDE_C001_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c001]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C001_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c063_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c063 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c063]` must be a TOML table".to_owned())?;
    for key in c063.keys() {
        if !CONFIG_OVERRIDE_C063_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c063]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C063_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_s078_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s078 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s078]` must be a TOML table".to_owned())?;
    for key in s078.keys() {
        if !CONFIG_OVERRIDE_S078_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s078]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S078_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_s079_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s079 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s079]` must be a TOML table".to_owned())?;
    for key in s079.keys() {
        if !CONFIG_OVERRIDE_S079_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s079]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S079_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_s080_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s080 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s080]` must be a TOML table".to_owned())?;
    for key in s080.keys() {
        if !CONFIG_OVERRIDE_S080_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s080]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S080_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }
    if let Some(count) = s080.get("count") {
        validate_s080_count_value(count)?;
    }

    Ok(())
}

fn validate_s080_count_value(value: &toml::Value) -> std::result::Result<(), String> {
    let count = value
        .as_str()
        .ok_or_else(|| "`[lint.rule-options.s080].count` must be a string".to_owned())?;
    let normalized = count.trim().to_ascii_lowercase();
    if CONFIG_OVERRIDE_S080_COUNT_VALUES.contains(&normalized.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "unsupported `[lint.rule-options.s080].count` value `{count}`; expected one of: {}",
            CONFIG_OVERRIDE_S080_COUNT_VALUES.join(", ")
        ))
    }
}

fn validate_s084_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s084 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s084]` must be a TOML table".to_owned())?;
    for key in s084.keys() {
        if !CONFIG_OVERRIDE_S084_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s084]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S084_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_s081_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s081 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s081]` must be a TOML table".to_owned())?;
    for key in s081.keys() {
        if !CONFIG_OVERRIDE_S081_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s081]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S081_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_s085_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let s085 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.s085]` must be a TOML table".to_owned())?;
    for key in s085.keys() {
        if !CONFIG_OVERRIDE_S085_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.s085]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_S085_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c158_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c158 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c158]` must be a TOML table".to_owned())?;
    for key in c158.keys() {
        if !CONFIG_OVERRIDE_C158_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c158]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C158_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c159_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c159 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c159]` must be a TOML table".to_owned())?;
    for key in c159.keys() {
        if !CONFIG_OVERRIDE_C159_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c159]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C159_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c160_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c160 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c160]` must be a TOML table".to_owned())?;
    for key in c160.keys() {
        if !CONFIG_OVERRIDE_C160_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c160]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C160_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c161_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c161 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c161]` must be a TOML table".to_owned())?;
    for key in c161.keys() {
        if !CONFIG_OVERRIDE_C161_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c161]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C161_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn validate_c162_rule_options_override(value: &toml::Value) -> std::result::Result<(), String> {
    let c162 = value
        .as_table()
        .ok_or_else(|| "`[lint.rule-options.c162]` must be a TOML table".to_owned())?;
    for key in c162.keys() {
        if !CONFIG_OVERRIDE_C162_RULE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "unsupported `[lint.rule-options.c162]` option `{key}`; expected one of: {}",
                CONFIG_OVERRIDE_C162_RULE_OPTION_KEYS.join(", ")
            ));
        }
    }

    Ok(())
}

fn invalid_config_argument(
    cmd: &clap::Command,
    arg: Option<&clap::Arg>,
    value: &str,
    detail: &str,
) -> clap::Error {
    use std::fmt::Write as _;

    let mut error = clap::Error::new(ErrorKind::ValueValidation).with_cmd(cmd);
    if let Some(arg) = arg {
        error.insert(
            ContextKind::InvalidArg,
            ContextValue::String(arg.to_string()),
        );
    }
    error.insert(
        ContextKind::InvalidValue,
        ContextValue::String(value.to_owned()),
    );

    let mut tip = "\
A `--config` flag must either be a path to a `.toml` configuration file
       or a TOML `<KEY> = <VALUE>` pair overriding a specific configuration
       option"
        .to_owned();

    if Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        && !value.contains('=')
    {
        let _ = write!(
            &mut tip,
            "\n\nIt looks like you were trying to pass a path to a configuration file.\nThe path `{value}` does not point to a configuration file."
        );
    } else {
        let _ = write!(&mut tip, "\n\n{detail}");
    }

    error.insert(
        ContextKind::Suggested,
        ContextValue::StyledStrs(vec![tip.into()]),
    );
    error
}

/// Parse a config-file indentation style value.
pub fn parse_config_indent_style(value: &str) -> Result<IndentStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tab" => Ok(IndentStyle::Tab),
        "space" => Ok(IndentStyle::Space),
        _ => Err(anyhow!(
            "unsupported `[format].indent-style` value `{value}`; expected one of: tab, space"
        )),
    }
}

fn base_dir_for_input(input: &Path) -> io::Result<PathBuf> {
    let normalized = normalize_path(input);
    let metadata = fs::metadata(&normalized)?;
    if metadata.is_dir() {
        Ok(normalized)
    } else {
        Ok(normalized
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")))
    }
}

fn find_config_root(start: &Path) -> io::Result<Option<PathBuf>> {
    let start = normalize_path(start);

    let mut current = start.as_path();
    loop {
        for filename in CONFIG_FILENAMES {
            let candidate = current.join(filename);
            match fs::metadata(&candidate) {
                Ok(metadata) if metadata.is_file() => return Ok(Some(current.to_path_buf())),
                Ok(_) => continue,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            }
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }

    Ok(None)
}

fn config_path_for_root(root: &Path) -> io::Result<Option<PathBuf>> {
    for filename in CONFIG_FILENAMES {
        let candidate = root.join(filename);
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(Some(candidate)),
            Ok(_) => continue,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        }
    }

    Ok(None)
}

/// Return the config file path located directly under `root`, if any.
pub fn discovered_config_path_for_root(root: &Path) -> io::Result<Option<PathBuf>> {
    config_path_for_root(root)
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn inline_config_overrides_validate_supported_keys() {
        let config = parse_config_override("format.indent-width = 2").unwrap();
        assert_eq!(config.format.indent_width, Some(2));
    }

    #[test]
    fn inline_config_overrides_validate_supported_check_keys() {
        let config = parse_config_override("check.embedded = false").unwrap();
        assert_eq!(config.check.embedded, Some(false));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_check_keys() {
        let err = parse_config_override("check.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[check]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_root_keys() {
        let err = parse_config_override("unknown.value = false").unwrap_err();
        assert!(err.contains("unsupported config option `unknown`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_format_keys() {
        let err = parse_config_override("format.line-length = 88").unwrap_err();
        assert!(err.contains("unsupported `[format]` option `line-length`"));
    }

    #[test]
    fn inline_config_overrides_validate_supported_lint_keys() {
        let config = parse_config_override("lint.select = ['C001']").unwrap();
        assert_eq!(config.lint.select, Some(vec!["C001".to_owned()]));
    }

    #[test]
    fn inline_config_overrides_validate_supported_run_keys() {
        let config = parse_config_override(
            "run.shell = 'gbash'\nrun.shell-version = '0.0'\nrun.shells.gbash = '0.0'\nrun.shells.bashkit = '0.2'",
        )
        .unwrap();
        assert_eq!(config.run.shell.as_deref(), Some("gbash"));
        assert_eq!(config.run.shell_version.as_deref(), Some("0.0"));
        assert_eq!(
            config.run.shells.get("gbash").map(String::as_str),
            Some("0.0")
        );
        assert_eq!(
            config.run.shells.get("bashkit").map(String::as_str),
            Some("0.2")
        );
    }

    #[test]
    fn inline_config_overrides_reject_unknown_run_keys() {
        let err = parse_config_override("run.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[run]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_run_shells_keys() {
        let err = parse_config_override("run.shells.fish = '4.0'").unwrap_err();
        assert!(err.contains("unsupported `[run.shells]` shell `fish`"));
    }

    #[test]
    fn inline_config_overrides_accept_busybox_shell_keys() {
        let config =
            parse_config_override("run.shell = 'busybox'\nrun.shells.busybox = '1.36.1'").unwrap();
        assert_eq!(config.run.shell.as_deref(), Some("busybox"));
        assert_eq!(
            config.run.shells.get("busybox").map(String::as_str),
            Some("1.36.1")
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.c001.treat-indirect-expansion-targets-as-used = false",
        )
        .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c001.as_ref())
                .and_then(|c001| c001.treat_indirect_expansion_targets_as_used),
            Some(false)
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_c063_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.c063.report-unreached-nested-definitions = true",
        )
        .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c063.as_ref())
                .and_then(|c063| c063.report_unreached_nested_definitions),
            Some(true)
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_s079_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.s079.allowed-forms = ['env-lookup']\n\
             lint.rule-options.s079.allowed-paths = ['/usr/bin/env bash']",
        )
        .unwrap();
        let s079 = config
            .lint
            .rule_options
            .as_ref()
            .and_then(|options| options.s079.as_ref())
            .expect("expected s079 options");

        assert_eq!(
            s079.allowed_forms.as_deref(),
            Some(&["env-lookup".to_owned()][..])
        );
        assert_eq!(
            s079.allowed_paths.as_deref(),
            Some(&["/usr/bin/env bash".to_owned()][..])
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_s080_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.s080.max-lines = 120\n\
             lint.rule-options.s080.count = 'non-comment-non-blank'",
        )
        .unwrap();
        let s080 = config
            .lint
            .rule_options
            .as_ref()
            .and_then(|options| options.s080.as_ref())
            .expect("expected s080 options");

        assert_eq!(s080.max_lines, Some(120));
        assert_eq!(s080.count.as_deref(), Some("non-comment-non-blank"));
    }

    #[test]
    fn inline_config_overrides_validate_supported_s081_rule_option_keys() {
        let config =
            parse_config_override("lint.rule-options.s081.ignore-shebang-only-files = true")
                .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s081.as_ref())
                .and_then(|s081| s081.ignore_shebang_only_files),
            Some(true)
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_s084_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.s084.require-globals = false\n\
             lint.rule-options.s084.require-arguments = false\n\
             lint.rule-options.s084.require-outputs = true\n\
             lint.rule-options.s084.require-returns = true",
        )
        .unwrap();
        let s084 = config
            .lint
            .rule_options
            .as_ref()
            .and_then(|options| options.s084.as_ref())
            .expect("missing s084 options");
        assert_eq!(s084.require_globals, Some(false));
        assert_eq!(s084.require_arguments, Some(false));
        assert_eq!(s084.require_outputs, Some(true));
        assert_eq!(s084.require_returns, Some(true));
    }

    #[test]
    fn inline_config_overrides_validate_supported_s085_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.s085.non-trivial-line-threshold = 20\n\
             lint.rule-options.s085.non-trivial-function-count = 3\n\
             lint.rule-options.s085.main-name = 'run'",
        )
        .unwrap();
        let s085 = config
            .lint
            .rule_options
            .as_ref()
            .and_then(|options| options.s085.as_ref())
            .expect("missing s085 options");

        assert_eq!(s085.non_trivial_line_threshold, Some(20));
        assert_eq!(s085.non_trivial_function_count, Some(3));
        assert_eq!(s085.main_name.as_deref(), Some("run"));
    }

    #[test]
    fn inline_config_overrides_validate_supported_s078_rule_option_keys() {
        let config =
            parse_config_override("lint.rule-options.s078.allowed-shells = ['bash', 'zsh']")
                .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s078.as_ref())
                .and_then(|s078| s078.allowed_shells.as_ref())
                .map(Vec::as_slice),
            Some(&["bash".to_owned(), "zsh".to_owned()][..])
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_c158_rule_option_keys() {
        let config = parse_config_override(
            "lint.rule-options.c158.treat-readonly-as-documented = false\n\
             lint.rule-options.c158.treat-export-as-intentional = false",
        )
        .unwrap();
        let c158 = config
            .lint
            .rule_options
            .as_ref()
            .and_then(|options| options.c158.as_ref())
            .expect("expected c158 rule options");
        assert_eq!(c158.treat_readonly_as_documented, Some(false));
        assert_eq!(c158.treat_export_as_intentional, Some(false));
    }

    #[test]
    fn inline_config_overrides_validate_supported_c159_rule_option_keys() {
        let config =
            parse_config_override("lint.rule-options.c159.allow-conditional-init = false").unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c159.as_ref())
                .and_then(|c159| c159.allow_conditional_init),
            Some(false)
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_c160_rule_option_keys() {
        let config =
            parse_config_override("lint.rule-options.c160.allowed-anchors = ['$SCRIPT_DIR']")
                .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c160.as_ref())
                .and_then(|c160| c160.allowed_anchors.as_ref())
                .map(Vec::as_slice),
            Some(&["$SCRIPT_DIR".to_owned()][..])
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_c161_rule_option_keys() {
        let config =
            parse_config_override("lint.rule-options.c161.ignore-after-source = false").unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c161.as_ref())
                .and_then(|c161| c161.ignore_after_source),
            Some(false)
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_c162_rule_option_keys() {
        let config =
            parse_config_override(r#"lint.rule-options.c162.treat-as-masking = ["typeset"]"#)
                .unwrap();
        assert_eq!(
            config
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c162.as_ref())
                .and_then(|c162| c162.treat_as_masking.as_ref()),
            Some(&vec!["typeset".to_owned()])
        );
    }

    #[test]
    fn inline_config_overrides_validate_supported_zsh_plugin_keys() {
        let config = parse_config_override(
            "lint.zsh.plugins.resolution = false\n\
             lint.zsh.plugins.roots.oh-my-zsh = '~/.oh-my-zsh'\n\
             lint.zsh.plugins.plugin-loads = [{ pattern = '**/.zshrc', framework = 'oh-my-zsh', name = 'git' }]\n\
             lint.zsh.plugins.theme-loads = [{ pattern = '**/.zshrc', framework = 'oh-my-zsh', name = 'agnoster' }]\n\
             lint.zsh.plugins.entrypoints = [{ pattern = '**/.zshrc', paths = ['./vendor/prompt.plugin.zsh'] }]",
        )
        .unwrap();
        let plugins = config
            .lint
            .zsh
            .as_ref()
            .and_then(|zsh| zsh.plugins.as_ref())
            .expect("missing zsh plugin config");
        assert_eq!(plugins.resolution, Some(false));
        assert_eq!(
            plugins
                .roots
                .as_ref()
                .and_then(|roots| roots.get("oh-my-zsh")),
            Some(&"~/.oh-my-zsh".to_owned())
        );
        assert_eq!(
            plugins
                .plugin_loads
                .as_ref()
                .and_then(|loads| loads.first())
                .map(|load| load.name.as_str()),
            Some("git")
        );
        assert_eq!(
            plugins
                .theme_loads
                .as_ref()
                .and_then(|loads| loads.first())
                .map(|load| load.name.as_str()),
            Some("agnoster")
        );
        assert_eq!(
            plugins
                .entrypoints
                .as_ref()
                .and_then(|loads| loads.first())
                .map(|load| load.paths.as_slice()),
            Some(&["./vendor/prompt.plugin.zsh".to_owned()][..])
        );
    }

    #[test]
    fn inline_config_overrides_reject_uninitialized_declaration_rule_option() {
        let err = parse_config_override(
            "lint.rule-options.c001.report-uninitialized-declarations = true",
        )
        .unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c001]` option"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_lint_zsh_keys() {
        let err = parse_config_override("lint.zsh.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.zsh]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_lint_zsh_plugin_keys() {
        let err = parse_config_override("lint.zsh.plugins.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.zsh.plugins]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_lint_keys() {
        let err = parse_config_override("lint.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.preview.enabled = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c001_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c001.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c001]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c063_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c063.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c063]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_s080_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.s080.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.s080]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_s080_count_values() {
        let err = parse_config_override("lint.rule-options.s080.count = 'non-comment-nonblank'")
            .unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.s080].count` value"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_s081_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.s081.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.s081]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_s084_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.s084.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.s084]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_s085_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.s085.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.s085]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c159_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c159.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c159]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c160_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c160.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c160]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c161_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c161.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c161]` option `preview`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_c162_rule_option_keys() {
        let err = parse_config_override("lint.rule-options.c162.preview = true").unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c162]` option `preview`"));
    }

    #[test]
    fn config_arguments_allow_multiple_inline_overrides_with_last_one_winning() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("format.indent-width = 2").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("format.indent-width = 4").unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(loaded.format.indent_width, Some(4));
    }

    #[test]
    fn lint_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.select = ['C001']").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.select = ['C002']").unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(loaded.lint.select, Some(vec!["C002".to_owned()]));
    }

    #[test]
    fn rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c001.treat-indirect-expansion-targets-as-used = true",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c001.treat-indirect-expansion-targets-as-used = false",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c001.as_ref())
                .and_then(|c001| c001.treat_indirect_expansion_targets_as_used),
            Some(false)
        );
    }

    #[test]
    fn c063_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c063.report-unreached-nested-definitions = true",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c063.report-unreached-nested-definitions = false",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c063.as_ref())
                .and_then(|c063| c063.report_unreached_nested_definitions),
            Some(false)
        );
    }

    #[test]
    fn s079_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s079.allowed-forms = ['env-lookup']")
                        .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.s079.allowed-forms = ['absolute-path']",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s079.as_ref())
                .and_then(|s079| s079.allowed_forms.as_ref())
                .map(Vec::as_slice),
            Some(&["absolute-path".to_owned()][..])
        );
    }

    #[test]
    fn s080_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s080.max-lines = 120").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s080.max-lines = 80").unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s080.as_ref())
                .and_then(|s080| s080.max_lines),
            Some(80)
        );
    }

    #[test]
    fn s081_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.s081.ignore-shebang-only-files = true",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.s081.ignore-shebang-only-files = false",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s081.as_ref())
                .and_then(|s081| s081.ignore_shebang_only_files),
            Some(false)
        );
    }

    #[test]
    fn s084_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s084.require-returns = true").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s084.require-returns = false")
                        .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s084.as_ref())
                .and_then(|s084| s084.require_returns),
            Some(false)
        );
    }

    #[test]
    fn s085_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s085.main-name = 'main'").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s085.main-name = 'run'").unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s085.as_ref())
                .and_then(|s085| s085.main_name.as_deref()),
            Some("run")
        );
    }

    #[test]
    fn s078_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s078.allowed-shells = ['bash']")
                        .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.s078.allowed-shells = ['zsh']")
                        .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.s078.as_ref())
                .and_then(|s078| s078.allowed_shells.as_ref())
                .map(Vec::as_slice),
            Some(&["zsh".to_owned()][..])
        );
    }

    #[test]
    fn c158_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c158.treat-export-as-intentional = true",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c158.treat-export-as-intentional = false",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c158.as_ref())
                .and_then(|c158| c158.treat_export_as_intentional),
            Some(false)
        );
    }

    #[test]
    fn c159_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.c159.allow-conditional-init = false")
                        .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.c159.allow-conditional-init = true")
                        .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c159.as_ref())
                .and_then(|c159| c159.allow_conditional_init),
            Some(true)
        );
    }

    #[test]
    fn c160_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c160.allowed-anchors = ['$SCRIPT_DIR']",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.rule-options.c160.allowed-anchors = ['$REPO_ROOT']",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c160.as_ref())
                .and_then(|c160| c160.allowed_anchors.as_ref())
                .map(Vec::as_slice),
            Some(&["$REPO_ROOT".to_owned()][..])
        );
    }

    #[test]
    fn c161_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.c161.ignore-after-source = false")
                        .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("lint.rule-options.c161.ignore-after-source = true")
                        .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c161.as_ref())
                .and_then(|c161| c161.ignore_after_source),
            Some(true)
        );
    }

    #[test]
    fn c162_rule_option_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        r#"lint.rule-options.c162.treat-as-masking = ["readonly", "typeset"]"#,
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(r#"lint.rule-options.c162.treat-as-masking = []"#)
                        .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(
            loaded
                .lint
                .rule_options
                .as_ref()
                .and_then(|options| options.c162.as_ref())
                .and_then(|c162| c162.treat_as_masking.as_ref()),
            Some(&Vec::<String>::new())
        );
    }

    #[test]
    fn zsh_plugin_config_arguments_merge_roots_and_append_loads() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.zsh.plugins.roots.oh-my-zsh = '/first'\n\
                         lint.zsh.plugins.plugin-loads = [{ pattern = '**/.zshrc', framework = 'oh-my-zsh', name = 'git' }]",
                    )
                    .unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override(
                        "lint.zsh.plugins.roots.oh-my-zsh = '/second'\n\
                         lint.zsh.plugins.plugin-loads = [{ pattern = '**/.zshrc', framework = 'oh-my-zsh', name = 'docker' }]",
                    )
                    .unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        let plugins = loaded
            .lint
            .zsh
            .as_ref()
            .and_then(|zsh| zsh.plugins.as_ref())
            .expect("missing zsh plugin config");
        assert_eq!(
            plugins
                .roots
                .as_ref()
                .and_then(|roots| roots.get("oh-my-zsh")),
            Some(&"/second".to_owned())
        );
        assert_eq!(
            plugins.plugin_loads.as_ref().map(|loads| loads
                .iter()
                .map(|load| load.name.clone())
                .collect::<Vec<_>>()),
            Some(vec!["git".to_owned(), "docker".to_owned()])
        );
    }

    #[test]
    fn run_config_arguments_allow_last_override_to_win() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("run.shell-version = '5.1'").unwrap(),
                )),
                SingleConfigArgument::SettingsOverride(Box::new(
                    parse_config_override("run.shell-version = '5.2'").unwrap(),
                )),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(loaded.run.shell_version.as_deref(), Some("5.2"));
    }

    #[test]
    fn isolated_rejects_explicit_config_files() {
        let tempdir = tempdir().unwrap();
        let config_path = tempdir.path().join("shuck.toml");
        fs::write(&config_path, "[format]\n").unwrap();

        let err =
            ConfigArguments::from_cli(vec![SingleConfigArgument::FilePath(config_path)], true)
                .unwrap_err();

        assert!(err.to_string().contains("cannot be used with `--isolated`"));
    }

    #[test]
    fn explicit_config_file_replaces_discovered_project_config() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[format]\nfunction-next-line = false\n",
        )
        .unwrap();

        let explicit = tempdir.path().join("override.toml");
        fs::write(&explicit, "[format]\nfunction-next-line = true\n").unwrap();

        let config =
            ConfigArguments::from_cli(vec![SingleConfigArgument::FilePath(explicit)], false)
                .unwrap();
        let loaded = load_project_config(tempdir.path(), &config).unwrap();

        assert_eq!(loaded.format.function_next_line, Some(true));
    }

    #[test]
    fn config_file_rejects_unknown_nested_rule_option_fields() {
        let tempdir = tempdir().unwrap();
        let config_path = tempdir.path().join("shuck.toml");
        fs::write(&config_path, "[lint.rule-options.c001]\npreview = true\n").unwrap();

        let err = load_project_config(
            tempdir.path(),
            &ConfigArguments::from_cli(vec![SingleConfigArgument::FilePath(config_path)], false)
                .unwrap(),
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("preview"));
    }

    #[test]
    fn config_file_rejects_unknown_nested_zsh_plugin_fields() {
        let tempdir = tempdir().unwrap();
        let config_path = tempdir.path().join("shuck.toml");
        fs::write(&config_path, "[lint.zsh.plugins]\npreview = true\n").unwrap();

        let err = load_project_config(
            tempdir.path(),
            &ConfigArguments::from_cli(vec![SingleConfigArgument::FilePath(config_path)], false)
                .unwrap(),
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("preview"));
    }

    #[test]
    fn isolated_only_uses_inline_overrides() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[format]\nfunction-next-line = true\n",
        )
        .unwrap();

        let config = ConfigArguments::from_cli(
            vec![SingleConfigArgument::SettingsOverride(Box::new(
                parse_config_override("format.indent-width = 2").unwrap(),
            ))],
            true,
        )
        .unwrap();
        let loaded = load_project_config(tempdir.path(), &config).unwrap();

        assert_eq!(loaded.format.function_next_line, None);
        assert_eq!(loaded.format.indent_width, Some(2));
    }

    #[test]
    fn project_root_resolution_can_ignore_config_roots() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(tempdir.path().join("shuck.toml"), "[format]\n").unwrap();

        assert_eq!(
            resolve_project_root_for_input(&nested, true).unwrap(),
            tempdir.path()
        );
        assert_eq!(
            resolve_project_root_for_input(&nested, false).unwrap(),
            nested
        );
    }
}
