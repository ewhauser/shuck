use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use shuck_formatter::{IndentStyle, ShellDialect, ShellFormatOptions};

const CONFIG_FILENAMES: [&str; 2] = [".shuck.toml", "shuck.toml"];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ShuckConfig {
    pub format: FormatConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct FormatConfig {
    pub dialect: Option<String>,
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
    pub(crate) fn apply_to(&self, mut options: ShellFormatOptions) -> Result<ShellFormatOptions> {
        if let Some(dialect) = self.dialect.as_deref() {
            options = options.with_dialect(parse_dialect(dialect)?);
        }
        if let Some(indent_style) = self.indent_style.as_deref() {
            options = options.with_indent_style(parse_indent_style(indent_style)?);
        }
        if let Some(indent_width) = self.indent_width {
            if indent_width == 0 {
                return Err(anyhow!("`[format].indent-width` must be at least 1"));
            }
            options = options.with_indent_width(indent_width);
        }
        if let Some(binary_next_line) = self.binary_next_line {
            options = options.with_binary_next_line(binary_next_line);
        }
        if let Some(switch_case_indent) = self.switch_case_indent {
            options = options.with_switch_case_indent(switch_case_indent);
        }
        if let Some(space_redirects) = self.space_redirects {
            options = options.with_space_redirects(space_redirects);
        }
        if let Some(keep_padding) = self.keep_padding {
            options = options.with_keep_padding(keep_padding);
        }
        if let Some(function_next_line) = self.function_next_line {
            options = options.with_function_next_line(function_next_line);
        }
        if let Some(never_split) = self.never_split {
            options = options.with_never_split(never_split);
        }

        Ok(options)
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

fn parse_dialect(value: &str) -> Result<ShellDialect> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ShellDialect::Auto),
        "bash" => Ok(ShellDialect::Bash),
        "posix" => Ok(ShellDialect::Posix),
        "mksh" => Ok(ShellDialect::Mksh),
        "zsh" => Ok(ShellDialect::Zsh),
        _ => Err(anyhow!(
            "unsupported `[format].dialect` value `{value}`; expected one of: auto, bash, posix, mksh, zsh"
        )),
    }
}

fn parse_indent_style(value: &str) -> Result<IndentStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tab" => Ok(IndentStyle::Tab),
        "space" => Ok(IndentStyle::Space),
        _ => Err(anyhow!(
            "unsupported `[format].indent-style` value `{value}`; expected one of: tab, space"
        )),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}
