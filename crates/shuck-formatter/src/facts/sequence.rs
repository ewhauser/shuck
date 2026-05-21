use rustc_hash::FxHashMap as HashMap;

use shuck_ast::{Span, StmtSeq};

use crate::comments::{SequenceCommentAttachment, SourceComment};

use super::FactSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SequenceSiteKey {
    pub(super) span: FactSpan,
    upper_bound: Option<usize>,
}

impl SequenceSiteKey {
    pub(super) fn new(sequence: &StmtSeq, upper_bound: Option<usize>) -> Self {
        Self {
            span: FactSpan::from(sequence.span),
            upper_bound,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SequenceSite<'a> {
    pub(super) sequence: &'a StmtSeq,
    pub(super) upper_bound: Option<usize>,
    pub(super) group_open_char: Option<char>,
    pub(super) open_suffix_span: Option<Span>,
    pub(super) open_end_offset: Option<usize>,
}

impl<'a> SequenceSite<'a> {
    pub(super) fn new(
        sequence: &'a StmtSeq,
        upper_bound: Option<usize>,
        group_open_char: Option<char>,
        open_suffix_span: Option<Span>,
        open_end_offset: Option<usize>,
    ) -> Self {
        Self {
            sequence,
            upper_bound,
            group_open_char,
            open_suffix_span,
            open_end_offset,
        }
    }

    pub(super) fn key(self) -> SequenceSiteKey {
        SequenceSiteKey::new(self.sequence, self.upper_bound)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SequenceFactsStore<'source> {
    pub(super) by_site: HashMap<SequenceSiteKey, SequenceFacts<'source>>,
    pub(super) by_span: HashMap<FactSpan, SequenceSiteKey>,
}

impl<'source> SequenceFactsStore<'source> {
    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.by_site.len() + self.by_span.len()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SequenceFacts<'source> {
    pub(super) comments: SequenceCommentAttachment<'source>,
    pub(super) first_rendered_lines: Vec<usize>,
    pub(super) group_open_suffix_span: Option<Span>,
    pub(super) group_attachment_span: Option<Span>,
    pub(super) open_end_offset: Option<usize>,
    pub(super) has_blank_line_after_open: bool,
    pub(super) has_blank_line_before_close: bool,
    pub(super) body_content_end: usize,
    pub(super) close_gap_start: usize,
    pub(super) contains_comments: bool,
    pub(super) contains_heredoc: bool,
    pub(super) contains_multiline_literal_source: bool,
    pub(super) contains_multistatement_pipeline_brace_group: bool,
}

impl<'source> SequenceFacts<'source> {
    pub(super) fn new(child_count: usize) -> Self {
        Self {
            comments: SequenceCommentAttachment::new(child_count),
            first_rendered_lines: vec![0; child_count],
            group_open_suffix_span: None,
            group_attachment_span: None,
            open_end_offset: None,
            has_blank_line_after_open: false,
            has_blank_line_before_close: false,
            body_content_end: 0,
            close_gap_start: 0,
            contains_comments: false,
            contains_heredoc: false,
            contains_multiline_literal_source: false,
            contains_multistatement_pipeline_brace_group: false,
        }
    }

    pub(crate) fn leading_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.comments.leading_for(index)
    }

    pub(crate) fn trailing_for(&self, index: usize) -> &[SourceComment<'source>] {
        self.comments.trailing_for(index)
    }

    pub(crate) fn dangling(&self) -> &[SourceComment<'source>] {
        self.comments.dangling()
    }

    pub(crate) fn is_ambiguous(&self) -> bool {
        self.comments.is_ambiguous()
    }

    pub(crate) fn has_comments(&self) -> bool {
        self.comments.has_comments()
    }

    pub(crate) fn contains_comments(&self) -> bool {
        self.contains_comments
    }

    pub(crate) fn contains_heredoc(&self) -> bool {
        self.contains_heredoc
    }

    pub(crate) fn contains_multiline_literal_source(&self) -> bool {
        self.contains_multiline_literal_source
    }

    pub(crate) fn contains_multistatement_pipeline_brace_group(&self) -> bool {
        self.contains_multistatement_pipeline_brace_group
    }

    pub(crate) fn first_rendered_line_for(&self, index: usize) -> usize {
        self.first_rendered_lines
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn group_open_suffix_span(&self) -> Option<Span> {
        self.group_open_suffix_span
    }

    pub(crate) fn group_attachment_span(&self) -> Option<Span> {
        self.group_attachment_span
    }

    pub(crate) fn open_end_offset(&self) -> Option<usize> {
        self.open_end_offset
    }

    pub(crate) fn has_blank_line_after_open(&self) -> bool {
        self.has_blank_line_after_open
    }

    pub(crate) fn has_blank_line_before_close(&self) -> bool {
        self.has_blank_line_before_close
    }

    pub(crate) fn body_content_end(&self) -> usize {
        self.body_content_end
    }

    pub(crate) fn close_gap_start(&self) -> usize {
        self.close_gap_start
    }
}
