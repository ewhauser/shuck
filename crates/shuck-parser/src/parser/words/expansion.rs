use super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn nested_dollar_paren_stmt_seq_from_source_or_text(
        &mut self,
        inner_text: &str,
        approximate_dollar_start: Position,
        fallback_base: Position,
    ) -> StmtSeq {
        if let Some((body_start, body_end)) =
            self.source_dollar_paren_body_span(inner_text, approximate_dollar_start)
        {
            return self.nested_stmt_seq_from_current_input(body_start, body_end);
        }

        self.nested_stmt_seq_from_source(inner_text, fallback_base)
    }

    pub(in crate::parser) fn source_dollar_paren_body_span(
        &self,
        inner_text: &str,
        approximate_dollar_start: Position,
    ) -> Option<(Position, Position)> {
        let needle = format!("$({inner_text})");
        let search_start = floor_char_boundary(
            self.input,
            approximate_dollar_start.offset.saturating_sub(512),
        );
        let search_end = ceil_char_boundary(
            self.input,
            (approximate_dollar_start.offset + needle.len() + 4096).min(self.input.len()),
        );
        let haystack = self.input.get(search_start..search_end)?;
        let (relative_start, _) =
            haystack
                .match_indices(&needle)
                .min_by_key(|(relative_start, _)| {
                    (search_start + relative_start).abs_diff(approximate_dollar_start.offset)
                })?;
        let subst_start = search_start + relative_start;
        let body_start_offset = subst_start + "$(".len();
        let body_end_offset = subst_start + needle.len() - ")".len();

        Some((
            self.lexer.position_at_offset(body_start_offset),
            self.lexer.position_at_offset(body_end_offset),
        ))
    }

    pub(in crate::parser) fn read_array_index(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> (SourceText, Option<SourceText>) {
        let start = *cursor;
        let mut text = (!source_backed).then(String::new);
        let mut end = *cursor;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;

        while let Some(&c) = chars.peek() {
            if c == ']' && bracket_depth == 0 && brace_depth == 0 {
                end = *cursor;
                Self::next_word_char_unwrap(chars, cursor);
                break;
            }

            match c {
                '[' => bracket_depth += 1,
                ']' => bracket_depth -= 1,
                '$' => {
                    let dollar = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = text.as_mut() {
                        text.push(dollar);
                    }
                    end = *cursor;
                    if chars.peek() == Some(&'{') {
                        brace_depth += 1;
                        let brace = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(text) = text.as_mut() {
                            text.push(brace);
                        }
                        end = *cursor;
                    }
                    continue;
                }
                '{' => brace_depth += 1,
                '}' if brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }

            let ch = Self::next_word_char_unwrap(chars, cursor);
            if let Some(text) = text.as_mut() {
                text.push(ch);
            }
            end = *cursor;
        }

        let span = Span::from_positions(start, end);
        if source_backed {
            self.subscript_source_text(span.slice(self.input), span)
        } else {
            let text = text.unwrap_or_default();
            self.subscript_source_text(&text, span)
        }
    }

    pub(in crate::parser) fn read_replacement_pattern(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> SourceText {
        let start = *cursor;

        if source_backed {
            let mut end = *cursor;
            let mut has_escaped_slash = false;
            let mut nested_parameter_depth = 0usize;
            let mut escaped = false;

            while let Some(&ch) = chars.peek() {
                if !escaped && nested_parameter_depth == 0 && (ch == '/' || ch == '}') {
                    end = *cursor;
                    break;
                }

                if ch == '\x00' {
                    Self::next_word_char_unwrap(chars, cursor);
                    if let Some(&literal) = chars.peek() {
                        if literal == '/' {
                            has_escaped_slash = true;
                        }
                        Self::next_word_char_unwrap(chars, cursor);
                    }
                    end = *cursor;
                    continue;
                }

                if escaped {
                    if ch == '/' {
                        has_escaped_slash = true;
                    }
                    Self::next_word_char_unwrap(chars, cursor);
                    escaped = false;
                    end = *cursor;
                    continue;
                }

                if ch == '\\' {
                    Self::next_word_char_unwrap(chars, cursor);
                    escaped = true;
                    end = *cursor;
                    continue;
                }

                if ch == '$' {
                    let mut lookahead = chars.clone();
                    lookahead.next();
                    if lookahead.peek() == Some(&'{') {
                        Self::next_word_char_unwrap(chars, cursor);
                        Self::next_word_char_unwrap(chars, cursor);
                        nested_parameter_depth += 1;
                        end = *cursor;
                        continue;
                    }
                }

                if ch == '}' && nested_parameter_depth > 0 {
                    Self::next_word_char_unwrap(chars, cursor);
                    nested_parameter_depth -= 1;
                    end = *cursor;
                    continue;
                }

                Self::next_word_char_unwrap(chars, cursor);
                end = *cursor;
            }

            let span = Span::from_positions(start, end);
            if has_escaped_slash {
                SourceText::cooked(span, span.slice(self.input).replace("\\/", "/"))
            } else {
                SourceText::source(span)
            }
        } else {
            let mut pattern = String::new();
            let mut end = *cursor;
            let mut nested_parameter_depth = 0usize;
            let mut escaped = false;
            while let Some(&ch) = chars.peek() {
                if !escaped && nested_parameter_depth == 0 && (ch == '/' || ch == '}') {
                    end = *cursor;
                    break;
                }
                if ch == '\x00' {
                    Self::next_word_char_unwrap(chars, cursor);
                    if chars.peek().is_some() {
                        pattern.push(Self::next_word_char_unwrap(chars, cursor));
                    }
                    end = *cursor;
                    continue;
                }
                if escaped {
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if consumed == '/' {
                        pattern.push('/');
                    } else {
                        pattern.push('\\');
                        pattern.push(consumed);
                    }
                    escaped = false;
                    end = *cursor;
                    continue;
                }
                if ch == '\\' {
                    Self::next_word_char_unwrap(chars, cursor);
                    escaped = true;
                    end = *cursor;
                    continue;
                }
                if ch == '$' {
                    let mut lookahead = chars.clone();
                    lookahead.next();
                    if lookahead.peek() == Some(&'{') {
                        pattern.push(Self::next_word_char_unwrap(chars, cursor));
                        pattern.push(Self::next_word_char_unwrap(chars, cursor));
                        nested_parameter_depth += 1;
                        end = *cursor;
                        continue;
                    }
                }
                if ch == '}' && nested_parameter_depth > 0 {
                    pattern.push(Self::next_word_char_unwrap(chars, cursor));
                    nested_parameter_depth -= 1;
                    end = *cursor;
                    continue;
                }
                pattern.push(Self::next_word_char_unwrap(chars, cursor));
                end = *cursor;
            }
            if escaped {
                pattern.push('\\');
            }
            self.source_text(pattern, start, end)
        }
    }

    pub(in crate::parser) fn decode_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_with_escape_mode(s, span, base, source_backed, source_backed)
    }

    pub(in crate::parser) fn decode_word_text_with_escape_mode(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
    ) -> Word {
        let mut parts = WordPartBuffer::new();
        self.decode_word_parts_into_with_escape_mode(
            s,
            base,
            source_backed,
            preserve_escaped_expansion_literals,
            &mut parts,
        );
        self.word_with_part_buffer(parts, span)
    }

    pub(in crate::parser) fn decode_word_text_with_options(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        options: DecodeWordPartsOptions,
    ) -> Word {
        let mut parts = WordPartBuffer::new();
        self.decode_word_parts_into_with_quote_fragments(
            s,
            base,
            source_backed,
            options,
            &mut parts,
        );
        self.word_with_part_buffer(parts, span)
    }

    pub(in crate::parser) fn decode_fragment_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_fragment_word_text_with_escape_mode(s, span, base, source_backed, source_backed)
    }

    pub(in crate::parser) fn decode_fragment_word_text_with_escape_mode(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
    ) -> Word {
        let mut parts = WordPartBuffer::new();
        self.decode_word_parts_into_with_quote_fragments(
            s,
            base,
            source_backed,
            DecodeWordPartsOptions {
                preserve_quote_fragments: true,
                parse_dollar_quotes: true,
                preserve_escaped_expansion_literals,
                ..DecodeWordPartsOptions::default()
            },
            &mut parts,
        );
        self.word_with_part_buffer(parts, span)
    }

    pub(in crate::parser) fn decode_quoted_segment_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_with_options(
            s,
            span,
            base,
            source_backed,
            DecodeWordPartsOptions {
                ambient_double_quotes: true,
                // Double-quoted segment contents treat `$'...'` and `$"..."`
                // as literal text, not nested quote forms.
                parse_dollar_quotes: false,
                preserve_escaped_expansion_literals: source_backed,
                parse_process_substitutions: false,
                ..DecodeWordPartsOptions::default()
            },
        )
    }

    pub(in crate::parser) fn decode_heredoc_body_text(
        &mut self,
        s: &str,
        span: Span,
        source_backed: bool,
    ) -> HeredocBody {
        let mut parts = WordPartBuffer::new();
        self.decode_word_parts_into_with_quote_fragments(
            s,
            span.start,
            source_backed,
            DecodeWordPartsOptions {
                preserve_escaped_expansion_literals: true,
                parse_process_substitutions: false,
                ..DecodeWordPartsOptions::default()
            },
            &mut parts,
        );
        let parts = parts
            .into_iter()
            .map(|part| self.heredoc_body_part_from_word_part_node(part, source_backed))
            .collect();
        self.heredoc_body_with_parts(parts, span, HeredocBodyMode::Expanding, source_backed)
    }

    pub(in crate::parser) fn parse_word_with_context(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        let (text, source_backed) = if source_backed && !self.source_matches(span, s) {
            (span.slice(self.input), true)
        } else {
            (s, source_backed)
        };

        self.decode_word_text_preserving_quotes_if_needed(text, span, base, source_backed)
    }

    pub(in crate::parser) fn arithmetic_expansion_word_part(
        &self,
        expression: SourceText,
        syntax: ArithmeticExpansionSyntax,
    ) -> WordPart {
        WordPart::ArithmeticExpansion {
            expression_ast: self
                .parse_source_text_as_arithmetic(&expression)
                .ok()
                .map(Box::new),
            expression_word_ast: Box::new(self.parse_source_text_as_word(&expression)),
            expression,
            syntax,
        }
    }

    pub(in crate::parser) fn parameter_expansion_word_part(
        &self,
        reference: VarRef,
        operator: ParameterOp,
        operand: Option<SourceText>,
        colon_variant: bool,
    ) -> WordPart {
        let operand_word_ast = self
            .parse_optional_source_text_as_word(operand.as_ref())
            .map(Box::new);
        WordPart::ParameterExpansion {
            reference,
            operator: Box::new(operator),
            operand,
            operand_word_ast,
            colon_variant,
        }
    }

    pub(in crate::parser) fn indirect_expansion_word_part(
        &self,
        reference: VarRef,
        operator: Option<ParameterOp>,
        operand: Option<SourceText>,
        colon_variant: bool,
    ) -> WordPart {
        let operand_word_ast = self
            .parse_optional_source_text_as_word(operand.as_ref())
            .map(Box::new);
        WordPart::IndirectExpansion {
            reference,
            operator: operator.map(Box::new),
            operand,
            operand_word_ast,
            colon_variant,
        }
    }

    pub(in crate::parser) fn substring_word_part(
        &self,
        reference: VarRef,
        offset: SourceText,
        length: Option<SourceText>,
    ) -> WordPart {
        let offset_ast = self
            .maybe_parse_source_text_as_arithmetic(&offset)
            .map(Box::new);
        let offset_word_ast = Box::new(self.parse_source_text_as_word(&offset));
        let length_ast = length
            .as_ref()
            .and_then(|length| self.maybe_parse_source_text_as_arithmetic(length))
            .map(Box::new);
        let length_word_ast = self
            .parse_optional_source_text_as_word(length.as_ref())
            .map(Box::new);
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
        }
    }

    pub(in crate::parser) fn parse_parameter_tail_without_subscript(
        &mut self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
        part_start: Position,
        brace_body_start: Position,
        var_name: &str,
    ) -> WordPart {
        if let Some(c) = chars.peek().copied() {
            match c {
                ':' => {
                    if self.zsh_parameter_suffix_looks_like_modifier(chars) {
                        let tail = self.read_brace_operand(chars, cursor, source_backed);
                        let raw_body = self.prefixed_parameter_raw_body(
                            var_name,
                            brace_body_start,
                            tail,
                            source_backed,
                        );
                        return self.zsh_parameter_word_part(raw_body, part_start, *cursor);
                    }

                    Self::next_word_char_unwrap(chars, cursor);
                    match chars.peek() {
                        Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?') => {
                            let op_char = Self::next_word_char_unwrap(chars, cursor);
                            let operand = self.read_brace_operand(chars, cursor, source_backed);
                            let operator = match op_char {
                                '-' => ParameterOp::UseDefault,
                                '=' => ParameterOp::AssignDefault,
                                '+' => ParameterOp::UseReplacement,
                                '?' => ParameterOp::Error,
                                _ => unreachable!(),
                            };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                                operator,
                                Some(operand),
                                true,
                            )
                        }
                        _ => {
                            let (offset, length) =
                                self.read_parameter_slice_parts(chars, cursor, source_backed);
                            Self::consume_word_char_if(chars, cursor, '}');
                            self.substring_word_part(
                                self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                                offset,
                                length,
                            )
                        }
                    }
                }
                '-' | '=' | '+' | '?' => {
                    let op_char = Self::next_word_char_unwrap(chars, cursor);
                    let operand = self.read_brace_operand(chars, cursor, source_backed);
                    let operator = match op_char {
                        '-' => ParameterOp::UseDefault,
                        '=' => ParameterOp::AssignDefault,
                        '+' => ParameterOp::UseReplacement,
                        '?' => ParameterOp::Error,
                        _ => unreachable!(),
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        Some(operand),
                        false,
                    )
                }
                '#' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    let longest = Self::consume_word_char_if(chars, cursor, '#');
                    let operand_text = self.read_brace_operand(chars, cursor, source_backed);
                    let pattern = self.pattern_from_source_text(&operand_text);
                    let operator = if longest {
                        ParameterOp::RemovePrefixLong { pattern }
                    } else {
                        ParameterOp::RemovePrefixShort { pattern }
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        None,
                        false,
                    )
                }
                '%' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    let longest = Self::consume_word_char_if(chars, cursor, '%');
                    let operand_text = self.read_brace_operand(chars, cursor, source_backed);
                    let pattern = self.pattern_from_source_text(&operand_text);
                    let operator = if longest {
                        ParameterOp::RemoveSuffixLong { pattern }
                    } else {
                        ParameterOp::RemoveSuffixShort { pattern }
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        None,
                        false,
                    )
                }
                '/' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    let replace_all = Self::consume_word_char_if(chars, cursor, '/');
                    let pattern_text = self.read_replacement_pattern(chars, cursor, source_backed);
                    let pattern = self.pattern_from_source_text(&pattern_text);
                    let (replacement, consumed_closing_brace) =
                        if Self::consume_word_char_if(chars, cursor, '/') {
                            let replacement = self.read_brace_operand(chars, cursor, source_backed);
                            (
                                replacement,
                                cursor.offset > 0
                                    && self.input_prefix_ends_with(cursor.offset, '}'),
                            )
                        } else {
                            (self.empty_source_text(*cursor), false)
                        };
                    if !consumed_closing_brace {
                        Self::consume_word_char_if(chars, cursor, '}');
                    }
                    if !self.input_span_ends_with(part_start, *cursor, '}')
                        && self.input_suffix_starts_with(cursor.offset, '}')
                    {
                        cursor.advance('}');
                    }
                    let operator = if replace_all {
                        ParameterOp::ReplaceAll {
                            pattern,
                            replacement_word_ast: Box::new(
                                self.parse_source_text_as_word(&replacement),
                            ),
                            replacement,
                        }
                    } else {
                        ParameterOp::ReplaceFirst {
                            pattern,
                            replacement_word_ast: Box::new(
                                self.parse_source_text_as_word(&replacement),
                            ),
                            replacement,
                        }
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        None,
                        false,
                    )
                }
                '^' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    let operator = if Self::consume_word_char_if(chars, cursor, '^') {
                        ParameterOp::UpperAll
                    } else {
                        ParameterOp::UpperFirst
                    };
                    let operand = if Self::consume_word_char_if(chars, cursor, '}') {
                        None
                    } else {
                        Some(self.read_brace_operand(chars, cursor, source_backed))
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        operand,
                        false,
                    )
                }
                ',' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    let operator = if Self::consume_word_char_if(chars, cursor, ',') {
                        ParameterOp::LowerAll
                    } else {
                        ParameterOp::LowerFirst
                    };
                    let operand = if Self::consume_word_char_if(chars, cursor, '}') {
                        None
                    } else {
                        Some(self.read_brace_operand(chars, cursor, source_backed))
                    };
                    self.parameter_expansion_word_part(
                        self.parameter_var_ref(part_start, "${", var_name, None, *cursor),
                        operator,
                        operand,
                        false,
                    )
                }
                '@' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    if chars.peek().is_some() {
                        let operator = Self::next_word_char_unwrap(chars, cursor);
                        Self::consume_word_char_if(chars, cursor, '}');
                        WordPart::Transformation {
                            reference: self
                                .parameter_var_ref(part_start, "${", var_name, None, *cursor),
                            operator,
                        }
                    } else {
                        Self::consume_word_char_if(chars, cursor, '}');
                        WordPart::Variable(var_name.into())
                    }
                }
                '}' => {
                    Self::next_word_char_unwrap(chars, cursor);
                    WordPart::Variable(var_name.into())
                }
                _ => {
                    while let Some(&next) = chars.peek() {
                        let consumed = Self::next_word_char_unwrap(chars, cursor);
                        if next == '}' || consumed == '}' {
                            break;
                        }
                    }
                    WordPart::Variable(var_name.into())
                }
            }
        } else {
            WordPart::Variable(var_name.into())
        }
    }

    pub(in crate::parser) fn array_slice_word_part(
        &self,
        reference: VarRef,
        offset: SourceText,
        length: Option<SourceText>,
    ) -> WordPart {
        let offset_ast = self
            .maybe_parse_source_text_as_arithmetic(&offset)
            .map(Box::new);
        let offset_word_ast = Box::new(self.parse_source_text_as_word(&offset));
        let length_ast = length
            .as_ref()
            .and_then(|length| self.maybe_parse_source_text_as_arithmetic(length))
            .map(Box::new);
        let length_word_ast = self
            .parse_optional_source_text_as_word(length.as_ref())
            .map(Box::new);
        WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
        }
    }

    pub(in crate::parser) fn read_parameter_slice_parts(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> (SourceText, Option<SourceText>) {
        let start = *cursor;
        let mut offset_end = None;
        let mut length_start = None;
        let mut parameter_brace_depth = 0usize;
        let mut literal_brace_depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_parameter_depth = 0usize;
        let mut escaped = false;
        let mut offset_text = (!source_backed).then(String::new);
        let mut length_text = (!source_backed).then(String::new);

        while let Some(&ch) = chars.peek() {
            if escaped {
                let consumed = Self::next_word_char_unwrap(chars, cursor);
                if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                    text.push(consumed);
                }
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => {
                    escaped = true;
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    double_quote_parameter_depth =
                        if in_double { parameter_brace_depth } else { 0 };
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                }
                '$' if !in_single => {
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                    if chars.peek() == Some(&'{') {
                        parameter_brace_depth += 1;
                        let brace = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                            text.push(brace);
                        }
                    }
                }
                '{' if !in_single && !in_double => {
                    literal_brace_depth += 1;
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                }
                ':' if !in_single
                    && !in_double
                    && parameter_brace_depth == 0
                    && literal_brace_depth == 0
                    && length_start.is_none() =>
                {
                    offset_end = Some(*cursor);
                    Self::next_word_char_unwrap(chars, cursor);
                    length_start = Some(*cursor);
                }
                '}' if !in_single
                    && (!in_double || parameter_brace_depth > double_quote_parameter_depth) =>
                {
                    if parameter_brace_depth > 0 {
                        parameter_brace_depth -= 1;
                        let consumed = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                            text.push(consumed);
                        }
                    } else if literal_brace_depth > 0 {
                        literal_brace_depth -= 1;
                        let consumed = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                            text.push(consumed);
                        }
                    } else {
                        break;
                    }
                }
                _ => {
                    let consumed = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = length_text.as_mut().or(offset_text.as_mut()) {
                        text.push(consumed);
                    }
                }
            }
        }

        if source_backed {
            match (offset_end, length_start) {
                (Some(offset_end), Some(length_start)) => (
                    SourceText::source(Span::from_positions(start, offset_end)),
                    Some(SourceText::source(Span::from_positions(
                        length_start,
                        *cursor,
                    ))),
                ),
                _ => (
                    SourceText::source(Span::from_positions(start, *cursor)),
                    None,
                ),
            }
        } else {
            match (offset_end, length_start) {
                (Some(offset_end), Some(length_start)) => (
                    self.source_text(offset_text.unwrap_or_default(), start, offset_end),
                    Some(self.source_text(length_text.unwrap_or_default(), length_start, *cursor)),
                ),
                _ => (
                    self.source_text(offset_text.unwrap_or_default(), start, *cursor),
                    None,
                ),
            }
        }
    }

    /// Read operand for brace expansion (everything until closing brace)
    pub(in crate::parser) fn read_brace_operand(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> SourceText {
        let start = *cursor;
        let mut depth = 1;
        let mut literal_brace_depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;
        let mut escaped = false;
        let use_source = source_backed && self.brace_operand_starts_at_source(chars, *cursor);
        let mut operand = (!use_source).then(String::new);

        while let Some(&c) = chars.peek() {
            if escaped {
                let ch = Self::next_word_char_unwrap(chars, cursor);
                if let Some(operand) = operand.as_mut() {
                    operand.push(ch);
                }
                escaped = false;
                continue;
            }

            if c == '\x00' {
                if operand.is_none() {
                    operand = Some(
                        Span::from_positions(start, *cursor)
                            .slice(self.input)
                            .into(),
                    );
                }
                Self::next_word_char_unwrap(chars, cursor);
                if chars.peek().is_some() {
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                continue;
            }

            match c {
                '\\' if !in_single => {
                    escaped = true;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    double_quote_depth = if in_double { depth } else { 0 };
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '$' if !in_single => {
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                    if chars.peek() == Some(&'{') {
                        depth += 1;
                        let brace = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(operand) = operand.as_mut() {
                            operand.push(brace);
                        }
                    }
                }
                '{' if !in_single && !in_double => {
                    literal_brace_depth += 1;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '}' if !in_single && (!in_double || depth > double_quote_depth) => {
                    if depth == 1 && literal_brace_depth > 0 {
                        let mut remaining = chars.clone();
                        remaining.next();
                        if Self::brace_operand_has_later_top_level_closer(remaining, depth) {
                            literal_brace_depth -= 1;
                            let ch = Self::next_word_char_unwrap(chars, cursor);
                            if let Some(operand) = operand.as_mut() {
                                operand.push(ch);
                            }
                            continue;
                        }
                    }

                    if depth == 1 {
                        let end = *cursor;
                        Self::next_word_char_unwrap(chars, cursor);
                        return if let Some(operand) = operand {
                            self.source_text(operand, start, end)
                        } else {
                            SourceText::source(Span::from_positions(start, end))
                        };
                    }
                    depth -= 1;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                _ => {
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
            }
        }
        if let Some(operand) = operand {
            self.source_text(operand, start, *cursor)
        } else {
            SourceText::source(Span::from_positions(start, *cursor))
        }
    }

    pub(in crate::parser) fn brace_operand_starts_at_source(
        &self,
        chars: &std::iter::Peekable<std::str::Chars<'_>>,
        cursor: Position,
    ) -> bool {
        let mut probe = chars.clone();
        let Some(first) = probe.next() else {
            return true;
        };
        let Some(source_suffix) = self.input.get(cursor.offset..) else {
            return false;
        };
        source_suffix.starts_with(first)
    }

    pub(in crate::parser) fn brace_operand_has_later_top_level_closer(
        mut chars: std::iter::Peekable<std::str::Chars<'_>>,
        target_depth: usize,
    ) -> bool {
        let mut depth = target_depth;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if ch == '\x00' {
                chars.next();
                continue;
            }

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => {
                    in_double = !in_double;
                    double_quote_depth = if in_double { depth } else { 0 };
                }
                '$' if !in_single && chars.peek() == Some(&'{') => {
                    chars.next();
                    depth += 1;
                }
                '}' if !in_single && (!in_double || depth > double_quote_depth) => {
                    if depth == target_depth {
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        false
    }
}
