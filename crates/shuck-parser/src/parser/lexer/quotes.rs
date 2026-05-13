use super::*;

impl<'a> Lexer<'a> {
    pub(in crate::parser) fn read_single_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        let segment = match self.read_single_quoted_segment() {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        Some(LexedToken::with_word_payload(TokenKind::LiteralWord, word))
    }

    pub(in crate::parser) fn read_single_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        debug_assert_eq!(self.peek_char(), Some('\''));

        let wrapper_start = self.current_position();
        self.consume_ascii_chars(1); // consume opening '
        let content_start = self.current_position();
        let can_borrow = self.reinject_buf.is_empty() && !self.rc_quotes_enabled();
        let mut content_end = content_start;
        let mut content = String::with_capacity(16);
        let mut closed = false;

        if can_borrow {
            let rest = self.cursor.rest();
            if let Some(quote_index) = memchr(b'\'', rest.as_bytes()) {
                self.consume_source_bytes(quote_index);
                content_end = self.current_position();
                self.consume_ascii_chars(1); // consume closing '
                closed = true;
            } else {
                self.consume_source_bytes(rest.len());
            }
        }

        while let Some(ch) = self.peek_char() {
            if closed {
                break;
            }
            if ch == '\'' {
                if self.rc_quotes_enabled() && self.second_char() == Some('\'') {
                    if !can_borrow {
                        content.push('\'');
                    }
                    self.advance();
                    self.advance();
                    continue;
                }
                content_end = self.current_position();
                self.consume_ascii_chars(1); // consume closing '
                closed = true;
                break;
            }
            if !can_borrow {
                content.push(ch);
            }
            self.advance();
        }

        if !closed {
            return Err(LexerErrorKind::SingleQuote);
        }

        let wrapper_span = Some(Span::from_positions(wrapper_start, self.current_position()));
        let content_span = Some(Span::from_positions(content_start, content_end));

