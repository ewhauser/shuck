use std::cmp::Reverse;

use shuck_ast::{TextRange, TextSize};

use crate::{CommentIndex, RegionIndex};

/// Precomputed syntax spans used to expand editor selections.
///
/// The index stores parser-owned ranges in source order. Queries return one
/// strict containment chain from the smallest syntax range at an offset to the
/// full file range, with equal spans removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionRangeIndex {
    ranges: Vec<TextRange>,
    file_range: TextRange,
}

impl SelectionRangeIndex {
    pub(crate) fn new(
        source: &str,
        comment_index: &CommentIndex,
        region_index: &RegionIndex,
    ) -> Self {
        let file_range = TextRange::new(TextSize::new(0), TextSize::new(source.len() as u32));
        let mut ranges = region_index.selection_ranges().to_vec();
        ranges.extend(comment_index.comments().iter().map(|comment| comment.range));
        ranges.push(file_range);
        ranges.retain(|range| valid_range(source, *range));
        ranges.sort_unstable_by_key(|range| (range.start(), range.end()));
        ranges.dedup();

        Self { ranges, file_range }
    }

    /// Return a strict inner-to-outer syntax containment chain for `offset`.
    ///
    /// Offsets past the end of the source are clamped to the file boundary.
    /// Whitespace, malformed syntax, and EOF always retain the file range even
    /// when no smaller parser-owned node contains the requested position.
    pub fn selection_chain(&self, offset: TextSize) -> Vec<TextRange> {
        let offset = TextSize::new(offset.to_u32().min(self.file_range.end().to_u32()));
        let end = self.ranges.partition_point(|range| range.start() <= offset);
        let mut candidates = self.ranges[..end]
            .iter()
            .copied()
            .filter(|range| contains_offset(*range, offset) || *range == self.file_range)
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|range| {
            (range.len().to_u32(), Reverse(range.start()), range.end())
        });
        candidates.dedup();

        let mut chain = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let Some(inner) = chain.last().copied() else {
                chain.push(candidate);
                continue;
            };
            if strictly_contains(candidate, inner) {
                chain.push(candidate);
            }
        }

        if chain.last().copied() != Some(self.file_range) {
            chain.push(self.file_range);
        }
        chain
    }
}

fn valid_range(source: &str, range: TextRange) -> bool {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    start <= end
        && end <= source.len()
        && source.is_char_boundary(start)
        && source.is_char_boundary(end)
}

fn contains_offset(range: TextRange, offset: TextSize) -> bool {
    range.start() <= offset && offset < range.end()
}

fn strictly_contains(outer: TextRange, inner: TextRange) -> bool {
    outer != inner && outer.start() <= inner.start() && inner.end() <= outer.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Indexer, IndexerOptions};
    use shuck_parser::{ShellDialect, parser::Parser};

    fn index(source: &str) -> Indexer {
        let parsed = Parser::with_dialect(source, ShellDialect::Bash).parse();
        Indexer::new_with_options(
            source,
            &parsed,
            IndexerOptions::new().with_selection_ranges(true),
        )
    }

    fn assert_strict_chain(chain: &[TextRange]) {
        assert!(!chain.is_empty());
        for pair in chain.windows(2) {
            assert!(
                strictly_contains(pair[1], pair[0]),
                "selection chain is not strict: {chain:?}"
            );
        }
    }

    #[test]
    fn expands_nested_substitutions_through_commands_and_file() {
        let source = "foo() {\n  printf '%s\\n' \"$(echo \"${arr[$((i + 1))]}\")\"\n}\n";
        let indexer = index(source);
        let offset = TextSize::new(source.find("i + 1").unwrap() as u32);
        let chain = indexer.selection_range_index().selection_chain(offset);

        assert_strict_chain(&chain);
        assert!(
            chain.len() >= 6,
            "selection chain was too shallow: {chain:?}"
        );
        assert_eq!(
            chain.last().copied(),
            Some(TextRange::new(
                TextSize::new(0),
                TextSize::new(source.len() as u32)
            ))
        );
    }

    #[test]
    fn handles_comments_whitespace_heredocs_and_malformed_input() {
        let source = "# comment\n\ncat <<EOF\nbody $value\nEOF\n";
        let indexer = index(source);

        let comment = indexer
            .selection_range_index()
            .selection_chain(TextSize::new(2));
        assert_strict_chain(&comment);
        assert_eq!(comment[0].start(), TextSize::new(0));

        let whitespace = indexer
            .selection_range_index()
            .selection_chain(TextSize::new("# comment\n".len() as u32));
        assert_strict_chain(&whitespace);

        let heredoc = indexer
            .selection_range_index()
            .selection_chain(TextSize::new(source.find("$value").unwrap() as u32));
        assert_strict_chain(&heredoc);
        assert!(
            heredoc.len() >= 3,
            "heredoc chain was too shallow: {heredoc:?}"
        );

        for malformed in ["if true; then\n  echo nope\n", "echo ok\r\n"] {
            let indexer = index(malformed);
            let chain = indexer
                .selection_range_index()
                .selection_chain(TextSize::new(malformed.len() as u32));
            assert_strict_chain(&chain);
            assert_eq!(chain.len(), 1);
        }
    }

    #[test]
    fn expands_conditional_and_redirect_expressions() {
        let source = "if [[ -n \"${value}\" ]]; then\n  printf '%s\\n' \"$value\" >\"$out\"\nfi\n";
        let indexer = index(source);

        for needle in ["value", "out"] {
            let offset = TextSize::new(source.rfind(needle).unwrap() as u32);
            let chain = indexer.selection_range_index().selection_chain(offset);
            assert_strict_chain(&chain);
            assert!(
                chain.len() >= 4,
                "selection chain for {needle:?} was too shallow: {chain:?}"
            );
        }
    }

    #[test]
    fn expands_standalone_and_loop_arithmetic_expressions() {
        for (source, needle) in [
            ("((total += arr[index + 1]))\n", "index"),
            (
                "for ((i = 0; i < limits[index]; i++)); do\n  :\ndone\n",
                "index",
            ),
        ] {
            let indexer = index(source);
            let offset = TextSize::new(source.find(needle).unwrap() as u32);
            let chain = indexer.selection_range_index().selection_chain(offset);
            assert_strict_chain(&chain);
            assert!(
                chain.len() >= 4,
                "arithmetic selection chain was too shallow: {chain:?}"
            );
        }
    }
}
