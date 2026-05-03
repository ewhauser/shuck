use lsp_types::{self as types, request as req};

use crate::format;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct FormatRange;

impl super::RequestHandler for FormatRange {
    type RequestType = req::RangeFormatting;
}

impl super::BackgroundDocumentRequestHandler for FormatRange {
    super::define_document_url!(params: &types::DocumentRangeFormattingParams);

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::DocumentRangeFormattingParams,
    ) -> crate::server::Result<crate::format::FormatResponse> {
        format::format_range(snapshot, client, params)
    }
}
