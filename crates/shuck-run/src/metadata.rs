use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::types::parse_shell_name;
use crate::{Shell, VersionConstraint};

#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptInfo {
    pub(crate) inferred_shell: Option<Shell>,
    pub(crate) metadata: Option<ScriptMetadata>,
}

pub(crate) fn read_script_info(path: &Path) -> Result<ScriptInfo> {
    let bytes = fs::read(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            anyhow!("{}: No such file", path.display())
        } else {
            anyhow!(err).context(format!("read {}", path.display()))
        }
    })?;
    let source = String::from_utf8_lossy(&bytes);
    Ok(ScriptInfo {
        inferred_shell: Shell::infer(&source, Some(path)),
        metadata: parse_script_metadata(&source)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptMetadata {
    pub(crate) shell: Shell,
    pub(crate) version: Option<VersionConstraint>,
}

pub(crate) fn parse_script_metadata(source: &str) -> Result<Option<ScriptMetadata>> {
    let mut start_line = None;
    let mut saw_body = false;
    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(start) = trimmed.strip_prefix("# /// shuck") {
            if !start.trim().is_empty() {
                bail!("invalid shuck metadata header");
            }
            if saw_body {
                bail!("shuck metadata blocks must appear before the script body");
            }
            if start_line.is_some() {
                bail!("multiple `# /// shuck` blocks are not allowed");
            }
            start_line = Some(line_index);
            break;
        }

        if trimmed.starts_with('#') {
            continue;
        }

        saw_body = true;
    }

    let Some(start_line) = start_line else {
        return Ok(None);
    };

    let mut body = String::new();
    let mut lines = source.lines().enumerate().skip(start_line + 1);
    for (_, line) in lines.by_ref() {
        let trimmed = line.trim();
        if trimmed == "# ///" {
            let block: MetadataBlock = toml::from_str(&body).context("parse shuck metadata")?;
            let shell = parse_shell_name(&block.shell)?;
            let version = block
                .version
                .as_deref()
                .map(VersionConstraint::parse)
                .transpose()?;
            for (_, trailing_line) in lines {
                let trailing = trailing_line.trim();
                if trailing.is_empty() {
                    continue;
                }
                if trailing == "# /// shuck" {
                    bail!("multiple `# /// shuck` blocks are not allowed");
                }
                if trailing.starts_with('#') {
                    continue;
                }
                break;
            }
            return Ok(Some(ScriptMetadata { shell, version }));
        }

        let Some(comment_body) = line.trim_start().strip_prefix('#') else {
            bail!("shuck metadata block must stay in the leading comment header");
        };
        body.push_str(comment_body.strip_prefix(' ').unwrap_or(comment_body));
        body.push('\n');
    }

    bail!("unterminated `# /// shuck` metadata block")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataBlock {
    shell: String,
    version: Option<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<BTreeMap<String, toml::Value>>,
}
