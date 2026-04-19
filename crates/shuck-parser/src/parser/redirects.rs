use super::*;

impl<'a> Parser<'a> {
    pub(super) fn fd_var_gap_allows_attachment(gap: &str) -> bool {
        if gap.is_empty() {
            return true;
        }

        let mut remaining = gap;
        while !remaining.is_empty() {
            if let Some(stripped) = remaining.strip_prefix("\\\r\n") {
                remaining = stripped;
                continue;
            }
            if let Some(stripped) = remaining.strip_prefix("\\\n") {
                remaining = stripped;
                continue;
            }
            return false;
        }

        true
    }

    pub(super) fn fd_var_from_text(text: &str, span: Span) -> Option<(Name, Span)> {
        if !text.starts_with('{') || !text.ends_with('}') || text.len() <= 2 {
            return None;
        }

        let inner = &text[1..text.len() - 1];
        if !inner.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }

        let start = span.start.advanced_by("{");
        let span = Span::from_positions(start, start.advanced_by(inner));
        Some((Name::from(inner), span))
    }

    pub(super) fn current_fd_var(&mut self) -> Option<(Name, Span)> {
        if let Some(token) = self.current_token.as_ref()
            && token.kind == TokenKind::Word
            && let Some(word) = token.word()
            && let Some(segment) = word.single_segment()
            && segment.kind() == LexedWordSegmentKind::Plain
            && !Self::word_text_needs_parse(segment.as_str())
            && let Some(fd_var) = Self::fd_var_from_text(
                segment.as_str(),
                segment.span().unwrap_or(self.current_span),
            )
        {
            return Some(fd_var);
        }

        let word = self.current_word_ref()?.clone();
        let text = self.literal_word_text(&word)?;
        Self::fd_var_from_text(&text, word.span)
    }

    pub(super) fn is_redirect_kind(kind: TokenKind) -> bool {
        REDIRECT_TOKENS.contains(kind)
    }

    pub(super) fn current_fd_value(&self) -> Option<i32> {
        self.current_token.as_ref().and_then(LexedToken::fd_value)
    }

    pub(super) fn current_fd_pair(&self) -> Option<(i32, i32)> {
        self.current_token
            .as_ref()
            .and_then(LexedToken::fd_pair_value)
    }
    pub(super) fn push_redirect_both_append(
        redirects: &mut Vec<Redirect>,
        operator_span: Span,
        target: Word,
    ) {
        redirects.push(Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::Append,
            span: Self::redirect_span(operator_span, &target),
            target: RedirectTarget::Word(target),
        });
        redirects.push(Redirect {
            fd: Some(2),
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::DupOutput,
            span: operator_span,
            target: RedirectTarget::Word(Word::literal("1")),
        });
    }

    pub(super) fn redirect_supports_fd_var(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::RedirectOut
                | TokenKind::Clobber
                | TokenKind::RedirectAppend
                | TokenKind::RedirectIn
                | TokenKind::RedirectReadWrite
                | TokenKind::HereString
                | TokenKind::RedirectBoth
                | TokenKind::DupOutput
                | TokenKind::DupInput
        )
    }

    pub(super) fn maybe_expect_word(&mut self, strict: bool) -> Result<Option<Word>> {
        if strict {
            self.expect_word().map(Some)
        } else {
            Ok(self.expect_word().ok())
        }
    }

    pub(super) fn consume_non_heredoc_redirect(
        &mut self,
        redirects: &mut Vec<Redirect>,
        fd_var: Option<Name>,
        fd_var_span: Option<Span>,
        strict: bool,
    ) -> Result<bool> {
        match self.current_token_kind {
            Some(TokenKind::RedirectOut) | Some(TokenKind::Clobber) => {
                let operator_span = self.current_span;
                let kind = if self.at(TokenKind::Clobber) {
                    RedirectKind::Clobber
                } else {
                    RedirectKind::Output
                };
                let fd = if fd_var.is_some() {
                    None
                } else {
                    self.current_fd_value()
                };
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd,
                        fd_var,
                        fd_var_span,
                        kind,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectAppend) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectIn) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectReadWrite) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::ReadWrite,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::HereString) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::HereString,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectBoth) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::OutputBoth,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectBothAppend) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    Self::push_redirect_both_append(redirects, operator_span, target);
                }
                Ok(true)
            }
            Some(TokenKind::DupOutput) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(1) },
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::DupOutput,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFd) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Output,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFdAppend) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFdReadWrite) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::ReadWrite,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::DupFd) => {
                let (src_fd, dst_fd) = self.current_fd_pair().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(src_fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupOutput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal(dst_fd.to_string())),
                });
                Ok(true)
            }
            Some(TokenKind::DupInput) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(0) },
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::DupInput,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::DupFdIn) => {
                let (src_fd, dst_fd) = self.current_fd_pair().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(src_fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupInput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal(dst_fd.to_string())),
                });
                Ok(true)
            }
            Some(TokenKind::DupFdClose) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupInput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal("-")),
                });
                Ok(true)
            }
            Some(TokenKind::RedirectFdIn) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn parse_trailing_redirects(&mut self) -> Vec<Redirect> {
        let mut redirects = Vec::new();
        let mut pending_fd_var = None;
        loop {
            let current_end = self.current_span.end.offset;
            let next_token = self
                .peek_next()
                .map(|token| (token.kind, token.span.start.offset));
            let input_len = self.input.len();
            if pending_fd_var.is_none()
                && let Some((fd_var, fd_var_span)) = self.current_fd_var()
                && let Some((next_kind, next_start)) = next_token
                && Self::is_redirect_kind(next_kind)
                && current_end <= next_start
                && next_start <= input_len
                && Self::fd_var_gap_allows_attachment(&self.input[current_end..next_start])
            {
                pending_fd_var = Some((fd_var, fd_var_span));
                self.advance();
                continue;
            }

            match self.current_token_kind {
                Some(TokenKind::HereDoc) | Some(TokenKind::HereDocStrip) => {
                    let strip_tabs = self.at(TokenKind::HereDocStrip);
                    let (fd_var, fd_var_span) = pending_fd_var.take().unzip();
                    if !self
                        .consume_heredoc_redirect(
                            strip_tabs,
                            &mut redirects,
                            fd_var,
                            fd_var_span,
                            false,
                            false,
                        )
                        .unwrap_or(false)
                    {
                        break;
                    }
                    continue;
                }
                Some(kind) => {
                    let (fd_var, fd_var_span) = if Self::redirect_supports_fd_var(kind) {
                        pending_fd_var.take().unzip()
                    } else {
                        let _ = pending_fd_var.take();
                        (None, None)
                    };

                    if self
                        .consume_non_heredoc_redirect(&mut redirects, fd_var, fd_var_span, false)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    break;
                }
                None => break,
            }
        }
        redirects
    }
}
