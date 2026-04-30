//! Source slicing helper bundling the script text with its line index.
//!
//! Mirrors the `Locator` pattern from `ruff_linter`. Helpers that need to
//! resolve byte offsets to [`Position`]s should accept a `Locator<'_>` instead
//! of separate `source: &str` and `line_index: &LineIndex` parameters; this
//! keeps signatures compact while still routing every offset lookup through
//! the precomputed [`LineIndex`].

use shuck_ast::{Position, TextSize};
use shuck_indexer::LineIndex;

#[derive(Clone, Copy, Debug)]
pub struct Locator<'a> {
    source: &'a str,
    line_index: &'a LineIndex,
}

impl<'a> Locator<'a> {
    pub fn new(source: &'a str, line_index: &'a LineIndex) -> Self {
        Self { source, line_index }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn line_index(&self) -> &'a LineIndex {
        self.line_index
    }

    /// Resolve a byte offset to a [`Position`] using the precomputed
    /// [`LineIndex`]. Returns `None` if `offset` is past the end of the source.
    #[inline]
    pub fn position_at_offset(&self, offset: usize) -> Option<Position> {
        if offset > self.source.len() {
            return None;
        }
        Some(self.position_at_offset_unchecked(offset))
    }

    /// Same as [`Locator::position_at_offset`] but without the bounds check.
    /// The caller must guarantee `offset <= source.len()` and that `offset`
    /// lies on a UTF-8 char boundary.
    #[inline]
    pub fn position_at_offset_unchecked(&self, offset: usize) -> Position {
        let line = self.line_index.line_number(TextSize::new(offset as u32));
        let line_start = self
            .line_index
            .line_start(line)
            .map(usize::from)
            .unwrap_or_default();

        Position {
            line,
            column: self.source[line_start..offset].chars().count() + 1,
            offset,
        }
    }
}
