mod comment_index;
mod line_index;
mod region_index;

pub use comment_index::{CommentIndex, IndexedComment};
pub use line_index::LineIndex;
pub use region_index::{RegionIndex, RegionKind};

use shuck_ast::TextSize;
use shuck_parser::parser::ParseOutput;

/// Pre-computed positional and structural index over a parsed shell script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexer {
    line_index: LineIndex,
    comment_index: CommentIndex,
    region_index: RegionIndex,
    continuation_lines: Vec<TextSize>,
}

impl Indexer {
    /// Build an index from parser output and the original source text.
    pub fn new(source: &str, output: &ParseOutput) -> Self {
        let line_index = LineIndex::new(source);
        let comment_index = CommentIndex::new(source, &line_index, &output.file);
        let region_index = RegionIndex::new(source, &output.file);
        let continuation_lines = collect_continuation_lines(source, &comment_index, &region_index);

        Self {
            line_index,
            comment_index,
            region_index,
            continuation_lines,
        }
    }

    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    pub fn comment_index(&self) -> &CommentIndex {
        &self.comment_index
    }

    pub fn region_index(&self) -> &RegionIndex {
        &self.region_index
    }

    /// Byte offsets of the start of each continuation line.
    pub fn continuation_line_starts(&self) -> &[TextSize] {
        &self.continuation_lines
    }

    /// Whether the given byte offset is on a continuation line.
    pub fn is_continuation(&self, offset: TextSize) -> bool {
        let line = self.line_index.line_number(offset);
        let Some(line_start) = self.line_index.line_start(line) else {
            return false;
        };

        contains_offset(&self.continuation_lines, line_start)
    }
}

fn collect_continuation_lines(
    source: &str,
    comment_index: &CommentIndex,
    region_index: &RegionIndex,
) -> Vec<TextSize> {
    let bytes = source.as_bytes();
    let mut continuation_lines = Vec::new();

    for index in 1..bytes.len() {
        if bytes[index] != b'\n' || bytes[index - 1] != b'\\' {
            continue;
        }

        let backslash_offset = TextSize::new((index - 1) as u32);
        if comment_index.is_comment(backslash_offset)
            || region_index.is_heredoc(backslash_offset)
            || region_index.is_quoted(backslash_offset)
        {
            continue;
        }

        continuation_lines.push(TextSize::new((index + 1) as u32));
    }

    continuation_lines
}

fn contains_offset(offsets: &[TextSize], offset: TextSize) -> bool {
    offsets.binary_search(&offset).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn index(source: &str) -> Indexer {
        let output = Parser::new(source).parse().unwrap();
        Indexer::new(source, &output)
    }

    #[test]
    fn detects_continuation_lines_without_allocating_source_copies() {
        let source = "echo foo \\\n  bar\necho \"foo\\\nbar\"\n";
        let indexer = index(source);

        assert_eq!(indexer.continuation_line_starts().len(), 1);
        assert!(indexer.is_continuation(TextSize::new(11)));
        assert!(!indexer.is_continuation(TextSize::new(28)));
    }

    #[test]
    fn round_trips_parser_output_into_regions_comments_and_lines() {
        let source = "\
#!/bin/bash
echo \"$(printf '%s' \"$name\")\" # inline
cat <<'EOF'
literal $body
EOF
";
        let indexer = index(source);

        assert_eq!(indexer.line_index().line_count(), 6);
        assert_eq!(indexer.comment_index().comments().len(), 2);

        let heredoc_offset = TextSize::new(source.find("literal $body").unwrap() as u32);
        assert_eq!(
            indexer.region_index().region_at(heredoc_offset),
            Some(RegionKind::Heredoc)
        );
        assert!(indexer.region_index().is_quoted(heredoc_offset));

        let name_offset = TextSize::new(source.find("$name").unwrap() as u32);
        assert_eq!(
            indexer.region_index().region_at(name_offset),
            Some(RegionKind::DoubleQuoted)
        );
    }
}
