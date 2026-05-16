use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct Completion;

impl super::RequestHandler for Completion {
    type RequestType = req::Completion;
}

impl super::BackgroundDocumentRequestHandler for Completion {
    fn document_url(params: &types::CompletionParams) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position.text_document.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::CompletionParams,
    ) -> crate::server::Result<editor_features::CompletionResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::CompletionParams,
    ) -> crate::server::Result<editor_features::CompletionResponse> {
        editor_features::completion(snapshot, client, params)
    }
}
