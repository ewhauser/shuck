use super::*;

impl<'a> Parser<'a> {
    pub(super) fn simple_word_from_token(
        &mut self,
        token: &LexedToken<'_>,
        span: Span,
    ) -> Option<Word> {
        let word = token.word()?;
        let source_backed = !token.flags.is_synthetic();

        if self.zsh_glob_word_parsing_enabled_at(span.start.offset)
            && let Some(segment) = word.single_segment()
            && segment.kind() == LexedWordSegmentKind::Plain
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(
                segment.as_str(),
                span,
                segment.span().is_some() && source_backed && segment.text_is_source_backed(),
            )
        {
            return Some(word);
        }
        let mut parts = Self::word_part_buffer_with_capacity(word.segments().size_hint().0);

        for segment in word.segments() {
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let content_span = Self::segment_content_span(segment, span);
            let raw_text = segment.as_str();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
            match segment.kind() {
                LexedWordSegmentKind::Plain
                | LexedWordSegmentKind::DoubleQuoted
                | LexedWordSegmentKind::DollarDoubleQuoted
                    if Self::word_text_needs_parse(text) =>
                {
                    return None;
                }
                LexedWordSegmentKind::Plain
                | LexedWordSegmentKind::SingleQuoted
                | LexedWordSegmentKind::DollarSingleQuoted
                | LexedWordSegmentKind::DoubleQuoted
                | LexedWordSegmentKind::DollarDoubleQuoted => {}
                LexedWordSegmentKind::Composite => return None,
            }

            let wrapper_span = Self::segment_wrapper_span(segment, span);
            let part = match segment.kind() {
                LexedWordSegmentKind::Plain => {
                    self.literal_part_from_text(text, content_span, source_backed)
                }
                LexedWordSegmentKind::SingleQuoted => {
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false)
                }
                LexedWordSegmentKind::DollarSingleQuoted => {
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true)
                }
                LexedWordSegmentKind::DoubleQuoted => self.double_quoted_literal_part_from_text(
                    text,
                    content_span,
                    wrapper_span,
                    source_backed,
                    false,
                ),
                LexedWordSegmentKind::DollarDoubleQuoted => self
                    .double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        true,
                    ),
                LexedWordSegmentKind::Composite => unreachable!(),
            };
            Self::push_word_part_node(&mut parts, part);
        }

        Some(self.word_with_part_buffer(parts, span))
    }

    pub(super) fn segment_content_span(segment: &LexedWordSegment<'_>, fallback: Span) -> Span {
        segment
            .span()
            .or_else(|| segment.wrapper_span())
            .unwrap_or(fallback)
    }

    pub(super) fn segment_wrapper_span(segment: &LexedWordSegment<'_>, fallback: Span) -> Span {
        segment
            .wrapper_span()
            .or_else(|| segment.span())
            .unwrap_or(fallback)
    }

    pub(super) fn literal_part_from_text(
        &self,
        text: &str,
        span: Span,
        source_backed: bool,
    ) -> WordPartNode {
        WordPartNode::new(
            WordPart::Literal(self.literal_text_from_str(
                text,
                span.start,
                span.end,
                source_backed,
            )),
            span,
        )
    }

    pub(super) fn single_quoted_part_from_text(
        &self,
        text: &str,
        content_span: Span,
        wrapper_span: Span,
        dollar: bool,
    ) -> WordPartNode {
        WordPartNode::new(
            WordPart::SingleQuoted {
                value: self.source_text_from_str(text, content_span.start, content_span.end),
                dollar,
            },
            wrapper_span,
        )
    }

    pub(super) fn double_quoted_literal_part_from_text(
        &self,
        text: &str,
        content_span: Span,
        wrapper_span: Span,
        source_backed: bool,
        dollar: bool,
    ) -> WordPartNode {
        WordPartNode::new(
            WordPart::DoubleQuoted {
                parts: vec![self.literal_part_from_text(text, content_span, source_backed)],
                dollar,
            },
            wrapper_span,
        )
    }

    pub(super) fn decode_word_from_token(
        &mut self,
        token: &LexedToken<'_>,
        span: Span,
    ) -> Option<Word> {
        let word = token.word()?;

        if word.single_segment().is_none()
            && !token.flags.is_synthetic()
            && let Some(source_text) = token.source_slice(self.input)
        {
            return Some(self.parse_word_with_context(source_text, span, span.start, true));
        }

        if let Some(segment) = word.single_segment() {
            let content_span = Self::segment_content_span(segment, span);
            let wrapper_span = Self::segment_wrapper_span(segment, span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let raw_text = segment.as_str();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
            let decode_text = if source_backed
                && !self.source_matches(content_span, text)
                && matches!(
                    segment.kind(),
                    LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted
                )
                && (!text.contains("$(") || text.contains("$(("))
            {
                content_span.slice(self.input)
            } else {
                text
            };
            let preserve_escaped_expansion_literals =
                source_backed && self.source_matches(content_span, decode_text);

            return match segment.kind() {
                LexedWordSegmentKind::SingleQuoted => Some(self.word_with_single_part(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false),
                    span,
                )),
                LexedWordSegmentKind::DollarSingleQuoted => Some(self.word_with_single_part(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true),
                    span,
                )),
                LexedWordSegmentKind::Plain if Self::word_text_needs_parse(text) => Some(
                    self.decode_word_text_preserving_quotes_if_needed_with_escape_mode(
                        text,
                        span,
                        content_span.start,
                        source_backed,
                        preserve_escaped_expansion_literals,
                    ),
                ),
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted
                    if Self::word_text_needs_parse(text) =>
                {
                    let inner = self.decode_quoted_segment_text(
                        decode_text,
                        content_span,
                        content_span.start,
                        source_backed,
                    );
                    Some(self.word_with_single_part(
                        WordPartNode::new(
                            WordPart::DoubleQuoted {
                                parts: inner.parts,
                                dollar: matches!(
                                    segment.kind(),
                                    LexedWordSegmentKind::DollarDoubleQuoted
                                ),
                            },
                            wrapper_span,
                        ),
                        span,
                    ))
                }
                LexedWordSegmentKind::Plain => Some(self.word_with_single_part(
                    self.literal_part_from_text(text, content_span, source_backed),
                    span,
                )),
                LexedWordSegmentKind::DoubleQuoted => Some(self.word_with_single_part(
                    self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        false,
                    ),
                    span,
                )),
                LexedWordSegmentKind::DollarDoubleQuoted => Some(self.word_with_single_part(
                    self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        true,
                    ),
                    span,
                )),
                LexedWordSegmentKind::Composite => None,
            };
        }

        let mut parts = Self::word_part_buffer_with_capacity(word.segments().size_hint().0);
        let mut cursor = span.start;

        for segment in word.segments() {
            let raw_text = segment.as_str();
            let content_span = if let Some(segment_span) = segment.span() {
                cursor = segment_span.end;
                segment_span
            } else {
                let start = cursor;
                let end = start.advanced_by(raw_text);
                cursor = end;
                Span::from_positions(start, end)
            };
            let wrapper_span = segment.wrapper_span().unwrap_or(content_span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
            let preserve_escaped_expansion_literals = source_backed;

            match segment.kind() {
                LexedWordSegmentKind::SingleQuoted => Self::push_word_part_node(
                    &mut parts,
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false),
                ),
                LexedWordSegmentKind::DollarSingleQuoted => Self::push_word_part_node(
                    &mut parts,
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true),
                ),
                LexedWordSegmentKind::Plain => {
                    if Self::word_text_needs_parse(text) {
                        parts.extend(
                            self.decode_word_text_preserving_quotes_if_needed_with_escape_mode(
                                text,
                                content_span,
                                content_span.start,
                                source_backed,
                                preserve_escaped_expansion_literals,
                            )
                            .parts,
                        );
                    } else {
                        Self::push_word_part_node(
                            &mut parts,
                            self.literal_part_from_text(text, content_span, source_backed),
                        );
                    }
                }
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted => {
                    if Self::word_text_needs_parse(text) {
                        let inner = self.decode_quoted_segment_text(
                            text,
                            content_span,
                            content_span.start,
                            source_backed,
                        );
                        Self::push_word_part_node(
                            &mut parts,
                            WordPartNode::new(
                                WordPart::DoubleQuoted {
                                    parts: inner.parts,
                                    dollar: matches!(
                                        segment.kind(),
                                        LexedWordSegmentKind::DollarDoubleQuoted
                                    ),
                                },
                                wrapper_span,
                            ),
                        );
                    } else {
                        Self::push_word_part_node(
                            &mut parts,
                            self.double_quoted_literal_part_from_text(
                                text,
                                content_span,
                                wrapper_span,
                                source_backed,
                                matches!(segment.kind(), LexedWordSegmentKind::DollarDoubleQuoted),
                            ),
                        );
                    }
                }
                LexedWordSegmentKind::Composite => return None,
            }
        }

        Some(self.word_with_part_buffer(parts, span))
    }

    pub(super) fn current_word_ref(&mut self) -> Option<&Word> {
        if self.current_word_cache.is_none() {
            self.current_word_cache = self.current_word();
        }

        self.current_word_cache.as_ref()
    }

    pub(super) fn current_word(&mut self) -> Option<Word> {
        if let Some(word) = self.current_word_cache.as_ref() {
            return Some(word.clone());
        }

        if let Some(word) = self.current_zsh_glob_word_from_source() {
            self.current_word_cache = Some(word.clone());
            return Some(word);
        }

        let span = self.current_span;
        if let Some(token) = self.current_token.clone()
            && let Some(word) = self.simple_word_from_token(&token, span)
        {
            return Some(word);
        }

        let token = self.current_token.take()?;
        let word = self.decode_word_from_token(&token, span);
        self.current_token = Some(token);
        if let Some(word) = word.as_ref() {
            self.current_word_cache = Some(word.clone());
        }
        word
    }

    pub(super) fn take_current_word(&mut self) -> Option<Word> {
        if let Some(word) = self.current_word_cache.take() {
            return Some(word);
        }

        if let Some(word) = self.current_zsh_glob_word_from_source() {
            return Some(word);
        }

        let span = self.current_span;
        if let Some(token) = self.current_token.clone()
            && let Some(word) = self.simple_word_from_token(&token, span)
        {
            return Some(word);
        }

        let token = self.current_token.take()?;
        let word = self.decode_word_from_token(&token, span);
        self.current_token = Some(token);
        word
    }

    pub(super) fn take_current_word_and_advance(&mut self) -> Option<Word> {
        let word = self.take_current_word()?;
        self.advance_past_word(&word);
        Some(word)
    }

    pub(super) fn advance_past_word(&mut self, word: &Word) {
        let stop_after_synthetic = self
            .current_token
            .as_ref()
            .is_some_and(|token| token.flags.is_synthetic());
        while self.current_token.is_some() && self.current_span.start.offset < word.span.end.offset
        {
            self.advance();
            if stop_after_synthetic
                && self
                    .current_token
                    .as_ref()
                    .is_none_or(|token| !token.flags.is_synthetic())
            {
                break;
            }
        }
    }

    pub(super) fn keyword_from_token(token: &LexedToken<'_>) -> Option<Keyword> {
        (token.kind == TokenKind::Word)
            .then(|| token.word_text())
            .flatten()
            .and_then(Self::classify_keyword)
    }

    pub(super) fn current_conditional_literal_word(&self) -> Option<Word> {
        match self.current_token_kind? {
            TokenKind::LeftBrace | TokenKind::RightBrace => Some(Word::literal_with_span(
                self.input[self.current_span.start.offset..self.current_span.end.offset]
                    .to_string(),
                self.current_span,
            )),
            _ => None,
        }
    }

    pub(super) fn current_name_token(&self) -> Option<(Name, Span)> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and_then(|_| self.current_word_str())
            .map(|word| (Name::from(word), self.current_span))
    }

    pub(super) fn current_static_token_text(&self) -> Option<(String, bool)> {
        let token = self.current_token.as_ref()?;
        let raw_text = token.word_string()?;
        let text_had_escape_markers = raw_text.contains('\x00');
        let text = if text_had_escape_markers {
            raw_text.replace('\x00', "")
        } else {
            raw_text
        };

        match token.kind {
            TokenKind::LiteralWord => Some((text, true)),
            TokenKind::QuotedWord if !Self::word_text_needs_parse(&text) => Some((text, true)),
            TokenKind::Word if !Self::word_text_needs_parse(&text) => Some((
                text,
                token.flags.has_cooked_text() || text_had_escape_markers,
            )),
            _ => None,
        }
    }
}
