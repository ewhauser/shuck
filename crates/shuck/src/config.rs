use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::discover::normalize_path;
use crate::format_settings::{FormatSettingsPatch, parse_config_indent_style};

const CONFIG_FILENAMES: [&str; 2] = [".shuck.toml", "shuck.toml"];
pub(crate) const CONFIG_DIALECT_UNSUPPORTED_ERROR: &str = "`[format].dialect` is not supported; formatter dialect is auto-discovered from the file name or shebang. Use `--dialect` for a per-run override";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ShuckConfig {
    pub format: FormatConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
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

pub(crate) fn resolve_project_root_for_input(input: &Path) -> io::Result<PathBuf> {
    let base_dir = base_dir_for_input(input)?;
    Ok(find_config_root(&base_dir)?.unwrap_or(base_dir))
}

pub(crate) fn resolve_project_root_for_file(
    file: &Path,
    fallback_start: &Path,
) -> io::Result<PathBuf> {
    let start = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback_start.to_path_buf());
    Ok(find_config_root(&start)?.unwrap_or_else(|| normalize_path(fallback_start)))
}

pub(crate) fn load_project_config(project_root: &Path) -> Result<ShuckConfig> {
    let Some(config_path) = config_path_for_root(project_root)? else {
        return Ok(ShuckConfig::default());
    };

    let source = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    toml::from_str(&source).with_context(|| format!("parse {}", config_path.display()))
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
