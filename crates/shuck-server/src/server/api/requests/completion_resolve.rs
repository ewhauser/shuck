use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, Session};

pub(crate) struct CompletionResolve;

impl super::RequestHandler for CompletionResolve {
    type RequestType = req::ResolveCompletionItem;
}

impl super::SyncRequestHandler for CompletionResolve {
    fn run(
        _session: &mut Session,
        _client: &Client,
        params: types::CompletionItem,
    ) -> crate::server::Result<types::CompletionItem> {
        editor_features::resolve_completion_item(params)
    }
}
