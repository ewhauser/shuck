use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::WorkspaceFunctionContext;

pub(crate) struct Rename;

pub(crate) struct RenameSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for Rename {
    type RequestType = req::Rename;
}

impl super::super::traits::BackgroundRequestHandler for Rename {
    type Snapshot = RenameSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::RenameParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        Ok(RenameSnapshot {
            document: session
                .take_snapshot(params.text_document_position.text_document.uri.clone()),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::RenameParams,
    ) -> crate::server::Result<editor_features::RenameResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        super::cross_file_rename::rename(document, snapshot.workspace, client, params)
    }
}
