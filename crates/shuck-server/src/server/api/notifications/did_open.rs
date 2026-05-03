use lsp_types as types;
use lsp_types::notification as notif;

use crate::TextDocument;
use crate::server::Result;
use crate::server::api::diagnostics::publish_diagnostics_for_document;
use crate::session::{Client, Session};

pub(crate) struct DidOpen;

impl super::super::traits::NotificationHandler for DidOpen {
    type NotificationType = notif::DidOpenTextDocument;
}

impl super::super::traits::SyncNotificationHandler for DidOpen {
    fn run(
        session: &mut Session,
        client: &Client,
        types::DidOpenTextDocumentParams {
            text_document:
                types::TextDocumentItem {
                    uri,
                    text,
                    version,
                    language_id,
                },
        }: types::DidOpenTextDocumentParams,
    ) -> Result<()> {
        let document = TextDocument::new(text, version).with_language_id(&language_id);
        session.open_text_document(uri.clone(), document);

        if !session.resolved_client_capabilities().pull_diagnostics
            && let Some(snapshot) = session.take_snapshot(uri)
        {
            publish_diagnostics_for_document(&snapshot, client)?;
        }

        Ok(())
    }
}
