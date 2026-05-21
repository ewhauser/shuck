use super::super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn parse_source_text_as_word(&self, text: &SourceText) -> Word {
        if let Some(word) = self.simple_source_text_as_word(text) {
            return word;
        }

        let span = text.span();
        let nested_profile = self
            .zsh_options_at_offset(span.start.offset)
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        if remaining_depth == 0 {
            return self.word_with_single_part(
                self.literal_part_from_text(text.slice(self.input), span, text.is_source_backed()),
                span,
            );
        }
        let reparse_depth = (remaining_depth - 1).min(SOURCE_TEXT_WORD_REPARSE_MAX_DEPTH);

        if !text.is_source_backed()
            && span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
        {
            let raw = span.slice(self.input);
            if raw.contains("\\\"") {
                return Self::parse_word_fragment_with_limits(
                    self.input,
                    raw,
                    span,
                    reparse_depth,
                    self.fuel,
                    nested_profile.clone(),
                );
            }
        }

        Self::parse_word_fragment_with_limits(
            self.input,
            text.slice(self.input),
            text.span(),
            reparse_depth,
            self.fuel,
            nested_profile,
        )
    }

    pub(in crate::parser) fn simple_source_text_as_word(&self, text: &SourceText) -> Option<Word> {
        if !text.is_source_backed() {
            return None;
        }

        let span = text.span();
        let raw = text.slice(self.input);
        if raw.is_empty() {
            return Some(Word::literal_with_span("", span));
        }

        if let Some(word) = self.simple_quoted_source_text_as_word(raw, span) {
            return Some(word);
        }

        if Self::word_text_needs_parse(raw)
            || raw.contains(['\'', '"', '\\'])
            || self.zsh_glob_word_parsing_enabled_at(span.start.offset)
        {
            return None;
        }

        Some(self.word_with_single_part(self.literal_part_from_text(raw, span, true), span))
    }

    pub(in crate::parser) fn simple_quoted_source_text_as_word(
        &self,
        raw: &str,
        span: Span,
    ) -> Option<Word> {
        if raw.len() < 2 {
            return None;
        }

        let quote = raw.as_bytes()[0];
        if quote != b'\'' && quote != b'"' || raw.as_bytes().last().copied() != Some(quote) {
            return None;
        }

        let inner = &raw[1..raw.len() - 1];
        if quote == b'\'' && inner.contains('\'') {
            return None;
        }

        let inner_start = span.start.advanced_by(&raw[..1]);
        let inner_end = inner_start.advanced_by(inner);
        let inner_span = Span::from_positions(inner_start, inner_end);
        let part = match quote {
            b'\'' => self.single_quoted_part_from_text(inner, inner_span, span, false),
            b'"' => {
                if Self::word_text_needs_parse(inner) || inner.contains(['\\', '"']) {
                    return None;
                }
                self.double_quoted_literal_part_from_text(inner, inner_span, span, true, false)
            }
            _ => unreachable!("quote is checked above"),
        };
        Some(self.word_with_single_part(part, span))
    }

    pub(in crate::parser) fn parse_optional_source_text_as_word(
        &self,
        text: Option<&SourceText>,
    ) -> Option<Word> {
        text.map(|text| self.parse_source_text_as_word(text))
    }
}
