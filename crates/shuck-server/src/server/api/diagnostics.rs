use lsp_types as types;

use crate::lint::generate_diagnostics;
use crate::server::Result;
use crate::session::{Client, DocumentQuery, DocumentSnapshot, Session};

/// Bring diagnostics for every open document up to date after a change that
/// can affect all of them, such as new client settings or an edited config
/// file.
///
/// Push-mode clients get a fresh `publishDiagnostics` per open document.
/// Pull-mode clients are asked to re-pull via `workspace/diagnostic/refresh`
/// when they advertise support for it; otherwise they re-pull on their own
/// schedule.
pub(super) fn refresh_open_document_diagnostics(session: &Session, client: &Client) -> Result<()> {
    let capabilities = session.resolved_client_capabilities();
    if capabilities.pull_diagnostics {
        if capabilities.workspace_refresh {
            client.send_request::<types::request::WorkspaceDiagnosticRefresh>(
                session,
                (),
                |_, _, ()| {},
            )?;
        }
        return Ok(());
    }

    for url in session.open_document_urls() {
        if let Some(snapshot) = session.take_snapshot(url) {
            publish_diagnostics_for_document(&snapshot, client)?;
        }
    }
    Ok(())
}

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