        if can_borrow {
            Ok(LexedWordSegment::borrowed_with_spans(
                LexedWordSegmentKind::SingleQuoted,
                &self.input[content_start.offset..content_end.offset],
                content_span,
                wrapper_span,
            ))
        } else {
            Ok(LexedWordSegment::owned_with_spans(
                LexedWordSegmentKind::SingleQuoted,
                content,
                content_span,
                wrapper_span,
            ))
        }
    }

    pub(in crate::parser) fn read_dollar_single_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        let segment = match self.read_dollar_single_quoted_segment() {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        let kind = if word.single_segment().is_some() {
            TokenKind::LiteralWord
        } else {
            TokenKind::Word
        };

        Some(LexedToken::with_word_payload(kind, word))
    }

    pub(in crate::parser) fn read_dollar_single_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        debug_assert_eq!(self.peek_char(), Some('$'));
        debug_assert_eq!(self.second_char(), Some('\''));

        let wrapper_start = self.current_position();
        self.consume_ascii_chars(2); // consume $'
        let content_start = self.current_position();
        let mut out = String::with_capacity(16);

        while let Some(ch) = self.peek_char() {
            if ch == '\'' {
                let content_end = self.current_position();
                self.advance();
                let wrapper_span =
                    Some(Span::from_positions(wrapper_start, self.current_position()));
                let content_span = Some(Span::from_positions(content_start, content_end));
                return Ok(LexedWordSegment::owned_with_spans(
                    LexedWordSegmentKind::DollarSingleQuoted,
                    out,
                    content_span,
                    wrapper_span,
                ));
            }

            if ch == '\\' {
                self.advance();
                if let Some(esc) = self.peek_char() {
                    self.advance();
                    match esc {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'a' => out.push('\x07'),
                        'b' => out.push('\x08'),
                        'f' => out.push('\x0C'),
                        'v' => out.push('\x0B'),
                        'e' | 'E' => out.push('\x1B'),
                        '\\' => out.push('\\'),
                        '\'' => out.push('\''),
                        '"' => out.push('"'),
                        '?' => out.push('?'),
                        'c' => {
                            if let Some(control) = self.peek_char() {
                                self.advance();
                                out.push(((control as u32 & 0x1F) as u8) as char);
                            } else {
                                out.push('\\');
                                out.push('c');
                            }
                        }
                        'x' => {
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u8::from_str_radix(&hex, 16) {
                                out.push(val as char);
                            }
                        }
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u32::from_str_radix(&hex, 16)
                                && let Some(c) = char::from_u32(val)
                            {
                                out.push(c);
                            }
                        }
                        'U' => {
                            let mut hex = String::new();
                            for _ in 0..8 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u32::from_str_radix(&hex, 16)
                                && let Some(c) = char::from_u32(val)
                            {
                                out.push(c);
                            }
                        }
                        '0'..='7' => {
                            let mut oct = String::new();
                            oct.push(esc);
                            for _ in 0..2 {
                                if let Some(o) = self.peek_char() {
                                    if o.is_ascii_digit() && o < '8' {
                                        oct.push(o);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u8::from_str_radix(&oct, 8) {
                                out.push(val as char);
                            }
                        }
                        _ => {
                            out.push('\\');
                            out.push(esc);
                        }
                    }
                } else {
                    out.push('\\');
                }
                continue;
            }

            out.push(ch);
            self.advance();
        }

        Err(LexerErrorKind::SingleQuote)
    }
    pub(in crate::parser) fn read_double_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.read_double_quoted_word(false)
    }

    pub(in crate::parser) fn read_dollar_double_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.read_double_quoted_word(true)
    }

    pub(in crate::parser) fn read_double_quoted_word(
        &mut self,
        dollar: bool,
    ) -> Option<LexedToken<'a>> {
        let segment = match self.read_double_quoted_segment_with_dollar(dollar) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        let kind = if word.single_segment().is_some() {
            TokenKind::QuotedWord
        } else {
            TokenKind::Word
        };

        Some(LexedToken::with_word_payload(kind, word))
    }

    pub(in crate::parser) fn read_double_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        self.read_double_quoted_segment_with_dollar(false)
    }

    pub(in crate::parser) fn read_dollar_double_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        self.read_double_quoted_segment_with_dollar(true)
    }

    pub(in crate::parser) fn read_double_quoted_segment_with_dollar(
        &mut self,
        dollar: bool,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        if dollar {
            debug_assert_eq!(self.peek_char(), Some('$'));
            debug_assert_eq!(self.second_char(), Some('"'));
        } else {
            debug_assert_eq!(self.peek_char(), Some('"'));
        }

        let wrapper_start = self.current_position();
        if dollar {
            self.consume_ascii_chars(2); // consume $"
        } else {
            self.consume_ascii_chars(1); // consume opening "
        }
        let content_start = self.current_position();
        let mut content_end = content_start;
        let mut simple = self.reinject_buf.is_empty();
        let mut borrowable = self.reinject_buf.is_empty();
        let mut content = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));
        let mut closed = false;

        while let Some(ch) = self.peek_char() {
            if simple {
                if self.reinject_buf.is_empty() {
                    let rest = self.cursor.rest();
                    match Self::find_double_quote_special(rest) {
                        Some(index) if index > 0 => {
                            self.consume_source_bytes(index);
                            continue;
                        }
                        None => {
                            self.consume_source_bytes(rest.len());
                            return Err(LexerErrorKind::DoubleQuote);
                        }
                        _ => {}
                    }
                }

                match ch {
                    '"' => {
                        content_end = self.current_position();
                        self.consume_ascii_chars(1); // consume closing "
                        closed = true;
                        break;
                    }
                    '\\' | '$' | '`' => {
                        simple = false;
                        if ch == '`' {
                            borrowable = false;
                            let capture_end = self.current_position();
                            self.ensure_capture_from_source(
                                &mut content,
                                content_start,
                                capture_end,
                            );
                        }
                    }
                    _ => {
                        self.advance();
                    }
                }
                if simple {
                    continue;
                }
            }

            match ch {
                '"' => {
                    if borrowable {
                        content_end = self.current_position();
                    }
                    self.consume_ascii_chars(1); // consume closing "
                    closed = true;
                    break;
                }
                '\\' => {
                    let escape_start = self.current_position();
                    self.advance();
                    if let Some(next) = self.peek_char() {
                        match next {
                            '\n' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                self.advance();
                            }
                            '$' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                Self::push_capture_char(&mut content, '\x00');
                                Self::push_capture_char(&mut content, '$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                if next == '\\' {
                                    Self::push_capture_char(&mut content, '\x00');
                                }
                                if next == '`' {
                                    Self::push_capture_char(&mut content, '\x00');
                                }
                                Self::push_capture_char(&mut content, next);
                                self.advance();
                                content_end = self.current_position();
                            }
                            _ => {
                                Self::push_capture_char(&mut content, '\\');
                                Self::push_capture_char(&mut content, next);
                                self.advance();
                                content_end = self.current_position();
                            }
                        }
                    }
                }
                '$' => {
                    Self::push_capture_char(&mut content, '$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        if self.second_char() == Some('(') {
                            self.read_arithmetic_expansion_into(&mut content);
                        } else {
                            Self::push_capture_char(&mut content, '(');
                            self.advance();
                            self.read_command_subst_into(&mut content);
                        }
                    } else if self.peek_char() == Some('{') {
                        Self::push_capture_char(&mut content, '{');
                        self.advance();
                        borrowable &= self.read_param_expansion_into(&mut content, content_start);
                    }
                    content_end = self.current_position();
                }
                '`' => {
                    borrowable = false;
                    let capture_end = self.current_position();
                    self.ensure_capture_from_source(&mut content, content_start, capture_end);
                    Self::push_capture_char(&mut content, '`');
                    self.advance(); // consume opening `
                    while let Some(c) = self.peek_char() {
                        Self::push_capture_char(&mut content, c);
                        self.advance();
                        if c == '`' {
                            break;
                        }
                        if c == '\\'
                            && let Some(next) = self.peek_char()
                        {
                            Self::push_capture_char(&mut content, next);
                            self.advance();
                        }
                    }
                    content_end = self.current_position();
                }
                _ => {
                    Self::push_capture_char(&mut content, ch);
                    self.advance();
                    content_end = self.current_position();
                }
            }
        }

        if !closed {
            return Err(LexerErrorKind::DoubleQuote);
        }

        let wrapper_span = Some(Span::from_positions(wrapper_start, self.current_position()));
        let content_span = Some(Span::from_positions(content_start, content_end));

        if borrowable {
            Ok(LexedWordSegment::borrowed_with_spans(
                if dollar {
                    LexedWordSegmentKind::DollarDoubleQuoted
                } else {
                    LexedWordSegmentKind::DoubleQuoted
                },
                &self.input[content_start.offset..content_end.offset],
                content_span,
                wrapper_span,
            ))
        } else {
            Ok(LexedWordSegment::owned_with_spans(
                if dollar {
                    LexedWordSegmentKind::DollarDoubleQuoted
                } else {
                    LexedWordSegmentKind::DoubleQuoted
                },
                content.unwrap_or_default(),
                content_span,
                wrapper_span,
            ))
        }
    }
}
