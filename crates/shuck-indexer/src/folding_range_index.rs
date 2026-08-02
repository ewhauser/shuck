use std::collections::BTreeSet;

use shuck_ast::{TextRange, TextSize};

use crate::{CommentIndex, LineIndex, RegionIndex};

/// Classification for a foldable shell source range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FoldingRangeKind {
    /// A syntax-backed shell construct, heredoc, or continuation region.
    Region,
    /// A consecutive block of own-line comments.
    Comment,
}

/// One validated, multiline folding range in the original source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedFoldingRange {
    range: TextRange,
    kind: FoldingRangeKind,
}

impl IndexedFoldingRange {
    /// Return the half-open byte range covered by this fold.
    pub fn range(self) -> TextRange {
        self.range
    }

    /// Return the classification for this fold.
    pub fn kind(self) -> FoldingRangeKind {
        self.kind
    }
}

/// Stable folding ranges derived from parser and line-layout indexes.
///
/// Ranges are sorted in source order, span at least two physical lines, and
/// never cross. Exact duplicates and line-equivalent nested ranges are removed
/// deterministically so protocol clients receive a stable result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldingRangeIndex {
    ranges: Vec<IndexedFoldingRange>,
}

impl FoldingRangeIndex {
    pub(crate) fn new(
        source: &str,
        line_index: &LineIndex,
        comment_index: &CommentIndex,
        region_index: &RegionIndex,
        continuation_lines: &[TextSize],
    ) -> Self {
        let mut candidates = region_index
            .structural_folding_ranges()
            .iter()
            .copied()
            .map(|range| IndexedFoldingRange {
                range,
                kind: FoldingRangeKind::Region,
            })
            .collect::<Vec<_>>();
        collect_comment_ranges(comment_index, &mut candidates);
        collect_continuation_ranges(source, line_index, continuation_lines, &mut candidates);

        let mut ranges = candidates
            .into_iter()
            .filter_map(|candidate| normalize(source, line_index, candidate))
            .collect::<Vec<_>>();
        ranges.sort_unstable_by_key(|candidate| {
            (
                candidate.range.start(),
                std::cmp::Reverse(candidate.range.end()),
                candidate.kind,
            )
        });

        let mut accepted: Vec<IndexedFoldingRange> = Vec::with_capacity(ranges.len());
        let mut line_ranges = BTreeSet::new();
        let mut nesting_ends = Vec::new();
        for candidate in ranges {
            let candidate_lines = physical_lines(line_index, candidate.range);
            if line_ranges.contains(&(candidate_lines, candidate.kind)) {
                continue;
            }
            while nesting_ends
                .last()
                .is_some_and(|end| *end <= candidate.range.start())
            {
                nesting_ends.pop();
            }
            if nesting_ends
                .last()
                .is_some_and(|end| *end < candidate.range.end())
            {
                continue;
            }
            line_ranges.insert((candidate_lines, candidate.kind));
            nesting_ends.push(candidate.range.end());
            accepted.push(candidate);
        }

        Self { ranges: accepted }
    }

    /// Return all validated folding ranges in deterministic source order.
    pub fn ranges(&self) -> &[IndexedFoldingRange] {
        &self.ranges
    }
}

fn collect_comment_ranges(comment_index: &CommentIndex, candidates: &mut Vec<IndexedFoldingRange>) {
    let mut start = None;
    let mut end = None;
    let mut last_line = 0usize;

    let flush = |start: &mut Option<TextSize>,
                 end: &mut Option<TextSize>,
                 candidates: &mut Vec<IndexedFoldingRange>| {
        if let (Some(start), Some(end)) = (start.take(), end.take()) {
            candidates.push(IndexedFoldingRange {
                range: TextRange::new(start, end),
                kind: FoldingRangeKind::Comment,
            });
        }
    };

    for comment in comment_index
        .comments()
        .iter()
        .filter(|comment| comment.is_own_line)
    {
        if start.is_some() && comment.line != last_line + 1 {
            flush(&mut start, &mut end, candidates);
        }
        if start.is_none() {
            start = Some(comment.range.start());
        }
        end = Some(comment.range.end());
        last_line = comment.line;
    }
    flush(&mut start, &mut end, candidates);
}

