use super::super::*;

impl<'a> Parser<'a> {
    pub(in crate::parser) fn parameter_word_part_from_legacy(
        &self,
        part: WordPart,
        part_start: Position,
        part_end: Position,
        source_backed: bool,
    ) -> WordPart {
        let span = Span::from_positions(part_start, part_end);
        let raw_body = self.parameter_raw_body_from_legacy(&part, span, source_backed);
        let raw_body_text = raw_body.slice(self.input).to_string();

        let syntax = match part {
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => Some(BourneParameterExpansion::Operation {
                reference,
                operator: self.enrich_parameter_operator(operator),
                operand,
                operand_word_ast,
                colon_variant,
            }),
            WordPart::Length(reference) | WordPart::ArrayLength(reference) => {
                Some(BourneParameterExpansion::Length { reference })
            }
            WordPart::ArrayAccess(reference) => {
                Some(BourneParameterExpansion::Access { reference })
            }
            WordPart::ArrayIndices(reference) => {
                Some(BourneParameterExpansion::Indices { reference })
            }
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => Some(BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            }),
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => Some(BourneParameterExpansion::Indirect {
                reference,
                operator: operator.map(|operator| self.enrich_parameter_operator(operator)),
                operand,
                operand_word_ast,
                colon_variant,
            }),
            WordPart::PrefixMatch { prefix, kind } => {
                Some(BourneParameterExpansion::PrefixMatch { prefix, kind })
            }
            WordPart::Transformation {
                reference,
                operator,
            } => Some(BourneParameterExpansion::Transformation {
                reference,
                operator,
            }),
            WordPart::Variable(name) if raw_body_text == name.as_str() => {
                Some(BourneParameterExpansion::Access {
                    reference: self.parameter_var_ref(
                        part_start,
                        "${",
                        name.as_str(),
                        None,
                        part_end,
                    ),
                })
            }
            other => return other,
        };

