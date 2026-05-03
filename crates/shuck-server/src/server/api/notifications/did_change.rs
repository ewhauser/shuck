use lsp_server::ErrorCode;
use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::server::api::LSPResult;
use crate::server::api::diagnostics::publish_diagnostics_for_document;
use crate::session::{Client, Session};

pub(crate) struct DidChange;

impl super::super::traits::NotificationHandler for DidChange {
    type NotificationType = notif::DidChangeTextDocument;
}

impl super::super::traits::SyncNotificationHandler for DidChange {
    fn run(
        session: &mut Session,
        client: &Client,
        types::DidChangeTextDocumentParams {
            text_document:
                types::VersionedTextDocumentIdentifier {
                    uri,
                    version: new_version,
                },
            content_changes,
        }: types::DidChangeTextDocumentParams,
    ) -> Result<()> {
        let key = session.key_from_url(uri);
        session
            .update_text_document(&key, content_changes, new_version)
            .with_failure_code(ErrorCode::InternalError)?;

        if !session.resolved_client_capabilities().pull_diagnostics {
            let snapshot = session.take_snapshot(key.into_url()).ok_or_else(|| {
                crate::server::Error::new(
                    anyhow::anyhow!("failed to take document snapshot after change"),
                    ErrorCode::InternalError,
                )
            })?;
            publish_diagnostics_for_document(&snapshot, client)?;
        }

        Ok(())
    }
}
