#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use lsp_types::{ClientCapabilities, FileEvent, Url};

use crate::analysis::{DocumentAnalysis, DocumentAnalysisCache};
use crate::edit::{DocumentKey, DocumentVersion};
use crate::session::request_queue::RequestQueue;
use crate::session::settings::{ClientSettings, GlobalClientSettings};
use crate::workspace::Workspaces;
use crate::{PositionEncoding, TextDocument};

pub(crate) use self::capabilities::ResolvedClientCapabilities;
pub use self::index::DocumentQuery;
pub(crate) use self::index::WorkspaceSettingsSnapshot;
pub(crate) use self::options::{AllOptions, WorkspaceOptionsMap};
pub use self::options::{
    ClientOptions, CompletionFeatureOptions, GlobalOptions, RenameFeatureOptions,
    WorkspaceSymbolFeatureOptions,
};
pub(crate) use self::settings::ShuckSettings;
pub use client::Client;

mod capabilities;
mod client;
mod index;
mod options;
mod request_queue;
mod settings;

/// Mutable LSP session state for open documents, workspaces, and settings.
pub struct Session {
    index: index::Index,
    position_encoding: PositionEncoding,
    global_settings: GlobalClientSettings,
    resolved_client_capabilities: Arc<ResolvedClientCapabilities>,
    workspace_symbols: Arc<crate::symbols::WorkspaceSymbolIndex>,
    analysis_cache: Arc<DocumentAnalysisCache>,
    request_queue: RequestQueue,
    shutdown_requested: bool,
}

/// Immutable view of one document plus resolved settings.
#[derive(Clone)]
pub struct DocumentSnapshot {
    resolved_client_capabilities: Arc<ResolvedClientCapabilities>,
    client_settings: Arc<ClientSettings>,
    document_ref: index::DocumentQuery,
    position_encoding: PositionEncoding,
    analysis_cache: Arc<DocumentAnalysisCache>,
    analysis_settings_epoch: u64,
}

impl Session {
    /// Create a session from client capabilities, global settings, and workspaces.
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
            workspace_symbols: Arc::new(crate::symbols::WorkspaceSymbolIndex::default()),
            analysis_cache: Arc::new(DocumentAnalysisCache::new()),
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

    /// Return the document key for an LSP document URL.
    pub fn key_from_url(&self, url: Url) -> DocumentKey {
        self.index.key_from_url(url)
    }

