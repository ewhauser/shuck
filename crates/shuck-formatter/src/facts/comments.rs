use rustc_hash::FxHashMap as HashMap;

use shuck_ast::Span;

use crate::comments::{BranchPrefixComment, CommentAttachmentModel, SourceComment, SourceMap};
use crate::streaming::comments_alignment::CommentAlignmentFacts;

use super::{FactSpan, gap_has_empty_physical_line, line_end_for_offset};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct OffsetRegionKey {
    start: usize,
    end: usize,
}

impl OffsetRegionKey {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BranchPrefixFacts {
    comments: Vec<BranchPrefixComment>,
    first_comment_offset: Option<usize>,
    has_blank_line_before_keyword: bool,
    has_blank_line_after_comments: bool,
}

impl BranchPrefixFacts {
    pub(super) fn new(
        source: &str,
        start: usize,
        end: usize,
        comments: Vec<BranchPrefixComment>,
    ) -> Self {
        let first_comment_offset = comments.first().map(|comment| comment.offset);
        let has_blank_line_before_keyword =
            gap_has_empty_physical_line(source, start, first_comment_offset.unwrap_or(end));
        let has_blank_line_after_comments = comments.last().is_some_and(|last| {
            line_end_for_offset(source, last.offset)
                .filter(|line_end| *line_end < end)
                .is_some_and(|line_end| gap_has_empty_physical_line(source, line_end, end))
        });

        Self {
            comments,
            first_comment_offset,
            has_blank_line_before_keyword,
            has_blank_line_after_comments,
        }
    }

    pub(crate) fn comments(&self) -> &[BranchPrefixComment] {
        &self.comments
    }

    pub(crate) fn first_comment_offset(&self) -> Option<usize> {
        self.first_comment_offset
    }

    pub(crate) fn has_blank_line_before_keyword(&self) -> bool {
        self.has_blank_line_before_keyword
    }

    pub(crate) fn has_blank_line_after_comments(&self) -> bool {
        self.has_blank_line_after_comments
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommentFacts<'source> {
    alignment: HashMap<FactSpan, CommentAlignmentFacts>,
    suffix_comments: HashMap<FactSpan, SourceComment<'source>>,
    close_suffix_comments: HashMap<FactSpan, SourceComment<'source>>,
    branch_prefix_facts: HashMap<OffsetRegionKey, BranchPrefixFacts>,
}

impl<'source> CommentFacts<'source> {
    pub(super) fn new(
        source: &'source str,
        source_map: &SourceMap<'source>,
        comment_attachments: &CommentAttachmentModel<'source>,
    ) -> Self {
        let alignment = comment_attachments
            .comments()
            .iter()
            .map(|comment| {
                (
                    FactSpan::from(comment.span()),
                    CommentAlignmentFacts::new(source, source_map, comment),
                )
            })
            .collect();

        Self {
            alignment,
            suffix_comments: HashMap::default(),
            close_suffix_comments: HashMap::default(),
            branch_prefix_facts: HashMap::default(),
        }
    }

    pub(super) fn trailing_comment(
        &self,
        source_map: &SourceMap<'source>,
        comment: SourceComment<'source>,
    ) -> InlineCommentPlan<'source> {
        InlineCommentPlan::new(
            comment,
            self.alignment_for(source_map, comment),
            InlineCommentPlacement::Trailing,
        )
    }

    pub(super) fn suffix_comment_for_span(
        &self,
        source_map: &SourceMap<'source>,
        span: Span,
    ) -> Option<InlineCommentPlan<'source>> {
        self.suffix_comments
            .get(&FactSpan::from(span))
            .copied()
            .map(|comment| self.trailing_comment(source_map, comment))
    }

    pub(super) fn close_suffix_comment_after_span(
        &self,
        source_map: &SourceMap<'source>,
        span: Span,
    ) -> Option<InlineCommentPlan<'source>> {
        let comment = self
            .close_suffix_comments
            .get(&FactSpan::from(span))
            .copied()
            .or_else(|| source_map.suffix_comment_after_span(span))?;
        Some(InlineCommentPlan::new(
            comment,
            self.alignment_for(source_map, comment),
            InlineCommentPlacement::CloseSuffix,
        ))
    }

    pub(super) fn branch_prefix_facts(
        &self,
        start: usize,
        end: usize,
    ) -> Option<&BranchPrefixFacts> {
        self.branch_prefix_facts
            .get(&OffsetRegionKey::new(start, end))
    }

    pub(super) fn insert_suffix_comment(&mut self, span: Span, comment: SourceComment<'source>) {
        self.suffix_comments.insert(FactSpan::from(span), comment);
    }

    pub(super) fn insert_close_suffix_comment(
        &mut self,
        span: Span,
        comment: SourceComment<'source>,
    ) {
        self.close_suffix_comments
            .insert(FactSpan::from(span), comment);
    }

    pub(super) fn insert_branch_prefix_facts(
        &mut self,
        start: usize,
        end: usize,
        facts: BranchPrefixFacts,
    ) {
        self.branch_prefix_facts
            .insert(OffsetRegionKey::new(start, end), facts);
    }

    fn alignment_for(
        &self,
        source_map: &SourceMap<'source>,
        comment: SourceComment<'source>,
    ) -> CommentAlignmentFacts {
        self.alignment
            .get(&FactSpan::from(comment.span()))
            .copied()
            .unwrap_or_else(|| {
                CommentAlignmentFacts::new(source_map.source(), source_map, &comment)
            })
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.alignment.len()
            + self.suffix_comments.len()
            + self.close_suffix_comments.len()
            + self.branch_prefix_facts.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineCommentPlan<'source> {
    comment: SourceComment<'source>,
    alignment: CommentAlignmentFacts,
    placement: InlineCommentPlacement,
}

impl<'source> InlineCommentPlan<'source> {
    fn new(
        comment: SourceComment<'source>,
        alignment: CommentAlignmentFacts,
        placement: InlineCommentPlacement,
    ) -> Self {
        Self {
            comment,
            alignment,
            placement,
        }
    }

    pub(crate) fn comment(self) -> SourceComment<'source> {
        self.comment
    }

    pub(crate) fn padding(
        self,
        source_map: &SourceMap<'_>,
        current_code_column: usize,
        current_indent_column: usize,
    ) -> usize {
        match self.placement {
            InlineCommentPlacement::Trailing => self.alignment.trailing_padding(
                source_map.source(),
                source_map,
                &self.comment,
                current_code_column,
                current_indent_column,
            ),
            InlineCommentPlacement::CloseSuffix => self.alignment.close_suffix_padding(
                source_map.source(),
                source_map,
                &self.comment,
                current_code_column,
                current_indent_column,
            ),
        }
    }

    pub(crate) fn has_alignment(
        self,
        source_map: &SourceMap<'_>,
        current_indent_column: usize,
    ) -> bool {
        self.alignment.has_trailing_alignment(
            source_map.source(),
            source_map,
            &self.comment,
            current_indent_column,
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum InlineCommentPlacement {
    Trailing,
    CloseSuffix,
}
