use super::*;
use shuck_ast::ArrayValueWord;
use smallvec::SmallVec;

mod array;
mod assignment;
mod decode;
mod expansion;
mod pattern;

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

struct PatternParser<'a> {
    input: &'a str,
    segments: Vec<PatternSegment<'a>>,
    full_span: Span,
    features: ZshGlobParseFeatures,
    allow_prefixed_bare_groups_without_ksh: bool,
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
    ambient_double_quotes: bool,
    parse_dollar_quotes: bool,
    preserve_escaped_expansion_literals: bool,
    parse_process_substitutions: bool,
}

impl Default for DecodeWordPartsOptions {
    fn default() -> Self {
        Self {
            preserve_quote_fragments: false,
            ambient_double_quotes: false,
            parse_dollar_quotes: false,
            preserve_escaped_expansion_literals: false,
            parse_process_substitutions: true,
        }
    }
}

impl<'a> PatternParser<'a> {
    const MAX_PATTERN_GROUP_DEPTH: usize = 8;
    const MAX_ZSH_CASE_GROUP_PRESCAN_BYTES: usize = 512;

    fn new(input: &'a str, word: &'a Word, features: ZshGlobParseFeatures) -> Self {
        Self::from_word_parts_with_options(input, &word.parts, word.span, features, false)
    }

    fn for_pattern_context(input: &'a str, word: &'a Word, features: ZshGlobParseFeatures) -> Self {
        Self::from_word_parts_with_options(input, &word.parts, word.span, features, true)
    }

    fn from_word_parts(
        input: &'a str,
        parts: &'a [WordPartNode],
        full_span: Span,
        features: ZshGlobParseFeatures,
    ) -> Self {
        Self::from_word_parts_with_options(input, parts, full_span, features, false)
    }

    fn from_word_parts_with_options(
        input: &'a str,
        parts: &'a [WordPartNode],
        full_span: Span,
        features: ZshGlobParseFeatures,
        allow_prefixed_bare_groups_without_ksh: bool,
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
            features,
            allow_prefixed_bare_groups_without_ksh,
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
        let mut pattern = self.parse_until(&mut cursor, false, 0);
        pattern.span = self.full_span;
        pattern
    }

