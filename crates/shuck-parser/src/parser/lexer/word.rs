use super::*;

impl<'a> Lexer<'a> {
    /// Check if this is a file descriptor redirect (e.g., 2>, 2>>, 2>&1)
    /// or just a regular word starting with a digit
    pub(in crate::parser) fn read_word_or_fd_redirect(&mut self) -> Option<LexedToken<'a>> {
        if let Some(first_digit) = self.peek_char().filter(|ch| ch.is_ascii_digit()) {
            let Some(fd) = first_digit.to_digit(10) else {
                unreachable!("peeked ASCII digit should convert to a base-10 digit");
            };
            let fd = fd as i32;

            match (self.second_char(), self.third_char()) {
                (Some('>'), Some('>')) => {
                    if self.fourth_char() == Some('|') {
                        self.consume_ascii_chars(4);
                    } else {
                        self.consume_ascii_chars(3);
                    }
                    return Some(LexedToken::fd(TokenKind::RedirectFdAppend, fd));
                }
                (Some('>'), Some('|')) => {
                    self.consume_ascii_chars(3);
                    return Some(LexedToken::fd(TokenKind::Clobber, fd));
                }
                (Some('>'), Some('&')) => {
                    self.consume_ascii_chars(3);

                    let mut target_str = String::with_capacity(4);
                    while let Some(c) = self.peek_char() {
                        if c.is_ascii_digit() {
                            target_str.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }

                    if target_str.is_empty() {
                        return Some(LexedToken::fd(TokenKind::RedirectFd, fd));
                    }

                    let target_fd: i32 = target_str.parse().unwrap_or(1);
                    return Some(LexedToken::fd_pair(TokenKind::DupFd, fd, target_fd));
                }
                (Some('>'), _) => {
                    self.consume_ascii_chars(2);
                    return Some(LexedToken::fd(TokenKind::RedirectFd, fd));
                }
                (Some('<'), Some('&')) => {
                    self.consume_ascii_chars(3);

                    let mut target_str = String::with_capacity(4);
                    while let Some(c) = self.peek_char() {
                        if c.is_ascii_digit() || c == '-' {
                            target_str.push(c);
                            self.advance();
                            if c == '-' {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    if target_str == "-" {
                        return Some(LexedToken::fd(TokenKind::DupFdClose, fd));
                    }
                    let target_fd: i32 = target_str.parse().unwrap_or(0);
                    return Some(LexedToken::fd_pair(TokenKind::DupFdIn, fd, target_fd));
                }
                (Some('<'), Some('>')) => {
                    self.consume_ascii_chars(3);
                    return Some(LexedToken::fd(TokenKind::RedirectFdReadWrite, fd));
                }
                (Some('<'), Some('<')) => {}
                (Some('<'), _) => {
                    self.consume_ascii_chars(2);
                    return Some(LexedToken::fd(TokenKind::RedirectFdIn, fd));
                }
                _ => {}
            }
        }

        // Not a fd redirect pattern, read as regular word
        self.read_word()
    }

    pub(in crate::parser) fn read_word_starting_with(
        &mut self,
        _prefix: &str,
        start: Position,
    ) -> Option<LexedToken<'a>> {
        let segment = match self.read_unquoted_segment(start) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        if segment.as_str().is_empty() {
            return None;
        }
        let mut lexed_word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut lexed_word) {
            return Some(LexedToken::error(kind));
        }
        Some(LexedToken::with_word_payload(TokenKind::Word, lexed_word))
    }

    pub(in crate::parser) fn read_word(&mut self) -> Option<LexedToken<'a>> {
        let start = self.current_position();

        if self.reinject_buf.is_empty() {
            let ascii_len = self.source_ascii_plain_word_len();
            let chunk = if ascii_len > 0
                && self
                    .cursor
                    .rest()
                    .as_bytes()
                    .get(ascii_len)
                    .is_none_or(|byte| byte.is_ascii())
            {
                self.consume_source_bytes(ascii_len);
                &self.input[start.offset..self.offset]
            } else {
                let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                self.advance_scanned_source_bytes(chunk.len());
                chunk
            };
            if !chunk.is_empty() {
                let continues = matches!(
                    self.peek_char(),
                    Some(next)
                        if Self::is_word_char(next)
                            || next == '$'
                            || matches!(next, '\'' | '"')
                            || next == '{'
                            || (next == '\\' && self.second_char() == Some('\n'))
                            || (next == '('
                                && (chunk.ends_with('=')
                                    || Self::word_can_take_parenthesized_suffix(chunk)))
                );
                let continues = continues
                    || (self.peek_char() == Some('(')
                        && (self.looks_like_zsh_alternative_glob_suffix(chunk)
                            || self.looks_like_zsh_glob_modifier_suffix(chunk)));

                if !continues {
                    let end = self.current_position();
                    return Some(LexedToken::borrowed_word(
                        TokenKind::Word,
                        &self.input[start.offset..self.offset],
                        Some(Span::from_positions(start, end)),
                    ));
                }

                if self.peek_char() == Some('(')
                    && (chunk.ends_with('=')
                        || Self::word_can_take_parenthesized_suffix(chunk)
                        || self.looks_like_zsh_alternative_glob_suffix(chunk)
                        || self.looks_like_zsh_glob_modifier_suffix(chunk))
                {
                    return self.read_complex_word(start);
                }

                let end = self.current_position();
                return self.finish_segmented_word(LexedWord::borrowed(
                    LexedWordSegmentKind::Plain,
                    &self.input[start.offset..self.offset],
                    Some(Span::from_positions(start, end)),
                ));
            }
        }

        self.read_complex_word(start)
    }

