use lsp_types as types;
use lsp_types::notification as notif;

use crate::server::Result;
use crate::server::api::LSPResult;
use crate::server::api::diagnostics::clear_diagnostics_for_document;
use crate::session::{Client, Session};

pub(crate) struct DidClose;

impl super::super::traits::NotificationHandler for DidClose {
    type NotificationType = notif::DidCloseTextDocument;
}

impl super::super::traits::SyncNotificationHandler for DidClose {
    fn run(
        session: &mut Session,
        client: &Client,
        types::DidCloseTextDocumentParams {
            text_document: types::TextDocumentIdentifier { uri },
        }: types::DidCloseTextDocumentParams,
    ) -> Result<()> {
        let key = session.key_from_url(uri);
        if let Some(snapshot) = session.take_snapshot(key.clone().into_url()) {
            clear_diagnostics_for_document(snapshot.query(), client)?;
        }
        session
            .close_document(&key)
            .with_failure_code(lsp_server::ErrorCode::InternalError)
    }
}
