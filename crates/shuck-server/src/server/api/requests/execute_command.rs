use lsp_types::{self as types, request as req};

use crate::fix;
use crate::session::{Client, Session};

pub(crate) struct ExecuteCommand;

impl super::RequestHandler for ExecuteCommand {
    type RequestType = req::ExecuteCommand;
}

impl super::SyncRequestHandler for ExecuteCommand {
    fn run(
        session: &mut Session,
        client: &Client,
        params: types::ExecuteCommandParams,
    ) -> crate::server::Result<Option<serde_json::Value>> {
        fix::execute_command(session, client, params)
    }
}
