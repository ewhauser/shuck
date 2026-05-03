#![allow(dead_code)]

use std::ops::Deref;

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
                let current_dir = std::env::current_dir()?;
                let uri = Url::from_file_path(current_dir)
                    .map_err(|()| anyhow::anyhow!("failed to create URL from current directory"))?;
                let options = client_options_for_url(&uri);
                vec![Workspace::default(uri).with_options(options)]
            };

        Ok(Self(workspaces))
    }
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
