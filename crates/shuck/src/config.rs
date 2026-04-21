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
const CONFIG_OVERRIDE_ROOT_KEYS: &[&str] = &["format"];
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub(crate) struct ShuckConfig {
    pub format: FormatConfig,
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
                    overrides.apply_overrides(config_override);
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
    SettingsOverride(ShuckConfig),
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
        self.format.apply_overrides(overrides.format);
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
    fn inline_config_overrides_reject_unknown_root_keys() {
        let err = parse_config_override("check.embedded = false").unwrap_err();
        assert!(err.contains("unsupported config option `check`"));
    }

    #[test]
    fn inline_config_overrides_reject_unknown_format_keys() {
        let err = parse_config_override("format.line-length = 88").unwrap_err();
        assert!(err.contains("unsupported `[format]` option `line-length`"));
    }

    #[test]
    fn config_arguments_allow_multiple_inline_overrides_with_last_one_winning() {
        let tempdir = tempdir().unwrap();
        let config = ConfigArguments::from_cli(
            vec![
                SingleConfigArgument::SettingsOverride(
                    parse_config_override("format.indent-width = 2").unwrap(),
                ),
                SingleConfigArgument::SettingsOverride(
                    parse_config_override("format.indent-width = 4").unwrap(),
                ),
            ],
            false,
        )
        .unwrap();

        let loaded = load_project_config(tempdir.path(), &config).unwrap();
        assert_eq!(loaded.format.indent_width, Some(4));
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
    fn isolated_only_uses_inline_overrides() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[format]\nfunction-next-line = true\n",
        )
        .unwrap();

        let config = ConfigArguments::from_cli(
            vec![SingleConfigArgument::SettingsOverride(
                parse_config_override("format.indent-width = 2").unwrap(),
            )],
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