    fn parse_until(
        &self,
        cursor: &mut PatternCursor,
        stop_at_group_delim: bool,
        group_depth: usize,
    ) -> Pattern {
        let start = cursor.position;
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut literal_start: Option<Position> = None;
        let mut literal_end = start;
        let allow_groups = group_depth < Self::MAX_PATTERN_GROUP_DEPTH;
        let mut unparsed_group_depth = 0usize;
        let mut pending_unparsed_group_open = false;

        while let Some(segment) = self.segments.get(cursor.segment_index) {
            if stop_at_group_delim
                && unparsed_group_depth == 0
                && self.peek_group_delimiter(*cursor).is_some()
            {
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

                    if allow_groups
                        && let Some((group, next_cursor)) =
                            self.try_parse_group(*cursor, group_depth)
                    {
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

                    if allow_groups
                        && let Some((group, next_cursor)) =
                            self.try_parse_bare_zsh_group(*cursor, group_depth)
                    {
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

                    let starts_unparsed_group = !allow_groups
                        && stop_at_group_delim
                        && unparsed_group_depth == 0
                        && self.starts_unparsed_pattern_group(*cursor);

                    if !starts_unparsed_group
                        && let Some((char_class, next_cursor)) = self.try_parse_char_class(*cursor)
                    {
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

                    if !starts_unparsed_group
                        && let Some((wildcard, next_cursor)) = self.try_parse_wildcard(*cursor)
                    {
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

                    let was_escaped = self.is_escaped(*cursor);
                    let Some((ch, span)) = self.consume_literal_char(cursor) else {
                        break;
                    };
                    if literal_start.is_none() {
                        literal_start = Some(span.start);
                    }
                    literal_end = span.end;
                    literal.push(ch);
                    if !allow_groups && stop_at_group_delim && !was_escaped {
                        if pending_unparsed_group_open && ch == '(' {
                            unparsed_group_depth = unparsed_group_depth.saturating_add(1);
                            pending_unparsed_group_open = false;
                        } else if unparsed_group_depth > 0 {
                            match ch {
                                '(' => {
                                    unparsed_group_depth = unparsed_group_depth.saturating_add(1)
                                }
                                ')' => {
                                    unparsed_group_depth = unparsed_group_depth.saturating_sub(1)
                                }
                                _ => {}
                            }
                        } else if starts_unparsed_group {
                            if ch == '(' {
                                unparsed_group_depth = unparsed_group_depth.saturating_add(1);
                            } else {
                                pending_unparsed_group_open = true;
                            }
                        }
                    }
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

    fn try_parse_bare_zsh_group(
        &self,
        cursor: PatternCursor,
        group_depth: usize,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        if !self.features.bare_groups {
            return None;
        }

        let opener = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || opener != '(' {
            return None;
        }
        if self.prefix_reserves_group_for_ksh(cursor) {
            return None;
        }
        if !self.has_bare_zsh_group_separator(cursor) {
            return None;
        }

        let start = cursor.position;
        let mut next_cursor = cursor;
        self.consume_literal_char(&mut next_cursor)?;

        let mut patterns = vec![self.parse_until(&mut next_cursor, true, group_depth + 1)];
        if self.peek_group_delimiter(next_cursor) != Some('|') {
            return None;
        }

        loop {
            if self.peek_group_delimiter(next_cursor) == Some('|') {
                self.consume_literal_char(&mut next_cursor)?;
                patterns.push(self.parse_until(&mut next_cursor, true, group_depth + 1));
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

    fn has_bare_zsh_group_separator(&self, cursor: PatternCursor) -> bool {
        let mut escaped = false;
        let mut paren_depth = 0usize;
        let mut scanned = 0usize;
        for (index, segment) in self.segments.iter().enumerate().skip(cursor.segment_index) {
            match segment {
                PatternSegment::Literal { text, .. } => {
                    let offset = if index == cursor.segment_index {
                        cursor.literal_offset + '('.len_utf8()
                    } else {
                        0
                    };
                    let Some(rest) = text.get(offset..) else {
                        return false;
                    };

                    for ch in rest.chars() {
                        scanned += ch.len_utf8();
                        if scanned > Self::MAX_ZSH_CASE_GROUP_PRESCAN_BYTES {
                            return true;
                        }

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
                }
                PatternSegment::Word(part) => {
                    scanned += part.span.end.offset.saturating_sub(part.span.start.offset);
                    if scanned > Self::MAX_ZSH_CASE_GROUP_PRESCAN_BYTES {
                        return true;
                    }
                    escaped = false;
                }
            }
        }

        false
    }

    fn starts_unparsed_pattern_group(&self, cursor: PatternCursor) -> bool {
        if self.is_escaped(cursor) {
            return false;
        }

        let Some(ch) = self.peek_literal_char(cursor) else {
            return false;
        };

        if self.features.bare_groups
            && ch == '('
            && !self.prefix_reserves_group_for_ksh(cursor)
            && self.has_bare_zsh_group_separator(cursor)
        {
            return true;
        }

        if !self.features.ksh_groups || !matches!(ch, '?' | '*' | '+' | '@' | '!') {
            return false;
        }

        let Some(PatternSegment::Literal { text, .. }) = self.segments.get(cursor.segment_index)
        else {
            return false;
        };
        let next_offset = cursor.literal_offset + ch.len_utf8();
        text.get(next_offset..)
            .is_some_and(|rest| rest.starts_with('('))
    }

    fn prefix_reserves_group_for_ksh(&self, cursor: PatternCursor) -> bool {
        self.is_immediately_preceded_by_ksh_group_operator(cursor)
            && (self.features.ksh_groups || !self.allow_prefixed_bare_groups_without_ksh)
    }

    fn is_immediately_preceded_by_ksh_group_operator(&self, cursor: PatternCursor) -> bool {
        let Some(PatternSegment::Literal { text, .. }) = self.segments.get(cursor.segment_index)
        else {
            return false;
        };
        if cursor.literal_offset == 0 {
            return false;
        }

        text[..cursor.literal_offset]
            .chars()
            .next_back()
            .is_some_and(|ch| matches!(ch, '?' | '*' | '+' | '@' | '!'))
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

    fn try_parse_group(
        &self,
        cursor: PatternCursor,
        group_depth: usize,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        let opener = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor)
            || !self.features.ksh_groups
            || !matches!(opener, '?' | '*' | '+' | '@' | '!')
        {
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
            patterns.push(self.parse_until(&mut next_cursor, true, group_depth + 1));
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
        let Some(PatternSegment::Literal { text, .. }) = self.segments.get(cursor.segment_index)
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

fn word_part_has_zsh_syntax_comma(part: &WordPart, comma_offset: usize) -> bool {
    match part {
        WordPart::ZshQualifiedGlob(glob) => glob
            .qualifiers
            .as_ref()
            .is_some_and(|qualifiers| span_contains_offset(qualifiers.span, comma_offset)),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter()
            .any(|part| word_part_has_zsh_syntax_comma(&part.kind, comma_offset)),
        WordPart::Parameter(expansion) => {
            parameter_expansion_has_zsh_syntax_comma(expansion, comma_offset)
        }
        WordPart::ParameterExpansion {
            reference,
            operand_word_ast,
            ..
        }
        | WordPart::IndirectExpansion {
            reference,
            operand_word_ast,
            ..
        } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
                || operand_word_ast
                    .as_deref()
                    .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
        }
        WordPart::Substring {
            reference,
            offset_word_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset_word_ast,
            length_word_ast,
            ..
        } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
                || word_has_zsh_syntax_comma(offset_word_ast, comma_offset)
                || length_word_ast
                    .as_deref()
                    .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
        }
        WordPart::ArithmeticExpansion {
            expression_word_ast,
            ..
        } => word_has_zsh_syntax_comma(expression_word_ast, comma_offset),
        _ => false,
    }
}

fn parameter_expansion_has_zsh_syntax_comma(
    expansion: &ParameterExpansion,
    comma_offset: usize,
) -> bool {
    match &expansion.syntax {
        ParameterExpansionSyntax::Bourne(expansion) => {
            bourne_parameter_expansion_has_zsh_syntax_comma(expansion, comma_offset)
        }
        ParameterExpansionSyntax::Zsh(expansion) => {
            zsh_parameter_expansion_has_zsh_syntax_comma(expansion, comma_offset)
        }
    }
}

fn bourne_parameter_expansion_has_zsh_syntax_comma(
    expansion: &BourneParameterExpansion,
    comma_offset: usize,
) -> bool {
    match expansion {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Length { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
        }
        BourneParameterExpansion::Indirect {
            reference,
            operand_word_ast,
            ..
        }
        | BourneParameterExpansion::Operation {
            reference,
            operand_word_ast,
            ..
        } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
                || operand_word_ast
                    .as_deref()
                    .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
        }
        BourneParameterExpansion::Slice {
            reference,
            offset_word_ast,
            length_word_ast,
            ..
        } => {
            var_ref_subscript_contains_offset(reference, comma_offset)
                || word_has_zsh_syntax_comma(offset_word_ast, comma_offset)
                || length_word_ast
                    .as_deref()
                    .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
        }
        BourneParameterExpansion::PrefixMatch { .. } => false,
    }
}

fn zsh_parameter_expansion_has_zsh_syntax_comma(
    expansion: &ZshParameterExpansion,
    comma_offset: usize,
) -> bool {
    zsh_expansion_target_has_zsh_syntax_comma(&expansion.target, comma_offset)
        || expansion.modifiers.iter().any(|modifier| {
            modifier
                .argument_word_ast()
                .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
        })
        || expansion
            .operation
            .as_ref()
            .is_some_and(|operation| zsh_operation_has_zsh_syntax_comma(operation, comma_offset))
}

fn zsh_expansion_target_has_zsh_syntax_comma(
    target: &ZshExpansionTarget,
    comma_offset: usize,
) -> bool {
    match target {
        ZshExpansionTarget::Reference(reference) => {
            var_ref_subscript_contains_offset(reference, comma_offset)
        }
        ZshExpansionTarget::Nested(expansion) => {
            parameter_expansion_has_zsh_syntax_comma(expansion, comma_offset)
        }
        ZshExpansionTarget::Word(word) => word_has_zsh_syntax_comma(word, comma_offset),
        ZshExpansionTarget::Empty => false,
    }
}

fn zsh_operation_has_zsh_syntax_comma(
    operation: &ZshExpansionOperation,
    comma_offset: usize,
) -> bool {
    operation
        .operand_word_ast()
        .is_some_and(|word| word_has_zsh_syntax_comma(word, comma_offset))
}

fn word_has_zsh_syntax_comma(word: &Word, comma_offset: usize) -> bool {
    word.parts
        .iter()
        .any(|part| word_part_has_zsh_syntax_comma(&part.kind, comma_offset))
}

fn var_ref_subscript_contains_offset(reference: &VarRef, offset: usize) -> bool {
    reference.subscript.as_deref().is_some_and(|subscript| {
        span_contains_offset(subscript.syntax_source_text().span(), offset)
    })
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    span.start.offset <= offset && offset < span.end.offset
}

#[inline]
fn try_pure_literal_end_position(
    bytes: &[u8],
    base: Position,
    options: DecodeWordPartsOptions,
) -> Option<Position> {
    let backslash_special =
        options.preserve_quote_fragments || options.preserve_escaped_expansion_literals;
    let quote_special = options.preserve_quote_fragments;
    let proc_subst_special = options.parse_process_substitutions;

    let mut newline_count: usize = 0;
    let mut last_newline: Option<usize> = None;

    for (i, &byte) in bytes.iter().enumerate() {
        if byte >= 0x80 {
            return None;
        }
        match byte {
            0 | b'$' | b'`' => return None,
            b'\\' if backslash_special => return None,
            b'\'' | b'"' if quote_special => return None,
            b'<' | b'>' if proc_subst_special => return None,
            b'\n' => {
                newline_count += 1;
                last_newline = Some(i);
            }
            _ => {}
        }
    }

    let len = bytes.len();
    Some(Position {
        line: base.line + newline_count,
        column: match last_newline {
            Some(idx) => len - idx,
            None => base.column + len,
        },
        offset: base.offset + len,
    })
}

fn source_prefix_ends_inside_double_quotes(prefix: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in prefix.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    in_double
}

fn source_prefix_has_same_line_escaped_double_quote_fragment(
    prefix: &str,
    ambient_double_quotes: bool,
) -> bool {
    let line = prefix.rsplit('\n').next().unwrap_or(prefix);
    let mut chars = line.trim_end_matches('\r').chars().peekable();
    let mut in_single = false;
    let mut in_double = ambient_double_quotes;

    while let Some(ch) = chars.next() {
        match ch {
            '\\' if !in_single && in_double && chars.peek() == Some(&'"') => return true,
            '\\' if !in_single => {}
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    false
}

fn floor_char_boundary(source: &str, mut offset: usize) -> usize {
    offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn ceil_char_boundary(source: &str, mut offset: usize) -> usize {
    offset = offset.min(source.len());
    while offset < source.len() && !source.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}
