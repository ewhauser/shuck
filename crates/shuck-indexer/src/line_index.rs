use shuck_ast::{TextRange, TextSize};

/// Maps between byte offsets and source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<TextSize>,
}

impl LineIndex {
    /// Build from source text.
    pub fn new(source: &str) -> Self {
        let mut line_starts =
            Vec::with_capacity(source.bytes().filter(|byte| *byte == b'\n').count() + 1);
        line_starts.push(TextSize::new(0));

        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(TextSize::new((index + 1) as u32));
            }
        }

        Self { line_starts }
    }

    /// Return the 1-based line number containing `offset`.
    pub fn line_number(&self, offset: TextSize) -> usize {
        self.line_starts.partition_point(|start| *start <= offset)
    }

    /// Return the byte offset of the start of the given 1-based line.
    pub fn line_start(&self, line: usize) -> Option<TextSize> {
        line.checked_sub(1)
            .and_then(|index| self.line_starts.get(index).copied())
    }

    /// Return the byte range of the given 1-based line (excluding newline).
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

    /// Return the total number of lines.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
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
}
