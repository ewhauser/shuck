use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};
use crate::workspace_functions::WorkspaceFunctionContext;

pub(crate) struct PrepareRename;

pub(crate) struct PrepareRenameSnapshot {
    document: Option<DocumentSnapshot>,
    workspace: WorkspaceFunctionContext,
}

impl super::RequestHandler for PrepareRename {
    type RequestType = req::PrepareRenameRequest;
}

impl super::super::traits::BackgroundRequestHandler for PrepareRename {
    type Snapshot = PrepareRenameSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::TextDocumentPositionParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        Ok(PrepareRenameSnapshot {
            document: session.take_snapshot(params.text_document.uri.clone()),
            workspace: session.workspace_function_context(cancellation),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::TextDocumentPositionParams,
    ) -> crate::server::Result<editor_features::PrepareRenameResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        super::cross_file_rename::prepare(document, snapshot.workspace, client, params)
    }
}