fn collect_continuation_ranges(
    source: &str,
    line_index: &LineIndex,
    continuation_lines: &[TextSize],
    candidates: &mut Vec<IndexedFoldingRange>,
) {
    let mut index = 0usize;
    while index < continuation_lines.len() {
        let first_line = line_index.line_number(continuation_lines[index]);
        let mut last_line = first_line;
        index += 1;
        while index < continuation_lines.len() {
            let line = line_index.line_number(continuation_lines[index]);
            if line != last_line + 1 {
                break;
            }
            last_line = line;
            index += 1;
        }

        let Some(start_line) = first_line.checked_sub(1) else {
            continue;
        };
        let (Some(start), Some(end_range)) = (
            line_index.line_start(start_line),
            line_index.line_range(last_line, source),
        ) else {
            continue;
        };
        candidates.push(IndexedFoldingRange {
            range: TextRange::new(start, end_range.end()),
            kind: FoldingRangeKind::Region,
        });
    }
}

fn normalize(
    source: &str,
    line_index: &LineIndex,
    candidate: IndexedFoldingRange,
) -> Option<IndexedFoldingRange> {
    let start = usize::from(candidate.range.start()).min(source.len());
    let end = usize::from(candidate.range.end()).min(source.len());
    let text = source.get(start..end)?;
    let trimmed_end = start + text.trim_end_matches(char::is_whitespace).len();
    if trimmed_end <= start {
        return None;
    }

    let range = TextRange::new(
        TextSize::new(start as u32),
        TextSize::new(trimmed_end as u32),
    );
    let (start_line, end_line) = physical_lines(line_index, range);
    (start_line < end_line).then_some(IndexedFoldingRange {
        range,
        kind: candidate.kind,
    })
}

fn physical_lines(line_index: &LineIndex, range: TextRange) -> (usize, usize) {
    let end_probe = range.end().to_u32().saturating_sub(1);
    (
        line_index.line_number(range.start()),
        line_index.line_number(TextSize::new(end_probe)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Indexer, IndexerOptions};
    use shuck_parser::{ShellDialect, parser::Parser};

    fn ranges(source: &str) -> Vec<(usize, usize, FoldingRangeKind)> {
        let parsed = Parser::with_dialect(source, ShellDialect::Bash).parse();
        let indexer = Indexer::new_with_options(
            source,
            &parsed,
            IndexerOptions::new().with_folding_ranges(true),
        );
        indexer
            .folding_range_index()
            .ranges()
            .iter()
            .map(|range| {
                let (start, end) = physical_lines(indexer.line_index(), range.range());
                (start, end, range.kind())
            })
            .collect()
    }

    #[test]
    fn indexes_nested_constructs_heredocs_comments_and_continuations() {
        let source = "# first\n# second\nfoo() {\n  if true; then\n    cat <<EOF\nbody\nEOF\n  fi\n}\nprintf '%s' \\\n  one \\\n  two\n";
        let indexed = ranges(source);

        assert!(indexed.contains(&(1, 2, FoldingRangeKind::Comment)));
        assert!(indexed.contains(&(3, 8, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(4, 7, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(5, 6, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(10, 12, FoldingRangeKind::Region)));
        assert!(indexed.iter().all(|(start, end, _)| start < end));
    }

    #[test]
    fn indexes_loops_case_subshells_and_brace_groups() {
        let source = "for x in a; do\n  while true; do\n    case $x in\n      a)\n        (\n          {\n            echo ok\n          }\n        )\n        ;;\n    esac\n  done\ndone\n";
        let indexed = ranges(source);

        assert!(indexed.contains(&(1, 12, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(2, 11, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(3, 10, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(5, 8, FoldingRangeKind::Region)));
        assert!(indexed.contains(&(6, 7, FoldingRangeKind::Region)));
    }

    #[test]
    fn keeps_known_ranges_for_incomplete_syntax_and_crlf() {
        let crlf = "printf '%s' \\\r\n  value\r\n";
        let indexed = ranges(crlf);

        assert!(
            indexed.contains(&(1, 2, FoldingRangeKind::Region)),
            "indexed ranges: {indexed:?}"
        );
        assert!(indexed.iter().all(|(start, end, _)| start < end));

        let incomplete = ranges("if true; then\n  echo incomplete\n");
        assert!(incomplete.iter().all(|(start, end, _)| start < end));

        for source in [
            "while true; do\n  echo incomplete\n",
            "until false; do\n  echo incomplete\n",
            "for ((i = 0; i < 3; i++)); do\n  echo incomplete\n",
        ] {
            assert!(
                ranges(source).is_empty(),
                "missing done must not produce a folding range for {source:?}"
            );
        }
    }

    #[test]
    fn removes_line_equivalent_duplicates_and_crossing_ranges() {
        let source = "foo() {\n  (\n    echo nested\n  )\n}\n";
        let indexed = ranges(source);

        let mut unique = indexed.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(indexed.len(), unique.len());
    }
}
