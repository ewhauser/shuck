use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_conditional(&mut self) -> Result<CompoundCommand> {
        self.ensure_double_bracket()?;
        let left_bracket_span = self.current_span;
        self.advance(); // consume '[['
        self.skip_conditional_newlines();

        let expression = self.parse_conditional_or(false)?;
        self.skip_conditional_newlines();

        let right_bracket_span = match self.current_token_kind {
            Some(TokenKind::DoubleRightBracket) => {
                let span = self.current_span;
                self.advance(); // consume ']]'
                span
            }
            None => {
                return Err(crate::error::Error::parse(
                    "unexpected end of input in [[ ]]".to_string(),
                ));
            }
            _ => return Err(self.error("expected ']]' to close conditional expression")),
        };

        Ok(CompoundCommand::Conditional(ConditionalCommand {
            expression,
            span: left_bracket_span.merge(right_bracket_span),
            left_bracket_span,
            right_bracket_span,
        }))
    }

    pub(super) fn skip_conditional_newlines(&mut self) {
        while self.at(TokenKind::Newline) {
            self.advance();
        }
    }

    pub(super) fn parse_conditional_or(
        &mut self,
        stop_at_right_paren: bool,
    ) -> Result<ConditionalExpr> {
        let mut expr = self.parse_conditional_and(stop_at_right_paren)?;

        loop {
            self.skip_conditional_newlines();
            if !self.at(TokenKind::Or) {
                break;
            }

            let op_span = self.current_span;
            self.advance();
            let right = self.parse_conditional_and(stop_at_right_paren)?;
            expr = ConditionalExpr::Binary(ConditionalBinaryExpr {
                left: Box::new(expr),
                op: ConditionalBinaryOp::Or,
                op_span,
                right: Box::new(right),
            });
        }

        Ok(expr)
    }

    pub(super) fn parse_conditional_and(
        &mut self,
        stop_at_right_paren: bool,
    ) -> Result<ConditionalExpr> {
        let mut expr = self.parse_conditional_term(stop_at_right_paren)?;

        loop {
            self.skip_conditional_newlines();
            if !self.at(TokenKind::And) {
                break;
            }

            let op_span = self.current_span;
            self.advance();
            let right = self.parse_conditional_term(stop_at_right_paren)?;
            expr = ConditionalExpr::Binary(ConditionalBinaryExpr {
                left: Box::new(expr),
                op: ConditionalBinaryOp::And,
                op_span,
                right: Box::new(right),
            });
        }

        Ok(expr)
    }

    pub(super) fn parse_conditional_term(
        &mut self,
        stop_at_right_paren: bool,
    ) -> Result<ConditionalExpr> {
        self.skip_conditional_newlines();

        if let Some(op) = self.current_conditional_unary_op() {
            let op_span = self.current_span;
            self.advance();
            self.skip_conditional_newlines();

            let expr = if matches!(op, ConditionalUnaryOp::Not) {
                self.parse_conditional_term(stop_at_right_paren)?
            } else {
                if matches!(
                    op,
                    ConditionalUnaryOp::VariableSet | ConditionalUnaryOp::ReferenceVariable
                ) {
                    let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                    self.conditional_var_ref_expr(word)
                } else {
                    let word = self.parse_conditional_operand_word()?;
                    ConditionalExpr::Word(word)
                }
            };

            return Ok(ConditionalExpr::Unary(ConditionalUnaryExpr {
                op,
                op_span,
                expr: Box::new(expr),
            }));
        }

        if self.at(TokenKind::DoubleLeftParen) {
            if self.dialect == ShellDialect::Zsh {
                let left_paren_span = self.current_span;
                if let Some(right_paren_span) = self.scan_arithmetic_command_close(left_paren_span)
                {
                    let span = left_paren_span.merge(right_paren_span);
                    let text = span.slice(self.input).to_string();
                    while self.current_token.is_some()
                        && self.current_span.start.offset < right_paren_span.end.offset
                    {
                        self.advance();
                    }
                    return Ok(ConditionalExpr::Word(
                        self.parse_word_with_context(&text, span, span.start, true),
                    ));
                }
            }
            self.split_current_double_left_paren();
        }

        let left = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();
            let expr = self.parse_conditional_or(true)?;
            self.skip_conditional_newlines();
            if self.at(TokenKind::DoubleRightParen) {
                self.split_current_double_right_paren();
            }
            if !self.at(TokenKind::RightParen) {
                return Err(self.error("expected ')' in conditional expression"));
            }
            let right_paren_span = self.current_span;
            self.advance();
            ConditionalExpr::Parenthesized(ConditionalParenExpr {
                left_paren_span,
                expr: Box::new(expr),
                right_paren_span,
            })
        } else {
            ConditionalExpr::Word(self.parse_conditional_operand_word()?)
        };

        self.skip_conditional_newlines();

        let Some(op) = self.current_conditional_comparison_op() else {
            return Ok(left);
        };

        let op_span = self.current_span;
        self.advance();
        self.skip_conditional_newlines();

        let right = match op {
            ConditionalBinaryOp::RegexMatch => {
                if self.at(TokenKind::LeftBrace) {
                    return Err(self.error("expected conditional operand"));
                }
                ConditionalExpr::Regex(self.collect_conditional_context_word(stop_at_right_paren)?)
            }
            ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe => {
                let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                ConditionalExpr::Pattern(self.pattern_from_conditional_word(&word))
            }
            _ => ConditionalExpr::Word(self.parse_conditional_operand_word()?),
        };

        Ok(ConditionalExpr::Binary(ConditionalBinaryExpr {
            left: Box::new(left),
            op,
            op_span,
            right: Box::new(right),
        }))
    }

    pub(super) fn parse_conditional_operand_word(&mut self) -> Result<Word> {
        self.skip_conditional_newlines();

        if let Some(word) = self.current_conditional_source_word(false) {
            self.advance_past_word(&word);
            self.restore_conditional_source_delimiter(word.span.end, false);
            return Ok(word);
        }

        if let Some(word) = self.take_current_word_and_advance() {
            return Ok(word);
        }

        let Some(word) = self.current_conditional_literal_word() else {
            return Err(self.error("expected conditional operand"));
        };
        self.advance_past_word(&word);
        Ok(word)
    }

    pub(super) fn conditional_var_ref_expr(&self, word: Word) -> ConditionalExpr {
        self.parse_var_ref_from_word(&word, SubscriptInterpretation::Contextual)
            .map(Box::new)
            .map(ConditionalExpr::VarRef)
            .unwrap_or(ConditionalExpr::Word(word))
    }

    pub(super) fn current_conditional_source_word(
        &mut self,
        stop_at_right_paren: bool,
    ) -> Option<Word> {
        let token = self.current_token.as_ref()?;
        if token.flags.is_synthetic() {
            return None;
        }

        if matches!(
            self.current_token_kind,
            Some(TokenKind::QuotedWord | TokenKind::LiteralWord)
        ) {
            return None;
        }

        let starts_with_paren = matches!(self.current_token_kind, Some(TokenKind::LeftParen));
        let starts_with_zsh_pattern_punct = matches!(
            self.current_token_kind,
            Some(TokenKind::RedirectIn | TokenKind::RedirectOut | TokenKind::RedirectReadWrite)
        ) && self.dialect == ShellDialect::Zsh;

        if !starts_with_paren
            && !starts_with_zsh_pattern_punct
            && !self.current_token_kind.is_some_and(TokenKind::is_word_like)
        {
            return None;
        }

        let start = self.current_span.start;
        let (text, end) =
            self.scan_conditional_source_word(start, stop_at_right_paren, starts_with_paren)?;
        let span = Span::from_positions(start, end);
        Some(self.parse_word_with_context(&text, span, start, true))
    }

    pub(super) fn scan_conditional_source_word(
        &self,
        start: Position,
        stop_at_right_paren: bool,
        starts_with_paren: bool,
    ) -> Option<(String, Position)> {
        if start.offset >= self.input.len() {
            return None;
        }

        let mut cursor = start;
        let mut text = String::new();
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut bracket_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut prev_char = None;

        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];
            if !in_single
                && !in_double
                && !in_backtick
                && !escaped
                && paren_depth == 0
                && brace_depth == 0
                && bracket_depth == 0
            {
                if rest.starts_with("]]")
                    || rest.starts_with("&&")
                    || rest.starts_with("||")
                    || (!starts_with_paren && rest.starts_with(')'))
                    || (stop_at_right_paren && rest.starts_with(')'))
                {
                    break;
                }

                let ch = rest.chars().next()?;
                if matches!(ch, ' ' | '\t' | '\n' | ';') {
                    break;
                }
            }

            let ch = self.input[cursor.offset..].chars().next()?;
            cursor.advance(ch);
            text.push(ch);

            if escaped {
                escaped = false;
                prev_char = Some(ch);
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '`' if !in_single => in_backtick = !in_backtick,
                '(' if !in_single && !in_double && brace_depth == 0 => paren_depth += 1,
                ')' if !in_single && !in_double && brace_depth == 0 && paren_depth > 0 => {
                    paren_depth -= 1
                }
                '{' if !in_single && !in_double && (brace_depth > 0 || prev_char == Some('$')) => {
                    brace_depth += 1
                }
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double && bracket_depth > 0 => bracket_depth -= 1,
                _ => {}
            }

            prev_char = Some(ch);
        }

        (!text.is_empty()).then_some((text, cursor))
    }

    pub(super) fn conditional_source_delimiter_after(
        &self,
        end: Position,
        stop_at_right_paren: bool,
    ) -> Option<(TokenKind, Span)> {
        let mut cursor = end;
        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];
            let ch = rest.chars().next()?;
            if matches!(ch, ' ' | '\t') {
                cursor.advance(ch);
                continue;
            }
            break;
        }

        let rest = self.input.get(cursor.offset..)?;
        let (kind, text) = if rest.starts_with("]]") {
            (TokenKind::DoubleRightBracket, "]]")
        } else if rest.starts_with("&&") {
            (TokenKind::And, "&&")
        } else if rest.starts_with("||") {
            (TokenKind::Or, "||")
        } else if stop_at_right_paren && rest.starts_with(')') {
            (TokenKind::RightParen, ")")
        } else {
            return None;
        };

        Some((kind, Span::from_positions(cursor, cursor.advanced_by(text))))
    }

    pub(super) fn restore_conditional_source_delimiter(
        &mut self,
        end: Position,
        stop_at_right_paren: bool,
    ) {
        let Some((kind, span)) = self.conditional_source_delimiter_after(end, stop_at_right_paren)
        else {
            return;
        };

        if self.current_token_kind == Some(kind) && self.current_span == span {
            return;
        }

        if let Some(current_kind) = self.current_token_kind
            && matches!(
                current_kind,
                TokenKind::Newline
                    | TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::RightParen
                    | TokenKind::DoubleRightBracket
            )
        {
            self.synthetic_tokens
                .push_front(SyntheticToken::punctuation(current_kind, self.current_span));
        }

        self.set_current_kind(kind, span);
    }

    pub(super) fn current_conditional_unary_op(&self) -> Option<ConditionalUnaryOp> {
        if !self.at(TokenKind::Word) {
            return None;
        }
        let word = self.current_word_str()?;

        Some(match word {
            "!" => ConditionalUnaryOp::Not,
            "-e" | "-a" => ConditionalUnaryOp::Exists,
            "-f" => ConditionalUnaryOp::RegularFile,
            "-d" => ConditionalUnaryOp::Directory,
            "-c" => ConditionalUnaryOp::CharacterSpecial,
            "-b" => ConditionalUnaryOp::BlockSpecial,
            "-p" => ConditionalUnaryOp::NamedPipe,
            "-S" => ConditionalUnaryOp::Socket,
            "-L" | "-h" => ConditionalUnaryOp::Symlink,
            "-k" => ConditionalUnaryOp::Sticky,
            "-g" => ConditionalUnaryOp::SetGroupId,
            "-u" => ConditionalUnaryOp::SetUserId,
            "-G" => ConditionalUnaryOp::GroupOwned,
            "-O" => ConditionalUnaryOp::UserOwned,
            "-N" => ConditionalUnaryOp::Modified,
            "-r" => ConditionalUnaryOp::Readable,
            "-w" => ConditionalUnaryOp::Writable,
            "-x" => ConditionalUnaryOp::Executable,
            "-s" => ConditionalUnaryOp::NonEmptyFile,
            "-t" => ConditionalUnaryOp::FdTerminal,
            "-z" => ConditionalUnaryOp::EmptyString,
            "-n" => ConditionalUnaryOp::NonEmptyString,
            "-o" => ConditionalUnaryOp::OptionSet,
            "-v" => ConditionalUnaryOp::VariableSet,
            "-R" => ConditionalUnaryOp::ReferenceVariable,
            _ => return None,
        })
    }

    pub(super) fn current_conditional_comparison_op(&self) -> Option<ConditionalBinaryOp> {
        match self.current_token_kind? {
            TokenKind::Word => Some(match self.current_word_str()? {
                "=" => ConditionalBinaryOp::PatternEqShort,
                "==" => ConditionalBinaryOp::PatternEq,
                "!=" => ConditionalBinaryOp::PatternNe,
                "=~" => ConditionalBinaryOp::RegexMatch,
                "-nt" => ConditionalBinaryOp::NewerThan,
                "-ot" => ConditionalBinaryOp::OlderThan,
                "-ef" => ConditionalBinaryOp::SameFile,
                "-eq" => ConditionalBinaryOp::ArithmeticEq,
                "-ne" => ConditionalBinaryOp::ArithmeticNe,
                "-le" => ConditionalBinaryOp::ArithmeticLe,
                "-ge" => ConditionalBinaryOp::ArithmeticGe,
                "-lt" => ConditionalBinaryOp::ArithmeticLt,
                "-gt" => ConditionalBinaryOp::ArithmeticGt,
                _ => return None,
            }),
            TokenKind::RedirectIn => Some(ConditionalBinaryOp::LexicalBefore),
            TokenKind::RedirectOut => Some(ConditionalBinaryOp::LexicalAfter),
            _ => None,
        }
    }

    pub(super) fn collect_conditional_context_word(
        &mut self,
        stop_at_right_paren: bool,
    ) -> Result<Word> {
        self.skip_conditional_newlines();

        if let Some(word) = self.current_conditional_source_word(stop_at_right_paren) {
            self.advance_past_word(&word);
            self.restore_conditional_source_delimiter(word.span.end, stop_at_right_paren);
            return Ok(word);
        }

        let mut first_word: Option<Word> = None;
        let mut parts = Vec::new();
        let mut start = None;
        let mut end = None;
        let mut previous_end: Option<Position> = None;
        let mut composite = false;
        let mut paren_depth = 0usize;

        loop {
            self.skip_conditional_newlines();

            match self.current_token_kind {
                Some(TokenKind::DoubleRightBracket) => break,
                Some(TokenKind::And) | Some(TokenKind::Or) if paren_depth == 0 => break,
                Some(TokenKind::RightParen) if stop_at_right_paren && paren_depth == 0 => break,
                None => break,
                _ => {}
            }

            if let Some(prev_end) = previous_end
                && prev_end.offset < self.current_span.start.offset
            {
                let gap_span = Span::from_positions(prev_end, self.current_span.start);
                let gap_text = gap_span.slice(self.input);
                if let Some(word) = first_word.take() {
                    parts.extend(word.parts);
                }
                if Self::source_text_needs_quote_preserving_decode(gap_text) {
                    let gap_word = self.decode_word_text_preserving_quotes_if_needed(
                        gap_text,
                        gap_span,
                        gap_span.start,
                        true,
                    );
                    parts.extend(gap_word.parts);
                } else {
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::source()),
                        gap_span,
                    ));
                }
                composite = true;
            }

            match self.current_token_kind {
                Some(TokenKind::Word | TokenKind::LiteralWord | TokenKind::QuotedWord) => {
                    let word = self
                        .take_current_word()
                        .ok_or_else(|| self.error("expected conditional operand"))?;
                    if start.is_none() {
                        start = Some(word.span.start);
                    } else {
                        if let Some(first) = first_word.take() {
                            parts.extend(first.parts);
                        }
                        composite = true;
                    }
                    end = Some(word.span.end);
                    if first_word.is_none() && !composite {
                        first_word = Some(word);
                    } else {
                        parts.extend(word.parts);
                    }
                    previous_end = Some(self.current_span.end);
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("(")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth += 1;
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::DoubleLeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("((")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth += 2;
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned(")")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(1);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("))")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(2);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::Pipe) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("|")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::And) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("&&")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::Or) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("||")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::RedirectIn)
                | Some(TokenKind::RedirectOut)
                | Some(TokenKind::RedirectReadWrite) => {
                    let literal = self.input
                        [self.current_span.start.offset..self.current_span.end.offset]
                        .to_string();
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(self.literal_text(
                            literal,
                            self.current_span.start,
                            self.current_span.end,
                            true,
                        )),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                _ => {
                    let literal = self.input
                        [self.current_span.start.offset..self.current_span.end.offset]
                        .to_string();
                    if literal.is_empty() {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(self.literal_text(
                            literal,
                            self.current_span.start,
                            self.current_span.end,
                            true,
                        )),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
            }
        }

        if !composite && let Some(word) = first_word {
            return Ok(word);
        }

        let (start, end) = match (start, end) {
            (Some(start), Some(end)) => (start, end),
            _ => return Err(self.error("expected conditional operand")),
        };

        Ok(self.word_with_parts(parts, Span::from_positions(start, end)))
    }

    pub(super) fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        self.ensure_arithmetic_command()?;
        let left_paren_span = self.current_span;
        let Some(right_paren_span) = self.scan_arithmetic_command_close(left_paren_span) else {
            return Err(Error::parse(
                "unexpected end of input in arithmetic command".to_string(),
            ));
        };
        while self.current_token.is_some()
            && self.current_span.start.offset < right_paren_span.end.offset
        {
            self.advance();
        }

        let expr_span = Self::optional_span(left_paren_span.end, right_paren_span.start);
        let expr_ast = self
            .parse_explicit_arithmetic_span(expr_span, "invalid arithmetic command")
            .ok()
            .flatten();
        Ok(CompoundCommand::Arithmetic(ArithmeticCommand {
            span: left_paren_span.merge(right_paren_span),
            left_paren_span,
            expr_span,
            expr_ast,
            right_paren_span,
        }))
    }

    pub(super) fn scan_arithmetic_command_close(&self, left_paren_span: Span) -> Option<Span> {
        let mut cursor = left_paren_span.end;
        let mut depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];

            if !in_single && !in_double && !in_backtick {
                if rest.starts_with("((") {
                    depth += 2;
                    cursor = cursor.advanced_by("((");
                    continue;
                }

                if rest.starts_with("))") {
                    if depth == 0 {
                        return Some(Span::from_positions(cursor, cursor.advanced_by("))")));
                    }
                    if depth == 1 {
                        cursor.advance(')');
                        return Some(Span::from_positions(cursor, cursor.advanced_by("))")));
                    }
                    depth -= 2;
                    cursor = cursor.advanced_by("))");
                    continue;
                }
            }

            let ch = rest.chars().next()?;
            cursor.advance(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if !in_single && !in_double && !in_backtick => depth += 1,
                ')' if !in_single && !in_double && !in_backtick && depth > 0 => depth -= 1,
                _ => {}
            }
        }

        None
    }
}
