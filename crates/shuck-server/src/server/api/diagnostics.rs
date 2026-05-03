use lsp_types as types;

use crate::lint::generate_diagnostics;
use crate::server::Result;
use crate::session::{Client, DocumentQuery, DocumentSnapshot};

pub(super) fn publish_diagnostics_for_document(
    snapshot: &DocumentSnapshot,
    client: &Client,
) -> Result<()> {
    client.send_notification::<types::notification::PublishDiagnostics>(
        types::PublishDiagnosticsParams {
            uri: snapshot.query().file_url().clone(),
            diagnostics: generate_diagnostics(snapshot),
            version: Some(snapshot.query().document().version()),
        },
    )?;
    Ok(())
}

pub(super) fn clear_diagnostics_for_document(query: &DocumentQuery, client: &Client) -> Result<()> {
    client.send_notification::<types::notification::PublishDiagnostics>(
        types::PublishDiagnosticsParams {
            uri: query.file_url().clone(),
            diagnostics: Vec::new(),
            version: Some(query.document().version()),
        },
    )?;
    Ok(())
}
