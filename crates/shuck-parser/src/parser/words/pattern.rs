use super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn pattern_from_word(&self, word: &Word) -> Pattern {
        PatternParser::new(
            self.input,
            word,
            self.zsh_glob_parse_features_at(word.span.start.offset),
        )
        .parse()
    }

    pub(in crate::parser) fn pattern_from_conditional_word(&self, word: &Word) -> Pattern {
        let features = self.zsh_glob_parse_features_at(word.span.start.offset);
        let source_backed_word = self
            .conditional_prefixed_bare_group_source_word(word, features)
            .unwrap_or_else(|| word.clone());

        PatternParser::for_pattern_context(self.input, &source_backed_word, features).parse()
    }

    pub(in crate::parser) fn conditional_prefixed_bare_group_source_word(
        &self,
        word: &Word,
        features: ZshGlobParseFeatures,
    ) -> Option<Word> {
        if features.ksh_groups || !features.bare_groups {
            return None;
        }

        let needs_reparse = self.word_is_safe_for_prefixed_bare_group_reparse(word)
            && Self::literal_text_has_prefixed_bare_group(word.span.slice(self.input));

        needs_reparse.then(|| {
            self.word_with_parts(
                vec![WordPartNode::new(
                    WordPart::Literal(LiteralText::source()),
                    word.span,
                )],
                word.span,
            )
        })
    }

    pub(in crate::parser) fn word_is_safe_for_prefixed_bare_group_reparse(
        &self,
        word: &Word,
    ) -> bool {
        word.parts.iter().all(|part| match &part.kind {
            WordPart::Literal(text) => text.is_source_backed(),
            WordPart::ZshQualifiedGlob(glob) => {
                Self::zsh_glob_is_safe_for_prefixed_bare_group_reparse(glob)
            }
            _ => false,
        })
    }

    pub(in crate::parser) fn zsh_glob_is_safe_for_prefixed_bare_group_reparse(
        glob: &ZshQualifiedGlob,
    ) -> bool {
        glob.segments
            .iter()
            .all(Self::zsh_glob_segment_is_safe_for_prefixed_bare_group_reparse)
            && glob.qualifiers.is_none()
    }

    pub(in crate::parser) fn zsh_glob_segment_is_safe_for_prefixed_bare_group_reparse(
        segment: &ZshGlobSegment,
    ) -> bool {
        match segment {
            ZshGlobSegment::Pattern(pattern) => {
                Self::pattern_is_safe_for_prefixed_bare_group_reparse(pattern)
            }
            ZshGlobSegment::InlineControl(_) => true,
        }
    }

    pub(in crate::parser) fn pattern_is_safe_for_prefixed_bare_group_reparse(
        pattern: &Pattern,
    ) -> bool {
        pattern.parts.iter().all(|part| match &part.kind {
            PatternPart::Literal(text) => text.is_source_backed(),
            PatternPart::AnyString | PatternPart::AnyChar => true,
            PatternPart::CharClass(text) => text.is_source_backed(),
            PatternPart::Group { patterns, .. } => patterns
                .iter()
                .all(Self::pattern_is_safe_for_prefixed_bare_group_reparse),
            PatternPart::Word(_) => false,
        })
    }

    pub(in crate::parser) fn literal_text_has_prefixed_bare_group(text: &str) -> bool {
        let mut escaped = false;
        let mut bracket_depth = 0usize;
        let mut previous_char = None;

        for (index, ch) in text.char_indices() {
            if escaped {
                escaped = false;
                previous_char = Some(ch);
                continue;
            }

            if ch == '\\' {
                escaped = true;
                previous_char = Some(ch);
                continue;
            }

            match ch {
                '[' => bracket_depth = bracket_depth.saturating_add(1),
                ']' if bracket_depth > 0 => bracket_depth -= 1,
                '(' if bracket_depth == 0
                    && matches!(previous_char, Some('?' | '*' | '+' | '@' | '!'))
                    && Self::literal_text_group_has_top_level_separator(text, index) =>
                {
                    return true;
                }
                _ => {}
            }

            previous_char = Some(ch);
        }

        false
    }

    pub(in crate::parser) fn literal_text_group_has_top_level_separator(
        text: &str,
        open_index: usize,
    ) -> bool {
        let mut escaped = false;
        let mut paren_depth = 0usize;

        for ch in text[open_index + '('.len_utf8()..].chars() {
            if escaped {
                escaped = false;
                continue;
            }

            if ch == '\\' {
                escaped = true;
                continue;
            }

            match ch {
                '(' => paren_depth = paren_depth.saturating_add(1),
                ')' if paren_depth == 0 => return false,
                ')' => paren_depth -= 1,
                '|' if paren_depth == 0 => return true,
                _ => {}
            }
        }

        false
    }

    pub(in crate::parser) fn pattern_from_zsh_case_span(&mut self, span: Span) -> Pattern {
        let text = span.slice(self.input);
        let word = if Self::source_text_needs_quote_preserving_decode(text) {
            self.decode_fragment_word_text(text, span, span.start, true)
        } else {
            self.decode_word_text(text, span, span.start, true)
        };
        let features = self.zsh_glob_parse_features_at(span.start.offset);
        self.pattern_from_zsh_case_word_with_features(&word, features)
    }

    pub(in crate::parser) fn pattern_from_zsh_case_word_with_features(
        &self,
        word: &Word,
        features: ZshGlobParseFeatures,
    ) -> Pattern {
        PatternParser::for_pattern_context(self.input, word, features).parse()
    }

    pub(in crate::parser) fn pattern_from_source_text(&mut self, text: &SourceText) -> Pattern {
        let span = text.span();
        if self.source_text_pattern_depth >= SOURCE_TEXT_PATTERN_REPARSE_MAX_DEPTH {
            return Pattern {
                parts: vec![PatternPartNode::new(
                    PatternPart::Literal(if text.is_source_backed() {
                        LiteralText::source()
                    } else {
                        LiteralText::owned(text.slice(self.input).to_string())
                    }),
                    span,
                )],
                span,
            };
        }

        self.source_text_pattern_depth += 1;
        let mut parts = WordPartBuffer::new();
        self.decode_word_parts_into_with_quote_fragments(
            text.slice(self.input),
            span.start,
            text.is_source_backed(),
            DecodeWordPartsOptions {
                preserve_quote_fragments: true,
                parse_dollar_quotes: true,
                preserve_escaped_expansion_literals: text.is_source_backed(),
                ..DecodeWordPartsOptions::default()
            },
            &mut parts,
        );
        let pattern = PatternParser::from_word_parts(
            self.input,
            &parts,
            span,
            self.zsh_glob_parse_features_at(span.start.offset),
        )
        .parse();
        self.source_text_pattern_depth -= 1;
        pattern
    }

    pub(in crate::parser) fn single_literal_word_text<'b>(
        &'b self,
        word: &'b Word,
    ) -> Option<&'b str> {
        if word.is_fully_quoted() || word.parts.len() != 1 {
            return None;
        }
        let WordPart::Literal(text) = &word.parts[0].kind else {
            return None;
        };
        Some(text.as_str(self.input, word.part_span(0)?))
    }

    pub(in crate::parser) fn literal_word_text(&self, word: &Word) -> Option<String> {
        let mut text = String::new();
        self.collect_literal_word_text(&word.parts, &mut text)?;
        Some(text)
    }

    pub(in crate::parser) fn source_text_needs_quote_preserving_decode(text: &str) -> bool {
        text.contains(['\'', '"'])
    }

    pub(in crate::parser) fn decode_word_text_preserving_quotes_if_needed(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_preserving_quotes_if_needed_with_escape_mode(
            s,
            span,
            base,
            source_backed,
            source_backed,
        )
    }

    pub(in crate::parser) fn decode_word_text_preserving_quotes_if_needed_with_escape_mode(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
    ) -> Word {
        if !Self::source_text_needs_quote_preserving_decode(s)
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(s, span, source_backed)
        {
            return word;
        }

        let preserve_quote_fragments = Self::source_text_needs_quote_preserving_decode(s)
            && (!source_backed || self.source_matches(span, s));

        if preserve_quote_fragments {
            self.decode_fragment_word_text_with_escape_mode(
                s,
                span,
                base,
                source_backed,
                preserve_escaped_expansion_literals,
            )
        } else {
            self.decode_word_text_with_escape_mode(
                s,
                span,
                base,
                source_backed,
                preserve_escaped_expansion_literals,
            )
        }
    }

    pub(in crate::parser) fn collect_literal_word_text(
        &self,
        parts: &[WordPartNode],
        out: &mut String,
    ) -> Option<()> {
        for part in parts {
            match &part.kind {
                WordPart::Literal(literal) => out.push_str(literal.as_str(self.input, part.span)),
                WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(self.input)),
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_literal_word_text(parts, out)?;
                }
                _ => return None,
            }
        }

        Some(())
    }
    pub(in crate::parser) fn is_assignment(
        word: &str,
        allow_zsh_numeric_assignments: bool,
    ) -> Option<(&str, Option<&str>, &str, bool)> {
        if !word.contains('=') {
            return None;
        }

        let mut ident_end = 0;
        let mut chars = word.char_indices();
        let (_, first) = chars.next()?;
        if first.is_ascii_digit() {
            if !allow_zsh_numeric_assignments {
                return None;
            }
            ident_end += first.len_utf8();
            for (index, ch) in chars {
                if ch.is_ascii_digit() {
                    ident_end = index + ch.len_utf8();
                } else {
                    break;
                }
            }
        } else {
            if !first.is_ascii_alphabetic() && first != '_' {
                return None;
            }
            ident_end += first.len_utf8();
            for (index, ch) in chars {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ident_end = index + ch.len_utf8();
                } else {
                    break;
                }
            }
        }

        let name = &word[..ident_end];
        let mut cursor = ident_end;
        let mut index = None;

        if word[cursor..].starts_with('[') {
            let mut close_index = None;
            let mut bracket_depth = 0_i32;
            let mut brace_depth = 0_i32;
            let mut paren_depth = 0_i32;
            let mut in_single = false;
            let mut in_double = false;
            let mut escaped = false;

            for (relative, ch) in word[cursor + 1..].char_indices() {
                let absolute = cursor + 1 + relative;
                if escaped {
                    escaped = false;
                    continue;
                }

                match ch {
                    '\\' if !in_single => escaped = true,
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    '[' if !in_single && !in_double => bracket_depth += 1,
                    ']' if !in_single && !in_double => {
                        if bracket_depth == 0 && brace_depth == 0 && paren_depth == 0 {
                            close_index = Some(absolute);
                            break;
                        }
                        bracket_depth -= 1;
                    }
                    '{' if !in_single && !in_double => brace_depth += 1,
                    '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                    '(' if !in_single && !in_double => paren_depth += 1,
                    ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                    _ => {}
                }
            }

            let close_index = close_index?;
            index = Some(&word[cursor + 1..close_index]);
            cursor = close_index + 1;
        }

        let (is_append, value) = if word[cursor..].starts_with("+=") {
            (true, &word[cursor + 2..])
        } else if word[cursor..].starts_with('=') {
            (false, &word[cursor + 1..])
        } else {
            return None;
        };

        Some((name, index, value, is_append))
    }

    pub(in crate::parser) fn scan_split_indexed_assignment(
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

        let first = *chars.peek()?;
        if !first.is_ascii_alphabetic() && first != '_' {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
        text.push_str(&Self::read_word_while(&mut chars, &mut cursor, |c| {
            c.is_ascii_alphanumeric() || c == '_'
        }));

        if chars.peek() != Some(&'[') {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));

        let mut bracket_depth = 1_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(ch) = Self::next_word_char(&mut chars, &mut cursor) {
            text.push(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double => {
                    bracket_depth -= 1;
                    if bracket_depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        if bracket_depth != 0 {
            return None;
        }

        if chars.peek() == Some(&'+') {
            text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
        }

        if chars.peek() != Some(&'=') {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));

        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(&ch) = chars.peek() {
            if !in_single
                && !in_double
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
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }
        }

        Some((text, cursor))
    }

    pub(in crate::parser) fn try_parse_split_indexed_assignment_from_text(
        &mut self,
    ) -> Option<Assignment> {
        if !self.at(TokenKind::Word) {
            return None;
        }

        let start = self.current_span.start;
        let (text, end) = self.scan_split_indexed_assignment(start)?;
        let span = Span::from_positions(start, end);
        let assignment = self.parse_assignment_from_text(
            &text,
            span,
            None,
            SubscriptInterpretation::Contextual,
        )?;

        while self.current_token.is_some() && self.current_span.start.offset < end.offset {
            self.advance();
        }

        Some(assignment)
    }

    pub(in crate::parser) fn infer_array_expr_kind(
        explicit_kind: Option<ArrayKind>,
        elements: &[ArrayElem],
    ) -> ArrayKind {
        explicit_kind.unwrap_or_else(|| {
            if elements
                .iter()
                .any(|element| !matches!(element, ArrayElem::Sequential(_)))
            {
                ArrayKind::Contextual
            } else {
                ArrayKind::Indexed
            }
        })
    }

    pub(in crate::parser) fn subscript_interpretation_from_array_kind(
        explicit_kind: Option<ArrayKind>,
    ) -> SubscriptInterpretation {
        match explicit_kind {
            Some(ArrayKind::Indexed) => SubscriptInterpretation::Indexed,
            Some(ArrayKind::Associative) => SubscriptInterpretation::Associative,
            _ => SubscriptInterpretation::Contextual,
        }
    }

    pub(in crate::parser) fn word_from_raw_text(&mut self, raw: &str, span: Span) -> Word {
        if raw.is_empty() {
            return Word::literal_with_span("", span);
        }

        self.parse_word_with_context(raw, span, span.start, self.source_matches(span, raw))
    }

    pub(in crate::parser) fn array_value_word_from_raw_text(
        &mut self,
        raw: &str,
        span: Span,
    ) -> ArrayValueWord {
        let word = self.word_from_raw_text(raw, span);
        let has_top_level_unquoted_comma =
            self.raw_text_has_top_level_unquoted_array_comma(raw, &word);
        ArrayValueWord::new(word, has_top_level_unquoted_comma)
    }

    pub(in crate::parser) fn split_compound_array_elements(
        &self,
        inner: &str,
    ) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut start: Option<usize> = None;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut paren_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut index = 0usize;

        while index < inner.len() {
            let Some(ch) = inner[index..].chars().next() else {
                break;
            };
            let next_index = index + ch.len_utf8();
            if start.is_none() {
                if ch.is_whitespace() {
                    index = next_index;
                    continue;
                }
                if ch == '#' {
                    while index < inner.len() {
                        let Some(comment_ch) = inner[index..].chars().next() else {
                            break;
                        };
                        index += comment_ch.len_utf8();
                        if comment_ch == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                start = Some(index);
            }

            if escaped {
                escaped = false;
                index = next_index;
                continue;
            }

            if ch == '$'
                && !in_single
                && let Some(end) = Self::scan_raw_dollar_paren_substitution_end(inner, index)
            {
                index = end;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '`' if !in_single => in_backtick = !in_backtick,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double && bracket_depth > 0 => bracket_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '#' if start == Some(index)
                    && !in_single
                    && !in_double
                    && !in_backtick
                    && bracket_depth == 0
                    && brace_depth == 0
                    && paren_depth == 0 =>
                {
                    start = None;
                    while index < inner.len() {
                        let Some(comment_ch) = inner[index..].chars().next() else {
                            break;
                        };
                        index += comment_ch.len_utf8();
                        if comment_ch == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                ch if ch.is_whitespace()
                    && !in_single
                    && !in_double
                    && !in_backtick
                    && bracket_depth == 0
                    && brace_depth == 0
                    && paren_depth == 0 =>
                {
                    if let Some(start) = start.take() {
                        ranges.push((start, index));
                    }
                }
                _ => {}
            }

            index = next_index;
        }

        if let Some(start) = start {
            ranges.push((start, inner.len()));
        }

        ranges
    }
}
