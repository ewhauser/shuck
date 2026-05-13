use super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn decode_word_parts_into_with_escape_mode(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
        parts: &mut WordPartBuffer,
    ) {
        self.decode_word_parts_into_with_quote_fragments(
            s,
            base,
            source_backed,
            DecodeWordPartsOptions {
                parse_dollar_quotes: true,
                preserve_escaped_expansion_literals,
                ..DecodeWordPartsOptions::default()
            },
            parts,
        );
    }

    pub(in crate::parser) fn decode_word_parts_into_with_quote_fragments(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        options: DecodeWordPartsOptions,
        parts: &mut WordPartBuffer,
    ) {
        if source_backed
            && !s.is_empty()
            && let Some(end) = try_pure_literal_end_position(s.as_bytes(), base, options)
        {
            Self::push_word_part(parts, WordPart::Literal(LiteralText::source()), base, end);
            return;
        }

        let mut chars = s.chars().peekable();
        let mut current = String::new();
        let mut current_start = base;
        let mut cursor = base;

        while chars.peek().is_some() {
            let part_start = cursor;
            let ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);

            if ch == '\x00' {
                if current.is_empty() {
                    current_start = part_start;
                }
                if let Some(literal_ch) = Self::next_word_char(&mut chars, &mut cursor) {
                    current.push(literal_ch);
                }
                continue;
            }

            if options.preserve_quote_fragments
                && ch == '\\'
                && matches!(chars.peek().copied(), Some('\'' | '"'))
            {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push(ch);
                current.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                continue;
            }

            if options.preserve_escaped_expansion_literals
                && ch == '\\'
                && matches!(chars.peek().copied(), Some('$' | '`' | '\\'))
            {
                if current.is_empty() {
                    current_start = part_start;
                }
                let literal_ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                current.push(literal_ch);
                continue;
            }

            if options.preserve_quote_fragments && ch == '\'' {
                self.flush_literal_part(
                    parts,
                    &mut current,
                    current_start,
                    part_start,
                    source_backed,
                );

                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut closed = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if c == '\'' {
                        closed = true;
                        break;
                    }
                    if let Some(content) = content.as_mut() {
                        content.push(c);
                    }
                    content_end = cursor;
                }

                if !closed {
                    if current.is_empty() {
                        current_start = part_start;
                    }
                    let fragment = if source_backed {
                        Span::from_positions(part_start, cursor)
                            .slice(self.input)
                            .to_string()
                    } else {
                        let mut fragment = String::from("'");
                        fragment.push_str(content.as_deref().unwrap_or_default());
                        fragment
                    };
                    current.push_str(&fragment);
                    continue;
                }

                Self::push_word_part(
                    parts,
                    WordPart::SingleQuoted {
                        value: if source_backed {
                            SourceText::source(Span::from_positions(content_start, content_end))
                        } else {
                            self.source_text(
                                content.unwrap_or_default(),
                                content_start,
                                content_end,
                            )
                        },
                        dollar: false,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if options.preserve_quote_fragments && ch == '"' {
                self.flush_literal_part(
                    parts,
                    &mut current,
                    current_start,
                    part_start,
                    source_backed,
                );

                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut escaped = false;
                let mut command_substitution_depth = 0usize;
                let mut closed = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if escaped {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        escaped = false;
                        continue;
                    }

                    if c == '$' && source_backed {
                        let relative_offset = cursor.offset.saturating_sub(base.offset);
                        let remaining = &s[relative_offset..];

                        if chars.peek() == Some(&'(')
                            && !remaining['('.len_utf8()..].starts_with('(')
                            && let Some(consumed) = lexer::scan_command_substitution_body_len(
                                &remaining['('.len_utf8()..],
                            )
                        {
                            if let Some(content) = content.as_mut() {
                                content.push('$');
                                content.push_str(&remaining[..'('.len_utf8() + consumed]);
                            }
                            for _ in remaining[..'('.len_utf8() + consumed].chars() {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            }
                            content_end = cursor;
                            continue;
                        }

                        if chars.peek() == Some(&'{')
                            && let Some(consumed) = Self::scan_array_parameter_expansion_len(
                                &remaining['{'.len_utf8()..],
                            )
                        {
                            if let Some(content) = content.as_mut() {
                                content.push('$');
                                content.push_str(&remaining[..'{'.len_utf8() + consumed]);
                            }
                            for _ in remaining[..'{'.len_utf8() + consumed].chars() {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            }
                            content_end = cursor;
                            continue;
                        }
                    }

                    if c == '`' {
                        if let Some(content) = content.as_mut() {
                            content.push('`');
                        }
                        while let Some(nested) = Self::next_word_char(&mut chars, &mut cursor) {
                            if let Some(content) = content.as_mut() {
                                content.push(nested);
                            }
                            content_end = cursor;
                            if nested == '\\' {
                                if let Some(escaped) = Self::next_word_char(&mut chars, &mut cursor)
                                {
                                    if let Some(content) = content.as_mut() {
                                        content.push(escaped);
                                    }
                                    content_end = cursor;
                                }
                                continue;
                            }
                            if nested == '`' {
                                break;
                            }
                        }
                        continue;
                    }

                    if c == '\\' {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        escaped = true;
                        continue;
                    }

                    if c == '$' && chars.peek() == Some(&'(') {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        let open = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                        if let Some(content) = content.as_mut() {
                            content.push(open);
                        }
                        content_end = cursor;
                        command_substitution_depth += 1;
                        continue;
                    }

                    if c == ')' && command_substitution_depth > 0 {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        command_substitution_depth -= 1;
                        continue;
                    }

                    if c == '"' && command_substitution_depth == 0 {
                        closed = true;
                        break;
                    }

                    if let Some(content) = content.as_mut() {
                        content.push(c);
                    }
                    content_end = cursor;
                }

                if !closed {
                    if current.is_empty() {
                        current_start = part_start;
                    }
                    let fragment = if source_backed {
                        Span::from_positions(part_start, cursor)
                            .slice(self.input)
                            .to_string()
                    } else {
                        let mut fragment = String::from("\"");
                        fragment.push_str(content.as_deref().unwrap_or_default());
                        fragment
                    };
                    current.push_str(&fragment);
                    continue;
                }

                let inner_span = Span::from_positions(content_start, content_end);
                let inner = if source_backed {
                    self.decode_word_text_with_options(
                        inner_span.slice(self.input),
                        inner_span,
                        content_start,
                        true,
                        DecodeWordPartsOptions {
                            ambient_double_quotes: true,
                            // `$'...'` and `$"..."` are literal inside ordinary
                            // double quotes, so nested decoding must not
                            // reactivate dollar-quote parsing here.
                            parse_dollar_quotes: false,
                            preserve_escaped_expansion_literals: source_backed,
                            parse_process_substitutions: false,
                            ..DecodeWordPartsOptions::default()
                        },
                    )
                } else {
                    let content = content.unwrap_or_default();
                    self.decode_word_text_with_options(
                        &content,
                        inner_span,
                        content_start,
                        false,
                        DecodeWordPartsOptions {
                            ambient_double_quotes: true,
                            // `$'...'` and `$"..."` are literal inside ordinary
                            // double quotes, so nested decoding must not
                            // reactivate dollar-quote parsing here.
                            parse_dollar_quotes: false,
                            parse_process_substitutions: false,
                            ..DecodeWordPartsOptions::default()
                        },
                    )
                };

                Self::push_word_part(
                    parts,
                    WordPart::DoubleQuoted {
                        parts: inner.parts,
                        dollar: false,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if ch == '`' {
                self.flush_literal_part(
                    parts,
                    &mut current,
                    current_start,
                    part_start,
                    source_backed,
                );

                let inner_start = cursor;
                let body = if source_backed {
                    let mut inner_end = inner_start;
                    let mut escaped = false;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if escaped {
                            escaped = false;
                            inner_end = cursor;
                            continue;
                        }

                        match c {
                            '\\' => {
                                escaped = true;
                                inner_end = cursor;
                            }
                            '`' => break,
                            _ => inner_end = cursor,
                        }
                    }
                    self.nested_stmt_seq_from_current_input(inner_start, inner_end)
                } else {
                    let mut cmd_str = String::new();
                    let mut escaped = false;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if escaped {
                            escaped = false;
                            cmd_str.push(c);
                            continue;
                        }

                        match c {
                            '\\' => {
                                escaped = true;
                                cmd_str.push(c);
                            }
                            '`' => break,
                            _ => cmd_str.push(c),
                        }
                    }
                    self.nested_stmt_seq_from_source(&cmd_str, inner_start)
                };

                Self::push_word_part(
                    parts,
                    WordPart::CommandSubstitution {
                        body,
                        syntax: CommandSubstitutionSyntax::Backtick,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if options.parse_process_substitutions
                && matches!(ch, '<' | '>')
                && chars.peek() == Some(&'(')
            {
                self.flush_literal_part(
                    parts,
                    &mut current,
                    current_start,
                    part_start,
                    source_backed,
                );

                let is_input = ch == '<';
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let inner_start = cursor;
                let body = if source_backed {
                    let remaining_word_text = chars.clone().collect::<String>();
                    let consumed = lexer::scan_command_substitution_body_len(&remaining_word_text);
                    let inner_end = if let Some(consumed) = consumed {
                        let consumed_text = &remaining_word_text[..consumed];
                        for _ in consumed_text.chars() {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                        }
                        let inner_text = consumed_text.strip_suffix(')').unwrap_or_default();
                        inner_start.advanced_by(inner_text)
                    } else {
                        let mut depth = 1;
                        let mut inner_end = inner_start;
                        while chars.peek().is_some() {
                            let c = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match c {
                                '(' => {
                                    depth += 1;
                                    inner_end = cursor;
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                    inner_end = cursor;
                                }
                                _ => inner_end = cursor,
                            }
                        }
                        inner_end
                    };
                    self.nested_stmt_seq_from_current_input(inner_start, inner_end)
                } else {
                    let mut cmd_str = String::new();
                    let mut depth = 1;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if c == '(' {
                            depth += 1;
                            cmd_str.push(c);
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            cmd_str.push(c);
                        } else {
                            cmd_str.push(c);
                        }
                    }
                    self.nested_stmt_seq_from_source(&cmd_str, inner_start)
                };

                Self::push_word_part(
                    parts,
                    WordPart::ProcessSubstitution { body, is_input },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if ch != '$' {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push(ch);
                continue;
            }

            self.flush_literal_part(
                parts,
                &mut current,
                current_start,
                part_start,
                source_backed,
            );

            if options.parse_dollar_quotes && chars.peek() == Some(&'\'') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let mut ansi = String::new();
                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if c == '\'' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(esc) = Self::next_word_char(&mut chars, &mut cursor) {
                            match esc {
                                'n' => ansi.push('\n'),
                                't' => ansi.push('\t'),
                                'r' => ansi.push('\r'),
                                'a' => ansi.push('\x07'),
                                'b' => ansi.push('\x08'),
                                'e' | 'E' => ansi.push('\x1B'),
                                '\\' => ansi.push('\\'),
                                '\'' => ansi.push('\''),
                                _ => {
                                    ansi.push('\\');
                                    ansi.push(esc);
                                }
                            }
                        }
                    } else {
                        ansi.push(c);
                    }
                }
                Self::push_word_part(
                    parts,
                    WordPart::SingleQuoted {
                        value: self.source_text(ansi, part_start, cursor),
                        dollar: true,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if options.parse_dollar_quotes && chars.peek() == Some(&'"') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut escaped = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if escaped {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        escaped = false;
                        continue;
                    }

                    match c {
                        '\\' => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                            escaped = true;
                        }
                        '"' => break,
                        _ => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                        }
                    }
                }

                let inner_span = Span::from_positions(content_start, content_end);
                let inner = if source_backed {
                    self.decode_word_text_with_options(
                        inner_span.slice(self.input),
                        inner_span,
                        content_start,
                        true,
                        DecodeWordPartsOptions {
                            ambient_double_quotes: true,
                            // Localized `$"..."` content uses double-quote
                            // semantics, so nested `$'...'` and `$"..."` stay
                            // literal here as well.
                            parse_dollar_quotes: false,
                            parse_process_substitutions: false,
                            ..DecodeWordPartsOptions::default()
                        },
                    )
                } else {
                    let content = content.unwrap_or_default();
                    self.decode_word_text_with_options(
                        &content,
                        inner_span,
                        content_start,
                        false,
                        DecodeWordPartsOptions {
                            ambient_double_quotes: true,
                            // Localized `$"..."` content uses double-quote
                            // semantics, so nested `$'...'` and `$"..."` stay
                            // literal here as well.
                            parse_dollar_quotes: false,
                            parse_process_substitutions: false,
                            ..DecodeWordPartsOptions::default()
                        },
                    )
                };

                Self::push_word_part(
                    parts,
                    WordPart::DoubleQuoted {
                        parts: inner.parts,
                        dollar: true,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'(') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                if chars.peek() == Some(&'(') {
                    Self::next_word_char_unwrap(&mut chars, &mut cursor);
                    let expr_start = cursor;
                    let expression = if source_backed {
                        let mut depth = 2;
                        let mut expr_end = expr_start;
                        while chars.peek().is_some() {
                            let c = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match c {
                                '(' => {
                                    depth += 1;
                                    expr_end = cursor;
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 1 {
                                        continue;
                                    }
                                    if depth == 0 {
                                        break;
                                    }
                                    expr_end = cursor;
                                }
                                _ => expr_end = cursor,
                            }
                        }
                        SourceText::source(Span::from_positions(expr_start, expr_end))
                    } else {
                        let mut expr = String::new();
                        let mut depth = 2;
                        while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                            if c == '(' {
                                depth += 1;
                                expr.push(c);
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                expr.push(c);
                            } else {
                                expr.push(c);
                            }
                        }
                        if expr.ends_with(')') {
                            expr.pop();
                        }
                        let expr_end = expr_start.advanced_by(&expr);
                        self.source_text(expr, expr_start, expr_end)
                    };
                    Self::push_word_part(
                        parts,
                        self.arithmetic_expansion_word_part(
                            expression,
                            ArithmeticExpansionSyntax::DollarParenParen,
                        ),
                        part_start,
                        cursor,
                    );
                } else {
                    let inner_start = cursor;
                    let prefix = Span::from_positions(current_start, part_start).slice(self.input);
                    let nested_source_base = if source_backed
                        && (source_prefix_ends_inside_double_quotes(prefix)
                            || (!options.preserve_quote_fragments
                                && source_prefix_has_same_line_escaped_double_quote_fragment(
                                    prefix,
                                    options.ambient_double_quotes,
                                ))) {
                        inner_start.advanced_by("(")
                    } else {
                        inner_start
                    };
                    let body = if source_backed {
                        let remaining_word_text = chars.clone().collect::<String>();
                        if let Some(consumed) =
                            lexer::scan_command_substitution_body_len(&remaining_word_text)
                        {
                            let consumed_text = &remaining_word_text[..consumed];
                            for _ in consumed_text.chars() {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            }
                            let inner_text = consumed_text.strip_suffix(')').unwrap_or_default();
                            self.nested_dollar_paren_stmt_seq_from_source_or_text(
                                inner_text,
                                part_start,
                                nested_source_base,
                            )
                        } else {
                            let mut cmd_str = String::new();
                            let mut depth = 1;
                            while chars.peek().is_some() {
                                let c = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                match c {
                                    '(' => {
                                        depth += 1;
                                        cmd_str.push(c);
                                    }
                                    ')' => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                        cmd_str.push(c);
                                    }
                                    _ => cmd_str.push(c),
                                }
                            }
                            self.nested_dollar_paren_stmt_seq_from_source_or_text(
                                &cmd_str,
                                part_start,
                                nested_source_base,
                            )
                        }
                    } else {
                        let mut cmd_str = String::new();
                        let mut depth = 1;
                        while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                            if c == '(' {
                                depth += 1;
                                cmd_str.push(c);
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                cmd_str.push(c);
                            } else {
                                cmd_str.push(c);
                            }
                        }
                        self.nested_stmt_seq_from_source(&cmd_str, inner_start)
                    };
                    Self::push_word_part(
                        parts,
                        WordPart::CommandSubstitution {
                            body,
                            syntax: CommandSubstitutionSyntax::DollarParen,
                        },
                        part_start,
                        cursor,
                    );
                }
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'[') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let expr_start = cursor;
                let expression = if source_backed {
                    let mut bracket_depth = 1_i32;
                    let mut expr_end = expr_start;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        match c {
                            '[' => {
                                bracket_depth += 1;
                                expr_end = cursor;
                            }
                            ']' => {
                                bracket_depth -= 1;
                                if bracket_depth == 0 {
                                    break;
                                }
                                expr_end = cursor;
                            }
                            _ => expr_end = cursor,
                        }
                    }
                    SourceText::source(Span::from_positions(expr_start, expr_end))
                } else {
                    let mut expr = String::new();
                    let mut bracket_depth = 1_i32;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        match c {
                            '[' => {
                                bracket_depth += 1;
                                expr.push(c);
                            }
                            ']' => {
                                bracket_depth -= 1;
                                if bracket_depth == 0 {
                                    break;
                                }
                                expr.push(c);
                            }
                            _ => expr.push(c),
                        }
                    }
                    let expr_end = expr_start.advanced_by(&expr);
                    self.source_text(expr, expr_start, expr_end)
                };
                Self::push_word_part(
                    parts,
                    self.arithmetic_expansion_word_part(
                        expression,
                        ArithmeticExpansionSyntax::LegacyBracket,
                    ),
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'{') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let brace_body_start = cursor;

                if matches!(
                    chars.peek(),
                    Some(&'(') | Some(&':') | Some(&'=') | Some(&'^') | Some(&'~') | Some(&'.')
                ) {
                    let raw_body = self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                    let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                    Self::push_word_part(parts, parameter, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                if self.zsh_parameter_requires_fallback(&mut chars) {
                    let raw_body = self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                    let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                    Self::push_word_part(parts, parameter, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '#') {
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::Variable("#".into()),
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }

                    let parses_as_special_parameter_length =
                        chars.peek().copied().is_some_and(|ch| {
                            matches!(ch, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                                || ch.is_ascii_digit()
                        }) && {
                            let mut lookahead = chars.clone();
                            lookahead.next();
                            matches!(lookahead.next(), Some('}'))
                        };

                    if !parses_as_special_parameter_length
                        && matches!(
                            chars.peek().copied(),
                            Some(':' | '-' | '=' | '+' | '?' | '#' | '%' | '/' | '^' | ',')
                        )
                    {
                        let raw_part = self.parse_parameter_tail_without_subscript(
                            &mut chars,
                            &mut cursor,
                            source_backed,
                            part_start,
                            brace_body_start,
                            "#",
                        );
                        let part = self.parameter_word_part_from_legacy(
                            raw_part,
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }

                    if self.zsh_parameter_requires_fallback(&mut chars) {
                        let tail = self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                        let raw_body = self.prefixed_parameter_raw_body(
                            "#",
                            brace_body_start,
                            tail,
                            source_backed,
                        );
                        let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                        Self::push_word_part(parts, parameter, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }

                    if self.zsh_length_parameter_requires_fallback(
                        &mut chars,
                        &cursor,
                        source_backed,
                    ) {
                        let tail = self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                        let raw_body = self.prefixed_parameter_raw_body(
                            "#",
                            brace_body_start,
                            tail,
                            source_backed,
                        );
                        let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                        Self::push_word_part(parts, parameter, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }

                    let var_name =
                        Self::read_word_while(&mut chars, &mut cursor, |c| c != '}' && c != '[');
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let (index, raw_index) =
                            self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let subscript = self.subscript_from_source_text(
                            index,
                            raw_index,
                            SubscriptInterpretation::Contextual,
                        );
                        let reference = self.parameter_var_ref(
                            part_start,
                            "${#",
                            &var_name,
                            Some(subscript),
                            cursor,
                        );
                        let part = if reference
                            .subscript
                            .as_deref()
                            .and_then(Subscript::selector)
                            .is_some()
                        {
                            WordPart::ArrayLength(reference)
                        } else {
                            WordPart::Length(reference)
                        };
                        let part = self.parameter_word_part_from_legacy(
                            part,
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else {
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::Length(
                                self.parameter_var_ref(part_start, "${#", &var_name, None, cursor),
                            ),
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    }
                    current_start = cursor;
                    continue;
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '!') {
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::Variable("!".into()),
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }

                    let mut var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                        !matches!(
                            c,
                            '}' | '['
                                | '*'
                                | '@'
                                | ':'
                                | '-'
                                | '='
                                | '+'
                                | '?'
                                | '#'
                                | '%'
                                | '/'
                                | '^'
                                | ','
                        )
                    });

                    if var_name.is_empty()
                        && matches!(
                            chars.peek().copied(),
                            Some('?' | '#' | '@' | '*' | '!' | '$' | '-')
                        )
                    {
                        var_name.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                    }

                    let subscript = if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let (index, raw_index) =
                            self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Some(self.subscript_from_source_text(
                            index,
                            raw_index,
                            SubscriptInterpretation::Contextual,
                        ))
                    } else {
                        None
                    };

                    if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        let reference =
                            self.parameter_var_ref(part_start, "${!", &var_name, subscript, cursor);
                        let part = self.parameter_word_part_from_legacy(
                            if reference
                                .subscript
                                .as_deref()
                                .and_then(Subscript::selector)
                                .is_some()
                            {
                                WordPart::ArrayIndices(reference)
                            } else {
                                self.indirect_expansion_word_part(reference, None, None, false)
                            },
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, ':') {
                        let operator = match chars.peek().copied() {
                            Some('-') => Some(ParameterOp::UseDefault),
                            Some('=') => Some(ParameterOp::AssignDefault),
                            Some('+') => Some(ParameterOp::UseReplacement),
                            Some('?') => Some(ParameterOp::Error),
                            _ => None,
                        };
                        if let Some(operator) = operator {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let part = self.parameter_word_part_from_legacy(
                                self.indirect_expansion_word_part(
                                    self.parameter_var_ref(
                                        part_start, "${!", &var_name, subscript, cursor,
                                    ),
                                    Some(operator),
                                    Some(operand),
                                    true,
                                ),
                                part_start,
                                cursor,
                                source_backed,
                            );
                            Self::push_word_part(parts, part, part_start, cursor);
                        } else {
                            let mut suffix = String::new();
                            while let Some(&c) = chars.peek() {
                                if c == '}' {
                                    Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                    break;
                                }
                                suffix.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                            }
                            Self::push_word_part(
                                parts,
                                WordPart::Variable(
                                    format!(
                                        "!{}{}{}",
                                        var_name,
                                        subscript
                                            .as_ref()
                                            .map(|subscript| {
                                                format!("[{}]", subscript.syntax_text(self.input))
                                            })
                                            .unwrap_or_default(),
                                        suffix
                                    )
                                    .into(),
                                ),
                                part_start,
                                cursor,
                            );
                        }
                    } else if matches!(
                        chars.peek(),
                        Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?')
                    ) {
                        let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                        let operand =
                            self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                        let operator = match op_char {
                            '-' => ParameterOp::UseDefault,
                            '=' => ParameterOp::AssignDefault,
                            '+' => ParameterOp::UseReplacement,
                            '?' => ParameterOp::Error,
                            _ => unreachable!(),
                        };
                        let part = self.parameter_word_part_from_legacy(
                            self.indirect_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start, "${!", &var_name, subscript, cursor,
                                ),
                                Some(operator),
                                Some(operand),
                                false,
                            ),
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else if matches!(chars.peek(), Some(&'#') | Some(&'%') | Some(&'/')) {
                        let reference =
                            self.parameter_var_ref(part_start, "${!", &var_name, subscript, cursor);
                        let part = match Self::next_word_char_unwrap(&mut chars, &mut cursor) {
                            '#' => {
                                let longest =
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '#');
                                let operand_text =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                                let pattern = self.pattern_from_source_text(&operand_text);
                                let operator = if longest {
                                    ParameterOp::RemovePrefixLong { pattern }
                                } else {
                                    ParameterOp::RemovePrefixShort { pattern }
                                };
                                self.indirect_expansion_word_part(
                                    reference,
                                    Some(operator),
                                    None,
                                    false,
                                )
                            }
                            '%' => {
                                let longest =
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '%');
                                let operand_text =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                                let pattern = self.pattern_from_source_text(&operand_text);
                                let operator = if longest {
                                    ParameterOp::RemoveSuffixLong { pattern }
                                } else {
                                    ParameterOp::RemoveSuffixShort { pattern }
                                };
                                self.indirect_expansion_word_part(
                                    reference,
                                    Some(operator),
                                    None,
                                    false,
                                )
                            }
                            '/' => {
                                let replace_all =
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '/');
                                let pattern_text = self.read_replacement_pattern(
                                    &mut chars,
                                    &mut cursor,
                                    source_backed,
                                );
                                let pattern = self.pattern_from_source_text(&pattern_text);
                                let (replacement, consumed_closing_brace) =
                                    if Self::consume_word_char_if(&mut chars, &mut cursor, '/') {
                                        let replacement = self.read_brace_operand(
                                            &mut chars,
                                            &mut cursor,
                                            source_backed,
                                        );
                                        (
                                            replacement,
                                            cursor.offset > 0
                                                && self.input_prefix_ends_with(cursor.offset, '}'),
                                        )
                                    } else {
                                        (self.empty_source_text(cursor), false)
                                    };
                                if !consumed_closing_brace {
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                }
                                if !self.input_span_ends_with(part_start, cursor, '}')
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
                                self.indirect_expansion_word_part(
                                    reference,
                                    Some(operator),
                                    None,
                                    false,
                                )
                            }
                            _ => unreachable!(),
                        };
                        let part = self.parameter_word_part_from_legacy(
                            part,
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else {
                        let mut suffix = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '}' {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                break;
                            }
                            suffix.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                        }
                        let part = if suffix.ends_with('*') || suffix.ends_with('@') {
                            let kind = if suffix.ends_with('@') {
                                PrefixMatchKind::At
                            } else {
                                PrefixMatchKind::Star
                            };
                            WordPart::PrefixMatch {
                                prefix: format!("{}{}", var_name, &suffix[..suffix.len() - 1])
                                    .into(),
                                kind,
                            }
                        } else {
                            WordPart::Variable(
                                format!(
                                    "!{}{}{}",
                                    var_name,
                                    subscript
                                        .as_ref()
                                        .map(|subscript| {
                                            format!("[{}]", subscript.syntax_text(self.input))
                                        })
                                        .unwrap_or_default(),
                                    suffix
                                )
                                .into(),
                            )
                        };
                        let part = self.parameter_word_part_from_legacy(
                            part,
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    }

                    current_start = cursor;
                    continue;
                }

                let mut var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                    c.is_ascii_alphanumeric() || c == '_'
                });

                if var_name.is_empty()
                    && let Some(&c) = chars.peek()
                    && matches!(c, '@' | '*' | '#' | '?' | '-' | '$' | '!')
                {
                    var_name.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                }

                if var_name.is_empty()
                    && self.dialect.features().zsh_parameter_modifiers
                    && chars.peek() == Some(&'+')
                {
                    let mut lookahead = chars.clone();
                    lookahead.next();
                    if lookahead
                        .peek()
                        .is_some_and(|next| Self::zsh_bare_parameter_target_start(*next))
                    {
                        let raw_body =
                            self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                        let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                        Self::push_word_part(parts, parameter, part_start, cursor);
                        current_start = cursor;
                        continue;
                    }
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                    let (index, raw_index) =
                        self.read_array_index(&mut chars, &mut cursor, source_backed);
                    let subscript = self.subscript_from_source_text(
                        index,
                        raw_index,
                        SubscriptInterpretation::Contextual,
                    );

                    let part = if let Some(next_c) = chars.peek().copied() {
                        if next_c == ':' {
                            let mut lookahead = chars.clone();
                            lookahead.next();
                            let is_param_op = matches!(
                                lookahead.peek(),
                                Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?')
                            );
                            if is_param_op {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let operand =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                                let operator = match op_char {
                                    '-' => ParameterOp::UseDefault,
                                    '=' => ParameterOp::AssignDefault,
                                    '+' => ParameterOp::UseReplacement,
                                    '?' => ParameterOp::Error,
                                    _ => unreachable!(),
                                };
                                self.parameter_expansion_word_part(
                                    self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
                                    operator,
                                    Some(operand),
                                    true,
                                )
                            } else if self.zsh_parameter_suffix_looks_like_modifier(&mut chars) {
                                let tail =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                                let raw_body = self.prefixed_parameter_raw_body(
                                    &format!("{}[{}]", var_name, subscript.syntax_text(self.input)),
                                    brace_body_start,
                                    tail,
                                    source_backed,
                                );
                                let parameter =
                                    self.zsh_parameter_word_part(raw_body, part_start, cursor);
                                Self::push_word_part(parts, parameter, part_start, cursor);
                                current_start = cursor;
                                continue;
                            } else {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let (offset, length) = self.read_parameter_slice_parts(
                                    &mut chars,
                                    &mut cursor,
                                    source_backed,
                                );
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                self.array_slice_word_part(
                                    self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
                                    offset,
                                    length,
                                )
                            }
                        } else if matches!(next_c, '-' | '+' | '=' | '?') {
                            let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let operator = match op_char {
                                '-' => ParameterOp::UseDefault,
                                '=' => ParameterOp::AssignDefault,
                                '+' => ParameterOp::UseReplacement,
                                '?' => ParameterOp::Error,
                                _ => unreachable!(),
                            };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                Some(operand),
                                false,
                            )
                        } else if next_c == '#' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let longest = Self::consume_word_char_if(&mut chars, &mut cursor, '#');
                            let operand_text =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let pattern = self.pattern_from_source_text(&operand_text);
                            let operator = if longest {
                                ParameterOp::RemovePrefixLong { pattern }
                            } else {
                                ParameterOp::RemovePrefixShort { pattern }
                            };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                None,
                                false,
                            )
                        } else if next_c == '%' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let longest = Self::consume_word_char_if(&mut chars, &mut cursor, '%');
                            let operand_text =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let pattern = self.pattern_from_source_text(&operand_text);
                            let operator = if longest {
                                ParameterOp::RemoveSuffixLong { pattern }
                            } else {
                                ParameterOp::RemoveSuffixShort { pattern }
                            };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                None,
                                false,
                            )
                        } else if next_c == '/' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let replace_all =
                                Self::consume_word_char_if(&mut chars, &mut cursor, '/');
                            let pattern_text = self.read_replacement_pattern(
                                &mut chars,
                                &mut cursor,
                                source_backed,
                            );
                            let pattern = self.pattern_from_source_text(&pattern_text);
                            let (replacement, consumed_closing_brace) =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '/') {
                                    let replacement = self.read_brace_operand(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    );
                                    (
                                        replacement,
                                        cursor.offset > 0
                                            && self.input_prefix_ends_with(cursor.offset, '}'),
                                    )
                                } else {
                                    (self.empty_source_text(cursor), false)
                                };
                            if !consumed_closing_brace {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            }
                            if !self.input_span_ends_with(part_start, cursor, '}')
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
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                None,
                                false,
                            )
                        } else if next_c == '^' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '^') {
                                    ParameterOp::UpperAll
                                } else {
                                    ParameterOp::UpperFirst
                                };
                            let operand =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                                    None
                                } else {
                                    Some(self.read_brace_operand(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    ))
                                };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                operand,
                                false,
                            )
                        } else if next_c == ',' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, ',') {
                                    ParameterOp::LowerAll
                                } else {
                                    ParameterOp::LowerFirst
                                };
                            let operand =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                                    None
                                } else {
                                    Some(self.read_brace_operand(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    ))
                                };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                operand,
                                false,
                            )
                        } else if next_c == '@' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            if chars.peek().is_some() {
                                let operator = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Transformation {
                                    reference: self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
                                    operator,
                                }
                            } else {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::ArrayAccess(self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ))
                            }
                        } else {
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ArrayAccess(self.parameter_var_ref(
                                part_start,
                                "${",
                                &var_name,
                                Some(subscript),
                                cursor,
                            ))
                        }
                    } else {
                        WordPart::ArrayAccess(self.parameter_var_ref(
                            part_start,
                            "${",
                            &var_name,
                            Some(subscript),
                            cursor,
                        ))
                    };

                    let part = self.parameter_word_part_from_legacy(
                        part,
                        part_start,
                        cursor,
                        source_backed,
                    );
                    Self::push_word_part(parts, part, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                let part = if chars.peek().is_some() {
                    self.parse_parameter_tail_without_subscript(
                        &mut chars,
                        &mut cursor,
                        source_backed,
                        part_start,
                        brace_body_start,
                        &var_name,
                    )
                } else {
                    WordPart::Variable(var_name.into())
                };

                let part = if cursor.offset > brace_body_start.offset {
                    self.parameter_word_part_from_legacy(part, part_start, cursor, source_backed)
                } else {
                    part
                };
                Self::push_word_part(parts, part, part_start, cursor);
                current_start = cursor;
                continue;
            }

            if let Some(&c) = chars.peek() {
                if let Some(part) = self.parse_zsh_bare_prefixed_parameter(
                    &mut chars,
                    &mut cursor,
                    part_start,
                    source_backed,
                ) {
                    Self::push_word_part(parts, part, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-') || c.is_ascii_digit() {
                    let name = Self::next_word_char_unwrap(&mut chars, &mut cursor).to_string();
                    Self::push_word_part(
                        parts,
                        WordPart::Variable(name.into()),
                        part_start,
                        cursor,
                    );
                    current_start = cursor;
                } else {
                    let mut var_name = String::new();
                    if self.dialect == ShellDialect::Zsh && chars.peek() == Some(&'+') {
                        let mut lookahead = chars.clone();
                        lookahead.next();
                        if lookahead
                            .peek()
                            .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
                        {
                            var_name.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                        }
                    }
                    var_name.push_str(&Self::read_word_while(&mut chars, &mut cursor, |c| {
                        c.is_ascii_alphanumeric() || c == '_'
                    }));
                    if !var_name.is_empty() {
                        let part = if self.dialect == ShellDialect::Zsh
                            && Self::consume_word_char_if(&mut chars, &mut cursor, '[')
                        {
                            let (index, raw_index) =
                                self.read_array_index(&mut chars, &mut cursor, source_backed);
                            let subscript = self.subscript_from_source_text(
                                index,
                                raw_index,
                                SubscriptInterpretation::Contextual,
                            );
                            WordPart::ArrayAccess(self.parameter_var_ref(
                                part_start,
                                "$",
                                &var_name,
                                Some(subscript),
                                cursor,
                            ))
                        } else {
                            WordPart::Variable(var_name.into())
                        };
                        Self::push_word_part(parts, part, part_start, cursor);
                        current_start = cursor;
                    } else {
                        if current.is_empty() {
                            current_start = part_start;
                        }
                        current.push('$');
                    }
                }
            } else {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push('$');
            }
        }

        self.flush_literal_part(parts, &mut current, current_start, cursor, source_backed);

        if parts.is_empty() {
            Self::push_word_part(
                parts,
                WordPart::Literal(self.literal_text(String::new(), base, cursor, source_backed)),
                base,
                cursor,
            );
        }
    }
}
