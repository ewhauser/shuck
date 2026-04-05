//! Source location tracking for error messages and $LINENO
//!
//! Provides position and span types for tracking source locations through
//! lexing, parsing, and execution.

/// A position in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// 1-based line number
    pub line: usize,
    /// 1-based column number (byte offset within line)
    pub column: usize,
    /// 0-based byte offset from start of input
    pub offset: usize,
}

impl Position {
    /// Create a new position at line 1, column 1, offset 0.
    pub fn new() -> Self {
        Self {
            line: 1,
            column: 1,
            offset: 0,
        }
    }

    /// Advance position by one character.
    pub fn advance(&mut self, ch: char) {
        self.offset += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    /// Return a new position advanced by every character in `text`.
    pub fn advanced_by(mut self, text: &str) -> Self {
        for ch in text.chars() {
            self.advance(ch);
        }
        self
    }

    /// Rebase a position from a nested source onto an absolute base position.
    pub fn rebased(self, base: Position) -> Self {
        Self {
            line: base.line + self.line.saturating_sub(1),
            column: if self.line <= 1 {
                base.column + self.column.saturating_sub(1)
            } else {
                self.column
            },
            offset: base.offset + self.offset,
        }
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A span of source code (start to end position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// Start position (inclusive)
    pub start: Position,
    /// End position (exclusive)
    pub end: Position,
}

impl Span {
    /// Create an empty span at the default position.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a span from start to end positions.
    pub fn from_positions(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    /// Create a span covering a single position.
    pub fn at(pos: Position) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Merge two spans into one covering both.
    pub fn merge(self, other: Span) -> Self {
        let start = if self.start.offset <= other.start.offset {
            self.start
        } else {
            other.start
        };
        let end = if self.end.offset >= other.end.offset {
            self.end
        } else {
            other.end
        };
        Self { start, end }
    }

    /// Rebase a span from a nested source onto an absolute base position.
    pub fn rebased(self, base: Position) -> Self {
        Self {
            start: self.start.rebased(base),
            end: self.end.rebased(base),
        }
    }

    /// Slice the source text covered by this span.
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        slice_with_byte_offsets(source, self.start.offset, self.end.offset)
    }

    /// Convert this span to a [`TextRange`] using only the byte offsets.
    pub fn to_range(self) -> TextRange {
        TextRange::new(
            TextSize::new(self.start.offset as u32),
            TextSize::new(self.end.offset as u32),
        )
    }

    /// Get the starting line number.
    pub fn line(&self) -> usize {
        self.start.line
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start.line == self.end.line {
            write!(f, "line {}", self.start.line)
        } else {
            write!(f, "lines {}-{}", self.start.line, self.end.line)
        }
    }
}

/// A byte offset in source text, analogous to ruff's `TextSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextSize(u32);

impl TextSize {
    /// Create a new `TextSize` from a raw `u32` byte offset.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the raw `u32` value.
    pub const fn to_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for TextSize {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<TextSize> for usize {
    fn from(size: TextSize) -> Self {
        size.0 as usize
    }
}

impl std::ops::Add for TextSize {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for TextSize {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// A half-open byte range in source text, analogous to ruff's `TextRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    /// Create a new range from start (inclusive) to end (exclusive).
    pub const fn new(start: TextSize, end: TextSize) -> Self {
        Self { start, end }
    }

    /// Start offset (inclusive).
    pub const fn start(self) -> TextSize {
        self.start
    }

    /// End offset (exclusive).
    pub const fn end(self) -> TextSize {
        self.end
    }

    /// Length in bytes.
    pub const fn len(self) -> TextSize {
        TextSize(self.end.0 - self.start.0)
    }

    /// Whether the range is empty.
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    /// Slice the source text covered by this range.
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        slice_with_byte_offsets(source, usize::from(self.start), usize::from(self.end))
    }

    /// Shift the range by adding a base offset to both start and end.
    pub fn offset_by(self, base: TextSize) -> Self {
        Self {
            start: self.start + base,
            end: self.end + base,
        }
    }
}

fn slice_with_byte_offsets(source: &str, start: usize, end: usize) -> &str {
    if start > end || end > source.len() {
        return "";
    }

    if let Some(slice) = source.get(start..end) {
        return slice;
    }

    let start = floor_char_boundary(source, start);
    let end = ceil_char_boundary(source, end);
    source.get(start..end).unwrap_or("")
}

fn floor_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset < source.len() && !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_advance() {
        let mut pos = Position::new();
        assert_eq!(pos.line, 1);
        assert_eq!(pos.column, 1);
        assert_eq!(pos.offset, 0);

        pos.advance('a');
        assert_eq!(pos.line, 1);
        assert_eq!(pos.column, 2);
        assert_eq!(pos.offset, 1);

        pos.advance('\n');
        assert_eq!(pos.line, 2);
        assert_eq!(pos.column, 1);
        assert_eq!(pos.offset, 2);

        pos.advance('b');
        assert_eq!(pos.line, 2);
        assert_eq!(pos.column, 2);
        assert_eq!(pos.offset, 3);
    }

    #[test]
    fn test_position_display() {
        let pos = Position {
            line: 5,
            column: 10,
            offset: 50,
        };
        assert_eq!(format!("{}", pos), "5:10");
    }

    #[test]
    fn test_span_merge() {
        let span1 = Span {
            start: Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            end: Position {
                line: 1,
                column: 5,
                offset: 4,
            },
        };
        let span2 = Span {
            start: Position {
                line: 1,
                column: 10,
                offset: 9,
            },
            end: Position {
                line: 2,
                column: 3,
                offset: 15,
            },
        };
        let merged = span1.merge(span2);
        assert_eq!(merged.start.offset, 0);
        assert_eq!(merged.end.offset, 15);
    }

    #[test]
    fn test_span_display() {
        let single_line = Span {
            start: Position {
                line: 3,
                column: 1,
                offset: 0,
            },
            end: Position {
                line: 3,
                column: 10,
                offset: 9,
            },
        };
        assert_eq!(format!("{}", single_line), "line 3");

        let multi_line = Span {
            start: Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            end: Position {
                line: 5,
                column: 1,
                offset: 50,
            },
        };
        assert_eq!(format!("{}", multi_line), "lines 1-5");
    }

    #[test]
    fn span_slice_handles_non_char_boundaries() {
        let source = "a─b";
        let span = Span::from_positions(
            Position {
                line: 1,
                column: 2,
                offset: 1,
            },
            Position {
                line: 1,
                column: 3,
                offset: 3,
            },
        );

        assert_eq!(span.slice(source), "─");
    }

    #[test]
    fn text_range_slice_handles_non_char_boundaries() {
        let source = "x🔉y";
        let range = TextRange::new(TextSize::new(1), TextSize::new(4));

        assert_eq!(range.slice(source), "🔉");
    }
}
