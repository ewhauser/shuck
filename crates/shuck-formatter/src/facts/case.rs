use rustc_hash::FxHashMap as HashMap;

use shuck_ast::{CaseCommand, CaseItem, Span};

use crate::command::stmt_span;
use crate::comments::{BranchPrefixComment, SourceComment};

use super::FactSpan;

#[derive(Debug, Clone, Default)]
pub(super) struct CaseFacts<'source> {
    case_facts: HashMap<FactSpan, CaseCommandFacts>,
    case_item_facts: HashMap<FactSpan, CaseItemFacts<'source>>,
}

impl<'source> CaseFacts<'source> {
    pub(super) fn case_command(&self, command: &CaseCommand) -> Option<&CaseCommandFacts> {
        self.case_facts.get(&FactSpan::from(command.span))
    }

    pub(super) fn case_item(&self, item: &CaseItem) -> Option<&CaseItemFacts<'source>> {
        self.case_item_facts.get(&case_item_key(item))
    }

    pub(super) fn insert_case_command(&mut self, command: &CaseCommand, facts: CaseCommandFacts) {
        self.case_facts.insert(FactSpan::from(command.span), facts);
    }

    pub(super) fn insert_case_item(&mut self, item: &CaseItem, facts: CaseItemFacts<'source>) {
        self.case_item_facts.insert(case_item_key(item), facts);
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.case_facts.len() + self.case_item_facts.len()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaseCommandFacts {
    pub(super) esac_span: Option<Span>,
    pub(super) body_fallback_upper_bound: usize,
    pub(super) has_blank_line_after_in: bool,
    pub(super) has_blank_line_before_esac: bool,
    pub(super) suffix_comments_before_esac: Vec<BranchPrefixComment>,
}

impl CaseCommandFacts {
    pub(crate) fn esac_span(&self) -> Option<Span> {
        self.esac_span
    }

    pub(crate) fn body_fallback_upper_bound(&self) -> usize {
        self.body_fallback_upper_bound
    }

    pub(crate) fn has_blank_line_after_in(&self) -> bool {
        self.has_blank_line_after_in
    }

    pub(crate) fn has_blank_line_before_esac(&self) -> bool {
        self.has_blank_line_before_esac
    }

    pub(crate) fn suffix_comments_before_esac(&self) -> &[BranchPrefixComment] {
        &self.suffix_comments_before_esac
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaseItemFacts<'source> {
    pub(super) suffix_comment_start_line: Option<usize>,
    pub(super) has_blank_line_before: bool,
    pub(super) has_blank_line_after_pattern: bool,
    pub(super) has_blank_line_before_terminator: bool,
    pub(super) prefix_comments: Vec<SourceComment<'source>>,
    pub(super) pattern_suffix_comment: Option<SourceComment<'source>>,
    pub(super) terminator_suffix_comment: Option<SourceComment<'source>>,
}

impl<'source> CaseItemFacts<'source> {
    pub(crate) fn suffix_comment_start_line(&self) -> Option<usize> {
        self.suffix_comment_start_line
    }

    pub(crate) fn has_blank_line_before(&self) -> bool {
        self.has_blank_line_before
    }

    pub(crate) fn has_blank_line_after_pattern(&self) -> bool {
        self.has_blank_line_after_pattern
    }

    pub(crate) fn has_blank_line_before_terminator(&self) -> bool {
        self.has_blank_line_before_terminator
    }

    pub(crate) fn prefix_comments(&self) -> &[SourceComment<'source>] {
        &self.prefix_comments
    }

    pub(crate) fn pattern_suffix_comment(&self) -> Option<SourceComment<'source>> {
        self.pattern_suffix_comment
    }

    pub(crate) fn terminator_suffix_comment(&self) -> Option<SourceComment<'source>> {
        self.terminator_suffix_comment
    }
}

fn case_item_key(item: &CaseItem) -> FactSpan {
    let start = item
        .patterns
        .first()
        .map(|pattern| pattern.span.start.offset)
        .unwrap_or(item.body.span.start.offset);
    let end = item
        .terminator_span
        .map(|span| span.end.offset)
        .or_else(|| item.body.last().map(|stmt| stmt_span(stmt).end.offset))
        .or_else(|| item.patterns.last().map(|pattern| pattern.span.end.offset))
        .unwrap_or(item.body.span.end.offset);
    FactSpan::from_offsets(start, end)
}
