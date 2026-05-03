use lsp_types::{self as types, request as req};

use crate::format;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct Format;

impl super::RequestHandler for Format {
    type RequestType = req::Formatting;
}

impl super::BackgroundDocumentRequestHandler for Format {
    super::define_document_url!(params: &types::DocumentFormattingParams);

    fn run_without_snapshot(
        _client: &Client,
        _params: types::DocumentFormattingParams,
    ) -> crate::server::Result<crate::format::FormatResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::DocumentFormattingParams,
    ) -> crate::server::Result<crate::format::FormatResponse> {
        format::format_document(snapshot, client, params)
    }
}
