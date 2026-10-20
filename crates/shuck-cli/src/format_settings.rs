use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use shuck_cache::{CacheKey, CacheKeyHasher};
#[cfg(test)]
use shuck_config::resolve::CONFIG_INDENT_WIDTH_ERROR;
use shuck_config::resolve::{ResolvePolicy, SettingsResolver, format_options_for_path};
use shuck_config::{ConfigArguments, FormatExclusions, FormatSettingsPatch, load_project_config};
use shuck_formatter::{IndentStyle, ShellDialect as FormatDialect, ShellFormatOptions};
use shuck_linter::CompiledPerFileShellList;

const CLI_INDENT_WIDTH_ERROR: &str = "`--indent-width` must be at least 1";

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedFormatSettings {
    options: ShellFormatOptions,
    exclusions: FormatExclusions,
    per_file_shell: Arc<CompiledPerFileShellList>,
    effective_per_file_shell: Vec<EffectivePerFileShell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectivePerFileShell {
    pattern: String,
    shell: String,
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
        self.exclusions.patterns().cache_key(state);
        self.effective_per_file_shell.cache_key(state);
    }
}

impl CacheKey for EffectivePerFileShell {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.pattern.cache_key(state);
        self.shell.cache_key(state);
    }
}

impl ResolvedFormatSettings {
    pub(crate) fn to_shell_format_options(&self) -> ShellFormatOptions {
        self.options.clone()
    }

    pub(crate) fn shell_format_options_for_path(&self, path: &Path) -> Result<ShellFormatOptions> {
        format_options_for_path(&self.options, &self.per_file_shell, path)
    }

    pub(crate) fn is_file_excluded(&self, path: &Path) -> bool {
        self.exclusions.is_excluded(path)
    }

    fn apply_patch(&mut self, patch: FormatSettingsPatch, indent_width_error: &str) -> Result<()> {
        if patch.indent_width == Some(0) {
            return Err(anyhow!(indent_width_error.to_owned()));
        }
        self.options = self.options.clone().with_patch(&patch);
        Ok(())
    }
}

pub(crate) fn resolve_project_format_settings(
    project_root: &Path,
    config_arguments: &ConfigArguments,
    cli_patch: FormatSettingsPatch,
) -> Result<ResolvedFormatSettings> {
    let config = load_project_config(project_root, config_arguments)?;
    let resolved =
        SettingsResolver::new(project_root, &config, ResolvePolicy::Strict).resolve_format()?;
    let effective_per_file_shell = resolved
        .per_file_shell
        .iter()
        .map(|entry| EffectivePerFileShell {
            pattern: entry.pattern().to_owned(),
            shell: entry.shell().as_str().to_owned(),
        })
        .collect();

    let mut settings = ResolvedFormatSettings {
        options: resolved.options,
        exclusions: resolved.exclusions,
        per_file_shell: resolved.compiled_per_file_shell,
        effective_per_file_shell,
    };
    settings.apply_patch(cli_patch, CLI_INDENT_WIDTH_ERROR)?;
    Ok(settings)
}

const fn indent_style_key(style: IndentStyle) -> u8 {
    match style {
        IndentStyle::Space => 0,
        IndentStyle::Tab => 1,
    }
}

const fn shell_dialect_key(dialect: FormatDialect) -> u8 {
    match dialect {
        FormatDialect::Auto => 0,
        FormatDialect::Bash => 1,
        FormatDialect::Posix => 2,
        FormatDialect::Mksh => 3,
        FormatDialect::Zsh => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::args::{FileSelectionArgs, FormatCommand, FormatDialectArg};
    use shuck_config::{CONFIG_DIALECT_UNSUPPORTED_ERROR, FormatConfig};
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

        assert_eq!(options.dialect(), FormatDialect::Auto);
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

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            args.format_settings_patch(),
        )
        .unwrap();
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
    fn top_level_per_file_shell_selects_formatter_dialect() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[per-file-shell]\n'dot_z*' = 'zsh'\n",
        )
        .unwrap();

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            format_args().format_settings_patch(),
        )
        .unwrap();

        assert_eq!(
            settings
                .shell_format_options_for_path(&tempdir.path().join("dot_zshenv"))
                .unwrap()
                .dialect(),
            FormatDialect::Zsh
        );
        assert_eq!(
            settings
                .shell_format_options_for_path(&tempdir.path().join("script.sh"))
                .unwrap()
                .dialect(),
            FormatDialect::Auto
        );
    }

    #[test]
    fn lint_per_file_shell_remains_a_formatter_compatibility_alias() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[lint]\nper-file-shell = { 'dot_z*' = 'zsh' }\n",
        )
        .unwrap();

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            format_args().format_settings_patch(),
        )
        .unwrap();

        assert_eq!(
            settings
                .shell_format_options_for_path(&tempdir.path().join("dot_zshenv"))
                .unwrap()
                .dialect(),
            FormatDialect::Zsh
        );
    }

    #[test]
    fn cli_dialect_overrides_per_file_shell() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[per-file-shell]\n'dot_z*' = 'zsh'\n",
        )
        .unwrap();
        let mut args = format_args();
        args.dialect = Some(FormatDialectArg::Bash);

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            args.format_settings_patch(),
        )
        .unwrap();

        assert_eq!(
            settings
                .shell_format_options_for_path(&tempdir.path().join("dot_zshenv"))
                .unwrap()
                .dialect(),
            FormatDialect::Bash
        );
    }

    #[test]
    fn conflicting_per_file_shell_mappings_error_for_matching_path() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[per-file-shell]\n'*' = 'bash'\n'dot_z*' = 'zsh'\n",
        )
        .unwrap();

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            format_args().format_settings_patch(),
        )
        .unwrap();
        let error = settings
            .shell_format_options_for_path(&tempdir.path().join("dot_zshenv"))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("conflicting per-file shell mappings")
        );
    }

    #[test]
    fn generic_ksh_mapping_is_rejected_for_formatting() {
        let tempdir = tempdir().unwrap();
        fs::write(
            tempdir.path().join("shuck.toml"),
            "[per-file-shell]\n'*.ksh' = 'ksh'\n",
        )
        .unwrap();

        let settings = resolve_project_format_settings(
            tempdir.path(),
            &ConfigArguments::default(),
            format_args().format_settings_patch(),
        )
        .unwrap();
        let error = settings
            .shell_format_options_for_path(&tempdir.path().join("script.ksh"))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("is not supported by the formatter")
        );
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
