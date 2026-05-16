use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct DocumentHighlight;

impl super::RequestHandler for DocumentHighlight {
    type RequestType = req::DocumentHighlightRequest;
}

impl super::BackgroundDocumentRequestHandler for DocumentHighlight {
    fn document_url(params: &types::DocumentHighlightParams) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position_params.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::DocumentHighlightParams,
    ) -> crate::server::Result<editor_features::DocumentHighlightResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::DocumentHighlightParams,
    ) -> crate::server::Result<editor_features::DocumentHighlightResponse> {
        editor_features::document_highlight(snapshot, client, params)
    }
}
