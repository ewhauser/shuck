use lsp_types::{self as types, request as req};

use crate::resolve;
use crate::server::Result;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct Hover;

impl super::RequestHandler for Hover {
    type RequestType = req::HoverRequest;
}

impl super::BackgroundDocumentRequestHandler for Hover {
    fn document_url(params: &types::HoverParams) -> std::borrow::Cow<'_, lsp_types::Url> {
        std::borrow::Cow::Borrowed(&params.text_document_position_params.text_document.uri)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::HoverParams,
    ) -> Result<Option<types::Hover>> {
        resolve::hover(snapshot, client, params)
    }
}
