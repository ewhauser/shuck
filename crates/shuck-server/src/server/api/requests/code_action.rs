use lsp_types::{self as types, request as req};

use crate::fix;
use crate::server::Result;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct CodeActions;

impl super::RequestHandler for CodeActions {
    type RequestType = req::CodeActionRequest;
}

impl super::BackgroundDocumentRequestHandler for CodeActions {
    super::define_document_url!(params: &types::CodeActionParams);

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::CodeActionParams,
    ) -> Result<Option<types::CodeActionResponse>> {
        fix::code_actions(snapshot, client, params)
    }
}
