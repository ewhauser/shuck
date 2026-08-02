use lsp_types as types;
use shuck_indexer::FoldingRangeKind;

use crate::session::{DocumentSnapshot, RequestCancellationToken};

pub(crate) type FoldingRangeResponse = Option<Vec<types::FoldingRange>>;

pub(crate) fn folding_ranges(
    snapshot: DocumentSnapshot,
    cancellation: &RequestCancellationToken,
) -> FoldingRangeResponse {
    if cancellation.is_cancelled() {
        return None;
    }
    let analysis = snapshot.analysis()?;
    let capabilities = snapshot.resolved_client_capabilities();
    let limit = capabilities
        .folding_range_limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(usize::MAX);
    let line_folding_only = capabilities.line_folding_only;
    let mut ranges = Vec::new();

    for indexed in analysis.indexer().folding_range_index().ranges() {
        if cancellation.is_cancelled() {
            return None;
        }
        if ranges.len() >= limit {
            break;
        }

        let range = crate::edit::to_lsp_range(
            indexed.range(),
            analysis.source(),
            analysis.line_index(),
            snapshot.encoding(),
        );
        ranges.push(types::FoldingRange {
            start_line: range.start.line,
            start_character: (!line_folding_only).then_some(range.start.character),
            end_line: range.end.line,
            end_character: (!line_folding_only).then_some(range.end.character),
            kind: match indexed.kind() {
                FoldingRangeKind::Region => None,
                FoldingRangeKind::Comment => Some(types::FoldingRangeKind::Comment),
            },
            collapsed_text: None,
        });
    }

    Some(ranges)
}
