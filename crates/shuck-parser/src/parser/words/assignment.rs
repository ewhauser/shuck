use super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn scan_compound_array_close(
        &self,
        open_paren_span: Span,
    ) -> Option<Span> {
        let mut cursor = open_paren_span.end;
        let mut paren_depth = 0_i32;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];
            let ch = rest.chars().next()?;
            let ch_start = cursor;
            cursor.advance(ch);

            if escaped {
                escaped = false;
                continue;
            }

            if ch == '$' && !in_single {
                let next_offset = ch_start.offset + ch.len_utf8();
                if self.input[next_offset..].starts_with("((")
                    && let Some(consumed) =
                        Self::scan_array_arithmetic_expansion_len(&self.input[next_offset + 2..])
                {
                    let end = next_offset + 2 + consumed;
                    cursor = ch_start.advanced_by(&self.input[ch_start.offset..end]);
                    continue;
                }

                if self.input[next_offset..].starts_with('{')
                    && let Some(consumed) =
                        Self::scan_array_parameter_expansion_len(&self.input[next_offset + 1..])
                {
                    let end = next_offset + 1 + consumed;
                    cursor = ch_start.advanced_by(&self.input[ch_start.offset..end]);
                    continue;
                }

                if let Some(end) =
                    Self::scan_raw_dollar_paren_substitution_end(self.input, ch_start.offset)
                {
                    cursor = ch_start.advanced_by(&self.input[ch_start.offset..end]);
                    continue;
                }
            }

            match ch {
                '#' if !in_single
                    && !in_double
                    && !in_backtick
                    && bracket_depth == 0
                    && brace_depth == 0
                    && paren_depth == 0
                    && Self::raw_source_hash_starts_comment(self.input, ch_start.offset) =>
                {
                    while cursor.offset < self.input.len() {
                        let Some(comment_ch) = self.input[cursor.offset..].chars().next() else {
                            break;
                        };
                        cursor.advance(comment_ch);
                        if comment_ch == '\n' {
                            break;
                        }
                    }
                }
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '[' if !in_single && !in_double && !in_backtick => bracket_depth += 1,
                ']' if !in_single && !in_double && !in_backtick && bracket_depth > 0 => {
                    bracket_depth -= 1;
                }
                '{' if !in_single && !in_double && !in_backtick => brace_depth += 1,
                '}' if !in_single && !in_double && !in_backtick && brace_depth > 0 => {
                    brace_depth -= 1;
                }
                '(' if !in_single && !in_double && !in_backtick => paren_depth += 1,
                ')' if !in_single && !in_double && !in_backtick => {
                    if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                        return Some(Span::from_positions(ch_start, cursor));
                    }
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Parse a simple command with redirections.
    pub(in crate::parser) fn collect_compound_array(
        &mut self,
        open_paren_span: Span,
        explicit_kind: Option<ArrayKind>,
    ) -> (ArrayExpr, Span) {
        if let Some(closing_span) = self.scan_compound_array_close(open_paren_span) {
            let inner =
                self.input[open_paren_span.end.offset..closing_span.start.offset].to_string();
            while self.current_token.is_some()
                && self.current_span.start.offset < closing_span.end.offset
            {
                self.advance();
            }

            let mut array =
                self.parse_array_expr_from_text(&inner, open_paren_span.end, explicit_kind);
            array.span = open_paren_span.merge(closing_span);
            return (array, closing_span);
        }

        let inner_start = open_paren_span.end;
        let mut closing_span = Span::new();
        let mut fallback = String::new();

        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    if !fallback.is_empty() {
                        fallback.push(' ');
                    }
                    if let Some(word) = self.current_source_like_word_text() {
                        fallback.push_str(&word);
                    }
                    self.advance();
                }
                None => break,
                _ => self.advance(),
            }
        }

        let inner = if closing_span != Span::new()
            && inner_start.offset <= closing_span.start.offset
            && closing_span.start.offset <= self.input.len()
        {
            self.input[inner_start.offset..closing_span.start.offset].to_string()
        } else {
            fallback
        };

        let mut array = self.parse_array_expr_from_text(&inner, inner_start, explicit_kind);
        array.span = if closing_span == Span::new() {
            open_paren_span
        } else {
            open_paren_span.merge(closing_span)
        };
        (array, closing_span)
    }

    pub(in crate::parser) fn trim_literal_prefix(
        &self,
        literal: LiteralText,
        span: Span,
        start: Position,
    ) -> Option<(LiteralText, Span)> {
        if start.offset <= span.start.offset {
            return Some((literal, span));
        }
        if start.offset >= span.end.offset {
            return None;
        }

        let trimmed_span = Span::from_positions(start, span.end);
        let literal = match literal {
            LiteralText::Source => LiteralText::source(),
            LiteralText::Owned(text) | LiteralText::CookedSource(text) => {
                let split_at = start.offset.saturating_sub(span.start.offset);
                LiteralText::owned(text.get(split_at..)?.to_string())
            }
        };
        Some((literal, trimmed_span))
    }

    pub(in crate::parser) fn trim_word_part_prefix(
        &self,
        part: WordPart,
        span: Span,
        start: Position,
    ) -> Option<(WordPart, Span)> {
        if start.offset <= span.start.offset {
            return Some((part, span));
        }
        if start.offset >= span.end.offset {
            return None;
        }

        match part {
            WordPart::Literal(literal) => self
                .trim_literal_prefix(literal, span, start)
                .map(|(literal, span)| (WordPart::Literal(literal), span)),
            _ => None,
        }
    }

    pub(in crate::parser) fn split_word_at(&self, word: Word, start: Position) -> Word {
        let value_span = Span::from_positions(start, word.span.end);
        let mut parts = Self::word_part_buffer_with_capacity(word.parts.len());

        for part in word.parts {
            if let Some((kind, span)) = self.trim_word_part_prefix(part.kind, part.span, start) {
                Self::push_word_part_node(&mut parts, WordPartNode::new(kind, span));
            }
        }

        self.word_with_part_buffer(parts, value_span)
    }

    pub(in crate::parser) fn parse_assignment_from_word(
        &mut self,
        word: Word,
        explicit_array_kind: Option<ArrayKind>,
        subscript_interpretation: SubscriptInterpretation,
    ) -> Option<Assignment> {
        let assignment_span = word.span;
        let ParsedWordTarget {
            name,
            name_span,
            subscript,
            boundary,
        } = self.parse_word_target(&word, subscript_interpretation, true)?;
        let WordTargetBoundary::Assignment {
            append,
            value_start,
        } = boundary
        else {
            return None;
        };
        let target = self.var_ref(name, name_span, subscript, assignment_span);
        let value_word = self.split_word_at(word, value_start);

        let value = if value_word.parts.is_empty() {
            AssignmentValue::Scalar(Word::literal_with_span(
                "",
                Span::from_positions(value_start, assignment_span.end),
            ))
        } else if let Some((inner, inner_start)) = self
            .compound_array_inner_text(&value_word)
            .map(|(inner, inner_start)| (inner.into_owned(), inner_start))
        {
            AssignmentValue::Compound(self.parse_array_expr_from_text(
                &inner,
                inner_start,
                explicit_array_kind,
            ))
        } else {
            AssignmentValue::Scalar(value_word)
        };

        Some(Assignment {
            target,
            value,
            append,
            span: assignment_span,
        })
    }

    pub(in crate::parser) fn parse_assignment_from_text(
        &mut self,
        w: &str,
        assignment_span: Span,
        explicit_array_kind: Option<ArrayKind>,
        subscript_interpretation: SubscriptInterpretation,
    ) -> Option<Assignment> {
        let source_backed = assignment_span.end.offset <= self.input.len()
            && assignment_span.slice(self.input) == w;
        let word = self.decode_word_text_preserving_quotes_if_needed(
            w,
            assignment_span,
            assignment_span.start,
            source_backed,
        );
        self.parse_assignment_from_word(word, explicit_array_kind, subscript_interpretation)
    }

    fn parse_word_target(
        &self,
        word: &Word,
        interpretation: SubscriptInterpretation,
        allow_assignment: bool,
    ) -> Option<ParsedWordTarget> {
        let first_part = word.parts.first()?;
        let WordPart::Literal(first_literal) = &first_part.kind else {
            return None;
        };
        let first_text = first_literal.as_str(self.input, first_part.span);
        let mut name_end = 0;
        let mut first_chars = first_text.char_indices();
        if let Some((_, first)) = first_chars.next() {
            if first.is_ascii_digit() && self.dialect.features().zsh_parameter_modifiers {
                name_end = first.len_utf8();
                for (offset, ch) in first_chars {
                    if ch.is_ascii_digit() {
                        name_end = offset + ch.len_utf8();
                    } else {
                        break;
                    }
                }
            } else {
                for (offset, ch) in first_text.char_indices() {
                    if (offset == 0 && (ch.is_ascii_alphabetic() || ch == '_'))
                        || (offset > 0 && (ch.is_ascii_alphanumeric() || ch == '_'))
                    {
                        name_end = offset + ch.len_utf8();
                    } else {
                        break;
                    }
                }
            }
        }
        if name_end == 0 {
            return None;
        }

        let name_text = &first_text[..name_end];
        let name = Name::from(name_text);
        let name_span =
            Span::from_positions(word.span.start, word.span.start.advanced_by(name_text));
        let mut after_name = name_end;
        let mut in_subscript = false;
        let mut bracket_depth = 0usize;
        let mut subscript_start = None;
        let mut subscript_end = None;
        let mut subscript_text = String::new();

        for (part_index, part) in word.parts.iter().enumerate() {
            match &part.kind {
                WordPart::Literal(text) => {
                    let text = text.as_str(self.input, part.span);
                    let mut offset = if part_index == 0 { after_name } else { 0 };
                    while offset < text.len() {
                        let ch = text[offset..].chars().next()?;
                        let next_offset = offset + ch.len_utf8();
                        let ch_start = part.span.start.advanced_by(&text[..offset]);
                        let ch_end = part.span.start.advanced_by(&text[..next_offset]);

                        if in_subscript {
                            match ch {
                                '[' => {
                                    bracket_depth += 1;
                                    subscript_text.push(ch);
                                }
                                ']' if bracket_depth == 0 => {
                                    subscript_end = Some(ch_start);
                                    in_subscript = false;
                                }
                                ']' => {
                                    bracket_depth -= 1;
                                    subscript_text.push(ch);
                                }
                                _ => subscript_text.push(ch),
                            }
                            offset = next_offset;
                            continue;
                        }

                        match ch {
                            '[' if subscript_start.is_none() => {
                                subscript_start = Some(ch_end);
                                in_subscript = true;
                            }
                            '=' if allow_assignment => {
                                return Some(ParsedWordTarget {
                                    name,
                                    name_span,
                                    subscript: self.build_target_subscript(
                                        subscript_text,
                                        subscript_start.zip(subscript_end),
                                        interpretation,
                                    )?,
                                    boundary: WordTargetBoundary::Assignment {
                                        append: false,
                                        value_start: ch_end,
                                    },
                                });
                            }
                            '+' if allow_assignment && text[next_offset..].starts_with('=') => {
                                return Some(ParsedWordTarget {
                                    name,
                                    name_span,
                                    subscript: self.build_target_subscript(
                                        subscript_text,
                                        subscript_start.zip(subscript_end),
                                        interpretation,
                                    )?,
                                    boundary: WordTargetBoundary::Assignment {
                                        append: true,
                                        value_start: part
                                            .span
                                            .start
                                            .advanced_by(&text[..next_offset + '='.len_utf8()]),
                                    },
                                });
                            }
                            _ => return None,
                        }
                        offset = next_offset;
                    }
                }
                _ => {
                    if !in_subscript {
                        return None;
                    }
                    subscript_text.push_str(self.word_part_syntax_text(part).as_ref());
                }
            }
            after_name = 0;
        }

        if in_subscript {
            return None;
        }

        Some(ParsedWordTarget {
            name,
            name_span,
            subscript: self.build_target_subscript(
                subscript_text,
                subscript_start.zip(subscript_end),
                interpretation,
            )?,
            boundary: WordTargetBoundary::EndOfWord,
        })
    }

    pub(in crate::parser) fn build_target_subscript(
        &self,
        text: String,
        span: Option<(Position, Position)>,
        interpretation: SubscriptInterpretation,
    ) -> Option<Option<Subscript>> {
        let Some((start, end)) = span else {
            return Some(None);
        };
        let subscript_span = Span::from_positions(start, end);
        let (text, raw) = self.subscript_source_text(&text, subscript_span);
        Some(Some(self.subscript_from_source_text(
            text,
            raw,
            interpretation,
        )))
    }

    pub(in crate::parser) fn zsh_parameter_requires_fallback(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        if !self.dialect.features().zsh_parameter_modifiers {
            return false;
        }

        match chars.peek().copied() {
            Some('"') | Some('\'') => true,
            Some(ch) if ch.is_ascii_digit() => self.zsh_numeric_parameter_requires_fallback(chars),
            Some('$') => {
                let mut lookahead = chars.clone();
                lookahead.next();
                matches!(lookahead.peek().copied(), Some('(' | '{' | '"' | '\''))
            }
            _ => false,
        }
    }

    pub(in crate::parser) fn zsh_numeric_parameter_requires_fallback(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        let mut lookahead = chars.clone();
        while matches!(lookahead.peek(), Some(ch) if ch.is_ascii_digit()) {
            lookahead.next();
        }

        if lookahead.peek().copied() != Some(':') {
            return false;
        }

        lookahead.next();
        Self::zsh_modifier_suffix_candidate_chars(&mut lookahead)
    }

    pub(in crate::parser) fn zsh_parameter_suffix_looks_like_modifier(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        if !self.dialect.features().zsh_parameter_modifiers || chars.peek().copied() != Some(':') {
            return false;
        }

        let mut lookahead = chars.clone();
        lookahead.next();
        Self::zsh_modifier_suffix_candidate_chars(&mut lookahead)
    }

    pub(in crate::parser) fn zsh_length_parameter_requires_fallback(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &Position,
        source_backed: bool,
    ) -> bool {
        if !self.dialect.features().zsh_parameter_modifiers {
            return false;
        }

        let mut lookahead = chars.clone();
        let mut lookahead_cursor = *cursor;
        let tail = self.read_brace_operand(&mut lookahead, &mut lookahead_cursor, source_backed);
        let raw_body = self.prefixed_parameter_raw_body("#", *cursor, tail, source_backed);
        self.find_zsh_operation_start(raw_body.slice(self.input))
            .is_some()
    }

    pub(in crate::parser) fn parse_zsh_bare_prefixed_parameter(
        &mut self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        part_start: Position,
        source_backed: bool,
    ) -> Option<WordPart> {
        if !self.dialect.features().zsh_parameter_modifiers
            || !chars
                .peek()
                .copied()
                .is_some_and(Self::zsh_bare_parameter_prefix_modifier)
        {
            return None;
        }

        let mut lookahead = chars.clone();
        while lookahead
            .peek()
            .copied()
            .is_some_and(Self::zsh_bare_parameter_prefix_modifier)
        {
            lookahead.next();
        }
        if !Self::zsh_bare_parameter_target_starts(&lookahead) {
            return None;
        }

        let raw_body_start = *cursor;
        let mut raw_body_text = String::new();
        while chars
            .peek()
            .copied()
            .is_some_and(Self::zsh_bare_parameter_prefix_modifier)
        {
            raw_body_text.push(Self::next_word_char_unwrap(chars, cursor));
        }

        let first = Self::next_word_char_unwrap(chars, cursor);
        raw_body_text.push(first);
        if first == '+' {
            raw_body_text.push_str(&Self::read_word_while(chars, cursor, |ch| {
                ch.is_ascii_alphanumeric() || ch == '_'
            }));
        } else if first.is_ascii_alphabetic() || first == '_' {
            raw_body_text.push_str(&Self::read_word_while(chars, cursor, |ch| {
                ch.is_ascii_alphanumeric() || ch == '_'
            }));
        }
        if Self::consume_word_char_if(chars, cursor, '[') {
            raw_body_text.push('[');
            let (index, raw_index) = self.read_array_index(chars, cursor, source_backed);
            raw_body_text.push_str(raw_index.as_ref().unwrap_or(&index).slice(self.input));
            raw_body_text.push(']');
        }

        let span = Span::from_positions(raw_body_start, *cursor);
        let raw_body = if source_backed
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == raw_body_text
        {
            span.slice(self.input).to_owned()
        } else {
            raw_body_text
        };
        let raw_body = self.source_text(raw_body, raw_body_start, *cursor);
        Some(self.zsh_parameter_word_part(raw_body, part_start, *cursor))
    }

    pub(in crate::parser) fn zsh_bare_parameter_prefix_modifier(ch: char) -> bool {
        matches!(ch, '=' | '^' | '~')
    }

    pub(in crate::parser) fn zsh_bare_parameter_target_start(ch: char) -> bool {
        matches!(ch, '?' | '#' | '@' | '*' | '!' | '$' | '-')
            || ch.is_ascii_alphanumeric()
            || ch == '_'
    }

    pub(in crate::parser) fn zsh_bare_parameter_target_starts(
        chars: &std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        let mut lookahead = chars.clone();
        match lookahead.next() {
            Some('+') => lookahead
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            Some(ch) => Self::zsh_bare_parameter_target_start(ch),
            None => false,
        }
    }

    pub(in crate::parser) fn zsh_modifier_suffix_candidate_chars(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    ) -> bool {
        let mut saw_segment = false;

        loop {
            let Some(first) = chars.next() else {
                return saw_segment;
            };

            if first == '}' {
                return saw_segment;
            }

            match first {
                'a' | 'A' | 'c' | 'e' | 'l' | 'P' | 'q' | 'Q' | 'r' | 'u' => {}
                'h' | 't' => {
                    while matches!(chars.peek(), Some(ch) if ch.is_ascii_digit()) {
                        chars.next();
                    }
                }
                _ => return false,
            }

            saw_segment = true;

            match chars.peek().copied() {
                Some(':') => {
                    chars.next();
                }
                Some('}') | None => return true,
                _ => return false,
            }
        }
    }

    pub(in crate::parser) fn prefixed_parameter_raw_body(
        &self,
        prefix: &str,
        prefix_start: Position,
        tail: SourceText,
        source_backed: bool,
    ) -> SourceText {
        if source_backed && tail.is_source_backed() {
            SourceText::source(Span::from_positions(prefix_start, tail.span().end))
        } else {
            let prefix_end = prefix_start.advanced_by(prefix);
            self.source_text(
                format!("{prefix}{}", tail.slice(self.input)),
                prefix_start,
                prefix_end.advanced_by(tail.slice(self.input)),
            )
        }
    }

    pub(in crate::parser) fn parse_var_ref_from_word(
        &self,
        word: &Word,
        interpretation: SubscriptInterpretation,
    ) -> Option<VarRef> {
        let ParsedWordTarget {
            name,
            name_span,
            subscript,
            boundary,
        } = self.parse_word_target(word, interpretation, false)?;
        matches!(boundary, WordTargetBoundary::EndOfWord)
            .then(|| self.var_ref(name, name_span, subscript, word.span))
    }

    pub(in crate::parser) fn is_valid_identifier(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }

        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    pub(in crate::parser) fn is_literal_flag_text(text: &str) -> bool {
        if text.contains('=') {
            return false;
        }

        let Some(first) = text.chars().next() else {
            return false;
        };
        if first != '-' && first != '+' {
            return false;
        }

        true
    }

    pub(in crate::parser) fn classify_decl_operand(
        &mut self,
        word: Word,
        explicit_array_kind: Option<ArrayKind>,
    ) -> DeclOperand {
        let interpretation = Self::subscript_interpretation_from_array_kind(explicit_array_kind);

        if self
            .single_literal_word_text(&word)
            .is_some_and(Self::is_literal_flag_text)
        {
            return DeclOperand::Flag(word);
        }

        if let Some(assignment) =
            self.parse_assignment_from_word(word.clone(), explicit_array_kind, interpretation)
        {
            return DeclOperand::Assignment(assignment);
        }

        if let Some(name) = self.parse_var_ref_from_word(&word, interpretation) {
            return DeclOperand::Name(name);
        }

        DeclOperand::Dynamic(word)
    }

    pub(in crate::parser) fn explicit_array_kind_from_flag_text(text: &str) -> Option<ArrayKind> {
        if !text.starts_with('-') {
            return None;
        }

        let mut explicit_kind = None;
        for flag in text.chars().skip(1) {
            match flag {
                'a' => explicit_kind = Some(ArrayKind::Indexed),
                'A' => explicit_kind = Some(ArrayKind::Associative),
                _ => {}
            }
        }
        explicit_kind
    }

    pub(in crate::parser) fn classify_decl_operands(
        &mut self,
        words: SmallVec<[Word; 2]>,
    ) -> SmallVec<[DeclOperand; 2]> {
        let mut explicit_array_kind = None;
        let mut operands = SmallVec::<[DeclOperand; 2]>::with_capacity(words.len());

        for word in words {
            if let Some(text) = self.single_literal_word_text(&word)
                && Self::is_literal_flag_text(text)
            {
                explicit_array_kind =
                    Self::explicit_array_kind_from_flag_text(text).or(explicit_array_kind);
                operands.push(DeclOperand::Flag(word));
                continue;
            }

            operands.push(self.classify_decl_operand(word, explicit_array_kind));
        }

        operands
    }

    /// Parse the value side of an assignment (`VAR=value`).
    /// Returns `Some((Assignment, needs_advance))` if the current word is an assignment.
    /// The bool indicates whether the caller must call `self.advance()` afterward.
    pub(in crate::parser) fn try_parse_assignment_with_shape(
        &mut self,
        raw: &str,
        assignment_shape: Option<(&str, Option<&str>, &str, bool)>,
    ) -> Option<(Assignment, bool)> {
        let (_, _, value_str, _) = assignment_shape?;

        // Empty value — check for arr=(...) syntax with separate tokens
        if value_str.is_empty() {
            let assignment_span = self.current_span;
            let word = self.current_word_ref()?.clone();
            let ParsedWordTarget {
                name,
                name_span,
                subscript,
                boundary,
            } = self.parse_word_target(&word, SubscriptInterpretation::Contextual, true)?;
            let WordTargetBoundary::Assignment {
                append,
                value_start,
            } = boundary
            else {
                return None;
            };
            let (target, is_append, value_start) = (
                self.var_ref(name, name_span, subscript, assignment_span),
                append,
                value_start,
            );
            self.advance();
            if self.at(TokenKind::LeftParen) {
                let open_paren_span = self.current_span;
                self.advance(); // consume '('
                let (array, close_span) = self.collect_compound_array(open_paren_span, None);
                return Some((
                    Assignment {
                        target,
                        value: AssignmentValue::Compound(array),
                        append: is_append,
                        span: Self::merge_optional_span(
                            assignment_span,
                            Self::merge_optional_span(open_paren_span, close_span),
                        ),
                    },
                    false,
                ));
            }
            // Empty assignment: VAR=
            let value_span = Span::from_positions(value_start, assignment_span.end);
            return Some((
                Assignment {
                    target,
                    value: AssignmentValue::Scalar(Word::literal_with_span("", value_span)),
                    append: is_append,
                    span: assignment_span,
                },
                false,
            ));
        }

        self.current_word()
            .and_then(|word| {
                self.parse_assignment_from_word(word, None, SubscriptInterpretation::Contextual)
            })
            .or_else(|| {
                self.parse_assignment_from_text(
                    raw,
                    self.current_span,
                    None,
                    SubscriptInterpretation::Contextual,
                )
            })
            .map(|assignment| (assignment, true))
    }

    /// Parse a compound array argument in arg position (e.g. `declare -a arr=(x y z)`).
    /// Called when the current word ends with `=` and the next token is `(`.
    /// Returns the compound word if successful, or `None` if not a compound assignment.
    pub(in crate::parser) fn try_parse_compound_array_arg(
        &mut self,
        saved_w: &str,
        saved_span: Span,
    ) -> Result<Option<Word>> {
        if !self.at(TokenKind::LeftParen) {
            return Ok(None);
        }

        let open_paren_span = self.current_span;
        if let Some(closing_span) = self.scan_compound_array_close(open_paren_span) {
            let paren_text = &self.input[open_paren_span.start.offset..closing_span.end.offset];
            let mut compound = String::with_capacity(saved_w.len() + paren_text.len());
            compound.push_str(saved_w);
            compound.push_str(paren_text);
            while self.current_token.is_some()
                && self.current_span.start.offset < closing_span.end.offset
            {
                self.advance();
            }
            let span = saved_span.merge(closing_span);
            return Ok(Some(self.word_from_raw_text(&compound, span)));
        }

        self.advance(); // consume '('
        let mut compound = String::with_capacity(saved_w.len() + 32);
        compound.push_str(saved_w);
        let mut closing_span = Span::new();
        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    let elem = self.current_source_like_word_text_or_error(
                        "compound array argument element",
                    )?;
                    compound.push(' ');
                    compound.push_str(&elem);
                    self.advance();
                }
                None => break,
                _ => {
                    self.advance();
                }
            }
        }

        let span = if closing_span == Span::new() {
            saved_span
        } else {
            saved_span.merge(closing_span)
        };

        if saved_span.start.offset <= span.end.offset && span.end.offset <= self.input.len() {
            let source = &self.input[saved_span.start.offset..span.end.offset];
            return Ok(Some(self.decode_word_text(
                source,
                span,
                saved_span.start,
                true,
            )));
        }

        Ok(Some(self.decode_word_text(
            &compound,
            span,
            saved_span.start,
            false,
        )))
    }

    /// Parse a heredoc redirect (`<<` or `<<-`) and any trailing redirects on the same line.
    pub(in crate::parser) fn expect_word(&mut self) -> Result<Word> {
        match self.current_token_kind {
            Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                // Process substitution <(cmd) or >(cmd)
                let is_input = self.at(TokenKind::ProcessSubIn);
                let process_span = self.current_span;
                self.advance();

                // Walk tokens until the matching closing paren, then reparse the original
                // source slice so nested command spans remain absolute.
                let mut depth = 1;
                let close_span = loop {
                    match self.current_token_kind {
                        Some(
                            TokenKind::LeftParen
                            | TokenKind::DoubleLeftParen
                            | TokenKind::ProcessSubIn
                            | TokenKind::ProcessSubOut,
                        ) => {
                            depth += match self.current_token_kind {
                                Some(TokenKind::DoubleLeftParen) => 2,
                                _ => 1,
                            };
                            self.advance();
                        }
                        Some(TokenKind::RightParen) => {
                            depth -= 1;
                            if depth == 0 {
                                let close_span = self.current_span;
                                self.advance();
                                break close_span;
                            }
                            self.advance();
                        }
                        Some(TokenKind::DoubleRightParen) => {
                            if depth == 1 {
                                self.split_current_double_right_paren();
                                continue;
                            }

                            let (_, second_span) =
                                Self::split_double_right_paren(self.current_span);
                            match depth {
                                0 => unreachable!("process substitution depth cannot underflow"),
                                2 => {
                                    self.advance();
                                    break second_span;
                                }
                                _ => {
                                    depth -= 2;
                                    self.advance();
                                }
                            }
                        }
                        None => {
                            return Err(Error::parse(
                                "unexpected end of input in process substitution".to_string(),
                            ));
                        }
                        _ => self.advance(),
                    }
                };

                let inner_start = process_span.end;
                let body = self.nested_stmt_seq_from_current_input(inner_start, close_span.start);

                Ok(self.word_with_parts(
                    vec![WordPartNode::new(
                        WordPart::ProcessSubstitution { body, is_input },
                        process_span.merge(close_span),
                    )],
                    process_span.merge(close_span),
                ))
            }
            _ => {
                let word = self
                    .take_current_word_and_advance()
                    .ok_or_else(|| self.error("expected word"))?;
                Ok(word)
            }
        }
    }
}
