#![allow(dead_code)]

use lsp_types as types;

use crate::session::{Client, DocumentSnapshot};

pub(crate) fn hover(
    _snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::HoverParams,
) -> crate::server::Result<Option<types::Hover>> {
    unimplemented!("shuck LSP hover is not implemented yet")
}
