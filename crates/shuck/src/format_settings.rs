use std::path::Path;

use anyhow::{Result, anyhow};
use shuck_cache::{CacheKey, CacheKeyHasher};
use shuck_formatter::{IndentStyle, ShellDialect, ShellFormatOptions};

use crate::config::load_project_config;

const CLI_INDENT_WIDTH_ERROR: &str = "`--indent-width` must be at least 1";
const CONFIG_INDENT_WIDTH_ERROR: &str = "`[format].indent-width` must be at least 1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FormatSettingsPatch {
    pub dialect: Option<ShellDialect>,
    pub indent_style: Option<IndentStyle>,
    pub indent_width: Option<u8>,
    pub binary_next_line: Option<bool>,
    pub switch_case_indent: Option<bool>,
    pub space_redirects: Option<bool>,
    pub keep_padding: Option<bool>,
    pub function_next_line: Option<bool>,
    pub never_split: Option<bool>,
    pub simplify: Option<bool>,
    pub minify: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResolvedFormatSettings {
    options: ShellFormatOptions,
}

impl CacheKey for ResolvedFormatSettings {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(b"effective-format-settings");
        state.write_u8(shell_dialect_key(self.options.dialect()));
        state.write_u8(indent_style_key(self.options.indent_style()));
        state.write_u8(self.options.indent_width());
        state.write_bool(self.options.binary_next_line());
        state.write_bool(self.options.switch_case_indent());
        state.write_bool(self.options.space_redirects());
        state.write_bool(self.options.keep_padding());
        state.write_bool(self.options.function_next_line());
        state.write_bool(self.options.never_split());
        state.write_bool(self.options.simplify());
        state.write_bool(self.options.minify());
    }
}

impl ResolvedFormatSettings {
    pub(crate) fn to_shell_format_options(&self) -> ShellFormatOptions {
        self.options.clone()
    }

    fn apply_patch(&mut self, patch: FormatSettingsPatch, indent_width_error: &str) -> Result<()> {
        let mut options = self.options.clone();

        if let Some(dialect) = patch.dialect {
            options = options.with_dialect(dialect);
        }
        if let Some(indent_style) = patch.indent_style {
            options = options.with_indent_style(indent_style);
        }
        if let Some(indent_width) = patch.indent_width {
            if indent_width == 0 {
                return Err(anyhow!(indent_width_error.to_owned()));
            }
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

        self.options = options;
        Ok(())
    }
}

pub(crate) fn resolve_project_format_settings(
    project_root: &Path,
    cli_patch: FormatSettingsPatch,
) -> Result<ResolvedFormatSettings> {
    let config = load_project_config(project_root)?;
    let config_patch = config.format.to_patch()?;

    let mut settings = ResolvedFormatSettings::default();
    settings.apply_patch(config_patch, CONFIG_INDENT_WIDTH_ERROR)?;
    settings.apply_patch(cli_patch, CLI_INDENT_WIDTH_ERROR)?;
    Ok(settings)
}

pub(crate) fn parse_config_indent_style(value: &str) -> Result<IndentStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tab" => Ok(IndentStyle::Tab),
        "space" => Ok(IndentStyle::Space),
        _ => Err(anyhow!(
            "unsupported `[format].indent-style` value `{value}`; expected one of: tab, space"
        )),
    }
}

const fn indent_style_key(style: IndentStyle) -> u8 {
    match style {
        IndentStyle::Space => 0,
        IndentStyle::Tab => 1,
    }
}

