use lsp_types::{self as types, request as req};

use crate::session::{Client, DocumentSnapshot};
use crate::symbols;

pub(crate) struct DocumentSymbols;

impl super::RequestHandler for DocumentSymbols {
    type RequestType = req::DocumentSymbolRequest;
}

impl super::BackgroundDocumentRequestHandler for DocumentSymbols {
    super::define_document_url!(params: &types::DocumentSymbolParams);

    fn run_without_snapshot(
        _client: &Client,
        _params: types::DocumentSymbolParams,
    ) -> crate::server::Result<symbols::DocumentSymbolResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::DocumentSymbolParams,
    ) -> crate::server::Result<symbols::DocumentSymbolResponse> {
        symbols::document_symbols(snapshot, client, params)
    }
}
