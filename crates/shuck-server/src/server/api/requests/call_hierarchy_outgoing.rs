use lsp_types::{self as types, request as req};

use crate::call_hierarchy;
use crate::server::Result;
use crate::session::{Client, RequestCancellationToken, Session};
use crate::workspace_functions::WorkspaceFunctionContext;

pub(crate) struct CallHierarchyOutgoingCalls;

impl super::RequestHandler for CallHierarchyOutgoingCalls {
    type RequestType = req::CallHierarchyOutgoingCalls;
}

impl super::super::traits::BackgroundRequestHandler for CallHierarchyOutgoingCalls {
    type Snapshot = WorkspaceFunctionContext;

    fn snapshot(
        session: &Session,
        _params: &types::CallHierarchyOutgoingCallsParams,
        cancellation: RequestCancellationToken,
    ) -> Result<Self::Snapshot> {
        Ok(session.workspace_function_context(cancellation))
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        params: types::CallHierarchyOutgoingCallsParams,
    ) -> Result<call_hierarchy::OutgoingResponse> {
        call_hierarchy::outgoing_calls(snapshot, params)
    }
}
