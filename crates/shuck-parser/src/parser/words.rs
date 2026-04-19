use super::*;
use shuck_ast::ArrayValueWord;
#[derive(Debug, Clone, Copy)]
struct PatternCursor {
    segment_index: usize,
    literal_offset: usize,
    position: Position,
}

enum PatternSegment<'a> {
    Literal { text: &'a str, span: Span },
    Word(&'a WordPartNode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternParseMode {
    Standard,
    ZshCase,
    ZshConditional,
}

struct PatternParser<'a> {
    input: &'a str,
    segments: Vec<PatternSegment<'a>>,
    full_span: Span,
    mode: PatternParseMode,
}

enum WordTargetBoundary {
    EndOfWord,
    Assignment { append: bool, value_start: Position },
}

struct ParsedWordTarget {
    name: Name,
    name_span: Span,
    subscript: Option<Subscript>,
    boundary: WordTargetBoundary,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DecodeWordPartsOptions {
    preserve_quote_fragments: bool,
    parse_dollar_quotes: bool,
    preserve_escaped_expansion_literals: bool,
    parse_process_substitutions: bool,
}

impl Default for DecodeWordPartsOptions {
    fn default() -> Self {
        Self {
            preserve_quote_fragments: false,
            parse_dollar_quotes: false,
            preserve_escaped_expansion_literals: false,
            parse_process_substitutions: true,
        }
    }
}

impl<'a> PatternParser<'a> {
    fn new(input: &'a str, word: &'a Word) -> Self {
        Self::from_word_parts_with_mode(input, &word.parts, word.span, PatternParseMode::Standard)
    }

    fn from_word_parts(input: &'a str, parts: &'a [WordPartNode], full_span: Span) -> Self {
        Self::from_word_parts_with_mode(input, parts, full_span, PatternParseMode::Standard)
    }

    fn with_mode(input: &'a str, word: &'a Word, mode: PatternParseMode) -> Self {
        Self::from_word_parts_with_mode(input, &word.parts, word.span, mode)
    }

    fn from_word_parts_with_mode(
        input: &'a str,
        parts: &'a [WordPartNode],
        full_span: Span,
        mode: PatternParseMode,
    ) -> Self {
        let mut segments = Vec::with_capacity(parts.len());

        for part in parts {
            match &part.kind {
                WordPart::Literal(text) => segments.push(PatternSegment::Literal {
                    text: text.as_str(input, part.span),
                    span: part.span,
                }),
                _ => segments.push(PatternSegment::Word(part)),
            }
        }

        Self {
            input,
            segments,
            full_span,
            mode,
        }
    }

    fn parse(&self) -> Pattern {
        let mut cursor = PatternCursor {
            segment_index: 0,
            literal_offset: 0,
            position: self
                .segments
                .first()
                .map(|segment| self.segment_start(segment))
                .unwrap_or(self.full_span.start),
        };
        let mut pattern = self.parse_until(&mut cursor, false);
        pattern.span = self.full_span;
        pattern
    }

    fn parse_until(&self, cursor: &mut PatternCursor, stop_at_group_delim: bool) -> Pattern {
        let start = cursor.position;
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut literal_start: Option<Position> = None;
        let mut literal_end = start;

        while let Some(segment) = self.segments.get(cursor.segment_index) {
            if stop_at_group_delim && self.peek_group_delimiter(*cursor).is_some() {
                break;
            }

            match segment {
                PatternSegment::Word(part) => {
                    self.flush_literal(&mut parts, &mut literal, &mut literal_start, literal_end);
                    parts.push(PatternPartNode::new(
                        PatternPart::Word(Word {
                            parts: vec![(*part).clone()],
                            span: part.span,
                            brace_syntax: Vec::new(),
                        }),
                        part.span,
                    ));
                    self.advance_to_next_segment(cursor);
                }
                PatternSegment::Literal { .. } => {
                    if self.peek_literal_char(*cursor).is_none() {
                        self.advance_to_next_segment(cursor);
                        continue;
                    }

                    if let Some((group, next_cursor)) = self.try_parse_group(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(group);
                        *cursor = next_cursor;
                        continue;
                    }

                    if let Some((group, next_cursor)) = self.try_parse_zsh_case_group(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(group);
                        *cursor = next_cursor;
                        continue;
                    }

                    if let Some((char_class, next_cursor)) = self.try_parse_char_class(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(char_class);
                        *cursor = next_cursor;
                        continue;
                    }

                    if let Some((wildcard, next_cursor)) = self.try_parse_wildcard(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(wildcard);
                        *cursor = next_cursor;
                        continue;
                    }

                    let Some((ch, span)) = self.consume_literal_char(cursor) else {
                        break;
                    };
                    if literal_start.is_none() {
                        literal_start = Some(span.start);
                    }
                    literal_end = span.end;
                    literal.push(ch);
                }
            }
        }

        self.flush_literal(&mut parts, &mut literal, &mut literal_start, literal_end);

        Pattern {
            span: if let (Some(first), Some(last)) = (parts.first(), parts.last()) {
                first.span.merge(last.span)
            } else {
                Span::from_positions(start, cursor.position)
            },
            parts,
        }
    }

    fn try_parse_zsh_case_group(
        &self,
        cursor: PatternCursor,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        if !matches!(
            self.mode,
            PatternParseMode::ZshCase | PatternParseMode::ZshConditional
        ) {
            return None;
        }

        let opener = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || opener != '(' {
            return None;
        }

        let start = cursor.position;
        let mut next_cursor = cursor;
        self.consume_literal_char(&mut next_cursor)?;

        let mut patterns = vec![self.parse_until(&mut next_cursor, true)];
        if self.peek_group_delimiter(next_cursor) != Some('|') {
            return None;
        }

        loop {
            if self.peek_group_delimiter(next_cursor) == Some('|') {
                self.consume_literal_char(&mut next_cursor)?;
                patterns.push(self.parse_until(&mut next_cursor, true));
                continue;
            }

            if self.peek_group_delimiter(next_cursor) == Some(')') {
                let (_, close_span) = self.consume_literal_char(&mut next_cursor)?;
                return Some((
                    PatternPartNode::new(
                        PatternPart::Group {
                            kind: PatternGroupKind::ExactlyOne,
                            patterns,
                        },
                        Span::from_positions(start, close_span.end),
                    ),
                    next_cursor,
                ));
            }

            return None;
        }
    }

