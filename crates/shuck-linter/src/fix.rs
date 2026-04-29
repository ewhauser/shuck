use std::cmp::Ordering;

use compact_str::CompactString;
use shuck_ast::{Span, TextRange, TextSize};

use crate::Diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Applicability {
    Safe,
    Unsafe,
}

impl Applicability {
    pub const fn includes(self, applicability: Self) -> bool {
        match self {
            Self::Safe => matches!(applicability, Self::Safe),
            Self::Unsafe => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixAvailability {
    None,
    Sometimes,
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Edit {
    range: TextRange,
    content: CompactString,
}

impl Edit {
    pub fn deletion(span: Span) -> Self {
        Self::deletion_at(span.start.offset, span.end.offset)
    }

    pub fn deletion_at(start: usize, end: usize) -> Self {
        Self::replacement_at(start, end, CompactString::default())
    }

    pub fn replacement(content: impl Into<CompactString>, span: Span) -> Self {
        Self::replacement_at(span.start.offset, span.end.offset, content)
    }

    pub fn replacement_at(start: usize, end: usize, content: impl Into<CompactString>) -> Self {
        let (start, end) = ordered_offsets(start, end);
        Self {
            range: TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32)),
            content: content.into(),
        }
    }

    pub fn insertion(offset: usize, content: impl Into<CompactString>) -> Self {
        Self::replacement_at(offset, offset, content)
    }

    pub const fn range(&self) -> TextRange {
        self.range
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    fn start_offset(&self) -> usize {
        usize::from(self.range.start())
    }

    fn end_offset(&self) -> usize {
        usize::from(self.range.end())
    }

    fn is_insertion(&self) -> bool {
        self.range.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    edits: Box<[Edit]>,
    applicability: Applicability,
}

impl Fix {
    pub fn new(applicability: Applicability, edits: impl IntoIterator<Item = Edit>) -> Self {
        let edits = edits.into_iter().collect::<Vec<_>>().into_boxed_slice();
        debug_assert!(!edits.is_empty(), "fixes should contain at least one edit");
        Self {
            edits,
            applicability,
        }
    }

    pub fn safe_edit(edit: Edit) -> Self {
        Self::safe_edits([edit])
    }

    pub fn safe_edits(edits: impl IntoIterator<Item = Edit>) -> Self {
        Self::new(Applicability::Safe, edits)
    }

    pub fn unsafe_edit(edit: Edit) -> Self {
        Self::unsafe_edits([edit])
    }

    pub fn unsafe_edits(edits: impl IntoIterator<Item = Edit>) -> Self {
        Self::new(Applicability::Unsafe, edits)
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub const fn applicability(&self) -> Applicability {
        self.applicability
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedFixes {
    pub code: String,
    pub fixes_applied: usize,
}

#[derive(Debug, Clone)]
struct CandidateFix {
    edits: Vec<Edit>,
}

pub fn apply_fixes(
    source: &str,
    diagnostics: &[Diagnostic],
    applicability: Applicability,
) -> AppliedFixes {
    let mut candidate_fixes = diagnostics
        .iter()
        .filter_map(|diagnostic| {
            let fix = diagnostic.fix.as_ref()?;
            applicability
                .includes(fix.applicability())
                .then(|| prepare_fix(fix))
        })
        .collect::<Vec<_>>();
    candidate_fixes.sort_by(compare_candidate_fixes);

    let mut applied_fixes = 0;
    let mut applied_edits = Vec::new();
    for candidate in candidate_fixes {
        if has_internal_conflicts(&candidate.edits) {
            continue;
        }
        if candidate.edits.iter().any(|edit| {
            applied_edits
                .iter()
                .any(|other| edits_conflict(edit, other))
        }) {
            continue;
        }
        applied_edits.extend(candidate.edits);
        applied_fixes += 1;
    }

    if applied_fixes == 0 {
        return AppliedFixes {
            code: source.to_owned(),
            fixes_applied: 0,
        };
    }

    applied_edits.sort_by(compare_edits);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for edit in applied_edits {
        let start = edit.start_offset();
        let end = edit.end_offset();
        debug_assert!(start <= end);
        debug_assert!(end <= source.len());
        debug_assert!(source.is_char_boundary(start));
        debug_assert!(source.is_char_boundary(end));

        output.push_str(&source[cursor..start]);
        output.push_str(edit.content());
        cursor = end;
    }
    output.push_str(&source[cursor..]);

    AppliedFixes {
        code: output,
        fixes_applied: applied_fixes,
    }
}

fn prepare_fix(fix: &Fix) -> CandidateFix {
    let mut edits = fix.edits().to_vec();
    edits.sort_by(compare_edits);
    CandidateFix { edits }
}

fn compare_candidate_fixes(left: &CandidateFix, right: &CandidateFix) -> Ordering {
    for (left_edit, right_edit) in left.edits.iter().zip(&right.edits) {
        let ordering = compare_edits(left_edit, right_edit);
        if !ordering.is_eq() {
            return ordering;
        }
    }

    left.edits.len().cmp(&right.edits.len())
}

fn compare_edits(left: &Edit, right: &Edit) -> Ordering {
    left.start_offset()
        .cmp(&right.start_offset())
        .then(left.end_offset().cmp(&right.end_offset()))
        .then(left.content().cmp(right.content()))
}

fn has_internal_conflicts(edits: &[Edit]) -> bool {
    edits
        .windows(2)
        .any(|window| edits_conflict(&window[0], &window[1]))
}

fn edits_conflict(left: &Edit, right: &Edit) -> bool {
    let left_start = left.start_offset();
    let left_end = left.end_offset();
    let right_start = right.start_offset();
    let right_end = right.end_offset();

    if left.is_insertion() && right.is_insertion() {
        return left_start == right_start;
    }

    if left.is_insertion() {
        return right_start <= left_start && left_start <= right_end;
    }

    if right.is_insertion() {
        return left_start <= right_start && right_start <= left_end;
    }

    left_start < right_end && right_start < left_end
}

fn ordered_offsets(start: usize, end: usize) -> (usize, usize) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Position, Span};

    use crate::{Diagnostic, Rule, Severity};

    use super::{Applicability, Edit, Fix, apply_fixes};

    fn diagnostic_with_fix(message: &str, fix: Fix) -> Diagnostic {
        Diagnostic {
            rule: Rule::AmpersandSemicolon,
            message: message.to_owned(),
            severity: Severity::Warning,
            span: span(0, 0),
            fix: Some(fix),
            fix_title: Some("apply test fix".to_owned()),
        }
    }

    fn span(start: usize, end: usize) -> Span {
        Span::from_positions(
            Position {
                line: 1,
                column: start + 1,
                offset: start,
            },
            Position {
                line: 1,
                column: end + 1,
                offset: end,
            },
        )
    }

    #[test]
    fn applies_one_safe_deletion_edit() {
        let source = "echo x &;\n";
        let diagnostics = vec![diagnostic_with_fix(
            "delete the semicolon",
            Fix::safe_edit(Edit::deletion(span(8, 9))),
        )];

        let fixed = apply_fixes(source, &diagnostics, Applicability::Safe);

        assert_eq!(fixed.code, "echo x &\n");
        assert_eq!(fixed.fixes_applied, 1);
    }

    #[test]
    fn applies_multiple_non_overlapping_edits() {
        let source = "a;b;c\n";
        let diagnostics = vec![
            diagnostic_with_fix("first", Fix::safe_edit(Edit::deletion(span(1, 2)))),
            diagnostic_with_fix("second", Fix::safe_edit(Edit::deletion(span(3, 4)))),
        ];

        let fixed = apply_fixes(source, &diagnostics, Applicability::Safe);

        assert_eq!(fixed.code, "abc\n");
        assert_eq!(fixed.fixes_applied, 2);
    }

    #[test]
    fn deconflicts_overlapping_edits_deterministically() {
        let source = "abcde\n";
        let diagnostics = vec![
            diagnostic_with_fix(
                "later wide edit",
                Fix::safe_edit(Edit::replacement_at(1, 4, "X")),
            ),
            diagnostic_with_fix(
                "earlier narrow edit",
                Fix::safe_edit(Edit::replacement_at(1, 2, "Y")),
            ),
        ];

        let fixed = apply_fixes(source, &diagnostics, Applicability::Safe);

        assert_eq!(fixed.code, "aYcde\n");
        assert_eq!(fixed.fixes_applied, 1);
    }
}
