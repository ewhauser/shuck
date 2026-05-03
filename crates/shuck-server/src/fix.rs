#![allow(dead_code)]

use lsp_types as types;

use crate::session::{Client, DocumentSnapshot, Session};

pub(crate) fn code_actions(
    _snapshot: DocumentSnapshot,
    _client: &Client,
    _params: types::CodeActionParams,
) -> crate::server::Result<Option<types::CodeActionResponse>> {
    unimplemented!("shuck LSP code actions are not implemented yet")
}

pub(crate) fn resolve_code_action(
    _client: &Client,
    _action: types::CodeAction,
) -> crate::server::Result<types::CodeAction> {
    unimplemented!("shuck LSP code action resolution is not implemented yet")
}

pub(crate) fn execute_command(
    _session: &mut Session,
    _client: &Client,
    _params: types::ExecuteCommandParams,
) -> crate::server::Result<Option<serde_json::Value>> {
    unimplemented!("shuck LSP executeCommand is not implemented yet")
}