const fn shell_dialect_key(dialect: ShellDialect) -> u8 {
    match dialect {
        ShellDialect::Auto => 0,
        ShellDialect::Bash => 1,
        ShellDialect::Posix => 2,
        ShellDialect::Mksh => 3,
        ShellDialect::Zsh => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::args::{FileSelectionArgs, FormatCommand};
    use crate::config::{CONFIG_DIALECT_UNSUPPORTED_ERROR, FormatConfig};

    fn format_args() -> FormatCommand {
        FormatCommand {
            files: vec![PathBuf::from(".")],
            check: false,
            diff: false,
            no_cache: false,
            stdin_filename: None,
            file_selection: FileSelectionArgs::default(),
            dialect: None,
            indent_style: None,
            indent_width: None,
            binary_next_line: false,
            no_binary_next_line: false,
            switch_case_indent: false,
            no_switch_case_indent: false,
            space_redirects: false,
            no_space_redirects: false,
            keep_padding: false,
            no_keep_padding: false,
            function_next_line: false,
            no_function_next_line: false,
            never_split: false,
            no_never_split: false,
            simplify: false,
            minify: false,
        }
    }

    #[test]
    fn defaults_match_formatter_defaults() {
        let settings = ResolvedFormatSettings::default();
        assert_eq!(
            settings.to_shell_format_options(),
            ShellFormatOptions::default()
        );
    }

    #[test]
    fn config_patch_overrides_defaults() {
        let config = FormatConfig {
            indent_style: Some("space".to_owned()),
            indent_width: Some(2),
            binary_next_line: Some(true),
            switch_case_indent: Some(true),
            space_redirects: Some(true),
            keep_padding: Some(true),
            function_next_line: Some(true),
            never_split: Some(true),
            ..FormatConfig::default()
        };

        let mut settings = ResolvedFormatSettings::default();
        settings
            .apply_patch(config.to_patch().unwrap(), CONFIG_INDENT_WIDTH_ERROR)
            .unwrap();
        let options = settings.to_shell_format_options();

        assert_eq!(options.dialect(), ShellDialect::Auto);
        assert_eq!(options.indent_style(), IndentStyle::Space);
        assert_eq!(options.indent_width(), 2);
        assert!(options.binary_next_line());
        assert!(options.switch_case_indent());
        assert!(options.space_redirects());
        assert!(options.keep_padding());
        assert!(options.function_next_line());
        assert!(options.never_split());
    }

    #[test]
    fn cli_patch_overrides_config_patch() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[format]\nfunction-next-line = false\nindent-width = 2\n",
        )
        .unwrap();

        let mut args = format_args();
        args.function_next_line = true;
        args.indent_width = Some(4);

        let settings =
            resolve_project_format_settings(tempdir.path(), args.format_settings_patch()).unwrap();
        let options = settings.to_shell_format_options();

        assert!(options.function_next_line());
        assert_eq!(options.indent_width(), 4);
    }

    #[test]
    fn configured_dialect_errors_with_migration_hint() {
        let config = FormatConfig {
            dialect: Some(toml::Value::String("zsh".to_owned())),
            ..FormatConfig::default()
        };

        let err = config.to_patch().unwrap_err();
        assert_eq!(err.to_string(), CONFIG_DIALECT_UNSUPPORTED_ERROR);
    }

    #[test]
    fn cli_patch_keeps_paired_boolean_tri_state() {
        let defaults = format_args().format_settings_patch();
        assert_eq!(defaults.function_next_line, None);
        assert_eq!(defaults.binary_next_line, None);

        let mut positive = format_args();
        positive.function_next_line = true;
        positive.binary_next_line = true;
        let positive = positive.format_settings_patch();
        assert_eq!(positive.function_next_line, Some(true));
        assert_eq!(positive.binary_next_line, Some(true));

        let mut negative = format_args();
        negative.no_function_next_line = true;
        negative.no_binary_next_line = true;
        let negative = negative.format_settings_patch();
        assert_eq!(negative.function_next_line, Some(false));
        assert_eq!(negative.binary_next_line, Some(false));
    }

    #[test]
    fn invalid_indent_width_errors_with_source_specific_message() {
        let mut config_settings = ResolvedFormatSettings::default();
        let config = FormatConfig {
            indent_width: Some(0),
            ..FormatConfig::default()
        };
        let config_err = config_settings
            .apply_patch(config.to_patch().unwrap(), CONFIG_INDENT_WIDTH_ERROR)
            .unwrap_err();
        assert_eq!(config_err.to_string(), CONFIG_INDENT_WIDTH_ERROR);

        let mut cli_settings = ResolvedFormatSettings::default();
        let mut args = format_args();
        args.indent_width = Some(0);
        let cli_err = cli_settings
            .apply_patch(args.format_settings_patch(), CLI_INDENT_WIDTH_ERROR)
            .unwrap_err();
        assert_eq!(cli_err.to_string(), CLI_INDENT_WIDTH_ERROR);
    }
}
