use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use url::Url;

pub(crate) fn fetch_url_to_path(url: &str, dest: &Path, verbose: bool) -> Result<()> {
    if let Some(source_path) = file_url_path(url)? {
        fs::copy(&source_path, dest)
            .with_context(|| format!("copy {} to {}", source_path.display(), dest.display()))?;
        return Ok(());
    }

    let mut command = Command::new("curl");
    command.arg("--fail").arg("--location");
    command.arg("--retry").arg("1").arg("--retry-all-errors");
    if !verbose {
        command.arg("--silent").arg("--show-error");
    }
    command.arg("--output").arg(dest);
    command.arg(url);
    let status = command.status().context("run curl")?;
    if !status.success() {
        bail!("curl failed while fetching {url}");
    }
    Ok(())
}

fn file_url_path(raw_url: &str) -> Result<Option<PathBuf>> {
    let Ok(parsed_url) = Url::parse(raw_url) else {
        return Ok(None);
    };
    if parsed_url.scheme() != "file" {
        return Ok(None);
    }
    parsed_url
        .to_file_path()
        .map(Some)
        .map_err(|_| anyhow!("invalid file URL `{raw_url}`"))
}
