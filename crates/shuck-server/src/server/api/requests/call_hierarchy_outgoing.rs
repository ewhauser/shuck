use lsp_types::{self as types, request as req};

use crate::call_hierarchy::{self, CallHierarchyContext};
use crate::server::Result;
use crate::session::{Client, Session};

pub(crate) struct CallHierarchyOutgoingCalls;

impl super::RequestHandler for CallHierarchyOutgoingCalls {
    type RequestType = req::CallHierarchyOutgoingCalls;
}

impl super::super::traits::BackgroundRequestHandler for CallHierarchyOutgoingCalls {
    type Snapshot = CallHierarchyContext;

    fn snapshot(
        session: &Session,
        _params: &types::CallHierarchyOutgoingCallsParams,
    ) -> Result<Self::Snapshot> {
        Ok(session.call_hierarchy_context())
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        params: types::CallHierarchyOutgoingCallsParams,
    ) -> Result<call_hierarchy::OutgoingResponse> {
        call_hierarchy::outgoing_calls(snapshot, params)
    }
}
