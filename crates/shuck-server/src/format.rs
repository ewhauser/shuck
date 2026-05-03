#![allow(dead_code)]

use lsp_types as types;

use crate::session::{Client, DocumentSnapshot};

pub(crate) type FormatResponse = Option<Vec<types::TextEdit>>;

pub(crate) fn format_document(
    _snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentFormattingParams,
) -> crate::server::Result<FormatResponse> {
    unimplemented!("shuck LSP document formatting is not implemented yet")
}

pub(crate) fn format_range(
    _snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::DocumentRangeFormattingParams,
) -> crate::server::Result<FormatResponse> {
    unimplemented!("shuck LSP range formatting is not implemented yet")
}
