use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct References;

impl super::RequestHandler for References {
    type RequestType = req::References;
}

impl super::BackgroundDocumentRequestHandler for References {
    fn document_url(params: &types::ReferenceParams) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::ReferenceParams,
    ) -> crate::server::Result<editor_features::ReferencesResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::ReferenceParams,
    ) -> crate::server::Result<editor_features::ReferencesResponse> {
        editor_features::references(snapshot, client, params)
    }
}
