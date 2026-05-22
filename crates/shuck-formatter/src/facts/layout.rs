use rustc_hash::FxHashMap as HashMap;

use shuck_ast::Span;

use super::FactSpan;

#[derive(Debug, Clone, Default)]
pub(super) struct LayoutFacts {
    pub(super) statements: HashMap<FactSpan, StmtFacts>,
    pub(super) words: HashMap<FactSpan, WordFacts>,
}

impl LayoutFacts {
    #[cfg(feature = "benchmarking")]
    pub(super) fn len(&self) -> usize {
        self.statements.len() + self.words.len()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StmtFacts {
    pub(super) attachment_span: Span,
    pub(super) render_span: Span,
    pub(super) rendered_start_line: usize,
    pub(super) rendered_end_line: usize,
    pub(super) has_trailing_comment: bool,
    pub(super) preserve_verbatim: bool,
    pub(super) contains_heredoc: bool,
}

impl StmtFacts {
    pub(crate) fn attachment_span(&self) -> Span {
        self.attachment_span
    }

    pub(crate) fn render_span(&self) -> Span {
        self.render_span
    }

    pub(crate) fn rendered_start_line(&self) -> usize {
        self.rendered_start_line
    }

    pub(crate) fn rendered_end_line(&self) -> usize {
        self.rendered_end_line
    }

    pub(crate) fn has_trailing_comment(&self) -> bool {
        self.has_trailing_comment
    }

    pub(crate) fn preserve_verbatim(&self) -> bool {
        self.preserve_verbatim
    }

    pub(crate) fn contains_heredoc(&self) -> bool {
        self.contains_heredoc
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WordFacts {
    pub(super) has_multiline_literal_source: bool,
}

impl WordFacts {
    pub(crate) fn has_multiline_literal_source(&self) -> bool {
        self.has_multiline_literal_source
    }
}
