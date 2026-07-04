use lsp_types::{self as types, request as req};

use crate::editor_features;
use crate::session::{Client, DocumentSnapshot};

pub(crate) struct CallHierarchyOutgoingCalls;

impl super::RequestHandler for CallHierarchyOutgoingCalls {
    type RequestType = req::CallHierarchyOutgoingCalls;
}

impl super::BackgroundDocumentRequestHandler for CallHierarchyOutgoingCalls {
    fn document_url(
        params: &types::CallHierarchyOutgoingCallsParams,
    ) -> std::borrow::Cow<'_, types::Url> {
        std::borrow::Cow::Borrowed(&params.item.uri)
    }

    fn run_without_snapshot(
        _client: &Client,
        _params: types::CallHierarchyOutgoingCallsParams,
    ) -> crate::server::Result<editor_features::CallHierarchyOutgoingResponse> {
        Ok(None)
    }

    fn run_with_snapshot(
        snapshot: DocumentSnapshot,
        client: &Client,
        params: types::CallHierarchyOutgoingCallsParams,
    ) -> crate::server::Result<editor_features::CallHierarchyOutgoingResponse> {
        editor_features::call_hierarchy_outgoing_calls(snapshot, client, params)
    }
}