        let Some(syntax) = syntax else {
            unreachable!("matched Some above");
        };
        WordPart::Parameter(Box::new(ParameterExpansion {
            syntax: ParameterExpansionSyntax::Bourne(syntax),
            span,
            raw_body,
        }))
    }

    pub(in crate::parser) fn enrich_parameter_operator(
        &self,
        operator: Box<ParameterOp>,
    ) -> Box<ParameterOp> {
        match *operator {
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            } => Box::new(ParameterOp::ReplaceFirst {
                pattern,
                replacement_word_ast: Box::new(self.parse_source_text_as_word(&replacement)),
                replacement,
            }),
            ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => Box::new(ParameterOp::ReplaceAll {
                pattern,
                replacement_word_ast: Box::new(self.parse_source_text_as_word(&replacement)),
                replacement,
            }),
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::RemovePrefixShort { .. }
            | ParameterOp::RemovePrefixLong { .. }
            | ParameterOp::RemoveSuffixShort { .. }
            | ParameterOp::RemoveSuffixLong { .. }
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => operator,
        }
    }

    pub(in crate::parser) fn parameter_raw_body_from_legacy(
        &self,
        part: &WordPart,
        span: Span,
        source_backed: bool,
    ) -> SourceText {
        if source_backed && span.end.offset <= self.input.len() {
            let syntax = span.slice(self.input);
            if let Some(body) = syntax
                .strip_prefix("${")
                .and_then(|syntax| syntax.strip_suffix('}'))
            {
                let start = span.start.advanced_by("${");
                let end = start.advanced_by(body);
                return SourceText::source(Span::from_positions(start, end));
            }
        }

        let mut syntax = String::new();
        self.push_word_part_syntax(&mut syntax, part, span);
        let body = syntax
            .strip_prefix("${")
            .and_then(|syntax| syntax.strip_suffix('}'))
            .unwrap_or(syntax.as_str())
            .to_string();
        SourceText::from(body)
    }

    pub(in crate::parser) fn zsh_parameter_word_part(
        &mut self,
        raw_body: SourceText,
        part_start: Position,
        part_end: Position,
    ) -> WordPart {
        let syntax = self.parse_zsh_parameter_syntax(&raw_body, raw_body.span().start);
        WordPart::Parameter(Box::new(ParameterExpansion {
            syntax: ParameterExpansionSyntax::Zsh(syntax),
            span: Span::from_positions(part_start, part_end),
            raw_body,
        }))
    }

    pub(in crate::parser) fn parse_zsh_modifier_group(
        &self,
        text: &str,
        base: Position,
        start: usize,
    ) -> Option<(usize, Vec<ZshModifier>)> {
        let rest = text.get(start..)?;
        if !rest.starts_with('(') {
            return None;
        }

        let close_rel = rest[1..].find(')')?;
        let close = start + 1 + close_rel;
        let group_text = &text[start..=close];
        let inner = &text[start + 1..close];
        let group_start = base.advanced_by(&text[..start]);
        let group_span = Span::from_positions(group_start, group_start.advanced_by(group_text));
        let mut modifiers = Vec::new();
        let mut index = 0usize;

        while index < inner.len() {
            let name = inner[index..].chars().next()?;
            index += name.len_utf8();

            let mut argument_delimiter = None;
            let mut argument = None;
            if matches!(name, 's' | 'j')
                && let Some(delimiter) = inner[index..].chars().next()
            {
                index += delimiter.len_utf8();
                let argument_start = index;
                while index < inner.len() {
                    let ch = inner[index..].chars().next()?;
                    if ch == delimiter {
                        let argument_text = &inner[argument_start..index];
                        let argument_base =
                            group_start.advanced_by(&group_text[..1 + argument_start]);
                        let argument_end = argument_base.advanced_by(argument_text);
                        argument_delimiter = Some(delimiter);
                        argument = Some(self.source_text(
                            argument_text.to_string(),
                            argument_base,
                            argument_end,
                        ));
                        index += delimiter.len_utf8();
                        break;
                    }
                    index += ch.len_utf8();
                }
            }

            let argument_word_ast = argument
                .as_ref()
                .map(|argument| Box::new(self.parse_source_text_as_word(argument)));

            modifiers.push(ZshModifier {
                name,
                argument,
                argument_word_ast,
                argument_delimiter,
                span: group_span,
            });
        }

        Some((close + 1, modifiers))
    }

    pub(in crate::parser) fn parse_zsh_parameter_syntax(
        &mut self,
        raw_body: &SourceText,
        base: Position,
    ) -> ZshParameterExpansion {
        let text = raw_body.slice(self.input);
        let mut index = 0;
        let mut modifiers = Vec::new();
        let mut length_prefix = None;
        let source_backed = raw_body.is_source_backed();

        while text[index..].starts_with('(')
            && let Some((next_index, group_modifiers)) =
                self.parse_zsh_modifier_group(text, base, index)
        {
            modifiers.extend(group_modifiers);
            index = next_index;
        }

        while index < text.len() {
            let Some(flag) = text[index..].chars().next() else {
                break;
            };
            match flag {
                '=' | '~' | '^' => {
                    let modifier_start = base.advanced_by(&text[..index]);
                    let modifier_end =
                        modifier_start.advanced_by(&text[index..index + flag.len_utf8()]);
                    modifiers.push(ZshModifier {
                        name: flag,
                        argument: None,
                        argument_word_ast: None,
                        argument_delimiter: None,
                        span: Span::from_positions(modifier_start, modifier_end),
                    });
                    index += flag.len_utf8();
                }
                '#' if length_prefix.is_none() => {
                    let prefix_start = base.advanced_by(&text[..index]);
                    let prefix_end = prefix_start.advanced_by("#");
                    length_prefix = Some(Span::from_positions(prefix_start, prefix_end));
                    index += '#'.len_utf8();
                }
                _ => break,
            }
        }

        let (target, operation_index) = if text[index..].starts_with("${") {
            let end = self
                .find_matching_parameter_end(&text[index..])
                .unwrap_or(text.len() - index);
            let nested_text = &text[index..index + end];
            let target =
                self.parse_nested_parameter_target(nested_text, base.advanced_by(&text[..index]));
            (target, index + end)
        } else if text[index..].starts_with(':') || text[index..].is_empty() {
            (ZshExpansionTarget::Empty, index)
        } else {
            let end = self
                .find_zsh_operation_start(&text[index..])
                .map(|offset| index + offset)
                .unwrap_or(text.len());
            let raw_target = &text[index..end];
            let trimmed = raw_target.trim();
            let target = if trimmed.is_empty() {
                ZshExpansionTarget::Empty
            } else {
                let leading = raw_target
                    .len()
                    .saturating_sub(raw_target.trim_start().len());
                let target_base = base.advanced_by(&text[..index + leading]);
                self.parse_zsh_target_from_text(
                    trimmed,
                    target_base,
                    source_backed && leading == 0 && trimmed.len() == raw_target.len(),
                )
            };
            (target, end)
        };

        let operation = (operation_index < text.len()).then(|| {
            self.parse_zsh_parameter_operation(
                &text[operation_index..],
                base.advanced_by(&text[..operation_index]),
            )
        });

        ZshParameterExpansion {
            target,
            modifiers,
            length_prefix,
            operation,
        }
    }

    pub(in crate::parser) fn parse_zsh_target_from_text(
        &mut self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> ZshExpansionTarget {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return ZshExpansionTarget::Empty;
        }

        if trimmed.starts_with("${") && trimmed.ends_with('}') {
            return self.parse_nested_parameter_target(trimmed, base);
        }

        if let Some(reference) = self.maybe_parse_loose_var_ref_target(trimmed) {
            return ZshExpansionTarget::Reference(reference);
        }

        let span = Span::from_positions(base, base.advanced_by(trimmed));
        let word = self.parse_word_with_context(trimmed, span, base, source_backed);
        if let Some(reference) =
            self.parse_var_ref_from_word(&word, SubscriptInterpretation::Contextual)
        {
            ZshExpansionTarget::Reference(reference)
        } else {
            ZshExpansionTarget::Word(Box::new(word))
        }
    }

    pub(in crate::parser) fn maybe_parse_loose_var_ref_target(&self, text: &str) -> Option<VarRef> {
        let trimmed = text.trim();
        Self::looks_like_plain_parameter_access(trimmed).then(|| self.parse_loose_var_ref(trimmed))
    }

    pub(in crate::parser) fn is_plain_special_parameter_name(name: &str) -> bool {
        matches!(name, "#" | "$" | "!" | "*" | "@" | "?" | "-") || name == "0"
    }

    pub(in crate::parser) fn is_plain_parameter_access_name(name: &str) -> bool {
        Self::is_valid_identifier(name)
            || name.bytes().all(|byte| byte.is_ascii_digit())
            || Self::is_plain_special_parameter_name(name)
    }

    pub(in crate::parser) fn looks_like_plain_parameter_access(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }

        let name = if let Some(open) = trimmed.find('[') {
            if !trimmed.ends_with(']') {
                return false;
            }
            &trimmed[..open]
        } else {
            trimmed
        };

        Self::is_plain_parameter_access_name(name)
            || name
                .strip_prefix('+')
                .is_some_and(Self::is_plain_parameter_access_name)
    }

    pub(in crate::parser) fn parse_nested_parameter_target(
        &mut self,
        text: &str,
        base: Position,
    ) -> ZshExpansionTarget {
        if !(text.starts_with("${") && text.ends_with('}')) {
            return self.parse_zsh_target_from_text(text, base, false);
        }

        let raw_body_start = base.advanced_by("${");
        let raw_body = self.source_text(
            text[2..text.len() - 1].to_string(),
            raw_body_start,
            base.advanced_by(&text[..text.len() - 1]),
        );
        let raw_body_text = raw_body.slice(self.input);
        let has_operation = self.find_zsh_operation_start(raw_body_text).is_some();
        let syntax = if Self::looks_like_plain_parameter_access(raw_body_text) && !has_operation {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: self.parse_loose_var_ref(raw_body_text),
            })
        } else if raw_body_text.starts_with('(')
            || raw_body_text.starts_with(':')
            || raw_body_text.starts_with('=')
            || raw_body_text.starts_with('^')
            || raw_body_text.starts_with('~')
            || raw_body_text.starts_with('.')
            || raw_body_text.starts_with('#')
            || raw_body_text.starts_with('"')
            || raw_body_text.starts_with('\'')
            || raw_body_text.starts_with('$')
            || has_operation
        {
            ParameterExpansionSyntax::Zsh(
                self.parse_zsh_parameter_syntax(&raw_body, raw_body_start),
            )
        } else {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: self.parse_loose_var_ref(raw_body_text),
            })
        };

        ZshExpansionTarget::Nested(Box::new(ParameterExpansion {
            syntax,
            span: Span::from_positions(base, base.advanced_by(text)),
            raw_body,
        }))
    }

    pub(in crate::parser) fn parse_loose_var_ref(&self, text: &str) -> VarRef {
        let trimmed = text.trim();
        if let Some(open) = trimmed.find('[')
            && trimmed.ends_with(']')
        {
            let name = &trimmed[..open];
            let subscript_text = &trimmed[open + 1..trimmed.len() - 1];
            let subscript = self.subscript_from_source_text(
                SourceText::from(subscript_text.to_string()),
                None,
                SubscriptInterpretation::Contextual,
            );
            return VarRef {
                name: Name::from(name),
                name_span: Span::new(),
                subscript: Some(Box::new(subscript)),
                span: Span::new(),
            };
        }

        VarRef {
            name: Name::from(trimmed),
            name_span: Span::new(),
            subscript: None,
            span: Span::new(),
        }
    }

    pub(in crate::parser) fn find_matching_parameter_end(&self, text: &str) -> Option<usize> {
        let mut depth = 0_i32;
        let mut chars = text.char_indices().peekable();

        while let Some((index, ch)) = chars.next() {
            match ch {
                '$' if chars.peek().is_some_and(|(_, next)| *next == '{') => {
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + ch.len_utf8());
                    }
                }
                _ => {}
            }
        }

        None
    }

    pub(in crate::parser) fn find_zsh_operation_start(&self, text: &str) -> Option<usize> {
        let mut bracket_depth = 0_usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for (index, ch) in text.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double && bracket_depth > 0 => bracket_depth -= 1,
                ':' if !in_single && !in_double && bracket_depth == 0 => return Some(index),
                '#' | '%' | '/' | '^' | ',' | '~'
                    if !in_single && !in_double && bracket_depth == 0 && index > 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }

        None
    }

    pub(in crate::parser) fn zsh_operation_source_text(
        &self,
        text: &str,
        base: Position,
        start: usize,
        end: usize,
    ) -> SourceText {
        self.source_text(
            text[start..end].to_string(),
            base.advanced_by(&text[..start]),
            base.advanced_by(&text[..end]),
        )
    }

    pub(in crate::parser) fn find_zsh_top_level_delimiter(
        &self,
        text: &str,
        delimiter: char,
    ) -> Option<usize> {
        let mut chars = text.char_indices().peekable();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut brace_depth = 0_usize;
        let mut paren_depth = 0_usize;

        while let Some((index, ch)) = chars.next() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '$' if !in_single => {
                    if let Some((_, next)) = chars.peek() {
                        if *next == '{' {
                            brace_depth += 1;
                            chars.next();
                        } else if *next == '(' {
                            paren_depth += 1;
                            chars.next();
                            if let Some((_, after)) = chars.peek()
                                && *after == '('
                            {
                                paren_depth += 1;
                                chars.next();
                            }
                        }
                    }
                }
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                _ if ch == delimiter
                    && !in_single
                    && !in_double
                    && brace_depth == 0
                    && paren_depth == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }

        None
    }

    pub(in crate::parser) fn zsh_simple_modifier_suffix_segment(segment: &str) -> bool {
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return false;
        };

        match first {
            'a' | 'A' | 'c' | 'e' | 'l' | 'P' | 'q' | 'Q' | 'r' | 'u' => chars.next().is_none(),
            'h' | 't' => chars.all(|ch| ch.is_ascii_digit()),
            _ => false,
        }
    }

    pub(in crate::parser) fn zsh_modifier_suffix_candidate(rest: &str) -> bool {
        if rest.is_empty() {
            return false;
        }

        let Some(first) = rest.chars().next() else {
            return false;
        };
        if first.is_ascii_digit()
            || first.is_ascii_whitespace()
            || matches!(first, '$' | '\'' | '"' | '(' | '{')
        {
            return false;
        }

        rest.split(':')
            .all(Self::zsh_simple_modifier_suffix_segment)
    }

    pub(in crate::parser) fn zsh_slice_candidate(rest: &str) -> bool {
        let Some(first) = rest.chars().next() else {
            return false;
        };

        !Self::zsh_modifier_suffix_candidate(rest)
            && (first.is_ascii_alphanumeric()
                || first == '_'
                || first.is_ascii_whitespace()
                || matches!(first, '$' | '\'' | '"' | '(' | '{'))
    }

    pub(in crate::parser) fn parse_zsh_parameter_operation(
        &self,
        text: &str,
        base: Position,
    ) -> ZshExpansionOperation {
        if let Some(operand) = text.strip_prefix(":#") {
            let operand = self.source_text(
                operand.to_string(),
                base.advanced_by(":#"),
                base.advanced_by(text),
            );
            return ZshExpansionOperation::PatternOperation {
                kind: ZshPatternOp::Filter,
                operand_word_ast: Box::new(self.parse_source_text_as_word(&operand)),
                operand,
            };
        }

        if let Some((kind, operand)) = text
            .strip_prefix(":-")
            .map(|operand| (ZshDefaultingOp::UseDefault, operand))
            .or_else(|| {
                text.strip_prefix(":=")
                    .map(|operand| (ZshDefaultingOp::AssignDefault, operand))
            })
            .or_else(|| {
                text.strip_prefix(":+")
                    .map(|operand| (ZshDefaultingOp::UseReplacement, operand))
            })
            .or_else(|| {
                text.strip_prefix(":?")
                    .map(|operand| (ZshDefaultingOp::Error, operand))
            })
        {
            let operand = self.source_text(
                operand.to_string(),
                base.advanced_by(&text[..2]),
                base.advanced_by(text),
            );
            return ZshExpansionOperation::Defaulting {
                kind,
                operand_word_ast: Box::new(self.parse_source_text_as_word(&operand)),
                operand,
                colon_variant: true,
            };
        }

        if let Some((kind, prefix_len)) = [
            ("##", ZshTrimOp::RemovePrefixLong),
            ("#", ZshTrimOp::RemovePrefixShort),
            ("%%", ZshTrimOp::RemoveSuffixLong),
            ("%", ZshTrimOp::RemoveSuffixShort),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| text.starts_with(prefix).then_some((kind, prefix.len())))
        {
            let operand = self.zsh_operation_source_text(text, base, prefix_len, text.len());
            return ZshExpansionOperation::TrimOperation {
                kind,
                operand_word_ast: Box::new(self.parse_source_text_as_word(&operand)),
                operand,
            };
        }

        if let Some((kind, prefix_len)) = [
            ("//", ZshReplacementOp::ReplaceAll),
            ("/#", ZshReplacementOp::ReplacePrefix),
            ("/%", ZshReplacementOp::ReplaceSuffix),
            ("/", ZshReplacementOp::ReplaceFirst),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| text.starts_with(prefix).then_some((kind, prefix.len())))
        {
            let rest = &text[prefix_len..];
            let separator = self.find_zsh_top_level_delimiter(rest, '/');
            let pattern_end = separator.unwrap_or(rest.len());
            let pattern =
                self.zsh_operation_source_text(text, base, prefix_len, prefix_len + pattern_end);
            let replacement = separator.map(|separator| {
                self.zsh_operation_source_text(text, base, prefix_len + separator + 1, text.len())
            });
            return ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern_word_ast: Box::new(self.parse_source_text_as_word(&pattern)),
                replacement_word_ast: self
                    .parse_optional_source_text_as_word(replacement.as_ref())
                    .map(Box::new),
                pattern,
                replacement,
            };
        }

        if let Some(rest) = text.strip_prefix(':') {
            if Self::zsh_modifier_suffix_candidate(rest) {
                let text = self.source_text(text.to_string(), base, base.advanced_by(text));
                return ZshExpansionOperation::Unknown {
                    word_ast: Box::new(self.parse_source_text_as_word(&text)),
                    text,
                };
            }

            if Self::zsh_slice_candidate(rest) {
                let separator = self.find_zsh_top_level_delimiter(rest, ':');
                let offset_end = separator.unwrap_or(rest.len());
                let offset = self.zsh_operation_source_text(text, base, 1, 1 + offset_end);
                let length = separator.map(|separator| {
                    self.zsh_operation_source_text(text, base, 1 + separator + 1, text.len())
                });
                return ZshExpansionOperation::Slice {
                    offset_word_ast: Box::new(self.parse_source_text_as_word(&offset)),
                    length_word_ast: self
                        .parse_optional_source_text_as_word(length.as_ref())
                        .map(Box::new),
                    offset,
                    length,
                };
            }
        }

        let text = self.source_text(text.to_string(), base, base.advanced_by(text));
        ZshExpansionOperation::Unknown {
            word_ast: Box::new(self.parse_source_text_as_word(&text)),
            text,
        }
    }
}