    pub(in crate::parser) fn finish_segmented_word(
        &mut self,
        mut lexed_word: LexedWord<'a>,
    ) -> Option<LexedToken<'a>> {
        if let Err(kind) = self.append_segmented_continuation(&mut lexed_word) {
            return Some(LexedToken::error(kind));
        }

        Some(LexedToken::with_word_payload(TokenKind::Word, lexed_word))
    }

    pub(in crate::parser) fn read_complex_word(
        &mut self,
        start: Position,
    ) -> Option<LexedToken<'a>> {
        if self.peek_char() == Some('$') {
            match self.second_char() {
                Some('\'') => return self.read_dollar_single_quoted_string(),
                Some('"') => return self.read_dollar_double_quoted_string(),
                _ => {}
            }
        }

        let segment = match self.read_unquoted_segment(start) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };

        if segment.as_str().is_empty() {
            return None;
        }

        self.finish_segmented_word(LexedWord::from_segment(segment))
    }

    pub(in crate::parser) fn read_unquoted_segment(
        &mut self,
        start: Position,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        let mut word = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));
        while let Some(ch) = self.peek_char() {
            if ch == '"' || ch == '\'' {
                break;
            } else if ch == '$' {
                if matches!(self.second_char(), Some('\'') | Some('"'))
                    && (self.current_position().offset > start.offset
                        || word.as_ref().is_some_and(|word| !word.is_empty()))
                {
                    break;
                }

                // Handle variable references and command substitution
                self.advance();

                Self::push_capture_char(&mut word, ch); // push the '$'

                // Check for $[ / $( / ${ forms before falling back to variable text.
                if self.peek_char() == Some('[') {
                    Self::push_capture_char(&mut word, '[');
                    self.advance();
                    if !self.read_legacy_arithmetic_into(&mut word, start) {
                        return Err(LexerErrorKind::CommandSubstitution);
                    }
                } else if self.peek_char() == Some('(') {
                    if self.second_char() == Some('(') {
                        if !self.read_arithmetic_expansion_into(&mut word) {
                            return Err(LexerErrorKind::CommandSubstitution);
                        }
                    } else {
                        Self::push_capture_char(&mut word, '(');
                        self.advance();
                        if !self.read_command_subst_into(&mut word) {
                            return Err(LexerErrorKind::CommandSubstitution);
                        }
                    }
                } else if self.peek_char() == Some('{') {
                    // ${VAR} format — track nested braces so ${a[${#b[@]}]}
                    // doesn't stop at the inner }.
                    Self::push_capture_char(&mut word, '{');
                    self.advance();
                    let _ = self.read_param_expansion_into(&mut word, start, false);
                } else {
                    // Check for special single-character variables ($?, $#, $@, $*, $!, $$, $-, $0-$9)
                    if let Some(c) = self.peek_char() {
                        if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                            || c.is_ascii_digit()
                        {
                            Self::push_capture_char(&mut word, c);
                            self.advance();
                        } else {
                            // Read variable name (alphanumeric + _)
                            while let Some(c) = self.peek_char() {
                                if c.is_ascii_alphanumeric() || c == '_' {
                                    Self::push_capture_char(&mut word, c);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else if ch == '{' {
                if self.looks_like_mid_word_brace_segment() {
                    // Keep balanced {...} forms attached to the current word so
                    // plain literals like foo{bar} and brace expansions stay intact.
                    Self::push_capture_char(&mut word, ch);
                    self.advance();
                    self.consume_mid_word_brace_segment(&mut word);
                } else {
                    // Unmatched literal braces in regexes like ^{ should not swallow
                    // trailing delimiters such as ]] or then.
                    Self::push_capture_char(&mut word, ch);
                    self.advance();
                }
            } else if ch == '`' {
                // Preserve legacy backticks verbatim so the parser can keep the
                // original syntax form.
                let capture_end = self.current_position();
                self.ensure_capture_from_source(&mut word, start, capture_end);
                Self::push_capture_char(&mut word, ch);
                self.advance(); // consume opening `
                let mut closed = false;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
                    self.advance();
                    if c == '`' {
                        closed = true;
                        break;
                    }
                    if c == '\\'
                        && let Some(next) = self.peek_char()
                    {
                        Self::push_capture_char(&mut word, next);
                        self.advance();
                    }
                }
                if !closed {
                    return Err(LexerErrorKind::BacktickSubstitution);
                }
            } else if ch == '\\' {
                let capture_end = self.current_position();
                self.ensure_capture_from_source(&mut word, start, capture_end);
                self.advance();
                if let Some(next) = self.peek_char() {
                    if next == '\n' {
                        // Line continuation: skip backslash + newline
                        self.advance();
                    } else {
                        // Escaped character: backslash quotes the next char
                        // (quote removal — only the literal char survives).
                        // Preserve source/decoded alignment with a sentinel so
                        // downstream word decoding keeps later spans anchored.
                        Self::push_capture_char(&mut word, '\x00');
                        Self::push_capture_char(&mut word, next);
                        self.advance();
                        if next == '{'
                            && self.current_word_surface_is_single_char(start, &word, '{')
                            && self.escaped_brace_sequence_looks_like_brace_expansion()
                        {
                            let mut depth = 1;
                            while let Some(c) = self.peek_char() {
                                Self::push_capture_char(&mut word, c);
                                self.advance();
                                match c {
                                    '{' => depth += 1,
                                    '}' => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                } else {
                    Self::push_capture_char(&mut word, '\\');
                }
            } else if ch == '('
                && self.current_word_surface_ends_with_char(start, &word, '=')
                && self.looks_like_assoc_assign()
            {
                // Associative compound assignment: var=([k]="v" ...) — keep entire
                // (...) as part of word so declare -A m=([k]="v") stays one token.
                Self::push_capture_char(&mut word, ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
                    self.advance();
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '"' => {
                            while let Some(qc) = self.peek_char() {
                                Self::push_capture_char(&mut word, qc);
                                self.advance();
                                if qc == '"' {
                                    break;
                                }
                                if qc == '\\'
                                    && let Some(esc) = self.peek_char()
                                {
                                    Self::push_capture_char(&mut word, esc);
                                    self.advance();
                                }
                            }
                        }
                        '\'' => {
                            while let Some(qc) = self.peek_char() {
                                Self::push_capture_char(&mut word, qc);
                                self.advance();
                                if qc == '\'' {
                                    break;
                                }
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                Self::push_capture_char(&mut word, esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if ch == '('
                && (self.current_word_surface_ends_with_extglob_prefix(start, &word)
                    || self.current_word_surface_can_take_zsh_glob_modifier_suffix(start, &word))
            {
                // Extglob and zsh glob modifiers consume through matching )
                // including nested parens.
                Self::push_capture_char(&mut word, ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
                    self.advance();
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                Self::push_capture_char(&mut word, esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if Self::is_plain_word_char(ch) {
                if self.reinject_buf.is_empty() {
                    let ascii_len = self.source_ascii_plain_word_len();
                    let chunk = if ascii_len > 0
                        && self
                            .cursor
                            .rest()
                            .as_bytes()
                            .get(ascii_len)
                            .is_none_or(|byte| byte.is_ascii())
                    {
                        self.consume_source_bytes(ascii_len);
                        &self.input[self.offset - ascii_len..self.offset]
                    } else {
                        let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                        self.advance_scanned_source_bytes(chunk.len());
                        chunk
                    };
                    Self::push_capture_str(&mut word, chunk);
                } else {
                    Self::push_capture_char(&mut word, ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        if let Some(word) = word {
            let span = Some(Span::from_positions(start, self.current_position()));
            Ok(LexedWordSegment::owned_with_spans(
                LexedWordSegmentKind::Plain,
                word,
                span,
                span,
            ))
        } else {
            let end = self.current_position();
            Ok(LexedWordSegment::borrowed(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..self.offset],
                Some(Span::from_positions(start, end)),
            ))
        }
    }
    pub(in crate::parser) fn read_plain_continuation_segment(
        &mut self,
    ) -> Option<LexedWordSegment<'a>> {
        let start = self.current_position();

        if self.reinject_buf.is_empty() {
            let ascii_len = self.source_ascii_plain_word_len();
            let chunk = if ascii_len > 0
                && self
                    .cursor
                    .rest()
                    .as_bytes()
                    .get(ascii_len)
                    .is_none_or(|byte| byte.is_ascii())
            {
                self.consume_source_bytes(ascii_len);
                &self.input[start.offset..self.offset]
            } else {
                let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                self.advance_scanned_source_bytes(chunk.len());
                chunk
            };
            if chunk.is_empty() {
                return None;
            }

            let end = self.current_position();
            return Some(LexedWordSegment::borrowed(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..self.offset],
                Some(Span::from_positions(start, end)),
            ));
        }

        let ch = self.peek_char()?;
        if !Self::is_plain_word_char(ch) {
            return None;
        }

        let mut text = String::with_capacity(16);
        while let Some(ch) = self.peek_char() {
            if !Self::is_plain_word_char(ch) {
                break;
            }
            text.push(ch);
            self.advance();
        }

        Some(LexedWordSegment::owned(LexedWordSegmentKind::Plain, text))
    }

    /// After a closing quote, read any adjacent quoted or unquoted word chars
    /// into `word`. Handles concatenation like `'foo'"bar"baz`.
    pub(in crate::parser) fn append_segmented_continuation(
        &mut self,
        word: &mut LexedWord<'a>,
    ) -> Result<(), LexerErrorKind> {
        loop {
            match self.peek_char() {
                Some('\\') if self.second_char() == Some('\n') => {
                    self.advance();
                    self.advance();
                    continue;
                }
                Some('\'') => {
                    word.push_segment(self.read_single_quoted_segment()?);
                }
                Some('"') => {
                    word.push_segment(self.read_double_quoted_segment()?);
                }
                Some('$') if self.second_char() == Some('\'') => {
                    word.push_segment(self.read_dollar_single_quoted_segment()?);
                }
                Some('$') if self.second_char() == Some('"') => {
                    word.push_segment(self.read_dollar_double_quoted_segment()?);
                }
                Some('(')
                    if Self::lexed_word_can_take_parenthesized_suffix(word)
                        || self.looks_like_zsh_alternative_glob_suffix(&word.joined_text())
                        || self.looks_like_zsh_glob_modifier_suffix(&word.joined_text()) =>
                {
                    let Some(segment) = self.read_parenthesized_word_suffix_segment() else {
                        unreachable!("peeked '(' should produce a suffix segment");
                    };
                    word.push_segment(segment);
                }
                _ => {
                    if let Some(segment) = self.read_plain_continuation_segment() {
                        word.push_segment(segment);
                        continue;
                    }

                    let start = self.current_position();
                    let plain = self.read_unquoted_segment(start)?;
                    if plain.as_str().is_empty() {
                        break;
                    }
                    word.push_segment(plain);
                }
            }
        }

        Ok(())
    }

    pub(in crate::parser) fn read_parenthesized_word_suffix_segment(
        &mut self,
    ) -> Option<LexedWordSegment<'a>> {
        debug_assert_eq!(self.peek_char(), Some('('));

        let start = self.current_position();
        let mut depth = 0usize;
        let mut escaped = false;
        let mut text = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));

        while let Some(ch) = self.peek_char() {
            if let Some(text) = text.as_mut() {
                text.push(ch);
            }
            self.advance();

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        let end = self.current_position();
        let span = Some(Span::from_positions(start, end));
        if let Some(text) = text {
            Some(LexedWordSegment::owned_with_spans(
                LexedWordSegmentKind::Plain,
                text,
                span,
                span,
            ))
        } else {
            Some(LexedWordSegment::borrowed_with_spans(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..end.offset],
                span,
                span,
            ))
        }
    }

    /// Check if the content starting with { looks like a brace expansion
    /// Brace expansion: {a,b,c} or {1..5} (contains , or ..)
    /// Brace group: { cmd; } (contains spaces, semicolons, newlines)
    /// Caps lookahead to prevent O(n^2) scanning when input
    /// contains many unmatched `{` characters (issue #997).
    pub(in crate::parser) fn looks_like_brace_expansion(&mut self) -> bool {
        const MAX_LOOKAHEAD: usize = 10_000;
        let brace_ccl_enabled = self.brace_ccl_enabled();

        let mut chars = self.lookahead_chars();

        // Skip the opening {
        if chars.next() != Some('{') {
            return false;
        }

        let mut depth = 1;
        let mut paren_depth = 0usize;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut escaped = false;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut prev_char = None;
        let mut scanned = 0usize;

        for ch in chars {
            scanned += 1;
            if scanned > MAX_LOOKAHEAD {
                return false;
            }

            let brace_surface_active = !in_single && !in_double && !in_backtick;
            let at_top_level = depth == 1 && paren_depth == 0 && brace_surface_active;

            match ch {
                _ if escaped => {
                    escaped = false;
                }
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if brace_surface_active && (paren_depth > 0 || prev_char == Some('$')) => {
                    paren_depth += 1
                }
                ')' if brace_surface_active && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double && !in_backtick => depth += 1,
                '}' if !in_single && !in_double && !in_backtick => {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching }, check if we have brace expansion markers
                        return has_comma || has_dot_dot || (brace_ccl_enabled && scanned > 1);
                    }
                }
                ',' if at_top_level => has_comma = true,
                '.' if at_top_level && prev_char == Some('.') => has_dot_dot = true,
                // Brace groups have whitespace/newlines/semicolons at depth 1
                ' ' | '\t' | '\n' | ';' if at_top_level => return false,
                _ => {}
            }
            prev_char = Some(ch);
        }

        false
    }

    pub(in crate::parser) fn consume_mid_word_brace_segment(&mut self, word: &mut Option<String>) {
        let mut brace_depth = 1usize;
        let mut paren_depth = 0usize;
        let mut escaped = false;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut prev_char = None;

        while let Some(ch) = self.peek_char() {
            Self::push_capture_char(word, ch);
            self.advance();

            if escaped {
                escaped = false;
                prev_char = Some(ch);
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if !in_single
                    && !in_double
                    && !in_backtick
                    && (paren_depth > 0 || prev_char == Some('$')) =>
                {
                    paren_depth += 1
                }
                ')' if !in_single && !in_double && !in_backtick && paren_depth > 0 => {
                    paren_depth -= 1
                }
                '{' if !in_single && !in_double && !in_backtick => brace_depth += 1,
                '}' if !in_single && !in_double && !in_backtick => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        break;
                    }
                }
                _ => {}
            }

            prev_char = Some(ch);
        }
    }

    pub(in crate::parser) fn consume_brace_word_body(&mut self, word: &mut String) {
        let mut brace_depth = 1usize;
        let mut paren_depth = 0usize;
        let mut escaped = false;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut prev_char = None;

        while let Some(ch) = self.peek_char() {
            word.push(ch);
            self.advance();

            if escaped {
                escaped = false;
                prev_char = Some(ch);
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if !in_single
                    && !in_double
                    && !in_backtick
                    && (paren_depth > 0 || prev_char == Some('$')) =>
                {
                    paren_depth += 1
                }
                ')' if !in_single && !in_double && !in_backtick && paren_depth > 0 => {
                    paren_depth -= 1
                }
                '{' if !in_single && !in_double && !in_backtick => brace_depth += 1,
                '}' if !in_single && !in_double && !in_backtick => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        break;
                    }
                }
                _ => {}
            }

            prev_char = Some(ch);
        }
    }

    /// Check whether a mid-word `{...}` segment can stay attached to the current
    /// word without crossing a top-level word boundary.
    pub(in crate::parser) fn looks_like_mid_word_brace_segment(&self) -> bool {
        const MAX_LOOKAHEAD: usize = 10_000;

        let mut chars = self.lookahead_chars();
        if chars.next() != Some('{') {
            return false;
        }

        let mut brace_depth = 1;
        let mut paren_depth = 0usize;
        let mut escaped = false;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut prev_char = None;
        let mut scanned = 0usize;

        for ch in chars {
            scanned += 1;
            if scanned > MAX_LOOKAHEAD {
                return false;
            }

            if !in_single
                && !in_double
                && !in_backtick
                && !escaped
                && brace_depth == 1
                && paren_depth == 0
                && matches!(ch, ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>')
            {
                return false;
            }

            if escaped {
                escaped = false;
                prev_char = Some(ch);
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if !in_single
                    && !in_double
                    && !in_backtick
                    && (paren_depth > 0 || prev_char == Some('$')) =>
                {
                    paren_depth += 1
                }
                ')' if !in_single && !in_double && !in_backtick && paren_depth > 0 => {
                    paren_depth -= 1
                }
                '{' if !in_single && !in_double && !in_backtick => brace_depth += 1,
                '}' if !in_single && !in_double && !in_backtick => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        return true;
                    }
                }
                _ => {}
            }

            prev_char = Some(ch);
        }

        false
    }

    /// Check if { is followed by whitespace (brace group start)
    pub(in crate::parser) fn is_brace_group_start(&self) -> bool {
        let mut chars = self.lookahead_chars();
        // Skip the opening {
        if chars.next() != Some('{') {
            return false;
        }
        // If next char is whitespace or newline, it's a brace group
        matches!(chars.next(), Some(' ') | Some('\t') | Some('\n') | None)
    }

    /// Check whether the text after an escaped `{` looks like a brace-expansion
    /// surface that should stay attached to the current word, e.g. `\{a,b}`.
    pub(in crate::parser) fn escaped_brace_sequence_looks_like_brace_expansion(&mut self) -> bool {
        const MAX_LOOKAHEAD: usize = 10_000;
        let brace_ccl_enabled = self.brace_ccl_enabled();

        let mut chars = self.lookahead_chars();
        let mut depth = 1;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut prev_char = None;
        let mut scanned = 0usize;

        for ch in chars.by_ref() {
            scanned += 1;
            if scanned > MAX_LOOKAHEAD {
                return false;
            }
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return has_comma || has_dot_dot || (brace_ccl_enabled && scanned > 1);
                    }
                }
                ',' if depth == 1 => has_comma = true,
                '.' if prev_char == Some('.') && depth == 1 => has_dot_dot = true,
                ' ' | '\t' | '\n' | ';' if depth == 1 => return false,
                _ => {}
            }
            prev_char = Some(ch);
        }

        false
    }

    pub(in crate::parser) fn brace_literal_starts_case_pattern_delimiter(&self) -> bool {
        let mut chars = self.lookahead_chars();
        if chars.next() != Some('{') {
            return false;
        }
        chars.next() == Some(')')
    }

    /// Read a {literal} pattern without comma/dot-dot as a word
    pub(in crate::parser) fn read_brace_literal_word(&mut self) -> Option<LexedToken<'a>> {
        let mut word = String::with_capacity(16);

        if let Some('{') = self.peek_char() {
            word.push('{');
            self.advance();
        } else {
            return None;
        }

        self.consume_brace_word_body(&mut word);

        while let Some(ch) = self.peek_char() {
            if Self::is_word_char(ch) {
                if self.reinject_buf.is_empty() {
                    let chunk = self.cursor.eat_while(Self::is_word_char);
                    word.push_str(chunk);
                    self.advance_scanned_source_bytes(chunk.len());
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        Some(LexedToken::owned_word(TokenKind::Word, word))
    }

    /// Read a brace expansion pattern as a word
    pub(in crate::parser) fn read_brace_expansion_word(&mut self) -> Option<LexedToken<'a>> {
        let mut word = String::with_capacity(16);

        // Read the opening {
        if let Some('{') = self.peek_char() {
            word.push('{');
            self.advance();
        } else {
            return None;
        }

        // Read until matching }
        self.consume_brace_word_body(&mut word);

        // Continue reading any suffix after the brace pattern
        while let Some(ch) = self.peek_char() {
            if Self::is_word_char(ch) || matches!(ch, '{' | '}') {
                if ch == '{' {
                    // Another brace pattern - include it
                    word.push(ch);
                    self.advance();
                    self.consume_brace_word_body(&mut word);
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        Some(LexedToken::owned_word(TokenKind::Word, word))
    }

    /// Peek ahead (without consuming) to see if `=(` starts an associative
    /// compound assignment like `([key]=val ...)`.  Returns true when the
    /// first non-whitespace char after `(` is `[`.
    pub(in crate::parser) fn looks_like_assoc_assign(&self) -> bool {
        let mut chars = self.lookahead_chars();
        // Skip the `(` we haven't consumed yet
        if chars.next() != Some('(') {
            return false;
        }
        // Skip optional whitespace
        for ch in chars {
            match ch {
                ' ' | '\t' => continue,
                '[' => return true,
                _ => return false,
            }
        }
        false
    }

    pub(in crate::parser) fn word_can_take_parenthesized_suffix(text: &str) -> bool {
        text.ends_with(['@', '?', '*', '+', '!']) || Self::looks_like_zsh_glob_qualifier_base(text)
    }

    pub(in crate::parser) fn looks_like_zsh_alternative_glob_suffix(
        &mut self,
        prefix: &str,
    ) -> bool {
        if self.current_zsh_options().is_none()
            || self.peek_char() != Some('(')
            || !prefix.ends_with('.')
        {
            return false;
        }

        let mut chars = self.lookahead_chars();
        if chars.next() != Some('(') {
            return false;
        }

        let mut depth = 1usize;
        let mut escaped = false;
        let mut saw_glob_marker = false;

        for ch in chars {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return saw_glob_marker;
                    }
                }
                '|' if depth == 1 => {
                    saw_glob_marker = true;
                }
                _ => {}
            }
        }

        false
    }

    pub(in crate::parser) fn looks_like_zsh_glob_modifier_suffix(&mut self, prefix: &str) -> bool {
        if self.current_zsh_options().is_none()
            || self.peek_char() != Some('(')
            || !prefix.contains('/')
        {
            return false;
        }

        let mut chars = self.lookahead_chars();
        matches!((chars.next(), chars.next()), (Some('('), Some(':')))
    }

    pub(in crate::parser) fn lexed_word_can_take_parenthesized_suffix(
        word: &LexedWord<'_>,
    ) -> bool {
        word.segments().any(|segment| {
            matches!(
                segment.kind(),
                LexedWordSegmentKind::SingleQuoted
                    | LexedWordSegmentKind::DollarSingleQuoted
                    | LexedWordSegmentKind::DoubleQuoted
                    | LexedWordSegmentKind::DollarDoubleQuoted
            )
        }) || Self::word_can_take_parenthesized_suffix(&word.joined_text())
    }

    pub(in crate::parser) fn looks_like_zsh_glob_qualifier_base(text: &str) -> bool {
        text.contains(['*', '?'])
            || text.ends_with('}') && text.contains("${")
            || text.ends_with(']')
                && text
                    .rfind('[')
                    .is_some_and(|open_bracket| !text[..open_bracket].ends_with('$'))
    }

    pub(in crate::parser) fn is_word_char(ch: char) -> bool {
        !matches!(
            ch,
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | '(' | ')' | '{' | '}' | '\'' | '"'
        )
    }

    const fn is_ascii_word_byte(byte: u8) -> bool {
        !matches!(
            byte,
            b' ' | b'\t'
                | b'\n'
                | b';'
                | b'|'
                | b'&'
                | b'>'
                | b'<'
                | b'('
                | b')'
                | b'{'
                | b'}'
                | b'\''
                | b'"'
        )
    }

    pub(in crate::parser) const fn is_ascii_plain_word_byte(byte: u8) -> bool {
        Self::is_ascii_word_byte(byte) && !matches!(byte, b'$' | b'{' | b'`' | b'\\')
    }

    pub(in crate::parser) fn is_plain_word_char(ch: char) -> bool {
        Self::is_word_char(ch) && !matches!(ch, '$' | '{' | '`' | '\\')
    }
}
