use lsp_types::{self as types, request as req};

use crate::folding;
use crate::session::{Client, DocumentSnapshot, RequestCancellationToken, Session};

pub(crate) struct FoldingRanges;

pub(crate) struct FoldingRangesSnapshot {
    document: Option<DocumentSnapshot>,
    cancellation: RequestCancellationToken,
}

impl super::RequestHandler for FoldingRanges {
    type RequestType = req::FoldingRangeRequest;
}

impl super::super::traits::BackgroundRequestHandler for FoldingRanges {
    type Snapshot = FoldingRangesSnapshot;

    fn snapshot(
        session: &Session,
        params: &types::FoldingRangeParams,
        cancellation: RequestCancellationToken,
    ) -> crate::server::Result<Self::Snapshot> {
        Ok(FoldingRangesSnapshot {
            document: session.take_snapshot(params.text_document.uri.clone()),
            cancellation,
        })
    }

    fn run_with_snapshot(
        snapshot: Self::Snapshot,
        _client: &Client,
        _params: types::FoldingRangeParams,
    ) -> crate::server::Result<folding::FoldingRangeResponse> {
        let Some(document) = snapshot.document else {
            return Ok(None);
        };
        Ok(folding::folding_ranges(document, &snapshot.cancellation))
    }
}
