use super::super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn literal_text(
        &self,
        text: String,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, &text) {
            LiteralText::source()
        } else if source_backed {
            LiteralText::cooked_source(text)
        } else {
            LiteralText::owned(text)
        }
    }

    pub(in crate::parser) fn literal_text_from_str(
        &self,
        text: &str,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        self.literal_text_impl(text, None, start, end, source_backed)
    }

    pub(in crate::parser) fn literal_text_impl(
        &self,
        text: &str,
        owned: Option<String>,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, text) {
            LiteralText::source()
        } else if source_backed {
            LiteralText::cooked_source(owned.unwrap_or_else(|| text.to_owned()))
        } else {
            LiteralText::owned(owned.unwrap_or_else(|| text.to_owned()))
        }
    }

    pub(in crate::parser) fn source_text(
        &self,
        text: String,
        start: Position,
        end: Position,
    ) -> SourceText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, &text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text)
        }
    }

    pub(in crate::parser) fn source_text_from_str(
        &self,
        text: &str,
        start: Position,
        end: Position,
    ) -> SourceText {
        self.source_text_impl(text, None, start, end)
    }

    pub(in crate::parser) fn source_text_impl(
        &self,
        text: &str,
        owned: Option<String>,
        start: Position,
        end: Position,
    ) -> SourceText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, owned.unwrap_or_else(|| text.to_owned()))
        }
    }

    pub(in crate::parser) fn empty_source_text(&self, pos: Position) -> SourceText {
        SourceText::source(Span::from_positions(pos, pos))
    }

    pub(in crate::parser) fn input_prefix_ends_with(&self, end_offset: usize, ch: char) -> bool {
        self.input
            .get(..end_offset)
            .is_some_and(|prefix| prefix.ends_with(ch))
    }

    pub(in crate::parser) fn input_span_ends_with(
        &self,
        start: Position,
        end: Position,
        ch: char,
    ) -> bool {
        self.input
            .get(start.offset..end.offset)
            .is_some_and(|slice| slice.ends_with(ch))
    }

    pub(in crate::parser) fn input_suffix_starts_with(
        &self,
        start_offset: usize,
        ch: char,
    ) -> bool {
        self.input
            .get(start_offset..)
            .is_some_and(|suffix| suffix.starts_with(ch))
    }

    pub(in crate::parser) fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && self
                .input
                .get(span.start.offset..span.end.offset)
                .is_some_and(|slice| slice == text)
    }
}