    /// Capture a document snapshot for diagnostics, hovers, or code actions.
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
            analysis_cache: self.analysis_cache.clone(),
            analysis_settings_epoch: self.analysis_cache.current_settings_epoch(),
        })
    }

    pub(crate) fn update_text_document(
        &mut self,
        key: &DocumentKey,
        content_changes: Vec<lsp_types::TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
    ) -> crate::Result<()> {
        let result =
            self.index
                .update_text_document(key, content_changes, new_version, self.encoding());
        if result.is_ok() {
            self.analysis_cache.invalidate_uri(&key.clone().into_url());
        }
        result
    }

    /// Open or replace an in-memory text document.
    pub fn open_text_document(&mut self, url: Url, document: TextDocument) {
        self.analysis_cache.invalidate_uri(&url);
        self.index.open_text_document(url, document);
    }

    pub(crate) fn close_document(&mut self, key: &DocumentKey) -> crate::Result<()> {
        self.index.close_document(key)?;
        self.analysis_cache.invalidate_uri(&key.clone().into_url());
        self.workspace_symbols
            .invalidate_uri(&key.clone().into_url());
        Ok(())
    }

    pub(crate) fn reload_settings(&mut self, changes: &[FileEvent], client: &Client) {
        self.index.reload_settings(changes, client);
        self.analysis_cache.clear();
        self.workspace_symbols.invalidate_file_events(changes);
    }

    pub(crate) fn open_workspace_folder(&mut self, url: Url, client: &Client) -> crate::Result<()> {
        self.index
            .open_workspace_folder(url, &self.global_settings, client)?;
        self.analysis_cache.clear();
        self.workspace_symbols.invalidate_all();
        Ok(())
    }

    pub(crate) fn close_workspace_folder(&mut self, url: &Url) -> crate::Result<()> {
        self.index.close_workspace_folder(url)?;
        self.analysis_cache.clear();
        self.workspace_symbols.invalidate_all();
        Ok(())
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

    pub(crate) fn set_project_settings_cache_enabled(&mut self, enabled: bool) {
        self.index.set_project_settings_cache_enabled(enabled);
    }

    pub(crate) fn update_client_options(&mut self, options: ClientOptions) {
        self.analysis_cache.clear();
        self.workspace_symbols.invalidate_all();
        self.global_settings.update_options(options);
        self.index.clear_project_settings_cache();
    }

    pub(crate) fn update_configuration(
        &mut self,
        options: ClientOptions,
        workspace_options: Option<WorkspaceOptionsMap>,
    ) {
        self.analysis_cache.clear();
        self.workspace_symbols.invalidate_all();
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

    pub(crate) fn workspace_symbol_context(&self) -> crate::symbols::WorkspaceSymbolContext {
        let workspace_settings = self.index.workspace_settings_snapshot();
        let workspace_roots = self.index.workspace_roots().to_vec();
        let mut settings_workspace_roots = workspace_roots.clone();
        for workspace in &workspace_settings {
            let Some(canonical_root) = &workspace.canonical_root else {
                continue;
            };
            if !settings_workspace_roots
                .iter()
                .any(|root| root == canonical_root)
            {
                settings_workspace_roots.push(canonical_root.clone());
            }
        }

        crate::symbols::WorkspaceSymbolContext {
            index: self.workspace_symbols.clone(),
            options: self.global_settings.workspace_symbol_options(),
            global_options: self.global_settings.options().clone(),
            workspace_settings,
            workspace_roots,
            settings_workspace_roots,
            open_documents: self.index.open_documents_snapshot(),
            encoding: self.position_encoding,
        }
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

    /// Return the query object used to access the underlying document and settings.
    pub fn query(&self) -> &index::DocumentQuery {
        &self.document_ref
    }

    pub(crate) fn encoding(&self) -> PositionEncoding {
        self.position_encoding
    }

    pub(crate) fn analysis(&self) -> Option<Arc<DocumentAnalysis>> {
        self.analysis_cache.get_or_build(self)
    }

    pub(crate) fn analysis_settings_epoch(&self) -> u64 {
        self.analysis_settings_epoch
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, DidChangeWatchedFilesClientCapabilities,
        TextDocumentContentChangeEvent, Url, WorkspaceClientCapabilities,
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

    fn make_test_session() -> (tempfile::TempDir, Session, Url) {
        let workspace = tempfile::tempdir().expect("workspace should be created");
        let workspace_uri =
            Url::from_file_path(workspace.path()).expect("workspace path should convert");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        let uri = Url::from_file_path(workspace.path().join("script.sh"))
            .expect("script path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("#!/bin/bash\nname=value\necho \"$name\"\n".to_owned(), 1)
                .with_language_id("shellscript"),
        );
        (workspace, session, uri)
    }

    #[test]
    fn document_analysis_cache_reuses_same_document_version() {
        let (_workspace, session, uri) = make_test_session();
        let first = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot")
            .analysis()
            .expect("shell document should have analysis");
        let second = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot")
            .analysis()
            .expect("shell document should have analysis");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn document_analysis_cache_invalidates_after_document_change() {
        let (_workspace, mut session, uri) = make_test_session();
        let before = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot")
            .analysis()
            .expect("shell document should have analysis");
        let key = session.key_from_url(uri.clone());

        session
            .update_text_document(
                &key,
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "#!/bin/bash\nother=value\necho \"$other\"\n".to_owned(),
                }],
                2,
            )
            .expect("document change should apply");

        let after = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot")
            .analysis()
            .expect("shell document should have analysis");

        assert!(!Arc::ptr_eq(&before, &after));
        assert!(after.source().contains("other=value"));
    }

    #[test]
    fn document_analysis_cache_invalidates_after_configuration_change() {
        let (_workspace, mut session, uri) = make_test_session();
        let stale_snapshot = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        let before = stale_snapshot
            .analysis()
            .expect("shell document should have analysis");

        session.update_client_options(ClientOptions {
            lint: Some(shuck_config::LintConfig {
                select: Some(vec!["C006".to_owned()]),
                ..shuck_config::LintConfig::default()
            }),
            ..ClientOptions::default()
        });

        let stale_after_clear = stale_snapshot
            .analysis()
            .expect("stale snapshot can still analyze its own settings epoch");
        let after = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot")
            .analysis()
            .expect("shell document should have analysis");

        assert!(!Arc::ptr_eq(&before, &after));
        assert!(!Arc::ptr_eq(&stale_after_clear, &after));
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
        session.set_project_settings_cache_enabled(true);
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
        session.set_project_settings_cache_enabled(true);

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
        session.set_project_settings_cache_enabled(true);

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

    #[test]
    fn nested_config_creation_switches_to_a_new_cache_key() {
        let workspace = tempfile::tempdir().expect("workspace should be created");
        std::fs::write(
            workspace.path().join(".shuck.toml"),
            "[lint]\nselect = ['C001']\n",
        )
        .expect("config should be written");
        let nested = workspace.path().join("nested");
        std::fs::create_dir_all(&nested).expect("nested dir should be created");
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
        session.set_project_settings_cache_enabled(true);

        let uri = Url::from_file_path(nested.join("script.sh"))
            .expect("test path should convert to a URL");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );

        let before = session
            .take_snapshot(uri.clone())
            .expect("test document should produce a snapshot");
        assert_eq!(
            before.shuck_settings().project_root(),
            Some(workspace.path())
        );
        assert!(
            before
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );

        std::fs::write(nested.join(".shuck.toml"), "[lint]\nselect = ['C006']\n")
            .expect("nested config should be written");

        let after = session
            .take_snapshot(uri)
            .expect("test document should produce a snapshot");
        assert_eq!(
            after.shuck_settings().project_root(),
            Some(nested.as_path())
        );
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
