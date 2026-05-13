use super::*;

impl<'a> Lexer<'a> {
    /// Get the next source-backed token from the input, skipping line comments.
    ///
    /// Returned tokens expose their [`TokenKind`] and source [`Span`]. Comments
    /// are omitted from this public stream; the parser uses an internal variant
    /// when it needs to preserve them for AST attachment.
    pub fn next_lexed_token(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.current_position();
        let token = self.next_lexed_token_inner(false)?;
        let end = self.current_position();
        Some(token.with_span(Span::from_positions(start, end)))
    }

    /// Get the next source-backed token from the input, preserving line comments.
    pub(in crate::parser) fn next_lexed_token_with_comments(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.current_position();
        let token = self.next_lexed_token_inner(true)?;
        let end = self.current_position();
        Some(token.with_span(Span::from_positions(start, end)))
    }

    /// Internal: get next token without recording position (called after whitespace skip)
    pub(in crate::parser) fn next_lexed_token_inner(
        &mut self,
        preserve_comments: bool,
    ) -> Option<LexedToken<'a>> {
        let ch = self.peek_char()?;

        match ch {
            '\n' => {
                self.consume_ascii_chars(1);
                Some(LexedToken::punctuation(TokenKind::Newline))
            }
            ';' => {
                if self.second_char() == Some(';') {
                    if self.third_char() == Some('&') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::DoubleSemiAmp)) // ;;&
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::DoubleSemicolon)) // ;;
                    }
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::SemiPipe)) // ;|
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::SemiAmp)) // ;&
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Semicolon))
                }
            }
            '|' => {
                if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::Or))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::PipeBoth))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Pipe))
                }
            }
            '&' => {
                if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::And))
                } else if self.second_char() == Some('>') {
                    if self.third_char() == Some('>') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::RedirectBothAppend))
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::RedirectBoth))
                    }
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::BackgroundPipe))
                } else if self.second_char() == Some('!') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::BackgroundBang))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Background))
                }
            }
            '>' => {
                if self.second_char() == Some('>') {
                    if self.third_char() == Some('|') {
                        self.consume_ascii_chars(3);
                    } else {
                        self.consume_ascii_chars(2);
                    }
                    Some(LexedToken::punctuation(TokenKind::RedirectAppend))
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::Clobber))
                } else if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::ProcessSubOut))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DupOutput))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RedirectOut))
                }
            }
            '<' => {
                if self.second_char() == Some('<') {
                    if self.third_char() == Some('<') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::HereString))
                    } else if self.third_char() == Some('-') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::HereDocStrip))
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::HereDoc))
                    }
                } else if self.second_char() == Some('>') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::RedirectReadWrite))
                } else if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::ProcessSubIn))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DupInput))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RedirectIn))
                }
            }
            '(' => {
                if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleLeftParen))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::LeftParen))
                }
            }
            ')' => {
                if self.second_char() == Some(')') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleRightParen))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RightParen))
                }
            }
            '{' => {
                let start = self.current_position();
                if self.ignore_braces_enabled() {
                    self.consume_ascii_chars(1);
                    match self.peek_char() {
                        Some(' ') | Some('\t') | Some('\n') | None => {
                            Some(LexedToken::borrowed_word(TokenKind::Word, "{", None))
                        }
                        _ => self.read_word_starting_with("{", start),
                    }
                } else if self.looks_like_brace_expansion() {
                    // Look ahead to see if this is a brace expansion like {a,b,c} or {1..5}
                    // vs a brace group like { cmd; }
                    // Note: { must be followed by space/newline to be a brace group
                    self.read_brace_expansion_word()
                } else if self.is_brace_group_start() {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::LeftBrace))
                } else if self.brace_literal_starts_case_pattern_delimiter() {
                    self.read_word_starting_with("{", start)
                } else {
                    self.read_brace_literal_word()
                }
            }
            '}' => {
                self.consume_ascii_chars(1);
                if self.ignore_close_braces_enabled() {
                    Some(LexedToken::borrowed_word(TokenKind::Word, "}", None))
                } else {
                    Some(LexedToken::punctuation(TokenKind::RightBrace))
                }
            }
            '[' => {
                let start = self.current_position();
                self.consume_ascii_chars(1);
                if self.peek_char() == Some('[')
                    && matches!(
                        self.second_char(),
                        Some(' ') | Some('\t') | Some('\n') | None
                    )
                {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::DoubleLeftBracket))
                } else {
                    // `[` can start the test command when followed by whitespace, or it can be
                    // ordinary word text such as a glob bracket expression.
                    //
                    // Read the whole token with the normal word scanner so forms like `[[z]`,
                    // `[hello"]"`, and `[+(])` stay attached to one word instead of producing
                    // structural tokens mid-word.
                    match self.peek_char() {
                        Some(' ') | Some('\t') | Some('\n') | None => {
                            Some(LexedToken::borrowed_word(TokenKind::Word, "[", None))
                        }
                        _ => self.read_word_starting_with("[", start),
                    }
                }
            }
            ']' => {
                if self.second_char() == Some(']') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleRightBracket))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::borrowed_word(TokenKind::Word, "]", None))
                }
            }
            '\'' => self.read_single_quoted_string(),
            '"' => self.read_double_quoted_string(),
            '#' => {
                if self.should_treat_hash_as_word_char() {
                    let start = self.current_position();
                    return self.read_word_starting_with("#", start);
                }
                if preserve_comments {
                    self.read_comment();
                    Some(LexedToken::comment())
                } else {
                    self.skip_comment();
                    self.next_lexed_token_inner(false)
                }
            }
            // Handle file descriptor redirects like 2> or 2>&1
            '0'..='9' => self.read_word_or_fd_redirect(),
            _ => self.read_word(),
        }
    }

    pub(in crate::parser) fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            if self.reinject_buf.is_empty() {
                let whitespace_len = self.source_horizontal_whitespace_len();
                if whitespace_len > 0 {
                    self.consume_source_bytes(whitespace_len);
                    continue;
                }

                if self.cursor.rest().starts_with("\\\n") {
                    self.consume_source_bytes(2);
                    continue;
                }
            }

            if ch == ' ' || ch == '\t' {
                self.consume_ascii_chars(1);
            } else if ch == '\\' {
                // Check for backslash-newline (line continuation) between tokens
                if self.second_char() == Some('\n') {
                    self.consume_ascii_chars(2);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    pub(in crate::parser) fn skip_comment(&mut self) {
        if self.reinject_buf.is_empty() {
            let end = self
                .cursor
                .find_byte(b'\n')
                .unwrap_or(self.cursor.rest().len());
            self.consume_source_bytes(end);
            return;
        }

        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    pub(in crate::parser) fn read_comment(&mut self) {
        debug_assert_eq!(self.peek_char(), Some('#'));

        if self.reinject_buf.is_empty() {
            let rest = self.cursor.rest();
            let end = self.cursor.find_byte(b'\n').unwrap_or(rest.len());
            self.consume_source_bytes(end);
            return;
        }

        self.advance(); // consume '#'

        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    pub(in crate::parser) fn is_inside_unclosed_double_paren_on_line(&self) -> bool {
        if !self.reinject_buf.is_empty() || self.offset > self.input.len() {
            return false;
        }

        let line_start = self.input[..self.offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let prefix = &self.input[line_start..self.offset];
        line_has_unclosed_double_paren(prefix)
    }
}
