use shuck_ast::{TextRange, TextSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawContinuationCandidate {
    pub(crate) line_start: TextSize,
    pub(crate) backslash: TextSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawContinuationMode {
    Ignore,
    ReturnOnly,
    StoreAndReturn,
}

/// Source line-ending style inferred while indexing physical lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEndingStyle {
    /// Unix-style `\n` line endings.
    #[default]
    Lf,
    /// Windows-style `\r\n` line endings.
    CrLf,
}

/// Maps between byte offsets and 1-based source lines.
///
/// `LineIndex` stores the byte offset of each physical line start in the source
/// text. It intentionally uses byte offsets, not character columns, so lookups
/// stay cheap and match parser spans exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<TextSize>,
    raw_continuation_line_starts: Vec<TextSize>,
    raw_continuation_backslashes: Vec<TextSize>,
    line_ending: LineEndingStyle,
}

impl LineIndex {
    /// Build a line index from source text.
    ///
    /// Empty source is treated as one empty line starting at byte offset `0`.
    /// Every byte immediately after `\n` is recorded as the start of the next
    /// line, including a trailing empty line when the source ends in a newline.
    ///
    pub fn new(source: &str) -> Self {
        Self::build(source, RawContinuationMode::Ignore).0
    }

    /// Build a line index that also stores raw backslash-newline candidates.
    ///
    /// Most callers should use [`Self::new`], which records line starts and
    /// line-ending style only. Formatter-style source-preserving consumers can
    /// use this constructor when they also need
    /// [`Self::raw_continuation_line_starts`] or
    /// [`Self::raw_continuation_backslashes`].
    pub fn with_raw_continuations(source: &str) -> Self {
        Self::build(source, RawContinuationMode::StoreAndReturn).0
    }

    pub(crate) fn build(
        source: &str,
        raw_continuation_mode: RawContinuationMode,
    ) -> (Self, Vec<RawContinuationCandidate>) {
        let bytes = source.as_bytes();
        let mut line_starts = Vec::new();
        let mut raw_continuation_line_starts = Vec::new();
        let mut raw_continuation_backslashes = Vec::new();
        let mut raw_continuations = Vec::new();
        let mut line_ending = LineEndingStyle::Lf;
        line_starts.push(TextSize::new(0));

        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'\n' {
                let next_line_start = TextSize::new((index + 1) as u32);
                line_starts.push(next_line_start);
                if index > 0 && bytes[index - 1] == b'\r' {
                    line_ending = LineEndingStyle::CrLf;
                }
                let backslash_index = if index > 0 && bytes[index - 1] == b'\r' {
                    index.checked_sub(2)
                } else {
                    index.checked_sub(1)
                };
                if let Some(backslash_index) = backslash_index
                    && bytes[backslash_index] == b'\\'
                {
                    let backslash = TextSize::new(backslash_index as u32);
                    match raw_continuation_mode {
                        RawContinuationMode::Ignore => {}
                        RawContinuationMode::ReturnOnly => {
                            raw_continuations.push(RawContinuationCandidate {
                                line_start: next_line_start,
                                backslash,
                            });
                        }
                        RawContinuationMode::StoreAndReturn => {
                            raw_continuations.push(RawContinuationCandidate {
                                line_start: next_line_start,
                                backslash,
                            });
                            raw_continuation_line_starts.push(next_line_start);
                            raw_continuation_backslashes.push(backslash);
                        }
                    }
                }
            }
        }

