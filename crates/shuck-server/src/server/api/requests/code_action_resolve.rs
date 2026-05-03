use lsp_types::{self as types, request as req};

use crate::fix;
use crate::session::{Client, Session};

pub(crate) struct CodeActionResolve;

impl super::RequestHandler for CodeActionResolve {
    type RequestType = req::CodeActionResolveRequest;
}

impl super::SyncRequestHandler for CodeActionResolve {
    fn run(
        _session: &mut Session,
        client: &Client,
        params: types::CodeAction,
    ) -> crate::server::Result<types::CodeAction> {
        fix::resolve_code_action(client, params)
    }
}
