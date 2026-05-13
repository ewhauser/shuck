use super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn scan_raw_dollar_paren_substitution_end(
        raw: &str,
        start: usize,
    ) -> Option<usize> {
        let tail = raw.get(start..)?;
        if !tail.starts_with("$(") || tail[2..].starts_with('(') {
            return None;
        }

        let body_start = start + 2;
        let consumed = lexer::scan_command_substitution_body_len(&raw[body_start..])?;
        Some(body_start + consumed)
    }

    pub(in crate::parser) fn raw_text_has_top_level_unquoted_array_comma(
        &self,
        raw: &str,
        word: &Word,
    ) -> bool {
        let mut index = 0usize;
        let mut in_single = false;
        let mut in_ansi_c_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut ansi_c_quote_pending = false;

        while index < raw.len() {
            let Some(ch) = raw[index..].chars().next() else {
                break;
            };
            let next_index = index + ch.len_utf8();
            let was_escaped = escaped;
            if ch == '\\' && !in_single {
                escaped = !escaped;
                ansi_c_quote_pending = false;
                index = next_index;
                continue;
            }
            escaped = false;

            if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped && ch == '$' {
                if raw[next_index..].starts_with("((")
                    && let Some(consumed) =
                        Self::scan_array_arithmetic_expansion_len(&raw[next_index + 2..])
                {
                    ansi_c_quote_pending = false;
                    index = next_index + 2 + consumed;
                    continue;
                }

                if raw[next_index..].starts_with('(')
                    && !raw[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len(
                        &raw[next_index + '('.len_utf8()..],
                    )
                {
                    ansi_c_quote_pending = false;
                    index = next_index + '('.len_utf8() + consumed;
                    continue;
                }

                if raw[next_index..].starts_with('{')
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len(
                        &raw[next_index + '{'.len_utf8()..],
                    )
                {
                    ansi_c_quote_pending = false;
                    index = next_index + '{'.len_utf8() + consumed;
                    continue;
                }
            }

            if !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped
                && matches!(ch, '<' | '>')
                && raw[next_index..].starts_with('(')
                && let Some(consumed) =
                    lexer::scan_command_substitution_body_len(&raw[next_index + '('.len_utf8()..])
            {
                ansi_c_quote_pending = false;
                index = next_index + '('.len_utf8() + consumed;
                continue;
            }

            match ch {
                '\'' if !in_double && !in_backtick && !was_escaped => {
                    if in_ansi_c_single {
                        in_ansi_c_single = false;
                    } else if !in_single && ansi_c_quote_pending {
                        in_ansi_c_single = true;
                    } else {
                        in_single = !in_single;
                    }
                }
                '"' if !in_single && !in_ansi_c_single && !was_escaped => in_double = !in_double,
                '`' if !in_single && !in_ansi_c_single && !in_double && !was_escaped => {
                    in_backtick = !in_backtick
                }
                ',' if !in_single && !in_ansi_c_single && !in_double && !in_backtick => {
                    let comma_offset = word.span.start.offset + index;
                    if !self.comma_is_brace_separator(word, comma_offset, was_escaped)
                        && !self.comma_is_zsh_word_syntax(word, comma_offset)
                    {
                        return true;
                    }
                }
                _ => {}
            }

            ansi_c_quote_pending = ch == '$'
                && !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped;
            index = next_index;
        }

        false
    }

    const MAX_ARRAY_NESTED_EXPANSION_SCAN_DEPTH: usize = 4;

    pub(in crate::parser) fn scan_array_arithmetic_expansion_len(text: &str) -> Option<usize> {
        Self::scan_array_arithmetic_expansion_len_inner(text, 0)
    }

    pub(in crate::parser) fn scan_array_arithmetic_expansion_len_inner(
        text: &str,
        scan_depth: usize,
    ) -> Option<usize> {
        if scan_depth >= Self::MAX_ARRAY_NESTED_EXPANSION_SCAN_DEPTH {
            return Self::scan_array_arithmetic_expansion_len_balanced(text);
        }

        let mut index = 0usize;
        let mut depth = 2usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while index < text.len() {
            let ch = text[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            let was_escaped = escaped;
            if ch == '\\' && !in_single {
                escaped = !escaped;
                index = next_index;
                continue;
            }
            escaped = false;

            if !in_single && !was_escaped && ch == '$' {
                if text[next_index..].starts_with("((")
                    && let Some(consumed) = Self::scan_array_arithmetic_expansion_len_inner(
                        &text[next_index + 2..],
                        scan_depth + 1,
                    )
                {
                    index = next_index + 2 + consumed;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                        &text[next_index + '('.len_utf8()..],
                        scan_depth + 1,
                    )
                {
                    index = next_index + '('.len_utf8() + consumed;
                    continue;
                }

                if text[next_index..].starts_with('{')
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len_inner(
                        &text[next_index + '{'.len_utf8()..],
                        scan_depth + 1,
                    )
                {
                    index = next_index + '{'.len_utf8() + consumed;
                    continue;
                }
            }

            match ch {
                '\'' if !in_double && !was_escaped => in_single = !in_single,
                '"' if !in_single && !was_escaped => in_double = !in_double,
                '(' if !in_single && !in_double && !was_escaped => depth += 1,
                ')' if !in_single && !in_double && !was_escaped => {
                    depth -= 1;
                    index = next_index;
                    if depth == 0 {
                        return Some(index);
                    }
                    continue;
                }
                _ => {}
            }

            index = next_index;
        }

        None
    }

    pub(in crate::parser) fn scan_array_arithmetic_expansion_len_balanced(
        text: &str,
    ) -> Option<usize> {
        let mut index = 0usize;
        let mut depth = 2usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while index < text.len() {
            let ch = text[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            let was_escaped = escaped;
            if ch == '\\' && !in_single {
                escaped = !escaped;
                index = next_index;
                continue;
            }
            escaped = false;

            if !in_single && !was_escaped && ch == '$' {
                if text[next_index..].starts_with('{')
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len_inner(
                        &text[next_index + '{'.len_utf8()..],
                        0,
                    )
                {
                    index = next_index + '{'.len_utf8() + consumed;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                        &text[next_index + '('.len_utf8()..],
                        0,
                    )
                {
                    index = next_index + '('.len_utf8() + consumed;
                    continue;
                }
            }

            match ch {
                '\'' if !in_double && !was_escaped => in_single = !in_single,
                '"' if !in_single && !was_escaped => in_double = !in_double,
                '(' if !in_single && !in_double && !was_escaped => depth = depth.saturating_add(1),
                ')' if !in_single && !in_double && !was_escaped => {
                    depth = depth.saturating_sub(1);
                    index = next_index;
                    if depth == 0 {
                        return Some(index);
                    }
                    continue;
                }
                _ => {}
            }

            index = next_index;
        }

        None
    }

    pub(in crate::parser) fn scan_array_parameter_expansion_len(text: &str) -> Option<usize> {
        Self::scan_array_parameter_expansion_len_inner(text, 0)
    }

    pub(in crate::parser) fn scan_array_parameter_expansion_len_inner(
        text: &str,
        depth: usize,
    ) -> Option<usize> {
        if depth >= Self::MAX_ARRAY_NESTED_EXPANSION_SCAN_DEPTH {
            return Self::scan_array_parameter_expansion_len_balanced(text);
        }

        let mut index = 0usize;
        let mut in_single = false;
        let mut in_ansi_c_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut ansi_c_quote_pending = false;

        while index < text.len() {
            let ch = text[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            let was_escaped = escaped;
            if ch == '\\' && !in_single {
                escaped = !escaped;
                index = next_index;
                ansi_c_quote_pending = false;
                continue;
            }
            escaped = false;

            if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped && ch == '$' {
                if text[next_index..].starts_with('{')
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len_inner(
                        &text[next_index + '{'.len_utf8()..],
                        depth + 1,
                    )
                {
                    index = next_index + '{'.len_utf8() + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with("((")
                    && let Some(consumed) = Self::scan_array_arithmetic_expansion_len_inner(
                        &text[next_index + 2..],
                        depth + 1,
                    )
                {
                    index = next_index + 2 + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                        &text[next_index + '('.len_utf8()..],
                        depth + 1,
                    )
                {
                    index = next_index + '('.len_utf8() + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }
            }

            if !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped
                && matches!(ch, '<' | '>')
                && text[next_index..].starts_with('(')
                && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                    &text[next_index + '('.len_utf8()..],
                    depth + 1,
                )
            {
                index = next_index + '('.len_utf8() + consumed;
                ansi_c_quote_pending = false;
                continue;
            }

            match ch {
                '\'' if !in_double && !in_backtick && !was_escaped => {
                    if in_ansi_c_single {
                        in_ansi_c_single = false;
                    } else if !in_single && ansi_c_quote_pending {
                        in_ansi_c_single = true;
                    } else {
                        in_single = !in_single;
                    }
                }
                '"' if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped => {
                    in_double = !in_double
                }
                '`' if !in_single && !in_ansi_c_single && !in_double && !was_escaped => {
                    in_backtick = !in_backtick
                }
                '}' if !in_single
                    && !in_ansi_c_single
                    && !in_double
                    && !in_backtick
                    && !was_escaped =>
                {
                    return Some(next_index);
                }
                _ => {}
            }

            ansi_c_quote_pending = ch == '$'
                && !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped;
            index = next_index;
        }

        None
    }

    pub(in crate::parser) fn scan_array_parameter_expansion_len_balanced(
        text: &str,
    ) -> Option<usize> {
        let mut index = 0usize;
        let mut brace_depth = 1usize;
        let mut in_single = false;
        let mut in_ansi_c_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut ansi_c_quote_pending = false;

        while index < text.len() {
            let ch = text[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            let was_escaped = escaped;
            if ch == '\\' && !in_single {
                escaped = !escaped;
                index = next_index;
                ansi_c_quote_pending = false;
                continue;
            }
            escaped = false;

            if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped && ch == '$' {
                if text[next_index..].starts_with("((")
                    && let Some(consumed) =
                        Self::scan_array_arithmetic_expansion_len_inner(&text[next_index + 2..], 0)
                {
                    index = next_index + 2 + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                        &text[next_index + '('.len_utf8()..],
                        0,
                    )
                {
                    index = next_index + '('.len_utf8() + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with('{') {
                    brace_depth = brace_depth.saturating_add(1);
                    index = next_index + '{'.len_utf8();
                    ansi_c_quote_pending = false;
                    continue;
                }
            }

            if !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped
                && matches!(ch, '<' | '>')
                && text[next_index..].starts_with('(')
                && let Some(consumed) = lexer::scan_command_substitution_body_len_inner(
                    &text[next_index + '('.len_utf8()..],
                    0,
                )
            {
                index = next_index + '('.len_utf8() + consumed;
                ansi_c_quote_pending = false;
                continue;
            }

            match ch {
                '\'' if !in_double && !in_backtick && !was_escaped => {
                    if in_ansi_c_single {
                        in_ansi_c_single = false;
                    } else if !in_single && ansi_c_quote_pending {
                        in_ansi_c_single = true;
                    } else {
                        in_single = !in_single;
                    }
                }
                '"' if !in_single && !in_ansi_c_single && !in_backtick && !was_escaped => {
                    in_double = !in_double
                }
                '`' if !in_single && !in_ansi_c_single && !in_double && !was_escaped => {
                    in_backtick = !in_backtick
                }
                '}' if !in_single
                    && !in_ansi_c_single
                    && !in_double
                    && !in_backtick
                    && !was_escaped =>
                {
                    brace_depth = brace_depth.saturating_sub(1);
                    if brace_depth == 0 {
                        return Some(next_index);
                    }
                }
                _ => {}
            }

            ansi_c_quote_pending = ch == '$'
                && !in_single
                && !in_ansi_c_single
                && !in_double
                && !in_backtick
                && !was_escaped;
            index = next_index;
        }

        None
    }

    pub(in crate::parser) fn comma_is_brace_separator(
        &self,
        word: &Word,
        offset: usize,
        escaped: bool,
    ) -> bool {
        if escaped {
            return false;
        }

        Self::inside_active_brace_expansion(word, offset)
            || self.inside_unquoted_brace_group(word, offset)
    }

    pub(in crate::parser) fn inside_active_brace_expansion(word: &Word, offset: usize) -> bool {
        word.brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.expands())
            .any(|brace| brace.span.start.offset <= offset && offset < brace.span.end.offset)
    }

    pub(in crate::parser) fn comma_is_zsh_word_syntax(
        &self,
        word: &Word,
        comma_offset: usize,
    ) -> bool {
        self.dialect == ShellDialect::Zsh
            && (word
                .parts
                .iter()
                .any(|part| word_part_has_zsh_syntax_comma(&part.kind, comma_offset))
                || self.comma_is_zsh_terminal_glob_group_syntax(word, comma_offset))
    }

    pub(in crate::parser) fn comma_is_zsh_terminal_glob_group_syntax(
        &self,
        word: &Word,
        comma_offset: usize,
    ) -> bool {
        if comma_offset < word.span.start.offset || word.span.end.offset <= comma_offset {
            return false;
        }

        let text = word.span.slice(self.input);
        let relative_comma = comma_offset - word.span.start.offset;
        let Some(group_start) = Self::enclosing_terminal_group_start(text, relative_comma) else {
            return false;
        };

        Self::text_has_zsh_glob_syntax(&text[..group_start])
    }

    pub(in crate::parser) fn enclosing_terminal_group_start(
        text: &str,
        target: usize,
    ) -> Option<usize> {
        if !text.ends_with(')') || target >= text.len() {
            return None;
        }

        let mut depth = 0usize;
        let mut group_start = None;
        for (index, ch) in text.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        group_start = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }

        let group_start = group_start?;
        (group_start < target && target < text.len() - ')'.len_utf8()).then_some(group_start)
    }

    pub(in crate::parser) fn text_has_zsh_glob_syntax(text: &str) -> bool {
        let mut escaped = false;
        for ch in text.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '*' | '?' | '[' => return true,
                _ => {}
            }
        }

        false
    }

    pub(in crate::parser) fn inside_unquoted_brace_group(
        &self,
        word: &Word,
        target_offset: usize,
    ) -> bool {
        let text = word.span.slice(self.input);
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut brace_depth = 0usize;

        for (index, ch) in text.char_indices() {
            let absolute = word.span.start.offset + index;

            if absolute == target_offset {
                return brace_depth > 0;
            }

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => {
                    escaped = true;
                    continue;
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    continue;
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    continue;
                }
                _ => {}
            }

            if in_single || in_double {
                continue;
            }

            match ch {
                '{' if !text[..index].ends_with('$') => brace_depth += 1,
                '}' if brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }
        }

        false
    }

    pub(in crate::parser) fn raw_source_hash_starts_comment(source: &str, index: usize) -> bool {
        source[..index]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace)
    }

    pub(in crate::parser) fn split_compound_array_key_value<'b>(
        &self,
        raw: &'b str,
    ) -> Option<(&'b str, &'b str, bool, usize, usize)> {
        if !raw.starts_with('[') {
            return None;
        }

        let mut close_index = None;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut paren_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        for (index, ch) in raw.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '`' if !in_single => in_backtick = !in_backtick,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double => {
                    if bracket_depth == 0 && brace_depth == 0 && paren_depth == 0 {
                        close_index = Some(index);
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
        let tail = &raw[close_index + 1..];
        let (append, value_offset) = if tail.starts_with("+=") {
            (true, 2)
        } else if tail.starts_with('=') {
            (false, 1)
        } else {
            return None;
        };

        Some((
            &raw[1..close_index],
            &tail[value_offset..],
            append,
            close_index,
            value_offset,
        ))
    }

    pub(in crate::parser) fn parse_compound_array_element(
        &mut self,
        raw: &str,
        span: Span,
        interpretation: SubscriptInterpretation,
    ) -> ArrayElem {
        if let Some((key_raw, value_raw, append, close_index, value_offset)) =
            self.split_compound_array_key_value(raw)
        {
            let key_start = span.start.advanced_by("[");
            let key_end = span.start.advanced_by(&raw[..close_index]);
            let key = self.subscript_from_text(
                key_raw,
                Span::from_positions(key_start, key_end),
                interpretation,
            );
            let value_start = span
                .start
                .advanced_by(&raw[..close_index + 1 + value_offset]);
            let value_span = Span::from_positions(value_start, span.end);
            let value = self.array_value_word_from_raw_text(value_raw, value_span);
            return if append {
                ArrayElem::KeyedAppend { key, value }
            } else {
                ArrayElem::Keyed { key, value }
            };
        }

        ArrayElem::Sequential(self.array_value_word_from_raw_text(raw, span))
    }

    pub(in crate::parser) fn parse_array_expr_from_text(
        &mut self,
        inner: &str,
        base: Position,
        explicit_kind: Option<ArrayKind>,
    ) -> ArrayExpr {
        let interpretation = Self::subscript_interpretation_from_array_kind(explicit_kind);
        let mut cursor = 0;
        let mut cursor_pos = base;
        let elements = self
            .split_compound_array_elements(inner)
            .into_iter()
            .map(|(start, end)| {
                if start > cursor {
                    cursor_pos = cursor_pos.advanced_by(&inner[cursor..start]);
                    cursor = start;
                }

                let start_pos = cursor_pos;
                let end_pos = start_pos.advanced_by(&inner[start..end]);
                let span = Span::from_positions(start_pos, end_pos);
                cursor = end;
                cursor_pos = end_pos;

                self.parse_compound_array_element(&inner[start..end], span, interpretation)
            })
            .collect::<Vec<_>>();

        let span = if let (Some(first), Some(last)) = (elements.first(), elements.last()) {
            first.span().merge(last.span())
        } else {
            Span::from_positions(base, base)
        };

        ArrayExpr {
            kind: Self::infer_array_expr_kind(explicit_kind, &elements),
            elements,
            span,
        }
    }
}
