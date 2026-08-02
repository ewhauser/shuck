use lsp_types::{self as types, request as req};

use crate::selection;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};

pub(crate) struct SelectionRanges;

pub(crate) struct SelectionRangesSnapshot {
    document: Option<DocumentSnapshot>,
    cancellation: RequestCancellationToken,
}

impl super::RequestHandler for SelectionRanges {
    type RequestType = req::SelectionRangeRequest;
}

impl super::super::traits::BackgroundRequestHandler for SelectionRanges {
    type Snapshot = SelectionRangesSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::SelectionRangeParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        Ok(SelectionRangesSnapshot {
            document: session.take_snapshot(params.text_document.uri.clone()),
            cancellation,
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        params: types::SelectionRangeParams,
    ) -> crate::server::Result<selection::SelectionRangeResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        Ok(selection::selection_ranges(
            document,
            &params.positions,
            &snapshot.cancellation,
        ))
    }
}
