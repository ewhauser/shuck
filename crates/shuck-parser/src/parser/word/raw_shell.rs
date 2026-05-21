use super::super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn current_zsh_glob_word_from_source(&mut self) -> Option<Word> {
        let kind = self.current_token_kind?;
        if !matches!(kind, TokenKind::LeftParen | TokenKind::Word) {
            return None;
        }
        // Non-zsh dialects only need this path to rescue `(#...)`-leading words
        // from being misparsed as subshells. For Word-kind tokens the regular
        // word-decode path already produces the right AST, so skip the byte
        // walk entirely.
        if kind == TokenKind::Word && !self.dialect.features().zsh_glob_qualifiers {
            return None;
        }

        let start = self.current_span.start;
        if !self.source_word_contains_zsh_glob_control(start) {
            return None;
        }
        let (text, end) = self.scan_source_word(start)?;
        if !text.contains("(#") {
            return None;
        }
        let span = Span::from_positions(start, end);
        if self.zsh_glob_word_parsing_enabled_at(span.start.offset)
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(&text, span, true)
        {
            return Some(word);
        }

        Some(self.parse_word_with_context(&text, span, start, true))
    }

    pub(in crate::parser) fn source_word_contains_zsh_glob_control(&self, start: Position) -> bool {
        if start.offset >= self.input.len() {
            return false;
        }

        let source = &self.input[start.offset..];
        let mut chars = source.chars().peekable();
        let mut cursor = start;
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut prev_char = None;

        while let Some(&ch) = chars.peek() {
            if !in_single
                && !in_double
                && !in_backtick
                && paren_depth == 0
                && brace_depth == 0
                && matches!(ch, ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | ')')
            {
                break;
            }

            let ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);
            if !in_single && !in_double && !in_backtick && prev_char == Some('(') && ch == '#' {
                return true;
            }

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
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }

            prev_char = Some(ch);
        }

        false
    }

    pub(in crate::parser) fn scan_source_word(
        &self,
        start: Position,
    ) -> Option<(String, Position)> {
        if start.offset >= self.input.len() {
            return None;
        }

        let source = &self.input[start.offset..];
        let mut chars = source.chars().peekable();
        let mut cursor = start;
        let mut text = String::new();
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while let Some(&ch) = chars.peek() {
            if !in_single
                && !in_double
                && !in_backtick
                && paren_depth == 0
                && brace_depth == 0
                && matches!(ch, ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | ')')
            {
                break;
            }

            let ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);
            text.push(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '`' if !in_single => in_backtick = !in_backtick,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }
        }

        (!text.is_empty()).then_some((text, cursor))
    }

    pub(in crate::parser) fn token_source_like_word_text(
        &self,
        token: &LexedToken<'a>,
    ) -> Option<Cow<'a, str>> {
        token
            .source_slice(self.input)
            .map(Cow::Borrowed)
            .or_else(|| {
                (token.span.start.offset <= token.span.end.offset
                    && token.span.end.offset <= self.input.len())
                .then(|| Cow::Borrowed(&self.input[token.span.start.offset..token.span.end.offset]))
            })
            .or_else(|| token.word_string().map(Cow::Owned))
    }

    pub(in crate::parser) fn current_source_like_word_text(&self) -> Option<Cow<'a, str>> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and(self.current_token.as_ref())
            .and_then(|token| self.token_source_like_word_text(token))
    }

    pub(in crate::parser) fn current_source_like_word_text_or_error(
        &self,
        context: &'static str,
    ) -> Result<Cow<'a, str>> {
        self.current_source_like_word_text().ok_or_else(|| {
            self.error(format!(
                "internal parser error: missing source text for {context}"
            ))
        })
    }
}
