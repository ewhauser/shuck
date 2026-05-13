use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_for(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'for'
        self.skip_newlines()?;

        // Check for C-style for loop: for ((init; cond; step))
        if self.at(TokenKind::DoubleLeftParen) {
            let result = self.parse_arithmetic_for_inner(start_span);
            self.pop_depth();
            return result;
        }

        let allow_zsh_targets = self.dialect == ShellDialect::Zsh;
        let targets = match self.parse_for_targets(allow_zsh_targets) {
            Ok(targets) => targets,
            Err(error) => {
                self.pop_depth();
                return Err(error);
            }
        };

        if allow_zsh_targets {
            self.skip_newlines()?;
        }

        let (words, header) = if allow_zsh_targets && self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();

            let mut words = SmallVec::<[Word; 2]>::new();
            while !self.at(TokenKind::RightParen) {
                if self.at(TokenKind::Newline) {
                    self.skip_newlines()?;
                    continue;
                }
                match self.current_token_kind {
                    Some(kind)
                        if kind.is_word_like()
                            || (self.dialect == ShellDialect::Zsh
                                && matches!(kind, TokenKind::LeftParen)) =>
                    {
                        if self.dialect == ShellDialect::Zsh
                            && self
                                .current_token
                                .as_ref()
                                .is_some_and(|token| !token.flags.is_synthetic())
                        {
                            let start = self.current_span.start;
                            if let Some((text, end)) = self.scan_source_word(start) {
                                let span = Span::from_positions(start, end);
                                let word = self.parse_word_with_context(&text, span, start, true);
                                self.advance_past_word(&word);
                                words.push(word);
                                continue;
                            }
                        }

                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected for word"))?;
                        words.push(word);
                    }
                    Some(_) | None => {
                        self.pop_depth();
                        return Err(self.error("expected ')' after for word list"));
                    }
                }
            }

            let right_paren_span = self.current_span;
            self.advance();
            if self.at(TokenKind::Semicolon) {
                self.advance();
            }
            self.skip_newlines()?;

            (
                Some(words),
                ForHeaderSurface::Paren {
                    left_paren_span,
                    right_paren_span,
                },
            )
        } else if self.is_keyword(Keyword::In) {
            let in_span = self.current_span;
            self.advance();

            let (words, saw_separator) = self.parse_for_word_list_until_body_separator()?;
            if !saw_separator {
                self.pop_depth();
                return Err(self.error("expected ';' or newline before for loop body"));
            }
            (
                Some(words),
                ForHeaderSurface::In {
                    in_span: Some(in_span),
                },
            )
        } else {
            if self.at(TokenKind::Semicolon) {
                self.advance();
            }
            self.skip_newlines()?;
            (None, ForHeaderSurface::In { in_span: None })
        };

        let (body, syntax, end_span) = match header {
            ForHeaderSurface::In { in_span }
                if allow_zsh_targets && self.at(TokenKind::LeftBrace) =>
            {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty for loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    body,
                    ForSyntax::InBrace {
                        in_span,
                        left_brace_span,
                        right_brace_span,
                    },
                    right_brace_span,
                )
            }
            ForHeaderSurface::Paren {
                left_paren_span,
                right_paren_span,
            } if allow_zsh_targets && self.at(TokenKind::LeftBrace) => {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty for loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    body,
                    ForSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    },
                    right_brace_span,
                )
            }
            ForHeaderSurface::In { in_span }
                if allow_zsh_targets && !self.is_keyword(Keyword::Do) =>
            {
                let stmt = self.parse_single_stmt_command()?;
                let span = stmt.span;
                (
                    Self::stmt_seq_with_span(span, vec![stmt]),
                    ForSyntax::InDirect { in_span },
                    span,
                )
            }
            ForHeaderSurface::In { in_span } => {
                let do_span = if self.is_keyword(Keyword::Do) {
                    self.current_span
                } else {
                    self.pop_depth();
                    return Err(self.error("expected 'do'"));
                };
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() && self.dialect != ShellDialect::Zsh {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty for loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                let body = if body.is_empty() {
                    Self::stmt_seq_with_span(body_span, Vec::new())
                } else {
                    Self::stmt_seq_with_span(body_span, body)
                };
                (
                    body,
                    ForSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    },
                    done_span,
                )
            }
            ForHeaderSurface::Paren {
                left_paren_span,
                right_paren_span,
            } if allow_zsh_targets && !self.is_keyword(Keyword::Do) => {
                let stmt = self.parse_single_stmt_command()?;
                let span = stmt.span;
                (
                    Self::stmt_seq_with_span(span, vec![stmt]),
                    ForSyntax::ParenDirect {
                        left_paren_span,
                        right_paren_span,
                    },
                    span,
                )
            }
            ForHeaderSurface::Paren {
                left_paren_span,
                right_paren_span,
            } => {
                let do_span = if self.is_keyword(Keyword::Do) {
                    self.current_span
                } else {
                    self.pop_depth();
                    return Err(self.error("expected 'do'"));
                };
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() && self.dialect != ShellDialect::Zsh {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty for loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                let body = if body.is_empty() {
                    Self::stmt_seq_with_span(body_span, Vec::new())
                } else {
                    Self::stmt_seq_with_span(body_span, body)
                };
                (
                    body,
                    ForSyntax::ParenDoDone {
                        left_paren_span,
                        right_paren_span,
                        do_span,
                        done_span,
                    },
                    done_span,
                )
            }
        };

        self.pop_depth();
        Ok(CompoundCommand::For(ForCommand {
            targets: targets.into_vec(),
            words: words.map(SmallVec::into_vec),
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    pub(super) fn parse_for_targets(
        &mut self,
        allow_zsh_targets: bool,
    ) -> Result<SmallVec<[ForTarget; 1]>> {
        let allow_digits = allow_zsh_targets;
        let first_target = self
            .current_for_target(allow_digits)
            .ok_or_else(|| Error::parse("expected variable name in for loop".to_string()))?;
        let first_word = first_target.word.clone();
        self.advance_past_word(&first_word);

        let mut targets = SmallVec::from_vec(vec![first_target]);
        if !allow_zsh_targets {
            return Ok(targets);
        }

        loop {
            if self.current_keyword() == Some(Keyword::In)
                || matches!(
                    self.current_token_kind,
                    Some(TokenKind::LeftParen | TokenKind::Semicolon | TokenKind::Newline)
                )
                || self.at(TokenKind::LeftBrace)
                || self.is_keyword(Keyword::Do)
            {
                break;
            }

            let target = self
                .current_for_target(true)
                .ok_or_else(|| Error::parse("expected variable name in for loop".to_string()))?;
            let word = target.word.clone();
            self.advance_past_word(&word);
            targets.push(target);
        }

        Ok(targets)
    }

    pub(super) fn current_for_target(&mut self, allow_digits: bool) -> Option<ForTarget> {
        let name = self.current_word_str().and_then(|name| {
            (Self::is_valid_identifier(name)
                || (allow_digits && name.bytes().all(|byte| byte.is_ascii_digit())))
            .then(|| Name::from(name))
        });
        let word = self.current_word()?;
        Some(ForTarget {
            span: word.span,
            word,
            name,
        })
    }

    pub(super) fn parse_for_word_list_until_body_separator(
        &mut self,
    ) -> Result<(SmallVec<[Word; 2]>, bool)> {
        let mut words = SmallVec::<[Word; 2]>::new();
        loop {
            match self.current_token_kind {
                Some(kind)
                    if kind.is_word_like()
                        || (self.dialect == ShellDialect::Zsh
                            && matches!(kind, TokenKind::LeftParen)) =>
                {
                    if self.dialect == ShellDialect::Zsh
                        && self
                            .current_token
                            .as_ref()
                            .is_some_and(|token| !token.flags.is_synthetic())
                    {
                        let start = self.current_span.start;
                        if let Some((text, end)) = self.scan_source_word(start) {
                            let span = Span::from_positions(start, end);
                            let word = self.parse_word_with_context(&text, span, start, true);
                            self.advance_past_word(&word);
                            words.push(word);
                            continue;
                        }
                    }

                    let word = self
                        .take_current_word_and_advance()
                        .ok_or_else(|| self.error("expected for word"))?;
                    words.push(word);
                }
                Some(TokenKind::Semicolon) => {
                    self.advance();
                    self.skip_newlines()?;
                    return Ok((words, true));
                }
                Some(TokenKind::Newline) => {
                    self.skip_newlines()?;
                    return Ok((words, true));
                }
                _ => return Ok((words, false)),
            }
        }
    }

    /// Parse a zsh repeat loop.
    pub(super) fn parse_repeat(&mut self) -> Result<CompoundCommand> {
        self.ensure_repeat_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'repeat'

        let count = match self.current_token_kind {
            Some(kind) if kind.is_word_like() => self.expect_word()?,
            _ => {
                self.pop_depth();
                return Err(self.error("expected loop count in repeat"));
            }
        };

        let (syntax, body, end_span) = match self.current_token_kind {
            _ if self.is_keyword(Keyword::Do) => {
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            Some(TokenKind::LeftBrace) => {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty repeat loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    RepeatSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    },
                    body,
                    right_brace_span,
                )
            }
            Some(TokenKind::Semicolon) => {
                self.advance();
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            Some(TokenKind::Newline) => {
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            _ => {
                let stmt = self.parse_single_stmt_command()?;
                let span = stmt.span;
                (
                    RepeatSyntax::Direct,
                    Self::stmt_seq_with_span(span, vec![stmt]),
                    span,
                )
            }
        };

        self.pop_depth();
        Ok(CompoundCommand::Repeat(RepeatCommand {
            count,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse a zsh foreach loop.
    pub(super) fn parse_foreach(&mut self) -> Result<CompoundCommand> {
        self.ensure_foreach_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'foreach'

        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
            _ => {
                self.pop_depth();
                return Err(self.error("expected variable name in foreach"));
            }
        };
        self.advance();

        let (words, body, syntax, end_span) = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();

            let mut words = SmallVec::<[Word; 2]>::new();
            while !self.at(TokenKind::RightParen) {
                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                    }
                    Some(_) | None => {
                        self.pop_depth();
                        return Err(self.error("expected ')' after foreach word list"));
                    }
                }
            }
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }

            let right_paren_span = self.current_span;
            self.advance();
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after foreach word list"));
            }

            let (body, left_brace_span, right_brace_span) = self.parse_brace_enclosed_stmt_seq(
                "syntax error: empty foreach loop body",
                BraceBodyContext::Ordinary,
            )?;
            (
                words,
                body,
                ForeachSyntax::ParenBrace {
                    left_paren_span,
                    right_paren_span,
                    left_brace_span,
                    right_brace_span,
                },
                right_brace_span,
            )
        } else if self.is_keyword(Keyword::In) {
            let in_span = self.current_span;
            self.advance();

            let mut words = SmallVec::<[Word; 2]>::new();
            let saw_separator = loop {
                match self.current_token_kind {
                    _ if self.current_keyword() == Some(Keyword::Do) => break false,
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                    }
                    Some(TokenKind::Semicolon) => {
                        self.advance();
                        break true;
                    }
                    Some(TokenKind::Newline) => {
                        self.skip_newlines()?;
                        break true;
                    }
                    _ => break false,
                }
            };
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }
            if !saw_separator {
                self.pop_depth();
                return Err(self.error("expected ';' or newline before 'do' in foreach"));
            }
            if !self.is_keyword(Keyword::Do) {
                self.pop_depth();
                return Err(self.error("expected 'do' in foreach"));
            }
            let do_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);
            if body.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty foreach loop body"));
            }
            if !self.is_keyword(Keyword::Done) {
                self.pop_depth();
                return Err(self.error("expected 'done'"));
            }
            let done_span = self.current_span;
            self.advance();
            (
                words,
                Self::stmt_seq_with_span(body_span, body),
                ForeachSyntax::InDoDone {
                    in_span,
                    do_span,
                    done_span,
                },
                done_span,
            )
        } else {
            self.pop_depth();
            return Err(self.error("expected '(' or 'in' after foreach variable"));
        };

        self.pop_depth();
        Ok(CompoundCommand::Foreach(ForeachCommand {
            variable,
            variable_span,
            words: words.into_vec(),
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse select loop: select var in list; do body; done
    pub(super) fn parse_select(&mut self) -> Result<CompoundCommand> {
        self.ensure_select_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'select'
        self.skip_newlines()?;

        // Expect variable name
        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
            _ => {
                self.pop_depth();
                return Err(Error::parse("expected variable name in select".to_string()));
            }
        };
        self.advance();

        // Expect 'in' keyword
        if !self.is_keyword(Keyword::In) {
            self.pop_depth();
            return Err(Error::parse("expected 'in' in select".to_string()));
        }
        self.advance(); // consume 'in'

        // Parse word list until do/newline/;
        let mut words = SmallVec::<[Word; 2]>::new();
        loop {
            match self.current_token_kind {
                _ if self.current_keyword() == Some(Keyword::Do) => break,
                Some(kind) if kind.is_word_like() => {
                    if let Some(word) = self.take_current_word_and_advance() {
                        words.push(word);
                    }
                }
                Some(TokenKind::Newline | TokenKind::Semicolon) => {
                    self.advance();
                    break;
                }
                _ => break,
            }
        }

        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty select loop body"));
        }
        let body = Self::stmt_seq_with_span(body_span, body);

        // Expect 'done'
        self.expect_keyword(Keyword::Done)?;

        self.pop_depth();
        Ok(CompoundCommand::Select(SelectCommand {
            variable,
            variable_span,
            words: words.into_vec(),
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse C-style arithmetic for loop inner: for ((init; cond; step)); do body; done
    /// Note: depth tracking is done by parse_for which calls this
    pub(super) fn parse_arithmetic_for_inner(
        &mut self,
        start_span: Span,
    ) -> Result<CompoundCommand> {
        self.ensure_arithmetic_for()?;
        let left_paren_span = self.current_span;
        self.advance(); // consume '(('

        let mut paren_depth = 0_i32;
        let mut segment_start = left_paren_span.end;
        let mut init_span = None;
        let mut first_semicolon_span = None;
        let mut condition_span = None;
        let mut second_semicolon_span = None;

        let right_paren_span = loop {
            match self.current_token_kind {
                Some(TokenKind::DoubleLeftParen) => {
                    paren_depth += 2;
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    paren_depth += 1;
                    self.advance();
                }
                Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                    paren_depth += 1;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if paren_depth == 0 {
                        let right_paren_span = self.current_span;
                        self.advance();
                        break right_paren_span;
                    }
                    if paren_depth == 1 {
                        break self.split_nested_arithmetic_close("arithmetic for header")?;
                    }
                    paren_depth -= 2;
                    self.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    self.advance();
                }
                Some(TokenKind::DoubleSemicolon) if paren_depth == 0 => {
                    let (first_span, second_span) = Self::split_double_semicolon(self.current_span);
                    Self::record_arithmetic_for_separator(
                        first_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    Self::record_arithmetic_for_separator(
                        second_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    self.advance();
                }
                Some(TokenKind::Semicolon) if paren_depth == 0 => {
                    Self::record_arithmetic_for_separator(
                        self.current_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    self.advance();
                }
                Some(_) => {
                    self.advance();
                }
                None => {
                    return Err(Error::parse(
                        "unexpected end of input in for loop".to_string(),
                    ));
                }
            }
        };

        let first_semicolon_span = first_semicolon_span
            .ok_or_else(|| Error::parse("expected ';' in arithmetic for header".to_string()))?;
        let second_semicolon_span = second_semicolon_span.ok_or_else(|| {
            Error::parse("expected second ';' in arithmetic for header".to_string())
        })?;
        let step_span = Self::optional_span(segment_start, right_paren_span.start);
        let init_ast =
            self.parse_explicit_arithmetic_span(init_span, "invalid arithmetic for init")?;
        let condition_ast = self
            .parse_explicit_arithmetic_span(condition_span, "invalid arithmetic for condition")?;
        let step_ast =
            self.parse_explicit_arithmetic_span(step_span, "invalid arithmetic for step")?;

        self.skip_newlines()?;

        // Skip optional semicolon after ))
        if self.at(TokenKind::Semicolon) {
            self.advance();
        }
        self.skip_newlines()?;

        let (body, end_span) = if self.at(TokenKind::LeftBrace) {
            let body = self.parse_brace_group(BraceBodyContext::Ordinary)?;
            let span = Self::compound_span(&body);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(body),
                        SmallVec::<[Redirect; 1]>::new(),
                    ))],
                ),
                self.current_span,
            )
        } else {
            // Expect 'do'
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            // Parse body
            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);

            // Bash requires at least one command in loop body
            if body.is_empty() {
                return Err(self.error("syntax error: empty for loop body"));
            }

            // Expect 'done'
            if !self.is_keyword(Keyword::Done) {
                return Err(self.error("expected 'done'"));
            }
            let done_span = self.current_span;
            self.advance();
            (Self::stmt_seq_with_span(body_span, body), done_span)
        };

        Ok(CompoundCommand::ArithmeticFor(Box::new(
            ArithmeticForCommand {
                left_paren_span,
                init_span,
                init_ast,
                first_semicolon_span,
                condition_span,
                condition_ast,
                second_semicolon_span,
                step_span,
                step_ast,
                right_paren_span,
                body,
                span: start_span.merge(end_span),
            },
        )))
    }

    /// Parse a while loop
    pub(super) fn parse_while(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'while'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let allow_brace_body = self.dialect == ShellDialect::Zsh && self.zsh_brace_bodies_enabled();
        let condition = self.parse_loop_condition_until_body_start(allow_brace_body)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        let (body, end_span) = if allow_brace_body && self.at(TokenKind::LeftBrace) {
            let body = self.parse_brace_group(BraceBodyContext::Ordinary)?;
            let span = Self::compound_span(&body);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(body),
                        SmallVec::<[Redirect; 1]>::new(),
                    ))],
                ),
                self.current_span,
            )
        } else if let Some((body, left_brace_span, right_brace_span)) = allow_brace_body
            .then(|| self.try_parse_compact_zsh_brace_body(BraceBodyContext::Ordinary))
            .transpose()?
            .flatten()
        {
            let brace_group = CompoundCommand::BraceGroup(body);
            let span = left_brace_span.merge(right_brace_span);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(brace_group),
                        SmallVec::<[Redirect; 1]>::new(),
                    ))],
                ),
                right_brace_span,
            )
        } else {
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);

            if body.is_empty() && self.dialect != ShellDialect::Zsh {
                self.pop_depth();
                return Err(self.error("syntax error: empty while loop body"));
            }
            let body = Self::stmt_seq_with_span(body_span, body);

            self.expect_keyword(Keyword::Done)?;
            (body, self.current_span)
        };

        self.pop_depth();
        Ok(CompoundCommand::While(WhileCommand {
            condition,
            body,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse an until loop
    pub(super) fn parse_until(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'until'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let allow_brace_body = self.dialect == ShellDialect::Zsh && self.zsh_brace_bodies_enabled();
        let condition = self.parse_loop_condition_until_body_start(allow_brace_body)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        let (body, end_span) = if allow_brace_body && self.at(TokenKind::LeftBrace) {
            let body = self.parse_brace_group(BraceBodyContext::Ordinary)?;
            let span = Self::compound_span(&body);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(body),
                        SmallVec::<[Redirect; 1]>::new(),
                    ))],
                ),
                self.current_span,
            )
        } else if let Some((body, left_brace_span, right_brace_span)) = allow_brace_body
            .then(|| self.try_parse_compact_zsh_brace_body(BraceBodyContext::Ordinary))
            .transpose()?
            .flatten()
        {
            let brace_group = CompoundCommand::BraceGroup(body);
            let span = left_brace_span.merge(right_brace_span);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(brace_group),
                        SmallVec::<[Redirect; 1]>::new(),
                    ))],
                ),
                right_brace_span,
            )
        } else {
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);

            if body.is_empty() && self.dialect != ShellDialect::Zsh {
                self.pop_depth();
                return Err(self.error("syntax error: empty until loop body"));
            }
            let body = Self::stmt_seq_with_span(body_span, body);

            self.expect_keyword(Keyword::Done)?;
            (body, self.current_span)
        };

        self.pop_depth();
        Ok(CompoundCommand::Until(UntilCommand {
            condition,
            body,
            span: start_span.merge(end_span),
        }))
    }
}
