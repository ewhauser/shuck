use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_function_body_command(
        &mut self,
        allow_bare_compound: bool,
    ) -> Result<Stmt> {
        if let Some(compound) = self.try_parse_compact_function_brace_body()? {
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        let compound = match self.current_keyword() {
            Some(Keyword::If) if allow_bare_compound => self.parse_if()?,
            Some(Keyword::For) if allow_bare_compound => self.parse_for()?,
            Some(Keyword::Repeat) if allow_bare_compound && self.zsh_short_repeat_enabled() => {
                self.parse_repeat()?
            }
            Some(Keyword::Foreach) if allow_bare_compound && self.zsh_short_loops_enabled() => {
                self.parse_foreach()?
            }
            Some(Keyword::While) if allow_bare_compound => self.parse_while()?,
            Some(Keyword::Until) if allow_bare_compound => self.parse_until()?,
            Some(Keyword::Case) if allow_bare_compound => self.parse_case()?,
            Some(Keyword::Select) if allow_bare_compound => self.parse_select()?,
            _ => match self.current_token_kind {
                Some(TokenKind::LeftBrace) => self.parse_brace_group(BraceBodyContext::Function)?,
                Some(TokenKind::LeftParen) => self.parse_subshell()?,
                Some(TokenKind::DoubleLeftBracket) if allow_bare_compound => {
                    self.parse_conditional()?
                }
                Some(TokenKind::DoubleLeftParen) if allow_bare_compound => {
                    if self.looks_like_command_style_double_paren() {
                        self.split_current_double_left_paren();
                        self.parse_subshell()?
                    } else {
                        let checkpoint = self.checkpoint();
                        if let Ok(compound) = self.parse_arithmetic_command() {
                            compound
                        } else {
                            self.restore(checkpoint);
                            self.split_current_double_left_paren();
                            self.parse_subshell()?
                        }
                    }
                }
                _ => {
                    return Err(Error::parse(
                        "expected compound command for function body".to_string(),
                    ));
                }
            },
        };
        let redirects = self.parse_trailing_redirects();
        Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
            Box::new(compound),
            redirects,
        )))
    }

    pub(super) fn parse_function_header_entry(&mut self) -> Result<FunctionHeaderEntry> {
        let word = self
            .take_current_function_header_word_and_advance()
            .ok_or_else(|| self.error("expected function name"))?;
        Ok(self.function_header_entry_from_word(word))
    }

    pub(super) fn parse_function_keyword_header_entry(&mut self) -> Result<FunctionHeaderEntry> {
        let word = self
            .take_current_function_header_word_and_advance()
            .or_else(|| self.take_current_function_keyword_name_and_advance())
            .ok_or_else(|| self.error("expected function name"))?;
        Ok(self.function_header_entry_from_word(word))
    }

    pub(super) fn take_current_function_header_word_and_advance(&mut self) -> Option<Word> {
        let span = self.current_span;
        if let Some(token) = self.current_token.clone()
            && let Some(word) = self.simple_word_from_token(&token, span)
        {
            self.advance_past_word(&word);
            return Some(word);
        }

        let token = self.current_token.take()?;
        let word = self.decode_word_from_token(&token, span);
        self.current_token = Some(token);
        if let Some(word) = word.as_ref() {
            self.advance_past_word(word);
        }
        word
    }

    pub(super) fn take_current_function_keyword_name_and_advance(&mut self) -> Option<Word> {
        let text = match self.current_token_kind? {
            TokenKind::DoubleLeftBracket => "[[",
            TokenKind::DoubleRightBracket => "]]",
            TokenKind::LeftBrace => "{",
            TokenKind::RightBrace => "}",
            _ => return None,
        };
        let word = Word::literal_with_span(text, self.current_span);
        self.advance();
        Some(word)
    }

    pub(super) fn function_header_entry_from_word(&self, word: Word) -> FunctionHeaderEntry {
        let static_name = self.literal_word_text(&word).map(Name::from);
        FunctionHeaderEntry { word, static_name }
    }

    pub(super) fn parse_function_parens_span(&mut self) -> Result<Span> {
        if !self.at(TokenKind::LeftParen) {
            return Err(self.error("expected '(' in function definition"));
        }
        let left_paren_span = self.current_span;
        self.advance();

        if !self.at(TokenKind::RightParen) {
            return Err(Error::parse(
                "expected ')' in function definition".to_string(),
            ));
        }
        let right_paren_span = self.current_span;
        self.advance();
        Ok(left_paren_span.merge(right_paren_span))
    }

    pub(super) fn parse_zsh_function_body_stmt(&mut self) -> Result<Stmt> {
        self.skip_newlines()?;

        if let Some(compound) = self.try_parse_compact_function_brace_body()? {
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        if self.at(TokenKind::LeftBrace) {
            let compound = self.parse_brace_group(BraceBodyContext::Function)?;
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        self.parse_single_stmt_command()
    }

    pub(super) fn parse_single_stmt_command(&mut self) -> Result<Stmt> {
        let mut stmt = self
            .parse_pipeline()?
            .ok_or_else(|| self.error("expected command"))?;

        let Some(kind) = self.current_token_kind else {
            return Ok(stmt);
        };
        let operator = match kind {
            TokenKind::And => Some((Some(BinaryOp::And), None, false)),
            TokenKind::Or => Some((Some(BinaryOp::Or), None, false)),
            TokenKind::Semicolon => Some((None, Some(StmtTerminator::Semicolon), true)),
            TokenKind::Background => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Plain)),
                true,
            )),
            TokenKind::BackgroundPipe => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Pipe)),
                true,
            )),
            TokenKind::BackgroundBang => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Bang)),
                true,
            )),
            _ => None,
        };
        let Some((binary_op, terminator, allow_empty_tail)) = operator else {
            return Ok(stmt);
        };
        let operator_span = self.current_span;
        self.advance();

        if let Some(binary_op) = binary_op {
            self.skip_newlines()?;
            if let Some(right) = self.parse_pipeline()? {
                stmt = Self::binary_stmt(stmt, binary_op, operator_span, right);
            }
            return Ok(stmt);
        }

        if allow_empty_tail
            && matches!(
                self.current_token_kind,
                Some(TokenKind::Semicolon | TokenKind::Newline)
            )
        {
            self.advance();
        }

        stmt.terminator = terminator;
        stmt.terminator_span = Some(operator_span);
        Ok(stmt)
    }

    pub(super) fn parse_anonymous_function_args(&mut self) -> Result<SmallVec<[Word; 2]>> {
        let mut args = SmallVec::<[Word; 2]>::new();
        while self.current_token_kind.is_some_and(TokenKind::is_word_like) {
            let word = self
                .take_current_word_and_advance()
                .ok_or_else(|| self.error("expected anonymous function argument"))?;
            args.push(word);
        }
        Ok(args)
    }

    /// Parse function definition with 'function' keyword: function name { body }
    pub(super) fn parse_function_keyword(&mut self) -> Result<Command> {
        self.ensure_function_keyword()?;
        let start_span = self.current_span;
        self.advance(); // consume 'function'
        self.skip_newlines()?;

        if self.dialect == ShellDialect::Zsh {
            let mut entries = Vec::new();
            while self.current_token_kind.is_some_and(TokenKind::is_word_like) {
                if !entries.is_empty() && self.current_token_is_compact_zsh_brace_body() {
                    break;
                }
                entries.push(self.parse_function_header_entry()?);
                if self.at(TokenKind::LeftParen) {
                    break;
                }
            }

            let trailing_parens_span = if !entries.is_empty() && self.at(TokenKind::LeftParen) {
                Some(self.parse_function_parens_span()?)
            } else {
                None
            };

            if entries.is_empty() {
                let body = self.parse_zsh_function_body_stmt()?;
                let args = self.parse_anonymous_function_args()?;
                let redirects = self.parse_trailing_redirects();
                let span = start_span.merge(self.current_span);
                return Ok(Command::AnonymousFunction(
                    AnonymousFunctionCommand {
                        surface: AnonymousFunctionSurface::FunctionKeyword {
                            function_keyword_span: start_span,
                        },
                        body: Box::new(body),
                        args: args.into_vec(),
                        span,
                    },
                    redirects,
                ));
            }

            let body = self.parse_zsh_function_body_stmt()?;
            let span = start_span.merge(self.current_span);
            return Ok(Command::Function(FunctionDef {
                header: FunctionHeader {
                    function_keyword_span: Some(start_span),
                    entries,
                    trailing_parens_span,
                },
                body: Box::new(body),
                span,
            }));
        }

        let entry = self.parse_function_keyword_header_entry()?;
        let saw_newline_after_name = self.skip_newlines_with_flag()?;
        let (trailing_parens_span, allow_bare_compound) = if self.at(TokenKind::LeftParen) {
            let parens_span = self.parse_function_parens_span()?;
            (Some(parens_span), self.skip_newlines_with_flag()?)
        } else {
            (None, saw_newline_after_name)
        };

        let body = self.parse_function_body_command(allow_bare_compound)?;
        let span = start_span.merge(self.current_span);

        Ok(Command::Function(FunctionDef {
            header: FunctionHeader {
                function_keyword_span: Some(start_span),
                entries: vec![entry],
                trailing_parens_span,
            },
            body: Box::new(body),
            span,
        }))
    }

    /// Parse POSIX-style function definition: name() { body }
    pub(super) fn parse_function_posix(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        let entry = self.parse_function_header_entry()?;
        let trailing_parens_span = self.parse_function_parens_span()?;

        self.finish_parse_function_posix(start_span, entry, trailing_parens_span)
    }

    pub(super) fn finish_parse_function_posix(
        &mut self,
        start_span: Span,
        entry: FunctionHeaderEntry,
        trailing_parens_span: Span,
    ) -> Result<Command> {
        let body = if self.dialect == ShellDialect::Zsh {
            self.parse_zsh_function_body_stmt()?
        } else {
            self.skip_newlines()?;
            self.parse_function_body_command(true)?
        };

        Ok(Command::Function(FunctionDef {
            header: FunctionHeader {
                function_keyword_span: None,
                entries: vec![entry],
                trailing_parens_span: Some(trailing_parens_span),
            },
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    pub(super) fn try_parse_zsh_attached_parens_function(&mut self) -> Result<Option<Command>> {
        if self.dialect != ShellDialect::Zsh || !self.at_word_like() {
            return Ok(None);
        }

        let Some(word_text) = self.current_source_like_word_text() else {
            return Ok(None);
        };
        let Some(header_text) = word_text.as_ref().strip_suffix("()") else {
            return Ok(None);
        };
        if header_text.is_empty() || header_text.contains('=') {
            return Ok(None);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if let Err(error) = self.skip_newlines() {
            self.restore(checkpoint);
            return Err(error);
        }
        if !self.at(TokenKind::LeftBrace) {
            self.restore(checkpoint);
            return Ok(None);
        }
        self.restore(checkpoint);

        let start_span = self.current_span;
        let header_span =
            Span::from_positions(start_span.start, start_span.start.advanced_by(header_text));
        let parens_span = Span::from_positions(header_span.end, start_span.end);
        let header_word =
            self.parse_word_with_context(header_text, header_span, header_span.start, true);
        let entry = self.function_header_entry_from_word(header_word);
        self.advance();

        self.finish_parse_function_posix(start_span, entry, parens_span)
            .map(Some)
    }

    pub(super) fn parse_anonymous_paren_function(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        let parens_span = self.parse_function_parens_span()?;
        let body = self.parse_zsh_function_body_stmt()?;
        let args = self.parse_anonymous_function_args()?;
        let redirects = self.parse_trailing_redirects();
        let span = start_span.merge(self.current_span);
        Ok(Command::AnonymousFunction(
            AnonymousFunctionCommand {
                surface: AnonymousFunctionSurface::Parens { parens_span },
                body: Box::new(body),
                args: args.into_vec(),
                span,
            },
            redirects,
        ))
    }
}
