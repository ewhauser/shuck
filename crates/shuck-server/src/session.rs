#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use lsp_types::{ClientCapabilities, FileEvent, Url};

use crate::edit::{DocumentKey, DocumentVersion};
use crate::session::request_queue::RequestQueue;
use crate::session::settings::{ClientSettings, GlobalClientSettings, ShuckSettings};
use crate::workspace::Workspaces;
use crate::{PositionEncoding, TextDocument};

pub(crate) use self::capabilities::ResolvedClientCapabilities;
pub use self::index::DocumentQuery;
pub(crate) use self::options::{AllOptions, WorkspaceOptionsMap};
pub use self::options::{ClientOptions, GlobalOptions};
pub use client::Client;

mod capabilities;
mod client;
mod index;
mod options;
mod request_queue;
mod settings;

pub struct Session {
    index: index::Index,
    position_encoding: PositionEncoding,
    global_settings: GlobalClientSettings,
    resolved_client_capabilities: Arc<ResolvedClientCapabilities>,
    request_queue: RequestQueue,
    shutdown_requested: bool,
}

pub struct DocumentSnapshot {
    resolved_client_capabilities: Arc<ResolvedClientCapabilities>,
    client_settings: Arc<ClientSettings>,
    document_ref: index::DocumentQuery,
    position_encoding: PositionEncoding,
}

impl Session {
    pub fn new(
        client_capabilities: &ClientCapabilities,
        position_encoding: PositionEncoding,
        global: GlobalClientSettings,
        workspaces: &Workspaces,
        client: &Client,
    ) -> crate::Result<Self> {
        Ok(Self {
            index: index::Index::new(workspaces, &global, client)?,
            position_encoding,
            global_settings: global,
            resolved_client_capabilities: Arc::new(ResolvedClientCapabilities::new(
                client_capabilities,
            )),
            request_queue: RequestQueue::new(),
            shutdown_requested: false,
        })
    }

    pub(crate) fn request_queue(&self) -> &RequestQueue {
        &self.request_queue
    }

    pub(crate) fn request_queue_mut(&mut self) -> &mut RequestQueue {
        &mut self.request_queue
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    pub(crate) fn set_shutdown_requested(&mut self, requested: bool) {
        self.shutdown_requested = requested;
    }

    pub fn key_from_url(&self, url: Url) -> DocumentKey {
        self.index.key_from_url(url)
    }

    pub fn take_snapshot(&self, url: Url) -> Option<DocumentSnapshot> {
        let key = self.key_from_url(url);
        Some(DocumentSnapshot {
            resolved_client_capabilities: self.resolved_client_capabilities.clone(),
            client_settings: self
                .index
                .client_settings(&key)
                .unwrap_or_else(|| self.global_settings.to_settings_arc()),
            document_ref: self.index.make_document_ref(key)?,
            position_encoding: self.position_encoding,
        })
    }

    pub(crate) fn update_text_document(
        &mut self,
        key: &DocumentKey,
        content_changes: Vec<lsp_types::TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
    ) -> crate::Result<()> {
        self.index
            .update_text_document(key, content_changes, new_version, self.encoding())
    }

    pub fn open_text_document(&mut self, url: Url, document: TextDocument) {
        self.index.open_text_document(url, document);
    }

    pub(crate) fn close_document(&mut self, key: &DocumentKey) -> crate::Result<()> {
        self.index.close_document(key)
    }

    pub(crate) fn reload_settings(&mut self, changes: &[FileEvent], client: &Client) {
        self.index.reload_settings(changes, client);
    }

    pub(crate) fn open_workspace_folder(&mut self, url: Url, client: &Client) -> crate::Result<()> {
        self.index
            .open_workspace_folder(url, &self.global_settings, client)
    }

    pub(crate) fn close_workspace_folder(&mut self, url: &Url) -> crate::Result<()> {
        self.index.close_workspace_folder(url)
    }

    pub(crate) fn resolved_client_capabilities(&self) -> &ResolvedClientCapabilities {
        &self.resolved_client_capabilities
    }

    pub(crate) fn encoding(&self) -> PositionEncoding {
        self.position_encoding
    }

    pub(crate) fn config_file_paths(&self) -> impl Iterator<Item = &Path> {
        self.index.config_file_paths()
    }
}

impl DocumentSnapshot {
    pub(crate) fn resolved_client_capabilities(&self) -> &ResolvedClientCapabilities {
        &self.resolved_client_capabilities
    }

    pub(crate) fn client_settings(&self) -> &ClientSettings {
        &self.client_settings
    }

    pub(crate) fn shuck_settings(&self) -> &ShuckSettings {
        self.document_ref.settings()
    }

    pub fn query(&self) -> &index::DocumentQuery {
        &self.document_ref
    }

    pub(crate) fn encoding(&self) -> PositionEncoding {
        self.position_encoding
    }
}
