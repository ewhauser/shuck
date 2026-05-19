use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_compound_list(&mut self, terminator: Keyword) -> Result<Vec<Stmt>> {
        self.parse_compound_list_until(KeywordSet::single(terminator))
    }

    /// Parse commands until one of the terminating keywords
    pub(super) fn parse_compound_list_until(
        &mut self,
        terminators: KeywordSet,
    ) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::new();

        loop {
            self.skip_command_separators()?;

            // Check for terminators
            if self
                .current_keyword()
                .is_some_and(|keyword| terminators.contains(keyword))
            {
                break;
            }

            if self.current_token.is_none() {
                break;
            }

            let command_stmts = self.parse_command_list_required()?;
            self.apply_stmt_list_effects(&command_stmts);
            stmts.extend(command_stmts);
        }

        Ok(stmts)
    }

    /// Reserved words that cannot start a simple command.
    /// These words are only special in command position, not as arguments.
    /// Check if a word cannot start a command
    pub(super) fn is_non_command_keyword(keyword: Keyword) -> bool {
        NON_COMMAND_KEYWORDS.contains(keyword)
    }

    /// Check if current token is a specific keyword
    pub(super) fn is_keyword(&self, keyword: Keyword) -> bool {
        self.current_keyword() == Some(keyword)
    }

    /// Expect a specific keyword
    pub(super) fn expect_keyword(&mut self, keyword: Keyword) -> Result<()> {
        if self.is_keyword(keyword) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!("expected '{}'", keyword)))
        }
    }
    pub(super) fn parse_simple_command(&mut self) -> Result<Option<SimpleCommand>> {
        self.tick()?;
        self.skip_newlines()?;
        self.check_error_token()?;
        let start_span = self.current_span;

        let mut assignments = SmallVec::<[Assignment; 1]>::new();
        let mut words = SmallVec::<[Word; 2]>::new();
        let mut redirects = SmallVec::<[Redirect; 1]>::new();

        loop {
            self.check_error_token()?;
            let next_kind_after_right_brace = if self.at(TokenKind::RightBrace) {
                self.peek_next_kind()
            } else {
                None
            };
            let right_brace_is_literal_argument = self.at(TokenKind::RightBrace)
                && !words.is_empty()
                && self.should_consume_right_brace_as_literal_argument(next_kind_after_right_brace);
            let has_simple_command_prefix =
                !words.is_empty() || !assignments.is_empty() || !redirects.is_empty();
            match self.current_token_kind {
                Some(kind) if kind.is_word_like() => {
                    // Bail out before touching word text when the token is a
                    // reserved word that cannot begin a simple command.
                    if words.is_empty()
                        && self
                            .current_keyword()
                            .is_some_and(Self::is_non_command_keyword)
                    {
                        break;
                    }

                    let is_literal = kind == TokenKind::LiteralWord;
                    let word_text =
                        self.current_source_like_word_text_or_error("simple command word")?;
                    let allow_zsh_numeric_assignments =
                        self.dialect.features().zsh_parameter_modifiers;
                    let assignment_shape = (!is_literal && words.is_empty()).then(|| {
                        Self::is_assignment(word_text.as_ref(), allow_zsh_numeric_assignments)
                    });
                    let assignment_shape = assignment_shape.flatten();

                    // Check for assignment (only before the command name, not for literal words)
                    if words.is_empty()
                        && !is_literal
                        && let Some((assignment, needs_advance)) = self
                            .try_parse_assignment_with_shape(word_text.as_ref(), assignment_shape)
                    {
                        if needs_advance {
                            self.advance();
                        }
                        assignments.push(assignment);
                        continue;
                    }

                    if words.is_empty()
                        && !is_literal
                        && assignment_shape.is_none()
                        && word_text.contains('[')
                        && let Some(assignment) =
                            self.try_parse_split_indexed_assignment_from_text()
                    {
                        assignments.push(assignment);
                        continue;
                    }

                    // Handle compound array assignment in arg position:
                    // declare -a arr=(x y z) → arr=(x y z) as single arg
                    if word_text.ends_with('=') && !words.is_empty() {
                        let original_word = self.current_word_ref().cloned();
                        let saved_span = self.current_span;
                        self.advance();
                        if let Some(word) =
                            self.try_parse_compound_array_arg(word_text.as_ref(), saved_span)?
                        {
                            words.push(word);
                            continue;
                        }
                        // Not a compound assignment — treat as regular word
                        if let Some(word) = original_word {
                            words.push(word);
                        }
                        continue;
                    }

                    if let Some(word) = self.take_current_word_and_advance() {
                        words.push(word);
                    }
                }
                Some(TokenKind::LeftParen) if !words.is_empty() => {
                    let Some(word) = self.take_current_word_and_advance() else {
                        break;
                    };
                    words.push(word);
                }
                Some(TokenKind::DoubleLeftBracket) if has_simple_command_prefix => {
                    let span = self.current_span;
                    let word = self.word_from_raw_text(span.slice(self.input), span);
                    self.advance();
                    words.push(word);
                }
                Some(TokenKind::DoubleRightBracket) if has_simple_command_prefix => {
                    let span = self.current_span;
                    let word = self.word_from_raw_text(span.slice(self.input), span);
                    self.advance();
                    words.push(word);
                }
                Some(TokenKind::Newline) => {
                    let next_kind = self.peek_next_kind();
                    let supports_fd_var = next_kind.is_some_and(|kind| {
                        matches!(kind, TokenKind::HereDoc | TokenKind::HereDocStrip)
                            || Self::redirect_supports_fd_var(kind)
                    });
                    if supports_fd_var {
                        let (fd_var, fd_var_span) = self.pop_line_continuation_fd_var(&mut words);
                        if let Some(fd_var) = fd_var {
                            self.advance();
                            if matches!(
                                self.current_token_kind,
                                Some(TokenKind::HereDoc | TokenKind::HereDocStrip)
                            ) {
                                self.parse_heredoc_redirect(
                                    self.current_token_kind == Some(TokenKind::HereDocStrip),
                                    &mut redirects,
                                    Some(fd_var),
                                    fd_var_span,
                                )?;
                                continue;
                            }

                            if self.consume_non_heredoc_redirect(
                                &mut redirects,
                                Some(fd_var),
                                fd_var_span,
                                true,
                            )? {
                                continue;
                            }
                        }
                    }
                    break;
                }
                Some(kind) if Self::is_redirect_kind(kind) => {
                    if matches!(kind, TokenKind::HereDoc | TokenKind::HereDocStrip) {
                        let (fd_var, fd_var_span) = if words
                            .last()
                            .is_some_and(|word| self.word_is_attached_to_current_token(word))
                        {
                            self.pop_fd_var(&mut words)
                        } else {
                            (None, None)
                        };
                        self.parse_heredoc_redirect(
                            kind == TokenKind::HereDocStrip,
                            &mut redirects,
                            fd_var,
                            fd_var_span,
                        )?;
                        continue;
                    }

                    let (fd_var, fd_var_span) = if Self::redirect_supports_fd_var(kind) {
                        if words
                            .last()
                            .is_some_and(|word| self.word_is_attached_to_current_token(word))
                        {
                            self.pop_fd_var(&mut words)
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                    if self.consume_non_heredoc_redirect(
                        &mut redirects,
                        fd_var,
                        fd_var_span,
                        true,
                    )? {
                        continue;
                    }
                    break;
                }
                Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                    let word = self.expect_word()?;
                    words.push(word);
                }
                // `{` can appear as a literal argument outside command position.
                Some(TokenKind::LeftBrace) if !words.is_empty() => {
                    words.push(Word::literal_with_span("{", self.current_span));
                    self.advance();
                }
                // Inside brace groups, a bare `}` can still be a literal
                // argument like `echo }`, but only when it's separated from the
                // preceding token by whitespace. Closers are handled in command
                // position by the enclosing brace parser.
                Some(TokenKind::RightBrace) if right_brace_is_literal_argument => {
                    words.push(Word::literal_with_span("}", self.current_span));
                    self.advance();
                }
                Some(TokenKind::Semicolon)
                | Some(TokenKind::Pipe)
                | Some(TokenKind::And)
                | Some(TokenKind::Or)
                | None => break,
                _ => break,
            }
        }

        // Handle assignment-only or redirect-only commands with no command word.
        if words.is_empty() && (!assignments.is_empty() || !redirects.is_empty()) {
            return Ok(Some(SimpleCommand {
                name: Word::literal(""),
                args: SmallVec::new(),
                redirects,
                assignments,
                span: start_span.merge(self.current_span),
            }));
        }

        if words.is_empty() {
            return Ok(None);
        }

        let name = words.remove(0);
        let args = words;

        Ok(Some(SimpleCommand {
            name,
            args,
            redirects,
            assignments,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Extract fd-variable name from `{varname}` pattern in the last word.
    /// If the last word is a single literal `{identifier}`, pop it and return the name.
    /// Used for `exec {var}>file` / `exec {var}>&-` syntax.
    pub(super) fn pop_fd_var(
        &self,
        words: &mut SmallVec<[Word; 2]>,
    ) -> (Option<Name>, Option<Span>) {
        if let Some(last) = words.last()
            && last.parts.len() == 1
            && let WordPart::Literal(ref s) = last.parts[0].kind
            && let Some(span) = last.part_span(0)
            && let text = s.as_str(self.input, span)
            && text.starts_with('{')
            && text.ends_with('}')
            && text.len() > 2
            && text[1..text.len() - 1]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
        {
            let var_name = text[1..text.len() - 1].to_string();
            let start = last.span.start.advanced_by("{");
            let span = Span::from_positions(start, start.advanced_by(&var_name));
            words.pop();
            return (Some(Name::from(var_name)), Some(span));
        }
        (None, None)
    }

    pub(super) fn word_is_attached_to_current_token(&self, word: &Word) -> bool {
        let start = word.span.end.offset;
        let end = self.current_span.start.offset;
        let input_len = self.input.len();
        start <= end
            && end <= input_len
            && Self::fd_var_gap_allows_attachment(&self.input[start..end])
    }

    pub(super) fn pop_line_continuation_fd_var(
        &self,
        words: &mut SmallVec<[Word; 2]>,
    ) -> (Option<Name>, Option<Span>) {
        let Some(last) = words.last() else {
            return (None, None);
        };
        let Some(text) = self.single_literal_word_text(last) else {
            return (None, None);
        };
        let Some(fd_text) = text.strip_suffix('\\') else {
            return (None, None);
        };
        let Some((fd_var, fd_var_span)) = Self::fd_var_from_text(fd_text, last.span) else {
            return (None, None);
        };
        words.pop();
        (Some(fd_var), Some(fd_var_span))
    }
}
