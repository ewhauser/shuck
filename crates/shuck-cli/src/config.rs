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

use crate::discover::normalize_path;
use crate::format_settings::{FormatSettingsPatch, parse_config_indent_style};

const CONFIG_FILENAMES: [&str; 2] = [".shuck.toml", "shuck.toml"];
pub(crate) const CONFIG_DIALECT_UNSUPPORTED_ERROR: &str = "`[format].dialect` is not supported; formatter dialect is auto-discovered from the file name or shebang. Use `--dialect` for a per-run override";
const CONFIG_OVERRIDE_ROOT_KEYS: &[&str] = &["check", "format", "lint"];
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
    "fixable",
    "unfixable",
    "extend-fixable",
    "rule-options",
];
const CONFIG_OVERRIDE_LINT_RULE_OPTION_KEYS: &[&str] = &["c001", "c063"];
const CONFIG_OVERRIDE_C001_RULE_OPTION_KEYS: &[&str] =
    &["treat-indirect-expansion-targets-as-used"];
const CONFIG_OVERRIDE_C063_RULE_OPTION_KEYS: &[&str] = &["report-unreached-nested-definitions"];

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub(crate) struct ShuckConfig {
    pub check: CheckConfig,
    pub format: FormatConfig,
    pub lint: LintConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct CheckConfig {
    pub embedded: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct FormatConfig {
    pub dialect: Option<toml::Value>,
    pub indent_style: Option<String>,
    pub indent_width: Option<u8>,
    pub binary_next_line: Option<bool>,
    pub switch_case_indent: Option<bool>,
    pub space_redirects: Option<bool>,
    pub keep_padding: Option<bool>,
    pub function_next_line: Option<bool>,
    pub never_split: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct LintConfig {
    pub select: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
    pub extend_select: Option<Vec<String>>,
    pub per_file_ignores: Option<BTreeMap<String, Vec<String>>>,
    pub extend_per_file_ignores: Option<BTreeMap<String, Vec<String>>>,
    pub fixable: Option<Vec<String>>,
    pub unfixable: Option<Vec<String>>,
    pub extend_fixable: Option<Vec<String>>,
    pub rule_options: Option<LintRuleOptionsConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct LintRuleOptionsConfig {
    pub c001: Option<C001RuleOptionsConfig>,
    pub c063: Option<C063RuleOptionsConfig>,
}

impl LintRuleOptionsConfig {
    fn apply_overrides(&mut self, overrides: Self) {
        if let Some(c001) = overrides.c001 {
            self.c001.get_or_insert_default().apply_overrides(c001);
        }
        if let Some(c063) = overrides.c063 {
            self.c063.get_or_insert_default().apply_overrides(c063);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct C001RuleOptionsConfig {
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct C063RuleOptionsConfig {
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

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ConfigArguments {
    isolated: bool,
    config_file: Option<PathBuf>,
    overrides: ShuckConfig,
}

impl ConfigArguments {
    pub(crate) fn from_cli(
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

    pub(crate) fn use_config_roots(&self) -> bool {
        !self.isolated && self.config_file.is_none()
    }

    pub(crate) fn explicit_config_file(&self) -> Option<&Path> {
        self.config_file.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SingleConfigArgument {
    FilePath(PathBuf),
    SettingsOverride(Box<ShuckConfig>),
}

#[derive(Clone)]
pub(crate) struct ConfigArgumentParser;

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

pub(crate) fn resolve_project_root_for_input(
    input: &Path,
    use_config_roots: bool,
) -> io::Result<PathBuf> {
    let base_dir = base_dir_for_input(input)?;
    if use_config_roots {
        Ok(find_config_root(&base_dir)?.unwrap_or(base_dir))
    } else {
        Ok(base_dir)
    }
}

pub(crate) fn resolve_project_root_for_file(
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

pub(crate) fn load_project_config(
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

impl FormatConfig {
    pub(crate) fn to_patch(&self) -> Result<FormatSettingsPatch> {
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
    }

    Ok(())
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

pub(crate) fn discovered_config_path_for_root(root: &Path) -> io::Result<Option<PathBuf>> {
    config_path_for_root(root)
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
    fn inline_config_overrides_reject_uninitialized_declaration_rule_option() {
        let err = parse_config_override(
            "lint.rule-options.c001.report-uninitialized-declarations = true",
        )
        .unwrap_err();
        assert!(err.contains("unsupported `[lint.rule-options.c001]` option"));
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
