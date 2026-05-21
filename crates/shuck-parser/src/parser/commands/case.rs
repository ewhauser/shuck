use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_case(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'case'
        self.skip_newlines()?;

        // Get the word to match against
        let word = self.expect_word()?;
        self.skip_newlines()?;

        // Expect 'in'
        self.expect_keyword(Keyword::In)?;
        self.skip_newlines()?;

        // Parse case items
        let mut cases = Vec::new();
        while !self.is_keyword(Keyword::Esac) && self.current_token.is_some() {
            self.skip_newlines()?;
            if self.is_keyword(Keyword::Esac) {
                break;
            }

            let patterns = match self.parse_case_patterns() {
                Ok(patterns) => patterns,
                Err(err) => {
                    self.pop_depth();
                    return Err(err);
                }
            };
            self.skip_newlines()?;

            // Parse commands until ;; or esac
            let body_start = self.current_span.start;
            let mut commands = Vec::new();
            while !self.is_case_terminator()
                && !self.is_keyword(Keyword::Esac)
                && self.current_token.is_some()
            {
                commands.extend(self.parse_command_list_required()?);
                self.skip_newlines()?;
            }

            let (terminator, terminator_span) = self.parse_case_terminator();
            let body_span = Span::from_positions(body_start, self.current_span.start);
            cases.push(CaseItem {
                patterns,
                body: Self::stmt_seq_with_span(body_span, commands),
                terminator,
                terminator_span,
            });
            self.skip_newlines()?;
        }

        // Expect 'esac'
        if !self.is_keyword(Keyword::Esac) {
            self.pop_depth();
            return Err(self.error("expected 'esac'"));
        }
        let esac_span = self.current_span;
        self.advance();

        self.pop_depth();
        Ok(CompoundCommand::Case(CaseCommand {
            word,
            cases,
            esac_span,
            span: start_span.merge(self.current_span),
        }))
    }

    pub(super) fn parse_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        self.record_zsh_case_group_parts_from_current_case_header();
        if self.dialect == ShellDialect::Zsh {
            self.parse_zsh_case_patterns()
        } else {
            self.parse_posix_case_patterns()
        }
    }

    pub(super) fn record_zsh_case_group_parts_from_current_case_header(&mut self) {
        let start = self.current_span.start;
        let mut split_features = self.zsh_glob_parse_features_at(start.offset);
        if self.dialect != ShellDialect::Zsh {
            split_features.bare_groups = true;
        }

        let Some((pattern_spans, _)) = (if self.input[start.offset..].starts_with('(')
            && let Some(wrapper_close) = self.scan_zsh_case_group_close(start)
            && self.case_wrapper_close_is_arm_delimiter(wrapper_close)
        {
            let inner_start = start.advanced_by("(");
            let inner_span = Span::from_positions(inner_start, wrapper_close.start);
            self.split_zsh_case_pattern_alternatives_with_features(inner_span, split_features)
                .map(|patterns| (patterns, wrapper_close))
        } else if let Some(delimiter_span) = self.scan_zsh_case_arm_delimiter(start) {
            let header_span = Span::from_positions(start, delimiter_span.start);
            self.split_zsh_case_pattern_alternatives_with_features(header_span, split_features)
                .map(|patterns| (patterns, delimiter_span))
        } else {
            None
        }) else {
            return;
        };

        for span in pattern_spans {
            let mut features = self.zsh_glob_parse_features_at(span.start.offset);
            if self.dialect != ShellDialect::Zsh {
                features.bare_groups = true;
            }
            let text = span.slice(self.input);
            let word = if Self::source_text_needs_quote_preserving_decode(text) {
                self.decode_fragment_word_text(text, span, span.start, true)
            } else {
                self.decode_word_text(text, span, span.start, true)
            };
            let pattern = self.pattern_from_zsh_case_word_with_features(&word, features);
            for (index, part) in pattern.parts.iter().enumerate() {
                if matches!(
                    &part.kind,
                    PatternPart::Group {
                        kind: PatternGroupKind::ExactlyOne,
                        ..
                    }
                ) && part.span.slice(self.input).starts_with('(')
                {
                    self.record_zsh_case_group_part(index, part.span);
                }
            }
        }
    }

    pub(super) fn parse_posix_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        if self.at(TokenKind::LeftParen) {
            self.advance();
        }

        let mut patterns = Vec::new();
        while self.at_word_like() {
            if let Some(word) = self.take_current_word_and_advance() {
                patterns.push(self.pattern_from_word(&word));
            }

            if self.at(TokenKind::Pipe) {
                self.advance();
            } else {
                break;
            }
        }

        if !self.at(TokenKind::RightParen) {
            return Err(self.error("expected ')' after case pattern"));
        }
        self.advance();

        Ok(patterns)
    }

    pub(super) fn parse_zsh_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        let (pattern_spans, delimiter_span) = self.scan_zsh_case_pattern_spans()?;
        let patterns = pattern_spans
            .into_iter()
            .map(|span| self.pattern_from_zsh_case_span(span))
            .collect::<Vec<_>>();

        while self.current_token.is_some()
            && self.current_span.start.offset < delimiter_span.end.offset
        {
            self.advance();
        }

        Ok(patterns)
    }

    pub(super) fn scan_zsh_case_pattern_spans(&self) -> Result<(Vec<Span>, Span)> {
        let start = self.current_span.start;
        let Some((spans, delimiter_span)) = self.try_scan_zsh_case_pattern_spans(start) else {
            return Err(self.error("expected ')' after case pattern"));
        };
        if spans.is_empty() {
            return Err(self.error("expected ')' after case pattern"));
        }
        Ok((spans, delimiter_span))
    }

    pub(super) fn try_scan_zsh_case_pattern_spans(
        &self,
        start: Position,
    ) -> Option<(Vec<Span>, Span)> {
        if self.input[start.offset..].starts_with('(')
            && let Some(wrapper_close) = self.scan_zsh_case_group_close(start)
            && self.case_wrapper_close_is_arm_delimiter(wrapper_close)
        {
            let inner_start = start.advanced_by("(");
            let inner_span = Span::from_positions(inner_start, wrapper_close.start);
            let patterns = self.split_zsh_case_pattern_alternatives(inner_span)?;
            return Some((patterns, wrapper_close));
        }

        let delimiter_span = self.scan_zsh_case_arm_delimiter(start)?;
        let header_span = Span::from_positions(start, delimiter_span.start);
        let patterns = self.split_zsh_case_pattern_alternatives(header_span)?;
        Some((patterns, delimiter_span))
    }

    pub(super) fn case_wrapper_close_is_arm_delimiter(&self, close_span: Span) -> bool {
        self.input[close_span.end.offset..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    }

    pub(super) fn split_zsh_case_pattern_alternatives(&self, span: Span) -> Option<Vec<Span>> {
        self.split_zsh_case_pattern_alternatives_with_features(
            span,
            self.zsh_glob_parse_features_at(span.start.offset),
        )
    }

    pub(super) fn split_zsh_case_pattern_alternatives_with_features(
        &self,
        span: Span,
        features: ZshGlobParseFeatures,
    ) -> Option<Vec<Span>> {
        let mut state = ZshCaseScanState::new(span.start);
        let mut chars = self.input[span.start.offset..span.end.offset]
            .chars()
            .peekable();
        let mut part_start = span.start;
        let mut parts = Vec::new();
        let mut previous_char = None;

        while let Some(ch) = chars.peek().copied() {
            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0 =>
                {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    let has_ksh_group_prefix =
                        matches!(previous_char, Some('?' | '*' | '+' | '@' | '!'));
                    let ksh_group_start = features.ksh_groups && has_ksh_group_prefix;
                    let bare_group_start =
                        features.bare_groups && (!has_ksh_group_prefix || !features.ksh_groups);
                    if bare_group_start || ksh_group_start {
                        state.paren_depth += 1;
                    }
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth > 0 =>
                {
                    state.paren_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '|' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth == 0 =>
                {
                    let end = state.position;
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    parts.push(
                        self.trim_zsh_case_pattern_span(Span::from_positions(part_start, end))?,
                    );
                    part_start = state.position;
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }

            previous_char = Some(ch);
        }

        parts.push(
            self.trim_zsh_case_pattern_span(Span::from_positions(part_start, state.position))?,
        );
        Some(parts)
    }

    pub(super) fn trim_zsh_case_pattern_span(&self, span: Span) -> Option<Span> {
        let text = span.slice(self.input);
        let trimmed_start = text.len() - text.trim_start_matches(char::is_whitespace).len();
        let trimmed_end = text.trim_end_matches(char::is_whitespace).len();
        let start = span.start.advanced_by(&text[..trimmed_start]);
        let end = span.start.advanced_by(&text[..trimmed_end]);
        Some(Span::from_positions(start, end))
    }

    pub(super) fn scan_zsh_case_group_close(&self, start: Position) -> Option<Span> {
        let mut state = ZshCaseScanState::new(start);
        let mut chars = self.input[start.offset..].chars().peekable();

        if Self::next_word_char_unwrap(&mut chars, &mut state.position) != '(' {
            return None;
        }
        state.paren_depth = 1;

        while let Some(ch) = chars.peek().copied() {
            let ch_start = state.position;

            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    state.paren_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth > 0 =>
                {
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    state.paren_depth -= 1;
                    if state.paren_depth == 0 {
                        return Some(Span::from_positions(ch_start, state.position));
                    }
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }
        }

        None
    }

    pub(super) fn scan_zsh_case_arm_delimiter(&self, start: Position) -> Option<Span> {
        let mut state = ZshCaseScanState::new(start);
        let mut chars = self.input[start.offset..].chars().peekable();

        while let Some(ch) = chars.peek().copied() {
            let ch_start = state.position;

            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    state.paren_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    if state.paren_depth == 0 {
                        return Some(Span::from_positions(ch_start, state.position));
                    }
                    state.paren_depth -= 1;
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }
        }

        None
    }
}
