use std::fs;
use std::path::{Path, PathBuf};

use super::{
    CliConfigOverride, CompatCliError, CompatColorMode, CompatFormat, CompatSeverityThreshold,
    parse_bool, parse_code_list, parse_optional_check_list, parse_source_path_list,
};

const CONFIG_FILENAME: &str = ".shellcheckrc";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompatConfig {
    pub check_sourced: Option<bool>,
    pub color: Option<CompatColorMode>,
    pub include_codes: Vec<String>,
    pub exclude_codes: Vec<String>,
    pub extended_analysis: Option<bool>,
    pub format: Option<CompatFormat>,
    pub enable_checks: Vec<String>,
    pub source_paths: Vec<String>,
    pub shell: Option<String>,
    pub severity: Option<CompatSeverityThreshold>,
    pub wiki_link_count: Option<usize>,
    pub external_sources: Option<bool>,
}

pub fn resolve_config_override(
    cwd: &Path,
    override_mode: &CliConfigOverride,
) -> Result<Option<PathBuf>, CompatCliError> {
    match override_mode {
        CliConfigOverride::Ignore => Ok(None),
        CliConfigOverride::Explicit(path) => Ok(Some(absolutize(cwd, path))),
        CliConfigOverride::Search => Ok(find_config(cwd)),
    }
}

pub fn load_config(path: &Path) -> Result<CompatConfig, CompatCliError> {
    let source = fs::read_to_string(path).map_err(|err| {
        CompatCliError::usage(4, format!("could not read {}: {err}", path.display()))
    })?;

    let mut config = CompatConfig::default();
    for (index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=').ok_or_else(|| {
            CompatCliError::usage(
                4,
                format!(
                    "{}:{}: expected key=value in shellcheck rc file",
                    path.display(),
                    index + 1
                ),
            )
        })?;
        apply_config_entry(&mut config, key.trim(), value.trim(), path, index + 1)?;
    }

    Ok(config)
}

fn apply_config_entry(
    config: &mut CompatConfig,
    key: &str,
    value: &str,
    path: &Path,
    line: usize,
) -> Result<(), CompatCliError> {
    let invalid =
        |detail: String| CompatCliError::usage(4, format!("{}:{line}: {detail}", path.display()));

    match key {
        "check-sourced" => {
            config.check_sourced = Some(
                parse_bool(value)
                    .ok_or_else(|| invalid("check-sourced expects a boolean value".to_owned()))?,
            );
        }
        "color" => {
            config.color = Some(
                value
                    .parse()
                    .map_err(|_| invalid("color expects auto, always, or never".to_owned()))?,
            );
        }
        "disable" => {
            config.exclude_codes.extend(parse_code_list(value));
        }
        "enable" => {
            config
                .enable_checks
                .extend(parse_optional_check_list(value));
        }
        "extended-analysis" => {
            config.extended_analysis =
                Some(parse_bool(value).ok_or_else(|| {
                    invalid("extended-analysis expects a boolean value".to_owned())
                })?);
        }
        "external-sources" => {
            config.external_sources =
                Some(parse_bool(value).ok_or_else(|| {
                    invalid("external-sources expects a boolean value".to_owned())
                })?);
        }
        "format" => {
            config.format = Some(value.parse().map_err(|_| {
                invalid(
                    "format expects checkstyle, diff, gcc, json, json1, quiet, or tty".to_owned(),
                )
            })?);
        }
        "include" => {
            config.include_codes.extend(parse_code_list(value));
        }
        "severity" => {
            config.severity = Some(value.parse().map_err(|_| {
                invalid("severity expects error, warning, info, or style".to_owned())
            })?);
        }
        "shell" => {
            config.shell = Some(value.to_owned());
        }
        "source-path" => {
            config.source_paths.extend(parse_source_path_list(value));
        }
        "wiki-link-count" => {
            let count = value.parse::<usize>().map_err(|_| {
                invalid("wiki-link-count expects a non-negative integer".to_owned())
            })?;
            config.wiki_link_count = Some(count);
        }
        _ => {
            return Err(invalid(format!("unsupported shellcheck rc key `{key}`")));
        }
    }

    Ok(())
}

fn find_config(cwd: &Path) -> Option<PathBuf> {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        let candidate = dir.join(CONFIG_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

fn absolutize(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_config_parses_disable_and_enable_lists() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join(".shellcheckrc");
        fs::write(
            &path,
            "disable=SC2086,2154\nenable=check-unassigned-uppercase\n",
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.exclude_codes, vec!["SC2086", "2154"]);
        assert_eq!(config.enable_checks, vec!["check-unassigned-uppercase"]);
    }

    #[test]
    fn resolve_config_search_walks_upward_from_cwd() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = tempdir.path().join("project");
        let subdir = root.join("sub");
        fs::create_dir_all(&subdir).unwrap();
        fs::write(root.join(".shellcheckrc"), "disable=SC2086\n").unwrap();

        let resolved = resolve_config_override(&subdir, &CliConfigOverride::Search).unwrap();
        assert_eq!(resolved, Some(root.join(".shellcheckrc")));
    }
}
