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

#[derive(Clone)]
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
        let cache_project_settings = supports_dynamic_watched_files(client_capabilities);
        Ok(Self {
            index: index::Index::new(workspaces, cache_project_settings, &global, client)?,
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
        let (settings, client_settings) = self
            .index
            .resolve_snapshot_settings(&url, self.global_settings.options());
        let key = self.key_from_url(url);
        Some(DocumentSnapshot {
            resolved_client_capabilities: self.resolved_client_capabilities.clone(),
            client_settings,
            document_ref: self.index.make_document_ref(key, settings)?,
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

    pub(crate) fn update_client_options(&mut self, options: ClientOptions) {
        self.global_settings.update_options(options);
        self.index.clear_project_settings_cache();
    }

    pub(crate) fn update_configuration(
        &mut self,
        options: ClientOptions,
        workspace_options: Option<WorkspaceOptionsMap>,
    ) {
        self.global_settings.update_options(options);
        if let Some(workspace_options) = workspace_options {
            self.index.update_workspace_options(workspace_options);
        } else {
            self.index.clear_project_settings_cache();
        }
    }

    pub(crate) fn open_document_count(&self) -> usize {
        self.index.open_document_count()
    }

    pub(crate) fn workspace_roots(&self) -> &[std::path::PathBuf] {
        self.index.workspace_roots()
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

fn supports_dynamic_watched_files(client_capabilities: &ClientCapabilities) -> bool {
    client_capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.did_change_watched_files)
        .and_then(|watched_files| watched_files.dynamic_registration)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, DidChangeWatchedFilesClientCapabilities, Url,
        WorkspaceClientCapabilities,
    };

    use super::*;
    use crate::{ClientOptions, GlobalOptions, TextDocument, Workspace, Workspaces};

    fn client_capabilities_with_dynamic_watched_files() -> ClientCapabilities {
        ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                did_change_watched_files: Some(DidChangeWatchedFilesClientCapabilities {
                    dynamic_registration: Some(true),
                    relative_pattern_support: None,
                }),
                ..WorkspaceClientCapabilities::default()
            }),
            ..ClientCapabilities::default()
        }
    }

    #[test]
    fn take_snapshot_merges_global_and_workspace_options() {
        let workspace_one = tempfile::tempdir().expect("workspace should be created");
        let workspace_two = tempfile::tempdir().expect("workspace should be created");
        let workspace_one_uri =
            Url::from_file_path(workspace_one.path()).expect("workspace path should convert");
        let workspace_two_uri =
            Url::from_file_path(workspace_two.path()).expect("workspace path should convert");

        let workspaces = Workspaces::new(vec![
            Workspace::default(workspace_one_uri),
            Workspace::new(workspace_two_uri.clone()).with_options(ClientOptions {
                lint: Some(shuck_config::LintConfig {
                    select: Some(vec!["C006".to_owned()]),
                    ..shuck_config::LintConfig::default()
                }),
                format: Some(shuck_config::FormatConfig {
                    indent_width: Some(2),
                    ..shuck_config::FormatConfig::default()
                }),
                fix_all: Some(false),
                ..ClientOptions::default()
            }),
        ]);
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &client_capabilities_with_dynamic_watched_files(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        session.update_client_options(ClientOptions {
            lint: Some(shuck_config::LintConfig {
                select: Some(vec!["C001".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            format: Some(shuck_config::FormatConfig {
                indent_style: Some("space".to_owned()),
                ..shuck_config::FormatConfig::default()
            }),
            show_syntax_errors: Some(true),
            ..ClientOptions::default()
        });

        let uri = Url::from_file_path(workspace_two.path().join("script.sh"))
            .expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );

        let snapshot = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot");

        assert!(
            snapshot
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
        assert_eq!(snapshot.shuck_settings().linter().rules.len(), 1);
        assert_eq!(
            snapshot.shuck_settings().formatter().indent_style(),
            shuck_formatter::IndentStyle::Space
        );
        assert_eq!(snapshot.shuck_settings().formatter().indent_width(), 2);
        assert!(!snapshot.client_settings().fix_all());
        assert!(snapshot.client_settings().show_syntax_errors());
    }

    #[test]
    fn update_configuration_updates_workspace_specific_options() {
        let workspace_one = tempfile::tempdir().expect("workspace should be created");
        let workspace_two = tempfile::tempdir().expect("workspace should be created");
        let workspace_one_uri =
            Url::from_file_path(workspace_one.path()).expect("workspace path should convert");
        let workspace_two_uri =
            Url::from_file_path(workspace_two.path()).expect("workspace path should convert");

        let workspaces = Workspaces::new(vec![
            Workspace::default(workspace_one_uri),
            Workspace::new(workspace_two_uri.clone()).with_options(ClientOptions {
                lint: Some(shuck_config::LintConfig {
                    select: Some(vec!["C006".to_owned()]),
                    ..shuck_config::LintConfig::default()
                }),
                ..ClientOptions::default()
            }),
        ]);
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &client_capabilities_with_dynamic_watched_files(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace_two.path().join("script.sh"))
            .expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );

        let before = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        assert!(
            before
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
        assert_eq!(before.shuck_settings().linter().rules.len(), 1);

        let mut workspace_options = WorkspaceOptionsMap::default();
        workspace_options.insert(
            workspace_two_uri,
            ClientOptions {
                lint: Some(shuck_config::LintConfig {
                    select: Some(vec!["C001".to_owned()]),
                    ..shuck_config::LintConfig::default()
                }),
                ..ClientOptions::default()
            },
        );
        session.update_configuration(ClientOptions::default(), Some(workspace_options));

        let after = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot");
        assert!(
            after
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(after.shuck_settings().linter().rules.len(), 1);
    }

    #[test]
    fn update_client_options_invalidates_cached_project_settings() {
        let workspace = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(
            workspace.path().join(".shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .expect("config should be written");
        let workspace_uri =
            Url::from_file_path(workspace.path()).expect("workspace path should convert");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &client_capabilities_with_dynamic_watched_files(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");

        let uri = Url::from_file_path(workspace.path().join("script.sh"))
            .expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );

        let before = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        assert!(
            before
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(before.shuck_settings().linter().rules.len(), 1);

        session.update_client_options(ClientOptions {
            lint: Some(shuck_config::LintConfig {
                select: Some(vec!["C006".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        });

        let after = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot");
        assert!(
            after
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UndefinedVariable)
        );
        assert_eq!(after.shuck_settings().linter().rules.len(), 1);
    }
}
