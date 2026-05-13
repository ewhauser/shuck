use super::*;

impl<'a> Parser<'a> {
    pub(super) fn advance_raw(&mut self) {
        #[cfg(feature = "benchmarking")]
        self.maybe_record_advance_raw_call();
        if let Some(peeked) = self.peeked_token.take() {
            self.set_current_spanned(peeked);
        } else {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if st.kind == TokenKind::Comment => {
                        self.maybe_record_comment(&st);
                    }
                    Some(st) => {
                        self.set_current_spanned(st);
                        break;
                    }
                    None => {
                        self.clear_current_token();
                        // Keep the last span for error reporting
                        break;
                    }
                }
            }
        }
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn maybe_record_set_current_spanned_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.parser_set_current_spanned_calls += 1;
        }
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn maybe_record_advance_raw_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.parser_advance_raw_calls += 1;
        }
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn finish_benchmark_counters(&self) -> ParserBenchmarkCounters {
        let mut counters = self.benchmark_counters.unwrap_or_default();
        counters.lexer_current_position_calls =
            self.lexer.benchmark_counters().current_position_calls;
        counters
    }

    pub(super) fn advance(&mut self) {
        let should_expand = std::mem::take(&mut self.expand_next_word);
        self.advance_raw();
        if should_expand {
            if self
                .current_token
                .as_ref()
                .is_some_and(|token| token.flags.is_synthetic())
            {
                self.expand_next_word = true;
            } else {
                self.maybe_expand_current_alias_chain();
            }
        }
    }

    /// Peek at the next token without consuming the current one
    pub(super) fn peek_next(&mut self) -> Option<&LexedToken<'a>> {
        if self.peeked_token.is_none() {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if st.kind == TokenKind::Comment => {
                        self.maybe_record_comment(&st);
                    }
                    other => {
                        self.peeked_token = other;
                        break;
                    }
                }
            }
        }
        self.peeked_token.as_ref()
    }

    pub(super) fn peek_next_kind(&mut self) -> Option<TokenKind> {
        self.peek_next()?;
        self.peeked_token.as_ref().map(|st| st.kind)
    }

    pub(super) fn peek_next_is(&mut self, kind: TokenKind) -> bool {
        self.peek_next_kind() == Some(kind)
    }

    pub(super) fn at(&self, kind: TokenKind) -> bool {
        self.current_token_kind == Some(kind)
    }

    pub(super) fn current_token_has_leading_whitespace(&self) -> bool {
        self.current_span.start.offset > 0
            && self.input[..self.current_span.start.offset]
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, ' ' | '\t' | '\n'))
    }

    pub(super) fn current_token_is_tight_to_next_token(&mut self) -> bool {
        let current_end = self.current_span.end.offset;
        self.peek_next()
            .is_some_and(|token| token.span.start.offset == current_end)
    }

    pub(super) fn at_in_set(&self, set: TokenSet) -> bool {
        self.current_token_kind
            .is_some_and(|kind| set.contains(kind))
    }

    pub(super) fn at_word_like(&self) -> bool {
        self.current_token_kind.is_some_and(TokenKind::is_word_like)
    }

    pub(super) fn current_word_str(&self) -> Option<&str> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and(self.current_token.as_ref())
            .and_then(LexedToken::word_text)
    }

    pub(super) fn classify_keyword(word: &str) -> Option<Keyword> {
        match word.as_bytes() {
            b"if" => Some(Keyword::If),
            b"for" => Some(Keyword::For),
            b"repeat" => Some(Keyword::Repeat),
            b"foreach" => Some(Keyword::Foreach),
            b"while" => Some(Keyword::While),
            b"until" => Some(Keyword::Until),
            b"case" => Some(Keyword::Case),
            b"select" => Some(Keyword::Select),
            b"time" => Some(Keyword::Time),
            b"coproc" => Some(Keyword::Coproc),
            b"function" => Some(Keyword::Function),
            b"always" => Some(Keyword::Always),
            b"then" => Some(Keyword::Then),
            b"else" => Some(Keyword::Else),
            b"elif" => Some(Keyword::Elif),
            b"fi" => Some(Keyword::Fi),
            b"do" => Some(Keyword::Do),
            b"done" => Some(Keyword::Done),
            b"esac" => Some(Keyword::Esac),
            b"in" => Some(Keyword::In),
            _ => None,
        }
    }

    pub(super) fn current_keyword(&self) -> Option<Keyword> {
        self.current_keyword
    }

    pub(super) fn looks_like_disabled_repeat_loop(&mut self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Repeat) {
            return Ok(false);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if !self.at_word_like() {
            self.restore(checkpoint);
            return Ok(false);
        }
        self.advance();

        let result = match self.current_token_kind {
            Some(TokenKind::LeftBrace) => Ok(true),
            Some(TokenKind::Semicolon) => {
                self.advance();
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                Ok(self.current_keyword() == Some(Keyword::Do))
            }
            Some(TokenKind::Newline) => {
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                Ok(self.current_keyword() == Some(Keyword::Do))
            }
            _ => Ok(false),
        };
        self.restore(checkpoint);
        result
    }

    pub(super) fn looks_like_disabled_foreach_loop(&mut self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Foreach) {
            return Ok(false);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if self.current_name_token().is_none() {
            self.restore(checkpoint);
            return Ok(false);
        }
        self.advance();

        let result = if self.at(TokenKind::LeftParen) {
            self.advance();
            let mut saw_word = false;
            while !self.at(TokenKind::RightParen) {
                if !self.at_word_like() {
                    self.restore(checkpoint);
                    return Ok(false);
                }
                saw_word = true;
                self.advance();
            }
            if !saw_word {
                self.restore(checkpoint);
                return Ok(false);
            }
            self.advance();
            Ok(self.at(TokenKind::LeftBrace))
        } else {
            if self.current_keyword() != Some(Keyword::In) {
                self.restore(checkpoint);
                return Ok(false);
            }
            self.advance();

            let mut saw_word = false;
            let saw_separator = loop {
                if self.current_keyword() == Some(Keyword::Do) {
                    break false;
                }

                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        saw_word = true;
                        self.advance();
                    }
                    Some(TokenKind::Semicolon) => {
                        self.advance();
                        break true;
                    }
                    Some(TokenKind::Newline) => {
                        if let Err(error) = self.skip_newlines() {
                            self.restore(checkpoint);
                            return Err(error);
                        }
                        break true;
                    }
                    _ => break false,
                }
            };

            Ok(saw_word && saw_separator && self.current_keyword() == Some(Keyword::Do))
        };
        self.restore(checkpoint);
        result
    }

    pub(super) fn skip_newlines_with_flag(&mut self) -> Result<bool> {
        let mut skipped = false;
        while self.at(TokenKind::Newline) {
            self.tick()?;
            self.advance();
            skipped = true;
        }
        Ok(skipped)
    }

    pub(super) fn skip_newlines(&mut self) -> Result<()> {
        self.skip_newlines_with_flag().map(|_| ())
    }
}
