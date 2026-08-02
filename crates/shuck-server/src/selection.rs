use lsp_types as types;

use crate::session::{DocumentSnapshot, RequestCancellationToken};

pub(crate) type SelectionRangeResponse = Option<Vec<types::SelectionRange>>;

pub(crate) fn selection_ranges(
    snapshot: DocumentSnapshot,
    positions: &[types::Position],
    cancellation: &RequestCancellationToken,
) -> SelectionRangeResponse {
    if cancellation.is_cancelled() {
        return None;
    }
    let analysis = snapshot.analysis()?;
    let mut selections = Vec::with_capacity(positions.len());

    for position in positions {
        if cancellation.is_cancelled() {
            return None;
        }
        let point = types::Range::new(*position, *position);
        let offset = crate::edit::to_text_range(
            &point,
            analysis.source(),
            analysis.line_index(),
            snapshot.encoding(),
        )
        .start();
        let chain = analysis
            .indexer()
            .selection_range_index()
            .selection_chain(offset);

        let mut parent = None;
        for range in chain.into_iter().rev() {
            if cancellation.is_cancelled() {
                return None;
            }
            parent = Some(Box::new(types::SelectionRange {
                range: crate::edit::to_lsp_range(
                    range,
                    analysis.source(),
                    analysis.line_index(),
                    snapshot.encoding(),
                ),
                parent,
            }));
        }
        let Some(selection) = parent else {
            continue;
        };
        selections.push(*selection);
    }

    Some(selections)
}
