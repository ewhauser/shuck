#![allow(dead_code)]

use std::io::Read as _;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use lsp_types::{Url, WorkspaceFolder};

use crate::session::{ClientOptions, WorkspaceOptionsMap};

/// Largest closed file the server reads from disk for cross-file features.
/// Shell sources this large are generated or vendored, and loading them would
/// only stall the request that touched them.
pub(crate) const MAX_WORKSPACE_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Read a regular file as UTF-8, returning `Ok(None)` instead of loading it
/// when it is not a regular file or holds more than `max_bytes` bytes.
///
/// The size is checked up front and the read itself is capped, so a file that
/// grows between the check and the read still cannot exceed the budget, and
/// special files such as FIFOs are never opened.
pub(crate) fn read_source_bounded(
    path: &Path,
    max_bytes: usize,
) -> std::io::Result<Option<String>> {
    // Check before opening: opening a FIFO for reading blocks until a writer
    // appears.
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Ok(None);
    }
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Ok(None);
    }
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut source = String::new();
    file.take(limit).read_to_string(&mut source)?;
    if source.len() > max_bytes {
        return Ok(None);
    }
    Ok(Some(source))
}

/// Collection of workspace folders known to the LSP session.
#[derive(Debug)]
pub struct Workspaces(Vec<Workspace>);

impl Workspaces {
    /// Create a workspace collection from explicit workspace entries.
    pub fn new(workspaces: Vec<Workspace>) -> Self {
        Self(workspaces)
    }

    pub(crate) fn from_workspace_folders(
        workspace_folders: Option<Vec<WorkspaceFolder>>,
        root_uri: Option<Url>,
        root_path: Option<String>,
        mut workspace_options: WorkspaceOptionsMap,
    ) -> crate::Result<Self> {
        let mut client_options_for_url =
            |url: &Url| workspace_options.remove(url).unwrap_or_default();

        let workspaces =
            if let Some(folders) = workspace_folders.filter(|folders| !folders.is_empty()) {
                folders
                    .into_iter()
                    .map(|folder| {
                        let options = client_options_for_url(&folder.uri);
                        Workspace::new(folder.uri).with_options(options)
                    })
                    .collect()
            } else {
                let uri = default_workspace_uri(root_uri, root_path)?;
                let options = client_options_for_url(&uri);
                vec![Workspace::default(uri).with_options(options)]
            };

        Ok(Self(workspaces))
    }
}

fn default_workspace_uri(root_uri: Option<Url>, root_path: Option<String>) -> crate::Result<Url> {
    if let Some(root_uri) = root_uri {
        return Ok(root_uri);
    }

    if let Some(root_path) = root_path
        && let Ok(root_uri) = Url::from_file_path(PathBuf::from(root_path))
    {
        return Ok(root_uri);
    }

    let current_dir = std::env::current_dir()?;
    Url::from_file_path(current_dir)
        .map_err(|()| anyhow::anyhow!("failed to create URL from current directory"))
}

impl Deref for Workspaces {
    type Target = [Workspace];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// One LSP workspace folder plus optional Shuck settings.
#[derive(Debug)]
pub struct Workspace {
    url: Url,
    options: Option<ClientOptions>,
    is_default: bool,
}

impl Workspace {
    /// Create a non-default workspace for `url`.
    pub fn new(url: Url) -> Self {
        Self {
            url,
            options: None,
            is_default: false,
        }
    }

    /// Create the default workspace for clients without workspace-folder support.
    pub fn default(url: Url) -> Self {
        Self {
            url,
            options: None,
            is_default: true,
        }
    }

    /// Return a copy with workspace-specific client options.
    #[must_use]
    pub fn with_options(mut self, options: ClientOptions) -> Self {
        self.options = Some(options);
        self
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn options(&self) -> Option<&ClientOptions> {
        self.options.as_ref()
    }

    pub(crate) fn is_default(&self) -> bool {
        self.is_default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_root_uri_when_workspace_folders_are_missing() {
        let root_uri = Url::parse("file:///tmp/shuck-root").expect("test URI should parse");

        let workspaces = Workspaces::from_workspace_folders(
            None,
            Some(root_uri.clone()),
            None,
            WorkspaceOptionsMap::default(),
        )
        .expect("workspace fallback should succeed");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].url(), &root_uri);
        assert!(workspaces[0].is_default());
    }

    #[test]
    fn uses_root_path_when_root_uri_is_missing() {
        let root_path = std::env::temp_dir().join("shuck-server-root-path");
        let expected_uri =
            Url::from_file_path(&root_path).expect("temporary directory should convert to a URL");

        let workspaces = Workspaces::from_workspace_folders(
            None,
            None,
            Some(root_path.display().to_string()),
            WorkspaceOptionsMap::default(),
        )
        .expect("workspace fallback should succeed");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].url(), &expected_uri);
        assert!(workspaces[0].is_default());
    }
}

#[cfg(test)]
mod bounded_read_tests {
    use super::*;

    #[test]
    fn bounded_reader_returns_small_files_and_skips_large_ones() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("script.sh");
        std::fs::write(&path, "echo hi\n").unwrap();

        assert_eq!(
            read_source_bounded(&path, 64).unwrap().as_deref(),
            Some("echo hi\n")
        );
        assert!(read_source_bounded(&path, 4).unwrap().is_none());
    }

    #[test]
    fn bounded_reader_skips_directories() {
        let directory = tempfile::tempdir().unwrap();

        assert!(read_source_bounded(directory.path(), 64).unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_reader_does_not_block_on_fifos() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pipe.sh");
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo should run");
        assert!(status.success());

        assert!(read_source_bounded(&path, 64).unwrap().is_none());
    }
}