    fn flush_literal(
        &self,
        parts: &mut Vec<PatternPartNode>,
        literal: &mut String,
        literal_start: &mut Option<Position>,
        literal_end: Position,
    ) {
        let Some(start) = literal_start.take() else {
            return;
        };
        let span = Span::from_positions(start, literal_end);
        let text = std::mem::take(literal);
        parts.push(PatternPartNode::new(
            PatternPart::Literal(self.literal_text(span, text)),
            span,
        ));
    }

    fn try_parse_wildcard(
        &self,
        cursor: PatternCursor,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        let ch = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || !matches!(ch, '*' | '?') {
            return None;
        }

        let mut next_cursor = cursor;
        let (_, span) = self.consume_literal_char(&mut next_cursor)?;
        let kind = if ch == '*' {
            PatternPart::AnyString
        } else {
            PatternPart::AnyChar
        };
        Some((PatternPartNode::new(kind, span), next_cursor))
    }

    fn try_parse_char_class(
        &self,
        cursor: PatternCursor,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        if self.is_escaped(cursor) || !text[cursor.literal_offset..].starts_with('[') {
            return None;
        }

        let end_offset = self.find_char_class_end(text, cursor.literal_offset)?;
        let raw = &text[cursor.literal_offset..end_offset];
        let start = cursor.position;
        let end = start.advanced_by(raw);
        let span = Span::from_positions(start, end);
        let mut next_cursor = cursor;
        next_cursor.literal_offset = end_offset;
        next_cursor.position = end;
        if end_offset == text.len() {
            self.advance_to_next_segment(&mut next_cursor);
        }

        Some((
            PatternPartNode::new(
                PatternPart::CharClass(self.source_text(span, raw.to_string())),
                span,
            ),
            next_cursor,
        ))
    }

    fn try_parse_group(&self, cursor: PatternCursor) -> Option<(PatternPartNode, PatternCursor)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        let opener = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || !matches!(opener, '?' | '*' | '+' | '@' | '!') {
            return None;
        }
        let mut chars = text[cursor.literal_offset..].chars();
        chars.next()?;
        if chars.next()? != '(' {
            return None;
        }

        let kind = match opener {
            '?' => PatternGroupKind::ZeroOrOne,
            '*' => PatternGroupKind::ZeroOrMore,
            '+' => PatternGroupKind::OneOrMore,
            '@' => PatternGroupKind::ExactlyOne,
            '!' => PatternGroupKind::NoneOf,
            _ => return None,
        };

        let start = cursor.position;
        let mut next_cursor = cursor;
        self.consume_literal_char(&mut next_cursor)?;
        self.consume_literal_char(&mut next_cursor)?;

        let mut patterns = Vec::new();
        loop {
            patterns.push(self.parse_until(&mut next_cursor, true));
            match self.peek_group_delimiter(next_cursor) {
                Some('|') => {
                    self.consume_literal_char(&mut next_cursor)?;
                }
                Some(')') => {
                    let (_, close_span) = self.consume_literal_char(&mut next_cursor)?;
                    return Some((
                        PatternPartNode::new(
                            PatternPart::Group { kind, patterns },
                            Span::from_positions(start, close_span.end),
                        ),
                        next_cursor,
                    ));
                }
                _ => return None,
            }
        }
    }

    fn find_char_class_end(&self, text: &str, start_offset: usize) -> Option<usize> {
        let mut cursor = start_offset + '['.len_utf8();
        let mut chars = text[cursor..].chars();

        if matches!(chars.next(), Some('!') | Some('^')) {
            cursor += 1;
        }
        if text[cursor..].starts_with(']') {
            cursor += 1;
        }

        while cursor < text.len() {
            let rest = &text[cursor..];
            let ch = rest.chars().next()?;

            if ch == '\\' {
                cursor += ch.len_utf8();
                if let Some(next) = text[cursor..].chars().next() {
                    cursor += next.len_utf8();
                }
                continue;
            }

            if ch == '['
                && let Some(class_kind) = text[cursor + 1..].chars().next()
                && matches!(class_kind, ':' | '.' | '=')
            {
                cursor += '['.len_utf8() + class_kind.len_utf8();
                loop {
                    let rest = &text[cursor..];
                    let inner = rest.chars().next()?;
                    cursor += inner.len_utf8();
                    if inner == class_kind && text[cursor..].starts_with(']') {
                        cursor += ']'.len_utf8();
                        break;
                    }
                }
                continue;
            }

            cursor += ch.len_utf8();
            if ch == ']' {
                return Some(cursor);
            }
        }

        None
    }

    fn peek_group_delimiter(&self, cursor: PatternCursor) -> Option<char> {
        let ch = self.peek_literal_char(cursor)?;
        (!self.is_escaped(cursor) && matches!(ch, '|' | ')')).then_some(ch)
    }

    fn peek_literal_char(&self, cursor: PatternCursor) -> Option<char> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        text[cursor.literal_offset..].chars().next()
    }

    fn is_escaped(&self, cursor: PatternCursor) -> bool {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index).unwrap()
        else {
            return false;
        };
        let mut backslashes = 0;
        let mut offset = cursor.literal_offset;
        while offset > 0 {
            offset -= 1;
            if text.as_bytes()[offset] != b'\\' {
                break;
            }
            backslashes += 1;
        }
        backslashes % 2 == 1
    }

    fn consume_literal_char(&self, cursor: &mut PatternCursor) -> Option<(char, Span)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        let ch = text[cursor.literal_offset..].chars().next()?;
        let start = cursor.position;
        cursor.literal_offset += ch.len_utf8();
        cursor.position.advance(ch);
        let span = Span::from_positions(start, cursor.position);

        if cursor.literal_offset == text.len() {
            self.advance_to_next_segment(cursor);
        }

        Some((ch, span))
    }

    fn advance_to_next_segment(&self, cursor: &mut PatternCursor) {
        cursor.segment_index += 1;
        cursor.literal_offset = 0;
        cursor.position = self
            .segments
            .get(cursor.segment_index)
            .map(|segment| self.segment_start(segment))
            .unwrap_or(self.full_span.end);
    }

    fn segment_start(&self, segment: &PatternSegment<'_>) -> Position {
        match segment {
            PatternSegment::Literal { span, .. } => span.start,
            PatternSegment::Word(part) => part.span.start,
        }
    }

    fn literal_text(&self, span: Span, text: String) -> LiteralText {
        if self.source_matches(span, &text) {
            LiteralText::source()
        } else {
            LiteralText::owned(text)
        }
    }

    fn source_text(&self, span: Span, text: String) -> SourceText {
        if self.source_matches(span, &text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text)
        }
    }

    fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == text
    }
}

