use lsp_types::{self as types, request as req};

use crate::server::Result;
use crate::session::{Client, Session};
use crate::symbols;

pub(crate) struct WorkspaceSymbols;

impl super::RequestHandler for WorkspaceSymbols {
    type RequestType = req::WorkspaceSymbolRequest;
}

impl super::super::traits::BackgroundRequestHandler for WorkspaceSymbols {
    type Snapshot = symbols::WorkspaceSymbolContext;

    fn snapshot(
        session: &Session,
        _params: &types::WorkspaceSymbolParams,
    ) -> Result<Self::Snapshot> {
        Ok(session.workspace_symbol_context())
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::WorkspaceSymbolParams,
    ) -> Result<symbols::WorkspaceSymbolResponse> {
        symbols::workspace_symbols(snapshot, client, params)
    }
}
