use shuck_ast::TextRange;

use crate::PositionEncoding;

pub(crate) trait RangeExt {
    fn to_text_range(
        &self,
        text: &str,
        index: &shuck_indexer::LineIndex,
        encoding: PositionEncoding,
    ) -> TextRange;
}

pub(crate) trait ToRangeExt {
    fn to_range(
        &self,
        text: &str,
        index: &shuck_indexer::LineIndex,
        encoding: PositionEncoding,
    ) -> lsp_types::Range;
}

impl RangeExt for lsp_types::Range {
    fn to_text_range(
        &self,
        text: &str,
        index: &shuck_indexer::LineIndex,
        encoding: PositionEncoding,
    ) -> TextRange {
        crate::edit::to_text_range(self, text, index, encoding)
    }
}

impl ToRangeExt for TextRange {
    fn to_range(
        &self,
        text: &str,
        index: &shuck_indexer::LineIndex,
        encoding: PositionEncoding,
    ) -> lsp_types::Range {
        crate::edit::to_lsp_range(*self, text, index, encoding)
    }
}
