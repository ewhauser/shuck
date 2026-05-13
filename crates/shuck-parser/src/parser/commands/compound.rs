use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_time(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.advance(); // consume 'time'
        self.skip_newlines()?;

        // Check for -p flag (POSIX format)
        let posix_format = if self.at(TokenKind::Word) && self.current_word_str() == Some("-p") {
            self.advance();
            self.skip_newlines()?;
            true
        } else {
            false
        };

        // Parse the command to time (if any)
        // time with no command is valid in bash (just outputs timing header)
        let command = self.parse_pipeline()?.map(Box::new);

        Ok(CompoundCommand::Time(TimeCommand {
            posix_format,
            command,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse a coproc command: `coproc [NAME] command`
    ///
    /// If the token after `coproc` is a simple word followed by a compound
    /// command (`{`, `(`, `while`, `for`, etc.), it is treated as the coproc
    /// name. Otherwise the command starts immediately and the default name
    /// "COPROC" is used.
    pub(super) fn parse_coproc(&mut self) -> Result<CompoundCommand> {
        self.ensure_coproc()?;
        let start_span = self.current_span;
        self.advance(); // consume 'coproc'
        self.skip_newlines()?;

        // Determine if next token is a NAME (simple word that is NOT a compound-
        // command keyword and is followed by a compound command start).
        let (name, name_span) = if self.at(TokenKind::Word) {
            if let Some(word) = self.current_word_str() {
                let word = word.to_string();
                let word_span = self.current_span;
                let is_compound_keyword = matches!(
                    word.as_str(),
                    "if" | "for" | "while" | "until" | "case" | "select" | "time" | "coproc"
                );
                let next_is_compound_start = matches!(
                    self.peek_next_kind(),
                    Some(TokenKind::LeftBrace | TokenKind::LeftParen)
                );
                if !is_compound_keyword && next_is_compound_start {
                    self.advance(); // consume the NAME
                    self.skip_newlines()?;
                    (Name::from(word), Some(word_span))
                } else {
                    (Name::new_static("COPROC"), None)
                }
            } else {
                (Name::new_static("COPROC"), None)
            }
        } else {
            (Name::new_static("COPROC"), None)
        };

        // Parse the command body (could be simple, compound, or pipeline)
        let body = self.parse_pipeline()?;
        let body = body.ok_or_else(|| self.error("coproc: missing command"))?;

        Ok(CompoundCommand::Coproc(CoprocCommand {
            name,
            name_span,
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Check if current token is ;; (case terminator)
    pub(super) fn is_case_terminator(&self) -> bool {
        matches!(
            self.current_token_kind,
            Some(TokenKind::DoubleSemicolon | TokenKind::SemiAmp | TokenKind::DoubleSemiAmp)
        ) || (self.dialect == ShellDialect::Zsh
            && self.current_token_kind == Some(TokenKind::SemiPipe))
    }

    /// Parse case terminator: `;;` (break), `;&` (fallthrough),
    /// `;;&` / `;|` (continue matching)
    pub(super) fn parse_case_terminator(&mut self) -> (CaseTerminator, Option<Span>) {
        match self.current_token_kind {
            Some(TokenKind::SemiAmp) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::FallThrough, Some(span))
            }
            Some(TokenKind::SemiPipe) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::ContinueMatching, Some(span))
            }
            Some(TokenKind::DoubleSemiAmp) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::Continue, Some(span))
            }
            Some(TokenKind::DoubleSemicolon) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::Break, Some(span))
            }
            _ => (CaseTerminator::Break, None),
        }
    }

    /// Parse a subshell (commands in parentheses)
    pub(super) fn parse_subshell(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        self.advance(); // consume '('
        self.skip_newlines()?;

        let body_start = self.current_span.start;
        let mut commands = Vec::new();
        while !matches!(
            self.current_token_kind,
            Some(TokenKind::RightParen | TokenKind::DoubleRightParen) | None
        ) {
            self.skip_newlines()?;
            if matches!(
                self.current_token_kind,
                Some(TokenKind::RightParen | TokenKind::DoubleRightParen)
            ) {
                break;
            }
            commands.extend(self.parse_command_list_required()?);
        }

        if self.at(TokenKind::DoubleRightParen) {
            // `))` at end of nested subshells: consume as single `)`, leave `)` for parent
            self.set_current_kind(TokenKind::RightParen, self.current_span);
        } else if !self.at(TokenKind::RightParen) {
            self.pop_depth();
            return Err(Error::parse("expected ')' to close subshell".to_string()));
        } else {
            self.advance(); // consume ')'
        }

        self.pop_depth();
        Ok(CompoundCommand::Subshell(Self::stmt_seq_with_span(
            Span::from_positions(body_start, self.current_span.start),
            commands,
        )))
    }

    /// Parse a brace group
    pub(super) fn parse_brace_group(
        &mut self,
        context: BraceBodyContext,
    ) -> Result<CompoundCommand> {
        self.push_depth()?;
        let (body, left_brace_span, right_brace_span) =
            self.parse_brace_enclosed_stmt_seq("syntax error: empty brace group", context)?;

        let always_span = self.peek_zsh_always_span();
        if let Some(span) = always_span {
            self.record_zsh_always_span(span);
        }

        let compound = if self.dialect.features().zsh_always && self.is_keyword(Keyword::Always) {
            self.record_zsh_always_span(self.current_span);
            self.advance();
            self.skip_newlines()?;
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after always"));
            }
            let (always_body, _, always_right_brace_span) = self.parse_brace_enclosed_stmt_seq(
                "syntax error: empty always clause",
                BraceBodyContext::Ordinary,
            )?;
            CompoundCommand::Always(AlwaysCommand {
                body,
                always_body,
                span: left_brace_span.merge(always_right_brace_span),
            })
        } else {
            let _ = right_brace_span;
            CompoundCommand::BraceGroup(body)
        };

        self.pop_depth();
        Ok(compound)
    }

    pub(super) fn parse_brace_enclosed_stmt_seq(
        &mut self,
        empty_error: &str,
        context: BraceBodyContext,
    ) -> Result<(StmtSeq, Span, Span)> {
        let left_brace_span = self.current_span;
        self.advance();
        self.brace_group_depth += 1;
        self.brace_body_stack.push(context);
        self.skip_command_separators()?;

        let body_start = self.current_span.start;
        let mut commands = Vec::new();
        while !matches!(self.current_token_kind, Some(TokenKind::RightBrace) | None) {
            self.skip_command_separators()?;
            if self.at(TokenKind::RightBrace) {
                break;
            }
            commands.extend(self.parse_command_list_required()?);
        }

        if !self.at(TokenKind::RightBrace) {
            self.brace_body_stack.pop();
            self.brace_group_depth -= 1;
            return Err(Error::parse(
                "expected '}' to close brace group".to_string(),
            ));
        }

        if commands.is_empty()
            && !(self.dialect == ShellDialect::Zsh && matches!(context, BraceBodyContext::Function))
        {
            self.brace_body_stack.pop();
            self.brace_group_depth -= 1;
            return Err(self.error(empty_error));
        }

        let right_brace_span = self.current_span;
        self.advance();
        self.brace_body_stack.pop();
        self.brace_group_depth -= 1;
        Ok((
            Self::stmt_seq_with_span(
                Span::from_positions(body_start, right_brace_span.start),
                commands,
            ),
            left_brace_span,
            right_brace_span,
        ))
    }

    pub(super) fn parse_if_condition_until_body_start(
        &mut self,
        allow_brace_body: bool,
    ) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::with_capacity(2);

        loop {
            self.skip_newlines()?;

            if !allow_brace_body
                && !stmts.is_empty()
                && self.current_brace_starts_zsh_if_body_fact()
            {
                self.record_zsh_brace_if_span(self.current_span);
            }

            if self.at(TokenKind::Semicolon) {
                let checkpoint = self.checkpoint();
                self.advance();
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                let brace_if_span = (!allow_brace_body
                    && !stmts.is_empty()
                    && self.current_brace_starts_zsh_if_body_fact())
                .then_some(self.current_span);
                if self.is_keyword(Keyword::Then)
                    || (allow_brace_body && !stmts.is_empty() && self.at(TokenKind::LeftBrace))
                {
                    if let Some(span) = brace_if_span {
                        self.record_zsh_brace_if_span(span);
                    }
                    break;
                }
                self.restore(checkpoint);
                if let Some(span) = brace_if_span {
                    self.record_zsh_brace_if_span(span);
                }
            }

            if self.is_keyword(Keyword::Then)
                || (allow_brace_body
                    && !stmts.is_empty()
                    && (self.at(TokenKind::LeftBrace)
                        || self.current_token_is_compact_zsh_brace_body()))
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

    pub(super) fn current_brace_starts_zsh_if_body_fact(&mut self) -> bool {
        if !self.at(TokenKind::LeftBrace) {
            return false;
        }

        let checkpoint = self.checkpoint();
        let reaches_then = loop {
            if self.skip_newlines().is_err() {
                break false;
            }
            if self.is_keyword(Keyword::Then) {
                break true;
            }
            if self.current_token.is_none() {
                break false;
            }
            if self.parse_command_list_required().is_err() {
                break false;
            }
        };
        self.restore(checkpoint);

        !reaches_then
    }

    pub(super) fn parse_loop_condition_until_body_start(
        &mut self,
        allow_brace_body: bool,
    ) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::with_capacity(2);

        loop {
            self.skip_newlines()?;

            if self.is_keyword(Keyword::Do)
                || (allow_brace_body
                    && !stmts.is_empty()
                    && (self.at(TokenKind::LeftBrace)
                        || self.current_token_is_compact_zsh_brace_body())
                    && self.current_brace_starts_zsh_loop_body_fact())
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

    pub(super) fn current_brace_starts_zsh_loop_body_fact(&mut self) -> bool {
        let checkpoint = self.checkpoint();
        let reaches_do = loop {
            if self.skip_newlines().is_err() {
                break false;
            }
            if self.is_keyword(Keyword::Do) {
                break true;
            }
            if self.current_token.is_none() {
                break false;
            }
            if self.parse_command_list_required().is_err() {
                break false;
            }
        };
        self.restore(checkpoint);

        !reaches_do
    }

    pub(super) fn current_token_is_compact_zsh_brace_body(&mut self) -> bool {
        self.current_source_like_word_text()
            .is_some_and(|text| text.starts_with('{') && text.ends_with('}'))
    }

    pub(super) fn peek_zsh_always_span(&mut self) -> Option<Span> {
        if !self.is_keyword(Keyword::Always) {
            return None;
        }

        let always_span = self.current_span;
        let checkpoint = self.checkpoint();
        self.advance();
        let result = match self.skip_newlines() {
            Ok(()) if self.at(TokenKind::LeftBrace) => Some(always_span),
            Ok(()) => None,
            Err(_) => None,
        };
        self.restore(checkpoint);
        result
    }

    pub(super) fn has_recorded_comment_between(
        &self,
        start_offset: usize,
        end_offset: usize,
    ) -> bool {
        self.comments.iter().any(|comment| {
            let comment_start = usize::from(comment.range.start());
            comment_start >= start_offset && comment_start < end_offset
        })
    }

    pub(super) fn rebase_nested_parse_error(&self, error: Error, base: Position) -> Error {
        let Error::Parse {
            message,
            line,
            column,
        } = error;

        if line == 0 {
            return Error::parse(message);
        }

        let rebased_line = base.line + line.saturating_sub(1);
        let rebased_column = if line == 1 {
            base.column + column.saturating_sub(1)
        } else {
            column
        };

        Error::parse_at(message, rebased_line, rebased_column)
    }

    pub(super) fn try_parse_compact_function_brace_body(
        &mut self,
    ) -> Result<Option<CompoundCommand>> {
        if self.dialect != ShellDialect::Zsh
            || !self.zsh_short_loops_enabled()
            || !self.at_word_like()
        {
            return Ok(None);
        }

        let Some(body_text) = self.current_source_like_word_text() else {
            return Ok(None);
        };
        let body_text = body_text.into_owned();
        let Some(inner) = body_text
            .strip_prefix('{')
            .and_then(|body| body.strip_suffix('}'))
        else {
            return Ok(None);
        };

        if !inner.is_empty()
            && !inner.chars().any(|ch| {
                matches!(
                    ch,
                    ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '$' | '"' | '\'' | '(' | ')'
                )
            })
        {
            return Ok(None);
        }

        let nested_profile = self
            .current_zsh_options()
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let mut nested =
            Parser::with_limits_and_profile(inner, self.max_depth, self.max_fuel, nested_profile);
        nested.aliases = self.aliases.clone();
        nested.expand_aliases = self.expand_aliases;
        nested.expand_next_word = self.expand_next_word;

        let inner_start = self.current_span.start.advanced_by("{");
        let mut output = nested.parse();
        if output.is_err() {
            return Err(self.rebase_nested_parse_error(output.strict_error(), inner_start));
        }
        Self::rebase_stmt_seq(&mut output.file.body, inner_start);
        self.advance();
        Ok(Some(CompoundCommand::BraceGroup(output.file.body)))
    }

    pub(super) fn try_parse_compact_zsh_brace_body(
        &mut self,
        context: BraceBodyContext,
    ) -> Result<Option<(StmtSeq, Span, Span)>> {
        if self.dialect != ShellDialect::Zsh
            || !self.zsh_brace_bodies_enabled()
            || !self.at_word_like()
        {
            return Ok(None);
        }

        let Some(body_text) = self.current_source_like_word_text() else {
            return Ok(None);
        };
        let body_text = body_text.into_owned();
        let Some(inner) = body_text
            .strip_prefix('{')
            .and_then(|body| body.strip_suffix('}'))
        else {
            return Ok(None);
        };

        if !inner.is_empty()
            && !inner.chars().any(|ch| {
                matches!(
                    ch,
                    ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '$' | '"' | '\'' | '(' | ')'
                )
            })
        {
            return Ok(None);
        }

        let nested_profile = self
            .current_zsh_options()
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let mut nested =
            Parser::with_limits_and_profile(inner, self.max_depth, self.max_fuel, nested_profile);
        nested.aliases = self.aliases.clone();
        nested.expand_aliases = self.expand_aliases;
        nested.expand_next_word = self.expand_next_word;

        let word_span = self.current_span;
        let inner_start = word_span.start.advanced_by("{");
        let mut output = nested.parse();
        if output.is_err() {
            return Err(self.rebase_nested_parse_error(output.strict_error(), inner_start));
        }
        Self::rebase_stmt_seq(&mut output.file.body, inner_start);

        let left_brace_span = Span::from_positions(word_span.start, inner_start);
        let right_brace_start = word_span
            .start
            .advanced_by(&body_text[..body_text.len() - 1]);
        let right_brace_span = Span::from_positions(right_brace_start, word_span.end);
        let body = Self::stmt_seq_with_span(
            Span::from_positions(inner_start, right_brace_start),
            output.file.body.stmts,
        );

        if body.is_empty()
            && !(self.dialect == ShellDialect::Zsh && matches!(context, BraceBodyContext::Function))
        {
            let message = match context {
                BraceBodyContext::Function => "syntax error: empty brace group",
                BraceBodyContext::IfClause => "syntax error: empty then clause",
                BraceBodyContext::Ordinary => "syntax error: empty brace group",
            };
            return Err(self.error(message));
        }

        self.advance();
        Ok(Some((body, left_brace_span, right_brace_span)))
    }

    pub(super) fn should_consume_right_brace_as_literal_argument(
        &mut self,
        next_kind_after_right_brace: Option<TokenKind>,
    ) -> bool {
        if !self.current_token_has_leading_whitespace() {
            return false;
        }

        if self.brace_group_depth == 0 {
            return true;
        }

        if self.dialect != ShellDialect::Zsh {
            return true;
        }

        next_kind_after_right_brace == Some(TokenKind::Semicolon)
            && self.current_token_is_tight_to_next_token()
            && self.next_token_after_tight_semicolon_is(TokenKind::RightBrace)
    }

    pub(super) fn next_token_after_tight_semicolon_is(&mut self, expected: TokenKind) -> bool {
        let checkpoint = self.checkpoint();
        self.advance();
        if !self.at(TokenKind::Semicolon) {
            self.restore(checkpoint);
            return false;
        }
        self.advance();
        let result = self.at(expected);
        self.restore(checkpoint);
        result
    }
}
