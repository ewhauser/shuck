use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use url::Url;

/// Connection budget for a single remote fetch.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Abort transfers that stay below this many bytes per second ...
const STALL_SPEED_LIMIT_BYTES: u64 = 1024;
/// ... for this long.
const STALL_SPEED_TIME: Duration = Duration::from_secs(30);

/// Upper bound on how long a fetch may take in total.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FetchBudget {
    /// Small JSON registry documents.
    Document,
    /// Shell archives.
    Archive,
}

impl FetchBudget {
    fn max_time(self) -> Duration {
        match self {
            Self::Document => Duration::from_secs(60),
            Self::Archive => Duration::from_secs(15 * 60),
        }
    }
}

/// Validate a registry URL that is about to be fetched.
///
/// Only `https` and `file` URLs are accepted, and `file` URLs are only honored when the
/// document that referenced them was itself loaded from the local filesystem. A remote
/// registry must never be able to point Shuck at arbitrary local paths.
pub(crate) fn validate_fetch_url(url: &Url, referrer: Option<&Url>) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "file" => match referrer {
            Some(referrer) if referrer.scheme() != "file" => {
                bail!("registry document `{referrer}` may not reference local file `{url}`")
            }
            _ => Ok(()),
        },
        scheme => bail!("unsupported URL scheme `{scheme}` in `{url}`; expected https or file"),
    }
}

pub(crate) fn fetch_url_to_path(
    url: &Url,
    dest: &Path,
    budget: FetchBudget,
    verbose: bool,
) -> Result<()> {
    if let Some(source_path) = file_url_path(url)? {
        fs::copy(&source_path, dest)
            .with_context(|| format!("copy {} to {}", source_path.display(), dest.display()))?;
        return Ok(());
    }
    if url.scheme() != "https" {
        bail!("refusing to fetch `{url}`: only https and file URLs are supported");
    }

    let mut command = Command::new("curl");
    // `-q` must come first so a user's ~/.curlrc cannot change how we fetch.
    command.arg("-q");
    command.arg("--fail").arg("--location");
    command.arg("--proto").arg("=https");
    command.arg("--proto-redir").arg("=https");
    command
        .arg("--connect-timeout")
        .arg(CONNECT_TIMEOUT.as_secs().to_string());
    command
        .arg("--max-time")
        .arg(budget.max_time().as_secs().to_string());
    command
        .arg("--speed-limit")
        .arg(STALL_SPEED_LIMIT_BYTES.to_string())
        .arg("--speed-time")
        .arg(STALL_SPEED_TIME.as_secs().to_string());
    command.arg("--retry").arg("1").arg("--retry-all-errors");
    command.arg("--output").arg(dest);
    command.stdin(Stdio::null()).stdout(Stdio::null());
    if verbose {
        command.stderr(Stdio::inherit());
    } else {
        // Capture diagnostics instead of letting them leak into the output of the
        // script being run; they are surfaced through the returned error instead.
        command.arg("--silent").arg("--show-error");
        command.stderr(Stdio::piped());
    }
    command.arg("--").arg(url.as_str());

    let output = command.output().context("run curl")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("curl failed while fetching {url}");
        }
        bail!("curl failed while fetching {url}: {stderr}");
    }
    Ok(())
}

fn file_url_path(url: &Url) -> Result<Option<PathBuf>> {
    if url.scheme() != "file" {
        return Ok(None);
    }
    url.to_file_path()
        .map(Some)
        .map_err(|_| anyhow!("invalid file URL `{url}`"))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::validate_fetch_url;

    #[test]
    fn remote_documents_cannot_reference_local_files() {
        let remote = Url::parse("https://example.com/index.json").unwrap();
        let local = Url::parse("file:///etc/passwd").unwrap();
        assert!(validate_fetch_url(&local, Some(&remote)).is_err());
        assert!(validate_fetch_url(&local, Some(&local)).is_ok());
        assert!(validate_fetch_url(&local, None).is_ok());
        assert!(validate_fetch_url(&remote, Some(&local)).is_ok());
    }

    #[test]
    fn only_https_and_file_schemes_are_accepted() {
        for raw in [
            "http://example.com/a.tar.gz",
            "ftp://example.com/a.tar.gz",
            "scp://host/a",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(validate_fetch_url(&url, None).is_err(), "{raw}");
        }
    }
}
