use lsp_types::{self as types, request as req};

use crate::call_hierarchy::{self, CallHierarchyContext};
use crate::server::Result;
use crate::session::{Client, Session};

pub(crate) struct CallHierarchyIncomingCalls;

impl super::RequestHandler for CallHierarchyIncomingCalls {
    type RequestType = req::CallHierarchyIncomingCalls;
}

impl super::super::traits::BackgroundRequestHandler for CallHierarchyIncomingCalls {
    type Snapshot = CallHierarchyContext;

    fn snapshot(
        session: &Session,
        _params: &types::CallHierarchyIncomingCallsParams,
    ) -> Result<Self::Snapshot> {
        Ok(session.call_hierarchy_context())
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        params: types::CallHierarchyIncomingCallsParams,
    ) -> Result<call_hierarchy::IncomingResponse> {
        call_hierarchy::incoming_calls(snapshot, params)
    }
}
