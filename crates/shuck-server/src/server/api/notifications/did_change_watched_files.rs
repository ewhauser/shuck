use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::session::{Client, Session};

pub(crate) struct DidChangeWatchedFiles;

impl super::super::traits::NotificationHandler for DidChangeWatchedFiles {
    type NotificationType = notif::DidChangeWatchedFiles;
}

impl super::super::traits::SyncNotificationHandler for DidChangeWatchedFiles {
    fn run(
        session: &mut Session,
        client: &Client,
        params: types::DidChangeWatchedFilesParams,
    ) -> Result<()> {
        session.reload_settings(&params.changes, client);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_types::{
        ClientCapabilities, DidChangeWatchedFilesClientCapabilities, FileChangeType, FileEvent,
        Url, WorkspaceClientCapabilities,
    };

    use super::*;
    use crate::server::api::traits::SyncNotificationHandler;
    use crate::{
        Client, GlobalOptions, PositionEncoding, Session, TextDocument, Workspace, Workspaces,
    };

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
    fn config_changes_are_reflected_on_the_next_snapshot() {
        let workspace_root = tempfile::tempdir().expect("tempdir should be created");
        let config_path = workspace_root.path().join(".shuck.toml");
        std::fs::write(&config_path, "[lint]\nselect = ['C001']\n")
            .expect("config should be written");
        let file_path = workspace_root.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(workspace_root.path()).expect("workspace path should convert");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
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
        let uri = Url::from_file_path(&file_path).expect("script path should convert to a URL");
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

        std::fs::write(&config_path, "[lint]\nselect = ['C006']\n")
            .expect("config should be updated");

        let cached = session
            .take_snapshot(uri.clone())
            .expect("settings should stay cached until invalidated");
        assert!(
            cached
                .shuck_settings()
                .linter()
                .rules
                .contains(shuck_linter::Rule::UnusedAssignment)
        );
        assert_eq!(cached.shuck_settings().linter().rules.len(), 1);

        DidChangeWatchedFiles::run(
            &mut session,
            &client,
            types::DidChangeWatchedFilesParams {
                changes: vec![FileEvent {
                    uri: Url::from_file_path(&config_path)
                        .expect("config path should convert to a URL"),
                    typ: FileChangeType::CHANGED,
                }],
            },
        )
        .expect("didChangeWatchedFiles should succeed");

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
    fn config_changes_refresh_without_dynamic_file_watch_support() {
        let workspace_root = tempfile::tempdir().expect("tempdir should be created");
        let config_path = workspace_root.path().join(".shuck.toml");
        std::fs::write(&config_path, "[lint]\nselect = ['C001']\n")
            .expect("config should be written");
        let file_path = workspace_root.path().join("script.sh");
        std::fs::write(&file_path, "foo=1\n").expect("source should be written");

        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, _client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(workspace_root.path()).expect("workspace path should convert");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            &ClientCapabilities::default(),
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        let uri = Url::from_file_path(&file_path).expect("script path should convert to a URL");
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

        std::fs::write(&config_path, "[lint]\nselect = ['C006']\n")
            .expect("config should be updated");

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
