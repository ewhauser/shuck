#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! Positional and structural indexes over parsed shell scripts.
//!
//! The indexer complements `shuck-parser` by building compact lookup tables for
//! source lines, comments, syntactic regions, heredoc bodies, and physical line
//! continuations. It is intended to be built once from parser output and then
//! shared by semantic analysis, lint rules, suppressions, formatters, and report
//! rendering.
//!
//! All positions are byte offsets represented with `shuck_ast::TextSize` and
//! `shuck_ast::TextRange`. The crate does not build a character index: callers
//! that need display columns should combine these byte offsets with the original
//! source text at the UI boundary.
//!
//! [`Indexer`] is the preferred construction path when parser output is
//! available. The lower-level indexes are also exported for integrations that
//! only need line mapping or that already have an AST-shaped source of comments
//! or regions.
mod comment_index;
#[allow(missing_docs)]
mod line_index;
#[allow(missing_docs)]
mod region_index;

/// Comment lookup types derived from parser output.
pub use comment_index::{CommentIndex, IndexedComment};
/// Line-based offset lookup utilities.
pub use line_index::LineIndex;
/// Structural region indexes over parsed shell source.
pub use region_index::{RegionIndex, RegionKind};

use shuck_ast::TextSize;
use shuck_parser::parser::ParseResult;

/// Pre-computed positional and structural index over a parsed shell script.
///
/// `Indexer` owns the line, comment, and syntactic-region indexes for one source
/// file. It also filters raw backslash-newline candidates into the continuation
/// lines that matter to shell analysis: continuations in comments, quoted text,
/// and heredoc bodies are excluded.
///
/// Build one `Indexer` for a parse result and pass references to downstream
/// analysis code. Query methods borrow precomputed data and do not walk the AST,
/// rescan the full source, or allocate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexer {
    line_index: LineIndex,
    comment_index: CommentIndex,
    region_index: RegionIndex,
    continuation_lines: Vec<TextSize>,
}

impl Indexer {
    /// Build all indexes from parser output and the original source text.
    ///
    /// `source` must be the exact text used to produce `output`; ranges in the
    /// parse result are interpreted as byte offsets into that string. Mismatched
    /// source text can make line and region queries meaningless, even though the
    /// constructor defensively avoids panicking on malformed comment ranges.
    pub fn new(source: &str, output: &ParseResult) -> Self {
        let line_index = LineIndex::new(source);
        let comment_index = CommentIndex::new(source, &line_index, &output.file);
        let region_index = RegionIndex::new(source, &output.file);
        let continuation_lines =
            collect_continuation_lines(&line_index, &comment_index, &region_index);

        Self {
            line_index,
            comment_index,
            region_index,
            continuation_lines,
        }
    }

    /// Return the line index for this source text.
    ///
    /// This is useful for converting diagnostic byte offsets to 1-based line
    /// numbers or for extracting line-local snippets from the original source.
    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// Return the comment index extracted from parser-owned comments.
    ///
    /// Comments are exposed in source order and include parser-recognized
    /// comments inside nested shell constructs.
    pub fn comment_index(&self) -> &CommentIndex {
        &self.comment_index
    }

    /// Return the syntactic region index for quoted, heredoc, and related spans.
    ///
    /// Region lookups are intended for rules and formatters that need to avoid
    /// interpreting bytes the same way in every syntactic context.
    pub fn region_index(&self) -> &RegionIndex {
        &self.region_index
    }

    /// Return byte offsets for the start of each semantic continuation line.
    ///
    /// Each offset points at the first byte of a physical line that continues
    /// the previous one because that previous line ended with an active
    /// backslash-newline. Continuations inside comments, quotes, and heredocs
    /// are filtered out.
    pub fn continuation_line_starts(&self) -> &[TextSize] {
        &self.continuation_lines
    }

    /// Return whether `offset` is on a semantic continuation line.
    ///
    /// The query first maps `offset` to its containing 1-based line, then checks
    /// whether that line starts at one of [`Self::continuation_line_starts`].
    /// Offsets past the final byte of the source are treated according to the
    /// last indexed line.
    pub fn is_continuation(&self, offset: TextSize) -> bool {
        let line = self.line_index.line_number(offset);
        let Some(line_start) = self.line_index.line_start(line) else {
            return false;
        };

        contains_offset(&self.continuation_lines, line_start)
    }
}

fn collect_continuation_lines(
    line_index: &LineIndex,
    comment_index: &CommentIndex,
    region_index: &RegionIndex,
) -> Vec<TextSize> {
    let mut continuation_lines = Vec::new();

    for line_start in line_index.raw_continuation_line_starts() {
        let backslash_offset = TextSize::new(line_start.to_u32() - 2);
        if comment_index.is_comment(backslash_offset)
            || region_index.is_heredoc(backslash_offset)
            || region_index.is_quoted(backslash_offset)
        {
            continue;
        }

        continuation_lines.push(*line_start);
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