impl<'a> Parser<'a> {
    pub(super) fn pattern_from_word(&self, word: &Word) -> Pattern {
        PatternParser::new(self.input, word).parse()
    }

    pub(super) fn pattern_from_conditional_word(&self, word: &Word) -> Pattern {
        let mode = if self.dialect == ShellDialect::Zsh {
            PatternParseMode::ZshConditional
        } else {
            PatternParseMode::Standard
        };
        PatternParser::with_mode(self.input, word, mode).parse()
    }

    pub(super) fn pattern_from_zsh_case_span(&mut self, span: Span) -> Pattern {
        let text = span.slice(self.input);
        let word = if Self::source_text_needs_quote_preserving_decode(text) {
            self.decode_fragment_word_text(text, span, span.start, true)
        } else {
            self.decode_word_text(text, span, span.start, true)
        };
        PatternParser::with_mode(self.input, &word, PatternParseMode::ZshCase).parse()
    }

    pub(super) fn pattern_from_source_text(&mut self, text: &SourceText) -> Pattern {
        let span = text.span();
        let mut parts = Vec::new();
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
        PatternParser::from_word_parts(self.input, &parts, span).parse()
    }

    pub(super) fn single_literal_word_text<'b>(&'b self, word: &'b Word) -> Option<&'b str> {
        if word.is_fully_quoted() || word.parts.len() != 1 {
            return None;
        }
        let WordPart::Literal(text) = &word.parts[0].kind else {
            return None;
        };
        Some(text.as_str(self.input, word.part_span(0)?))
    }

    pub(super) fn literal_word_text(&self, word: &Word) -> Option<String> {
        let mut text = String::new();
        self.collect_literal_word_text(&word.parts, &mut text)?;
        Some(text)
    }

    pub(super) fn source_text_needs_quote_preserving_decode(text: &str) -> bool {
        text.contains(['\'', '"'])
    }

