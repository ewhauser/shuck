use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::server::api::diagnostics::refresh_open_document_diagnostics;
use crate::session::{Client, Session};

pub(crate) struct DidChangeConfiguration;

impl super::super::traits::NotificationHandler for DidChangeConfiguration {
    type NotificationType = notif::DidChangeConfiguration;
}

impl super::super::traits::SyncNotificationHandler for DidChangeConfiguration {
    fn run(
        session: &mut Session,
        client: &Client,
        params: types::DidChangeConfigurationParams,
    ) -> Result<()> {
        let all_options = crate::session::AllOptions::from_value(params.settings);
        let global_settings = all_options.global.into_settings(client.clone());
        session.update_configuration(global_settings.options().clone(), all_options.workspace);
        refresh_open_document_diagnostics(session, client)
    }
}

#[cfg(test)]
mod tests {
    use crossbeam::channel;
    use lsp_server::Message;
    use lsp_types::{ClientCapabilities, Url};

    use super::*;
    use crate::server::api::traits::SyncNotificationHandler;
    use crate::{GlobalOptions, PositionEncoding, TextDocument, Workspace, Workspaces};

    fn session_with_open_document(
        capabilities: &ClientCapabilities,
    ) -> (Session, Client, channel::Receiver<Message>, Url) {
        let workspace_root = tempfile::tempdir().expect("tempdir should be created");
        let (main_loop_sender, _main_loop_receiver) = channel::unbounded();
        let (client_sender, client_receiver) = channel::unbounded();
        let client = Client::new(main_loop_sender, client_sender);
        let workspace_uri =
            Url::from_file_path(workspace_root.path()).expect("workspace path should convert");
        let workspaces = Workspaces::new(vec![Workspace::default(workspace_uri)]);
        let global = GlobalOptions::default().into_settings(client.clone());
        let mut session = Session::new(
            capabilities,
            PositionEncoding::UTF16,
            global,
            &workspaces,
            &client,
        )
        .expect("test session should initialize");
        let uri = Url::from_file_path(workspace_root.path().join("script.sh"))
            .expect("script path should convert");
        session.open_text_document(
            uri.clone(),
            TextDocument::new("foo=1\n".to_owned(), 1).with_language_id("shellscript"),
        );
        (session, client, client_receiver, uri)
    }

    fn change_settings(session: &mut Session, client: &Client, settings: serde_json::Value) {
        DidChangeConfiguration::run(
            session,
            client,
            types::DidChangeConfigurationParams { settings },
        )
        .expect("didChangeConfiguration should succeed");
    }

    #[test]
    fn push_clients_receive_republished_diagnostics_after_settings_change() {
        let (mut session, client, client_receiver, uri) =
            session_with_open_document(&ClientCapabilities::default());

        change_settings(
            &mut session,
            &client,
            serde_json::json!({ "lint": { "select": ["C006"] } }),
        );

        let Ok(Message::Notification(notification)) = client_receiver.try_recv() else {
            panic!("settings change should republish diagnostics");
        };
        assert_eq!(notification.method, "textDocument/publishDiagnostics");
        let params: types::PublishDiagnosticsParams =
            serde_json::from_value(notification.params).expect("params should deserialize");
        assert_eq!(params.uri, uri);
        assert!(
            params.diagnostics.iter().all(|diagnostic| diagnostic.code
                != Some(types::NumberOrString::String("C001".to_owned())))
        );
    }

    #[test]
    fn pull_clients_with_refresh_support_are_asked_to_repull() {
        let capabilities: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "textDocument": { "diagnostic": {} },
            "workspace": { "diagnostic": { "refreshSupport": true } }
        }))
        .expect("capabilities should deserialize");
        let (mut session, client, client_receiver, _uri) =
            session_with_open_document(&capabilities);

        change_settings(&mut session, &client, serde_json::Value::Null);

        let Ok(Message::Request(request)) = client_receiver.try_recv() else {
            panic!("settings change should request a diagnostic refresh");
        };
        assert_eq!(request.method, "workspace/diagnostic/refresh");
        assert!(client_receiver.try_recv().is_err());
    }

    #[test]
    fn pull_clients_without_refresh_support_receive_nothing() {
        let capabilities: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "textDocument": { "diagnostic": {} }
        }))
        .expect("capabilities should deserialize");
        let (mut session, client, client_receiver, _uri) =
            session_with_open_document(&capabilities);

        change_settings(&mut session, &client, serde_json::Value::Null);

        assert!(client_receiver.try_recv().is_err());
    }
}
