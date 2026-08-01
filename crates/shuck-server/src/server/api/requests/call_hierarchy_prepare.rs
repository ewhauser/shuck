use lsp_types::{self as types, request as req};

use crate::call_hierarchy::{self, CallHierarchyContext};
use crate::editor_features::CallHierarchyPrepareResponse;
use crate::session::{Client, DocumentSnapshot, Session};

pub(crate) struct CallHierarchyPrepare;

pub(crate) struct CallHierarchyPrepareSnapshot {
    document: Option<DocumentSnapshot>,
    context: CallHierarchyContext,
}

impl super::RequestHandler for CallHierarchyPrepare {
    type RequestType = req::CallHierarchyPrepare;
}

impl super::super::traits::BackgroundRequestHandler for CallHierarchyPrepare {
    type Snapshot = CallHierarchyPrepareSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::CallHierarchyPrepareParams,
    ) -> crate::server::Result<Self::Snapshot> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        Ok(CallHierarchyPrepareSnapshot {
            document: session.take_snapshot(uri),
            context: session.call_hierarchy_context(),
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        client: &Client,
        params: types::CallHierarchyPrepareParams,
    ) -> crate::server::Result<CallHierarchyPrepareResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        call_hierarchy::prepare_call_hierarchy(snapshot.context, document, client, params)
    }
}
