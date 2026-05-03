#![allow(dead_code)]

use std::ops::Deref;
use std::path::PathBuf;

use lsp_types::{Url, WorkspaceFolder};

use crate::session::{ClientOptions, WorkspaceOptionsMap};

#[derive(Debug)]
pub struct Workspaces(Vec<Workspace>);

impl Workspaces {
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

#[derive(Debug)]
pub struct Workspace {
    url: Url,
    options: Option<ClientOptions>,
    is_default: bool,
}

impl Workspace {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            options: None,
            is_default: false,
        }
    }

    pub fn default(url: Url) -> Self {
        Self {
            url,
            options: None,
            is_default: true,
        }
    }

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