    pub(super) fn decode_word_text_preserving_quotes_if_needed(
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

    pub(super) fn decode_word_text_preserving_quotes_if_needed_with_escape_mode(
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

    pub(super) fn collect_literal_word_text(
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
    pub(super) fn is_assignment(word: &str) -> Option<(&str, Option<&str>, &str, bool)> {
        if !word.contains('=') {
            return None;
        }

        let mut ident_end = 0;
        let mut chars = word.char_indices();
        let (_, first) = chars.next()?;
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

    pub(super) fn scan_split_indexed_assignment(
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

    pub(super) fn try_parse_split_indexed_assignment_from_text(&mut self) -> Option<Assignment> {
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

    pub(super) fn infer_array_expr_kind(
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

    pub(super) fn subscript_interpretation_from_array_kind(
        explicit_kind: Option<ArrayKind>,
    ) -> SubscriptInterpretation {
        match explicit_kind {
            Some(ArrayKind::Indexed) => SubscriptInterpretation::Indexed,
            Some(ArrayKind::Associative) => SubscriptInterpretation::Associative,
            _ => SubscriptInterpretation::Contextual,
        }
    }

    pub(super) fn word_from_raw_text(&mut self, raw: &str, span: Span) -> Word {
        if raw.is_empty() {
            return Word::literal_with_span("", span);
        }

        self.parse_word_with_context(raw, span, span.start, self.source_matches(span, raw))
    }

    pub(super) fn array_value_word_from_raw_text(
        &mut self,
        raw: &str,
        span: Span,
    ) -> ArrayValueWord {
        let word = self.word_from_raw_text(raw, span);
        let has_top_level_unquoted_comma =
            self.raw_text_has_top_level_unquoted_array_comma(raw, &word);
        ArrayValueWord::new(word, has_top_level_unquoted_comma)
    }

    pub(super) fn split_compound_array_elements(&self, inner: &str) -> Vec<(usize, usize)> {
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
            let ch = inner[index..]
                .chars()
                .next()
                .expect("index is within bounds while scanning array elements");
            let next_index = index + ch.len_utf8();
            if start.is_none() {
                if ch.is_whitespace() {
                    index = next_index;
                    continue;
                }
                if ch == '#' {
                    while index < inner.len() {
                        let comment_ch = inner[index..]
                            .chars()
                            .next()
                            .expect("index is within bounds while skipping array comment");
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
                        let comment_ch = inner[index..]
                            .chars()
                            .next()
                            .expect("index is within bounds while skipping array comment");
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

    fn scan_raw_dollar_paren_substitution_end(raw: &str, start: usize) -> Option<usize> {
        let tail = raw.get(start..)?;
        if !tail.starts_with("$(") || tail[2..].starts_with('(') {
            return None;
        }

        let body_start = start + 2;
        let consumed = lexer::scan_command_substitution_body_len(&raw[body_start..])?;
        Some(body_start + consumed)
    }

    fn raw_text_has_top_level_unquoted_array_comma(&self, raw: &str, word: &Word) -> bool {
        let mut index = 0usize;
        let mut in_single = false;
        let mut in_ansi_c_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut ansi_c_quote_pending = false;

        while index < raw.len() {
            let ch = raw[index..]
                .chars()
                .next()
                .expect("index is within bounds while scanning parser-owned array surface");
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
                    if !self.comma_is_brace_separator(word, comma_offset, was_escaped) {
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

    fn scan_array_arithmetic_expansion_len(text: &str) -> Option<usize> {
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
                    && let Some(consumed) =
                        Self::scan_array_arithmetic_expansion_len(&text[next_index + 2..])
                {
                    index = next_index + 2 + consumed;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len(
                        &text[next_index + '('.len_utf8()..],
                    )
                {
                    index = next_index + '('.len_utf8() + consumed;
                    continue;
                }

                if text[next_index..].starts_with('{')
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len(
                        &text[next_index + '{'.len_utf8()..],
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

    fn scan_array_parameter_expansion_len(text: &str) -> Option<usize> {
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
                    && let Some(consumed) = Self::scan_array_parameter_expansion_len(
                        &text[next_index + '{'.len_utf8()..],
                    )
                {
                    index = next_index + '{'.len_utf8() + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with("((")
                    && let Some(consumed) =
                        Self::scan_array_arithmetic_expansion_len(&text[next_index + 2..])
                {
                    index = next_index + 2 + consumed;
                    ansi_c_quote_pending = false;
                    continue;
                }

                if text[next_index..].starts_with('(')
                    && !text[next_index + '('.len_utf8()..].starts_with('(')
                    && let Some(consumed) = lexer::scan_command_substitution_body_len(
                        &text[next_index + '('.len_utf8()..],
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
                && let Some(consumed) =
                    lexer::scan_command_substitution_body_len(&text[next_index + '('.len_utf8()..])
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

    fn comma_is_brace_separator(&self, word: &Word, offset: usize, escaped: bool) -> bool {
        if escaped {
            return false;
        }

        Self::inside_active_brace_expansion(word, offset)
            || self.inside_unquoted_brace_group(word, offset)
    }

    fn inside_active_brace_expansion(word: &Word, offset: usize) -> bool {
        word.brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.expands())
            .any(|brace| brace.span.start.offset <= offset && offset < brace.span.end.offset)
    }

    fn inside_unquoted_brace_group(&self, word: &Word, target_offset: usize) -> bool {
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

    fn raw_source_hash_starts_comment(source: &str, index: usize) -> bool {
        source[..index]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace)
    }

    pub(super) fn split_compound_array_key_value<'b>(
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

    pub(super) fn parse_compound_array_element(
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

    pub(super) fn parse_array_expr_from_text(
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

    fn scan_compound_array_close(&self, open_paren_span: Span) -> Option<Span> {
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
                        let comment_ch = self.input[cursor.offset..]
                            .chars()
                            .next()
                            .expect("cursor is within bounds while skipping array comment");
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
    pub(super) fn collect_compound_array(
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

    pub(super) fn trim_literal_prefix(
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

    pub(super) fn trim_word_part_prefix(
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

    pub(super) fn split_word_at(&self, word: Word, start: Position) -> Word {
        let value_span = Span::from_positions(start, word.span.end);
        let mut parts = Vec::new();

        for part in word.parts {
            if let Some((kind, span)) = self.trim_word_part_prefix(part.kind, part.span, start) {
                parts.push(WordPartNode::new(kind, span));
            }
        }

        self.word_with_parts(parts, value_span)
    }

    pub(super) fn word_syntax_is_source_backed(&self, word: &Word) -> bool {
        word.span.end.offset <= self.input.len()
            && word
                .parts
                .first()
                .is_none_or(|part| part.span.start == word.span.start)
            && word
                .parts
                .last()
                .is_none_or(|part| part.span.end == word.span.end)
            && word
                .parts
                .iter()
                .all(|part| self.word_part_syntax_is_source_backed(&part.kind, part.span))
    }

    pub(super) fn word_part_syntax_is_source_backed(&self, part: &WordPart, span: Span) -> bool {
        span.end.offset <= self.input.len()
            && match part {
                WordPart::Literal(text) => text.is_source_backed(),
                WordPart::ZshQualifiedGlob(glob) => {
                    glob.segments
                        .iter()
                        .all(Self::zsh_glob_segment_is_source_backed)
                        && glob.qualifiers.as_ref().is_none_or(|group| {
                            self.zsh_glob_qualifier_group_is_source_backed(group)
                        })
                }
                WordPart::SingleQuoted { value, .. } => value.is_source_backed(),
                WordPart::DoubleQuoted { parts, .. } => parts
                    .iter()
                    .all(|part| self.word_part_syntax_is_source_backed(&part.kind, part.span)),
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::PrefixMatch { .. } => true,
                WordPart::ArithmeticExpansion { expression, .. } => expression.is_source_backed(),
                WordPart::Parameter(parameter) => parameter.raw_body.is_source_backed(),
                WordPart::ParameterExpansion {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    reference.is_source_backed()
                        && self.parameter_operator_is_source_backed(operator)
                        && operand.as_ref().is_none_or(SourceText::is_source_backed)
                }
                WordPart::Length(reference)
                | WordPart::ArrayAccess(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::ArrayIndices(reference)
                | WordPart::Transformation { reference, .. } => reference.is_source_backed(),
                WordPart::Substring {
                    reference,
                    offset,
                    length,
                    ..
                }
                | WordPart::ArraySlice {
                    reference,
                    offset,
                    length,
                    ..
                } => {
                    reference.is_source_backed()
                        && offset.is_source_backed()
                        && length.as_ref().is_none_or(SourceText::is_source_backed)
                }
                WordPart::IndirectExpansion {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    reference.is_source_backed()
                        && operator.is_none()
                        && operand.as_ref().is_none_or(SourceText::is_source_backed)
                }
            }
    }

    pub(super) fn parameter_operator_is_source_backed(&self, operator: &ParameterOp) -> bool {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => pattern.is_source_backed(),
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => pattern.is_source_backed() && replacement.is_source_backed(),
            _ => true,
        }
    }

    pub(super) fn zsh_glob_qualifier_group_is_source_backed(
        &self,
        group: &ZshGlobQualifierGroup,
    ) -> bool {
        group
            .fragments
            .iter()
            .all(Self::zsh_glob_qualifier_is_source_backed)
    }

    pub(super) fn zsh_glob_segment_is_source_backed(segment: &ZshGlobSegment) -> bool {
        match segment {
            ZshGlobSegment::Pattern(pattern) => pattern.is_source_backed(),
            ZshGlobSegment::InlineControl(control) => {
                Self::zsh_inline_glob_control_is_source_backed(control)
            }
        }
    }

    pub(super) fn zsh_inline_glob_control_is_source_backed(
        _control: &ZshInlineGlobControl,
    ) -> bool {
        true
    }

    pub(super) fn zsh_glob_qualifier_is_source_backed(fragment: &ZshGlobQualifier) -> bool {
        match fragment {
            ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => true,
            ZshGlobQualifier::LetterSequence { text, .. } => text.is_source_backed(),
            ZshGlobQualifier::NumericArgument { start, end, .. } => {
                start.is_source_backed() && end.as_ref().is_none_or(SourceText::is_source_backed)
            }
        }
    }

    pub(super) fn word_part_syntax_text<'b>(&'b self, part: &'b WordPartNode) -> Cow<'b, str> {
        if self.word_part_syntax_is_source_backed(&part.kind, part.span) {
            Cow::Borrowed(part.span.slice(self.input))
        } else {
            let mut syntax = String::new();
            self.push_word_part_syntax(&mut syntax, &part.kind, part.span);
            Cow::Owned(syntax)
        }
    }

    pub(super) fn compound_array_inner_text<'b>(
        &'b self,
        word: &'b Word,
    ) -> Option<(Cow<'b, str>, Position)> {
        let first = word.parts.first()?;
        let last = word.parts.last()?;
        let first_syntax = self.word_part_syntax_text(first);
        let last_syntax = self.word_part_syntax_text(last);

        if !first_syntax.starts_with('(') || !last_syntax.ends_with(')') {
            return None;
        }

        let inner_start = word.span.start.advanced_by("(");
        if self.word_syntax_is_source_backed(word) {
            let syntax = word.span.slice(self.input);
            return Some((
                Cow::Borrowed(&syntax[1..syntax.len().saturating_sub(1)]),
                inner_start,
            ));
        }

        let mut inner = String::new();
        for (index, part) in word.parts.iter().enumerate() {
            let syntax = self.word_part_syntax_text(part);
            let start = if index == 0 { 1 } else { 0 };
            let end = syntax.len() - usize::from(index + 1 == word.parts.len());
            if start < end {
                inner.push_str(&syntax[start..end]);
            }
        }

        Some((Cow::Owned(inner), inner_start))
    }

    pub(super) fn push_word_part_syntax(&self, out: &mut String, part: &WordPart, span: Span) {
        if self.word_part_syntax_is_source_backed(part, span) {
            out.push_str(span.slice(self.input));
            return;
        }

        match part {
            WordPart::Literal(text) => out.push_str(text.as_str(self.input, span)),
            WordPart::ZshQualifiedGlob(glob) => {
                for segment in &glob.segments {
                    self.push_zsh_glob_segment_syntax(out, segment);
                }
                if let Some(qualifiers) = &glob.qualifiers {
                    self.push_zsh_glob_qualifier_group_syntax(out, qualifiers);
                }
            }
            WordPart::SingleQuoted { value, dollar } => {
                if *dollar {
                    out.push('$');
                }
                out.push('\'');
                out.push_str(value.slice(self.input));
                out.push('\'');
            }
            WordPart::DoubleQuoted { parts, dollar } => {
                if *dollar {
                    out.push('$');
                }
                out.push('"');
                for part in parts {
                    self.push_word_part_syntax(out, &part.kind, part.span);
                }
                out.push('"');
            }
            WordPart::Variable(name) => {
                out.push('$');
                out.push_str(name.as_str());
            }
            WordPart::CommandSubstitution { syntax, .. } => match syntax {
                CommandSubstitutionSyntax::DollarParen => out.push_str("$()"),
                CommandSubstitutionSyntax::Backtick => out.push_str("``"),
            },
            WordPart::ArithmeticExpansion {
                expression, syntax, ..
            } => match syntax {
                ArithmeticExpansionSyntax::DollarParenParen => {
                    out.push_str("$((");
                    out.push_str(expression.slice(self.input));
                    out.push_str("))");
                }
                ArithmeticExpansionSyntax::LegacyBracket => {
                    out.push_str("$[");
                    out.push_str(expression.slice(self.input));
                    out.push(']');
                }
            },
            WordPart::Parameter(parameter) => {
                out.push_str("${");
                out.push_str(parameter.raw_body.slice(self.input));
                out.push('}');
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                colon_variant,
                ..
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                self.push_parameter_operator_syntax(
                    out,
                    operator,
                    operand.as_ref(),
                    *colon_variant,
                );
                out.push('}');
            }
            WordPart::Length(reference) => {
                out.push_str("${#");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayAccess(reference) => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayLength(reference) => {
                out.push_str("${#");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayIndices(reference) => {
                out.push_str("${!");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::Substring {
                reference,
                offset,
                length,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                length,
                ..
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push(':');
                out.push_str(offset.slice(self.input));
                if let Some(length) = length {
                    out.push(':');
                    out.push_str(length.slice(self.input));
                }
                out.push('}');
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                colon_variant,
                ..
            } => {
                out.push_str("${!");
                self.push_var_ref_syntax(out, reference);
                if let Some(operator) = operator {
                    self.push_parameter_operator_syntax(
                        out,
                        operator,
                        operand.as_ref(),
                        *colon_variant,
                    );
                }
                out.push('}');
            }
            WordPart::PrefixMatch { prefix, kind } => {
                out.push_str("${!");
                out.push_str(prefix.as_str());
                out.push(kind.as_char());
                out.push('}');
            }
            WordPart::ProcessSubstitution { is_input, .. } => {
                out.push(if *is_input { '<' } else { '>' });
                out.push_str("()");
            }
            WordPart::Transformation {
                reference,
                operator,
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push('@');
                out.push(*operator);
                out.push('}');
            }
        }
    }

    pub(super) fn push_zsh_glob_qualifier_group_syntax(
        &self,
        out: &mut String,
        group: &ZshGlobQualifierGroup,
    ) {
        match group.kind {
            ZshGlobQualifierKind::Classic => out.push('('),
            ZshGlobQualifierKind::HashQ => out.push_str("(#q"),
        }
        for fragment in &group.fragments {
            match fragment {
                ZshGlobQualifier::Negation { .. } => out.push('^'),
                ZshGlobQualifier::Flag { name, .. } => out.push(*name),
                ZshGlobQualifier::LetterSequence { text, .. } => {
                    out.push_str(text.slice(self.input));
                }
                ZshGlobQualifier::NumericArgument { start, end, .. } => {
                    out.push('[');
                    out.push_str(start.slice(self.input));
                    if let Some(end) = end {
                        out.push(',');
                        out.push_str(end.slice(self.input));
                    }
                    out.push(']');
                }
            }
        }
        out.push(')');
    }

    pub(super) fn push_zsh_glob_segment_syntax(&self, out: &mut String, segment: &ZshGlobSegment) {
        match segment {
            ZshGlobSegment::Pattern(pattern) => self.push_pattern_syntax(out, pattern),
            ZshGlobSegment::InlineControl(control) => match control {
                ZshInlineGlobControl::CaseInsensitive { .. } => out.push_str("(#i)"),
                ZshInlineGlobControl::Backreferences { .. } => out.push_str("(#b)"),
                ZshInlineGlobControl::StartAnchor { .. } => out.push_str("(#s)"),
                ZshInlineGlobControl::EndAnchor { .. } => out.push_str("(#e)"),
            },
        }
    }

    pub(super) fn push_var_ref_syntax(&self, out: &mut String, reference: &VarRef) {
        out.push_str(reference.name.as_str());
        if let Some(subscript) = &reference.subscript {
            out.push('[');
            out.push_str(subscript.syntax_text(self.input));
            out.push(']');
        }
    }

    pub(super) fn push_parameter_operator_syntax(
        &self,
        out: &mut String,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        colon_variant: bool,
    ) {
        let colon = if colon_variant { ":" } else { "" };
        match operator {
            ParameterOp::UseDefault => {
                out.push_str(colon);
                out.push('-');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::AssignDefault => {
                out.push_str(colon);
                out.push('=');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::UseReplacement => {
                out.push_str(colon);
                out.push('+');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::Error => {
                out.push_str(colon);
                out.push('?');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::RemovePrefixShort { pattern } => {
                out.push('#');
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemovePrefixLong { pattern } => {
                out.push_str("##");
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemoveSuffixShort { pattern } => {
                out.push('%');
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemoveSuffixLong { pattern } => {
                out.push_str("%%");
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            } => {
                out.push('/');
                self.push_pattern_syntax(out, pattern);
                out.push('/');
                out.push_str(replacement.slice(self.input));
            }
            ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => {
                out.push_str("//");
                self.push_pattern_syntax(out, pattern);
                out.push('/');
                out.push_str(replacement.slice(self.input));
            }
            ParameterOp::UpperFirst => out.push('^'),
            ParameterOp::UpperAll => out.push_str("^^"),
            ParameterOp::LowerFirst => out.push(','),
            ParameterOp::LowerAll => out.push_str(",,"),
        }
    }

    pub(super) fn push_pattern_syntax(&self, out: &mut String, pattern: &Pattern) {
        if pattern.is_source_backed() && pattern.span.end.offset <= self.input.len() {
            out.push_str(pattern.span.slice(self.input));
            return;
        }

        for part in &pattern.parts {
            self.push_pattern_part_syntax(out, &part.kind, part.span);
        }
    }

    pub(super) fn push_pattern_part_syntax(
        &self,
        out: &mut String,
        part: &PatternPart,
        span: Span,
    ) {
        match part {
            PatternPart::Literal(text) => out.push_str(text.as_str(self.input, span)),
            PatternPart::AnyString => out.push('*'),
            PatternPart::AnyChar => out.push('?'),
            PatternPart::CharClass(text) => out.push_str(text.slice(self.input)),
            PatternPart::Group { kind, patterns } => {
                out.push(kind.prefix());
                out.push('(');
                for (index, pattern) in patterns.iter().enumerate() {
                    if index > 0 {
                        out.push('|');
                    }
                    self.push_pattern_syntax(out, pattern);
                }
                out.push(')');
            }
            PatternPart::Word(word) => {
                for part in &word.parts {
                    self.push_word_part_syntax(out, &part.kind, part.span);
                }
            }
        }
    }

    pub(super) fn parse_assignment_from_word(
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

    pub(super) fn parse_assignment_from_text(
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
        for (offset, ch) in first_text.char_indices() {
            if (offset == 0 && (ch.is_ascii_alphabetic() || ch == '_'))
                || (offset > 0 && (ch.is_ascii_alphanumeric() || ch == '_'))
            {
                name_end = offset + ch.len_utf8();
            } else {
                break;
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

    pub(super) fn build_target_subscript(
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

    fn zsh_parameter_requires_fallback(
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

    fn zsh_numeric_parameter_requires_fallback(
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

    fn zsh_parameter_suffix_looks_like_modifier(
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

    fn zsh_modifier_suffix_candidate_chars(
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

    fn prefixed_parameter_raw_body(
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

    pub(super) fn parse_var_ref_from_word(
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

    pub(super) fn is_valid_identifier(name: &str) -> bool {
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

    pub(super) fn is_literal_flag_text(text: &str) -> bool {
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

    pub(super) fn classify_decl_operand(
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

    pub(super) fn explicit_array_kind_from_flag_text(text: &str) -> Option<ArrayKind> {
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

    pub(super) fn classify_decl_operands(&mut self, words: Vec<Word>) -> Vec<DeclOperand> {
        let mut explicit_array_kind = None;
        let mut operands = Vec::with_capacity(words.len());

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
    pub(super) fn try_parse_assignment_with_shape(
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
    pub(super) fn try_parse_compound_array_arg(
        &mut self,
        saved_w: String,
        saved_span: Span,
    ) -> Option<Word> {
        if !self.at(TokenKind::LeftParen) {
            return None;
        }

        let open_paren_span = self.current_span;
        if let Some(closing_span) = self.scan_compound_array_close(open_paren_span) {
            let paren_text = &self.input[open_paren_span.start.offset..closing_span.end.offset];
            let mut compound = saved_w;
            compound.push_str(paren_text);
            while self.current_token.is_some()
                && self.current_span.start.offset < closing_span.end.offset
            {
                self.advance();
            }
            let span = saved_span.merge(closing_span);
            return Some(self.word_from_raw_text(&compound, span));
        }

        self.advance(); // consume '('
        let mut compound = saved_w;
        let mut closing_span = Span::new();
        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    let elem = self.current_source_like_word_text().unwrap();
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
            return Some(self.decode_word_text(source, span, saved_span.start, true));
        }

        Some(self.decode_word_text(&compound, span, saved_span.start, false))
    }

    /// Parse a heredoc redirect (`<<` or `<<-`) and any trailing redirects on the same line.
    pub(super) fn expect_word(&mut self) -> Result<Word> {
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
                        Some(TokenKind::LeftParen) => {
                            depth += 1;
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

    fn decode_word_parts_into_with_escape_mode(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
        parts: &mut Vec<WordPartNode>,
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

    pub(super) fn decode_word_parts_into_with_quote_fragments(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        options: DecodeWordPartsOptions,
        parts: &mut Vec<WordPartNode>,
    ) {
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
                    if literal_ch == '$' && chars.peek() == Some(&'{') {
                        self.consume_escaped_braced_parameter_literal(
                            &mut chars,
                            &mut cursor,
                            &mut current,
                        );
                    }
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
                if literal_ch == '$' && chars.peek() == Some(&'{') {
                    self.consume_escaped_braced_parameter_literal(
                        &mut chars,
                        &mut cursor,
                        &mut current,
                    );
                }
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

                    match c {
                        '\\' => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                            escaped = true;
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        _ => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                        }
                    }
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
                            // `$'...'` and `$"..."` are literal inside ordinary
                            // double quotes, so nested decoding must not
                            // reactivate dollar-quote parsing here.
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
                    let body_start = inner_start.advanced_by("(");
                    let had_prefix = current_start != part_start;
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
                            let body_start =
                                if inner_text.chars().next().is_some_and(char::is_whitespace) {
                                    inner_start
                                } else {
                                    body_start
                                };
                            if had_prefix {
                                self.nested_stmt_seq_from_source(inner_text, body_start)
                            } else {
                                let inner_end = inner_start.advanced_by(inner_text);
                                self.nested_stmt_seq_from_current_input(inner_start, inner_end)
                            }
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
                            if had_prefix {
                                self.nested_stmt_seq_from_source(&cmd_str, body_start)
                            } else {
                                self.nested_stmt_seq_from_current_input(
                                    inner_start,
                                    inner_start.advanced_by(&cmd_str),
                                )
                            }
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
                        if had_prefix {
                            self.nested_stmt_seq_from_source(&cmd_str, body_start)
                        } else {
                            self.nested_stmt_seq_from_current_input(
                                inner_start,
                                inner_start.advanced_by(&cmd_str),
                            )
                        }
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
                            .as_ref()
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
                    let var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
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
                                .as_ref()
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
                                                && self.input[..cursor.offset].ends_with('}'),
                                        )
                                    } else {
                                        (self.empty_source_text(cursor), false)
                                    };
                                if !consumed_closing_brace {
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                }
                                if !Span::from_positions(part_start, cursor)
                                    .slice(self.input)
                                    .ends_with('}')
                                    && self.input[cursor.offset..].starts_with('}')
                                {
                                    cursor.advance('}');
                                }
                                let operator = if replace_all {
                                    ParameterOp::ReplaceAll {
                                        pattern,
                                        replacement_word_ast: self
                                            .parse_source_text_as_word(&replacement),
                                        replacement,
                                    }
                                } else {
                                    ParameterOp::ReplaceFirst {
                                        pattern,
                                        replacement_word_ast: self
                                            .parse_source_text_as_word(&replacement),
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
                                            && self.input[..cursor.offset].ends_with('}'),
                                    )
                                } else {
                                    (self.empty_source_text(cursor), false)
                                };
                            if !consumed_closing_brace {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            }
                            if !Span::from_positions(part_start, cursor)
                                .slice(self.input)
                                .ends_with('}')
                                && self.input[cursor.offset..].starts_with('}')
                            {
                                cursor.advance('}');
                            }
                            let operator = if replace_all {
                                ParameterOp::ReplaceAll {
                                    pattern,
                                    replacement_word_ast: self
                                        .parse_source_text_as_word(&replacement),
                                    replacement,
                                }
                            } else {
                                ParameterOp::ReplaceFirst {
                                    pattern,
                                    replacement_word_ast: self
                                        .parse_source_text_as_word(&replacement),
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

                let part = if let Some(c) = chars.peek().copied() {
                    match c {
                        ':' => {
                            if self.zsh_parameter_suffix_looks_like_modifier(&mut chars) {
                                let tail =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                                let raw_body = self.prefixed_parameter_raw_body(
                                    &var_name,
                                    brace_body_start,
                                    tail,
                                    source_backed,
                                );
                                let parameter =
                                    self.zsh_parameter_word_part(raw_body, part_start, cursor);
                                Self::push_word_part(parts, parameter, part_start, cursor);
                                current_start = cursor;
                                continue;
                            }

                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match chars.peek() {
                                Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?') => {
                                    let op_char =
                                        Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                    let operand = self.read_brace_operand(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    );
                                    let operator = match op_char {
                                        '-' => ParameterOp::UseDefault,
                                        '=' => ParameterOp::AssignDefault,
                                        '+' => ParameterOp::UseReplacement,
                                        '?' => ParameterOp::Error,
                                        _ => unreachable!(),
                                    };
                                    self.parameter_expansion_word_part(
                                        self.parameter_var_ref(
                                            part_start, "${", &var_name, None, cursor,
                                        ),
                                        operator,
                                        Some(operand),
                                        true,
                                    )
                                }
                                _ => {
                                    let (offset, length) = self.read_parameter_slice_parts(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    );
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                    self.substring_word_part(
                                        self.parameter_var_ref(
                                            part_start, "${", &var_name, None, cursor,
                                        ),
                                        offset,
                                        length,
                                    )
                                }
                            }
                        }
                        '-' | '=' | '+' | '?' => {
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
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                Some(operand),
                                false,
                            )
                        }
                        '#' => {
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
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                None,
                                false,
                            )
                        }
                        '%' => {
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
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                None,
                                false,
                            )
                        }
                        '/' => {
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
                                            && self.input[..cursor.offset].ends_with('}'),
                                    )
                                } else {
                                    (self.empty_source_text(cursor), false)
                                };
                            if !consumed_closing_brace {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            }
                            if !Span::from_positions(part_start, cursor)
                                .slice(self.input)
                                .ends_with('}')
                                && self.input[cursor.offset..].starts_with('}')
                            {
                                cursor.advance('}');
                            }
                            let operator = if replace_all {
                                ParameterOp::ReplaceAll {
                                    pattern,
                                    replacement_word_ast: self
                                        .parse_source_text_as_word(&replacement),
                                    replacement,
                                }
                            } else {
                                ParameterOp::ReplaceFirst {
                                    pattern,
                                    replacement_word_ast: self
                                        .parse_source_text_as_word(&replacement),
                                    replacement,
                                }
                            };
                            self.parameter_expansion_word_part(
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                None,
                                false,
                            )
                        }
                        '^' => {
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
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                operand,
                                false,
                            )
                        }
                        ',' => {
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
                                self.parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                operand,
                                false,
                            )
                        }
                        '@' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            if chars.peek().is_some() {
                                let operator = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Transformation {
                                    reference: self.parameter_var_ref(
                                        part_start, "${", &var_name, None, cursor,
                                    ),
                                    operator,
                                }
                            } else {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Variable(var_name.into())
                            }
                        }
                        '}' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            WordPart::Variable(var_name.into())
                        }
                        _ => {
                            while let Some(&next) = chars.peek() {
                                let consumed = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                if next == '}' || consumed == '}' {
                                    break;
                                }
                            }
                            WordPart::Variable(var_name.into())
                        }
                    }
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
                    let var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                        c.is_ascii_alphanumeric() || c == '_'
                    });
                    if !var_name.is_empty() {
                        Self::push_word_part(
                            parts,
                            WordPart::Variable(var_name.into()),
                            part_start,
                            cursor,
                        );
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

    fn consume_escaped_braced_parameter_literal(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        current: &mut String,
    ) {
        if chars.peek() != Some(&'{') {
            return;
        }

        current.push(Self::next_word_char_unwrap(chars, cursor));

        let mut depth = 1usize;
        let mut literal_brace_depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;

        while let Some(&c) = chars.peek() {
            match c {
                '\\' if !in_single => {
                    Self::next_word_char_unwrap(chars, cursor);
                    if let Some(&escaped) = chars.peek() {
                        if !in_double || matches!(escaped, '$' | '"' | '\\' | '`' | '\n') {
                            current.push(Self::next_word_char_unwrap(chars, cursor));
                        } else {
                            current.push('\\');
                        }
                    } else {
                        current.push('\\');
                    }
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    current.push(Self::next_word_char_unwrap(chars, cursor));
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    double_quote_depth = if in_double { depth } else { 0 };
                    current.push(Self::next_word_char_unwrap(chars, cursor));
                }
                '$' if !in_single => {
                    current.push(Self::next_word_char_unwrap(chars, cursor));
                    if chars.peek() == Some(&'{') {
                        depth += 1;
                        current.push(Self::next_word_char_unwrap(chars, cursor));
                    }
                }
                '{' if !in_single && !in_double => {
                    literal_brace_depth += 1;
                    current.push(Self::next_word_char_unwrap(chars, cursor));
                }
                '}' if !in_single && (!in_double || depth > double_quote_depth) => {
                    if depth == 1 && literal_brace_depth > 0 {
                        let mut remaining = chars.clone();
                        remaining.next();
                        if Self::brace_operand_has_later_top_level_closer(remaining, depth) {
                            literal_brace_depth -= 1;
                            current.push(Self::next_word_char_unwrap(chars, cursor));
                            continue;
                        }
                    }

                    current.push(Self::next_word_char_unwrap(chars, cursor));
                    if depth == 1 {
                        break;
                    }
                    depth -= 1;
                }
                _ => current.push(Self::next_word_char_unwrap(chars, cursor)),
            }
        }
    }

    pub(super) fn read_array_index(
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

    pub(super) fn read_replacement_pattern(
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

    pub(super) fn decode_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_with_escape_mode(s, span, base, source_backed, source_backed)
    }

    fn decode_word_text_with_escape_mode(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
    ) -> Word {
        let mut parts = Vec::new();
        self.decode_word_parts_into_with_escape_mode(
            s,
            base,
            source_backed,
            preserve_escaped_expansion_literals,
            &mut parts,
        );
        self.word_with_parts(parts, span)
    }

    fn decode_word_text_with_options(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        options: DecodeWordPartsOptions,
    ) -> Word {
        let mut parts = Vec::new();
        self.decode_word_parts_into_with_quote_fragments(
            s,
            base,
            source_backed,
            options,
            &mut parts,
        );
        self.word_with_parts(parts, span)
    }

    pub(super) fn decode_fragment_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_fragment_word_text_with_escape_mode(s, span, base, source_backed, source_backed)
    }

    fn decode_fragment_word_text_with_escape_mode(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
        preserve_escaped_expansion_literals: bool,
    ) -> Word {
        let mut parts = Vec::new();
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
        self.word_with_parts(parts, span)
    }

    pub(super) fn decode_quoted_segment_text(
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
                // Double-quoted segment contents treat `$'...'` and `$"..."`
                // as literal text, not nested quote forms.
                parse_dollar_quotes: false,
                parse_process_substitutions: false,
                ..DecodeWordPartsOptions::default()
            },
        )
    }

    pub(super) fn decode_heredoc_body_text(
        &mut self,
        s: &str,
        span: Span,
        source_backed: bool,
    ) -> HeredocBody {
        let mut parts = Vec::new();
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
            .map(Self::heredoc_body_part_from_word_part_node)
            .collect();
        self.heredoc_body_with_parts(parts, span, HeredocBodyMode::Expanding, source_backed)
    }

    pub(super) fn parse_word_with_context(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_preserving_quotes_if_needed(s, span, base, source_backed)
    }

    fn arithmetic_expansion_word_part(
        &self,
        expression: SourceText,
        syntax: ArithmeticExpansionSyntax,
    ) -> WordPart {
        WordPart::ArithmeticExpansion {
            expression_ast: self.parse_source_text_as_arithmetic(&expression).ok(),
            expression_word_ast: self.parse_source_text_as_word(&expression),
            expression,
            syntax,
        }
    }

    fn parameter_expansion_word_part(
        &self,
        reference: VarRef,
        operator: ParameterOp,
        operand: Option<SourceText>,
        colon_variant: bool,
    ) -> WordPart {
        let operand_word_ast = self.parse_optional_source_text_as_word(operand.as_ref());
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            colon_variant,
        }
    }

    fn indirect_expansion_word_part(
        &self,
        reference: VarRef,
        operator: Option<ParameterOp>,
        operand: Option<SourceText>,
        colon_variant: bool,
    ) -> WordPart {
        let operand_word_ast = self.parse_optional_source_text_as_word(operand.as_ref());
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            colon_variant,
        }
    }

    fn substring_word_part(
        &self,
        reference: VarRef,
        offset: SourceText,
        length: Option<SourceText>,
    ) -> WordPart {
        let offset_ast = self.maybe_parse_source_text_as_arithmetic(&offset);
        let offset_word_ast = self.parse_source_text_as_word(&offset);
        let length_ast = length
            .as_ref()
            .and_then(|length| self.maybe_parse_source_text_as_arithmetic(length));
        let length_word_ast = self.parse_optional_source_text_as_word(length.as_ref());
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

    fn array_slice_word_part(
        &self,
        reference: VarRef,
        offset: SourceText,
        length: Option<SourceText>,
    ) -> WordPart {
        let offset_ast = self.maybe_parse_source_text_as_arithmetic(&offset);
        let offset_word_ast = self.parse_source_text_as_word(&offset);
        let length_ast = length
            .as_ref()
            .and_then(|length| self.maybe_parse_source_text_as_arithmetic(length));
        let length_word_ast = self.parse_optional_source_text_as_word(length.as_ref());
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

    fn read_parameter_slice_parts(
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
    pub(super) fn read_brace_operand(
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
        let mut operand = (!source_backed).then(String::new);

        while let Some(&c) = chars.peek() {
            if escaped {
                let ch = Self::next_word_char_unwrap(chars, cursor);
                if let Some(operand) = operand.as_mut() {
                    operand.push(ch);
                }
                escaped = false;
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
                        return if source_backed {
                            SourceText::source(Span::from_positions(start, end))
                        } else {
                            self.source_text(operand.unwrap_or_default(), start, end)
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
        if source_backed {
            SourceText::source(Span::from_positions(start, *cursor))
        } else {
            self.source_text(operand.unwrap_or_default(), start, *cursor)
        }
    }

    fn brace_operand_has_later_top_level_closer(
        mut chars: std::iter::Peekable<std::str::Chars<'_>>,
        target_depth: usize,
    ) -> bool {
        let mut depth = target_depth;
        let mut in_single = false;
        let mut in_double = false;
        let mut double_quote_depth = 0usize;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
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
