use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct CallHierarchyIncomingCalls;

impl super::RequestHandler for CallHierarchyIncomingCalls {
    type RequestType = req::CallHierarchyIncomingCalls;
}

impl super::BackgroundDocumentRequestHandler for CallHierarchyIncomingCalls {
    fn document_url(
        params: &types::CallHierarchyIncomingCallsParams,
    ) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.item.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::CallHierarchyIncomingCallsParams,
    ) -> crate::server::Result<editor_features::CallHierarchyIncomingResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::CallHierarchyIncomingCallsParams,
    ) -> crate::server::Result<editor_features::CallHierarchyIncomingResponse> {
        editor_features::call_hierarchy_incoming_calls(snapshot, client, params)
    }
}
