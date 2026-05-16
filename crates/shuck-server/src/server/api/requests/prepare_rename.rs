use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct PrepareRename;

impl super::RequestHandler for PrepareRename {
    type RequestType = req::PrepareRenameRequest;
}

impl super::BackgroundDocumentRequestHandler for PrepareRename {
    fn document_url(
        params: &types::TextDocumentPositionParams,
    ) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::TextDocumentPositionParams,
    ) -> crate::server::Result<editor_features::PrepareRenameResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::TextDocumentPositionParams,
    ) -> crate::server::Result<editor_features::PrepareRenameResponse> {
        editor_features::prepare_rename(snapshot, client, params)
    }
}
