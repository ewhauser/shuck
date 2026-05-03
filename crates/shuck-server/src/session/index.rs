#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::anyhow;
use lsp_types::{FileEvent, Url};
use rustc_hash::FxHashMap;

use crate::edit::{DocumentKey, DocumentVersion};
use crate::session::Client;
use crate::session::settings::{ClientSettings, GlobalClientSettings, ShuckSettings};
use crate::workspace::Workspaces;
use crate::{PositionEncoding, TextDocument};

#[derive(Default)]
pub(crate) struct Index {
    documents: FxHashMap<Url, Arc<TextDocument>>,
    workspace_roots: Vec<PathBuf>,
    client_settings: Arc<ClientSettings>,
}

#[derive(Clone)]
pub enum DocumentQuery {
    Text {
        file_url: Url,
        document: Arc<TextDocument>,
        settings: Arc<ShuckSettings>,
    },
}

impl Index {
    pub(super) fn new(
        workspaces: &Workspaces,
        global: &GlobalClientSettings,
        _client: &Client,
    ) -> crate::Result<Self> {
        let workspace_roots = workspaces
            .iter()
            .filter_map(|workspace| workspace.url().to_file_path().ok())
            .collect();
        Ok(Self {
            documents: FxHashMap::default(),
            workspace_roots,
            client_settings: global.to_settings_arc(),
        })
    }

    pub(super) fn key_from_url(&self, url: Url) -> DocumentKey {
        DocumentKey::Text(url)
    }

    pub(super) fn make_document_ref(&self, key: DocumentKey) -> Option<DocumentQuery> {
        let DocumentKey::Text(url) = key;
        let document = self.documents.get(&url)?.clone();
        Some(DocumentQuery::Text {
            file_url: url,
            document,
            settings: Arc::new(ShuckSettings),
        })
    }

    pub(super) fn client_settings(&self, key: &DocumentKey) -> Option<Arc<ClientSettings>> {
        let DocumentKey::Text(url) = key;
        self.documents
            .contains_key(url)
            .then(|| self.client_settings.clone())
    }

    pub(super) fn update_text_document(
        &mut self,
        key: &DocumentKey,
        content_changes: Vec<lsp_types::TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
        encoding: PositionEncoding,
    ) -> crate::Result<()> {
        let DocumentKey::Text(url) = key;
        let Some(document) = self.documents.get_mut(url) else {
            return Err(anyhow!(
                "text document URI does not point to an open document"
            ));
        };

        std::sync::Arc::make_mut(document).apply_changes(content_changes, new_version, encoding);
        Ok(())
    }

    pub(super) fn open_text_document(&mut self, url: Url, document: TextDocument) {
        self.documents.insert(url, Arc::new(document));
    }

    pub(super) fn close_document(&mut self, key: &DocumentKey) -> crate::Result<()> {
        let DocumentKey::Text(url) = key;
        self.documents
            .remove(url)
            .map(|_| ())
            .ok_or_else(|| anyhow!("document is not open: {url}"))
    }

    pub(super) fn reload_settings(&mut self, _changes: &[FileEvent], _client: &Client) {}

    pub(super) fn open_workspace_folder(
        &mut self,
        url: Url,
        _global: &GlobalClientSettings,
        _client: &Client,
    ) -> crate::Result<()> {
        let path = url
            .to_file_path()
            .map_err(|()| anyhow!("failed to convert workspace URL to file path: {url}"))?;
        if !self.workspace_roots.contains(&path) {
            self.workspace_roots.push(path);
        }
        Ok(())
    }

    pub(super) fn close_workspace_folder(&mut self, workspace_url: &Url) -> crate::Result<()> {
        let path = workspace_url.to_file_path().map_err(|()| {
            anyhow!("failed to convert workspace URL to file path: {workspace_url}")
        })?;
        self.workspace_roots.retain(|root| root != &path);
        self.documents.retain(|url, _| {
            url.to_file_path()
                .map(|file_path| !file_path.starts_with(&path))
                .unwrap_or(true)
        });
        Ok(())
    }

    pub(super) fn config_file_paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::empty()
    }
}

impl DocumentQuery {
    pub(crate) fn file_url(&self) -> &Url {
        match self {
            Self::Text { file_url, .. } => file_url,
        }
    }

    pub(crate) fn document(&self) -> &Arc<TextDocument> {
        match self {
            Self::Text { document, .. } => document,
        }
    }

    pub(crate) fn settings(&self) -> &ShuckSettings {
        match self {
            Self::Text { settings, .. } => settings,
        }
    }
}
