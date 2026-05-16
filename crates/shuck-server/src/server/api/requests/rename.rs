use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct Rename;

impl super::RequestHandler for Rename {
    type RequestType = req::Rename;
}

impl super::BackgroundDocumentRequestHandler for Rename {
    fn document_url(params: &types::RenameParams) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::RenameParams,
    ) -> crate::server::Result<editor_features::RenameResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::RenameParams,
    ) -> crate::server::Result<editor_features::RenameResponse> {
        editor_features::rename(snapshot, client, params)
    }
}