        (
            Self {
                line_starts,
                raw_continuation_line_starts,
                raw_continuation_backslashes,
                line_ending,
            },
            raw_continuations,
        )
    }

    /// Return the 1-based line number containing `offset`.
    ///
    /// `offset` is a byte offset into the source. Offsets at a line start belong
    /// to that new line, while offsets past the final indexed line map to the
    /// last line. This method does not validate that `offset` is within the
    /// original source length.
    pub fn line_number(&self, offset: TextSize) -> usize {
        self.line_starts.partition_point(|start| *start <= offset)
    }

    /// Return the byte offset of the start of a 1-based line.
    ///
    /// Returns `None` for line `0` or for lines beyond the indexed source.
    pub fn line_start(&self, line: usize) -> Option<TextSize> {
        line.checked_sub(1)
            .and_then(|index| self.line_starts.get(index).copied())
    }

    /// Return the byte range of a 1-based line, excluding its trailing newline.
    ///
    /// `source` must be the same text used to construct the index. The returned
    /// range starts at [`Self::line_start`] and ends before a trailing `\n` when
    /// present; a preceding `\r` is left in the range so callers can decide how
    /// to handle CRLF display. Returns `None` for invalid line numbers.
    pub fn line_range(&self, line: usize, source: &str) -> Option<TextRange> {
        let start = self.line_start(line)?;
        let line_index = line.checked_sub(1)?;
        let next_start = self
            .line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or_else(|| TextSize::new(source.len() as u32));

        let mut end = next_start;
        if usize::from(next_start) > usize::from(start)
            && source.as_bytes()[usize::from(next_start) - 1] == b'\n'
        {
            end = TextSize::new(next_start.to_u32() - 1);
        }

        Some(TextRange::new(start, end))
    }

    /// Return the byte offset for a 1-based line and 1-based column.
    ///
    /// `column` may point just past the final character on the line, in which
    /// case the returned offset is the end of [`Self::line_range`].
    pub fn offset_for_line_column(
        &self,
        line: usize,
        column: usize,
        source: &str,
    ) -> Option<TextSize> {
        if line == 0 || column == 0 {
            return None;
        }

        let range = self.line_range(line, source)?;
        let line_start = usize::from(range.start());
        let line_end = usize::from(range.end());
        let line_text = source.get(line_start..line_end)?;

        let mut current_column = 1usize;
        for (relative_offset, _) in line_text.char_indices() {
            if current_column == column {
                return Some(TextSize::new((line_start + relative_offset) as u32));
            }
            current_column += 1;
        }

        (current_column == column).then_some(range.end())
    }

    /// Return the number of physical lines recorded for the source.
    ///
    /// The count is always at least `1`, even for empty source.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Return the source line-ending style observed while indexing lines.
    ///
    /// A file with any `\r\n` line ending is treated as CRLF, matching the
    /// formatter's existing preservation behavior.
    pub fn line_ending(&self) -> LineEndingStyle {
        self.line_ending
    }

    /// Return byte offsets for physical line starts after a raw backslash-newline.
    ///
    /// These are unfiltered lexical candidates and are only retained when the
    /// line index is built with [`Self::with_raw_continuations`], or through an
    /// [`crate::Indexer`] with source-layout indexes enabled. Use
    /// [`Indexer::continuation_line_starts`](crate::Indexer::continuation_line_starts)
    /// when comments, quoted regions, and heredoc bodies should be ignored.
    pub fn raw_continuation_line_starts(&self) -> &[TextSize] {
        &self.raw_continuation_line_starts
    }

    /// Return byte offsets for the backslashes that introduce raw line continuations.
    ///
    /// These are unfiltered lexical candidates paired with
    /// [`Self::raw_continuation_line_starts`] and are only retained when raw
    /// continuation storage is requested. The offset points at the backslash
    /// immediately before the physical line ending, including CRLF input where
    /// the carriage return sits between the backslash and newline.
    pub fn raw_continuation_backslashes(&self) -> &[TextSize] {
        &self.raw_continuation_backslashes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_empty_source_as_one_line() {
        let index = LineIndex::new("");

        assert_eq!(index.line_count(), 1);
        assert_eq!(index.line_number(TextSize::new(0)), 1);
        assert_eq!(index.line_start(1), Some(TextSize::new(0)));
    }

    #[test]
    fn tracks_multiple_lines_and_ranges() {
        let source = "one\ntwo\nthree";
        let index = LineIndex::new(source);

        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_number(TextSize::new(0)), 1);
        assert_eq!(index.line_number(TextSize::new(4)), 2);
        assert_eq!(index.line_number(TextSize::new(source.len() as u32)), 3);
        assert_eq!(index.line_range(2, source).unwrap().slice(source), "two");
    }

    #[test]
    fn tracks_raw_continuation_candidates_while_collecting_lines() {
        let source = "echo foo \\\n  bar\necho \"foo\\\nbar\"\n";
        let index = LineIndex::with_raw_continuations(source);

        assert_eq!(
            index.raw_continuation_line_starts(),
            &[TextSize::new(11), TextSize::new(28)]
        );
    }

    #[test]
    fn skips_raw_continuation_storage_by_default() {
        let index = LineIndex::new("echo foo \\\n  bar\n");

        assert!(index.raw_continuation_line_starts().is_empty());
        assert!(index.raw_continuation_backslashes().is_empty());
    }

    #[test]
    fn detects_crlf_line_endings_during_line_indexing() {
        let index = LineIndex::with_raw_continuations("one \\\r\ntwo\n");

        assert_eq!(index.line_ending(), LineEndingStyle::CrLf);
        assert_eq!(index.raw_continuation_line_starts(), &[TextSize::new(7)]);
        assert_eq!(index.raw_continuation_backslashes(), &[TextSize::new(4)]);
    }

    #[test]
    fn handles_unicode_offsets_without_character_reindexing() {
        let source = "caf\u{00E9}\nnext";
        let index = LineIndex::new(source);
        let accent_offset = source.find('\u{00E9}').unwrap() as u32;

        assert_eq!(index.line_number(TextSize::new(accent_offset)), 1);
        assert_eq!(
            index.line_range(1, source).unwrap().slice(source),
            "caf\u{00E9}"
        );
    }

    #[test]
    fn resolves_line_and_column_to_offsets() {
        let source = "alpha\nb\u{e9}ta\n";
        let index = LineIndex::new(source);

        assert_eq!(
            index.offset_for_line_column(1, 1, source),
            Some(TextSize::new(0))
        );
        assert_eq!(
            index.offset_for_line_column(2, 1, source),
            Some(TextSize::new("alpha\n".len() as u32))
        );
        assert_eq!(
            index.offset_for_line_column(2, 3, source),
            Some(TextSize::new("alpha\nb\u{e9}".len() as u32))
        );
        assert_eq!(
            index.offset_for_line_column(2, 5, source),
            Some(TextSize::new("alpha\nb\u{e9}ta".len() as u32))
        );
        assert_eq!(
            index.offset_for_line_column(3, 1, source),
            Some(TextSize::new(source.len() as u32))
        );
    }
}
