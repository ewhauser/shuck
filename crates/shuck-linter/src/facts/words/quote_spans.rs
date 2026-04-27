pub(super) fn word_occurrence_with_context<'a>(
    nodes: &[WordNode<'a>],
    occurrences: &'a [WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    span: Span,
    context: WordFactContext,
) -> Option<&'a WordOccurrence> {
    word_index
        .get(&FactSpan::new(span))
        .into_iter()
        .flat_map(|indices| indices.iter().copied())
        .map(|id| &occurrences[id.index()])
        .find(|fact| occurrence_span(nodes, fact) == span && fact.context == context)
}

pub(super) fn occurrence_static_text<'a>(
    nodes: &'a [WordNode<'a>],
    occurrence: &WordOccurrence,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    let node = &nodes[occurrence.node_id.index()];
    word_node_derived(node)
        .static_text
        .map(Cow::Borrowed)
        .or_else(|| static_word_text(node.word, source))
}

pub(super) fn word_occurrence_is_pure_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let word = occurrence_word(nodes, fact);
    !word_spans::word_double_quoted_scalar_only_expansion_spans(word).is_empty()
        || !word_spans::word_quoted_all_elements_array_slice_spans(word).is_empty()
        || word_occurrence_is_double_quoted_command_substitution_only(
            nodes, fact, fact_store, source,
        )
        || word_occurrence_is_escaped_double_quoted_dynamic(
            nodes, fact, fact_store, source,
        )
}

pub(super) fn collect_unquoted_literal_between_double_quoted_segments_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;

    for index in 0..word.parts.len() {
        let middle_is_nested = command_depth > 0 || parameter_depth > 0;

        if index > 0 && index + 1 < word.parts.len() {
            let left = &word.parts[index - 1];
            let middle = &word.parts[index];
            let right = &word.parts[index + 1];

            if let (
                WordPart::DoubleQuoted {
                    parts: left_inner, ..
                },
                WordPart::Literal(text),
                WordPart::DoubleQuoted {
                    parts: right_inner, ..
                },
            ) = (&left.kind, &middle.kind, &right.kind)
            {
                let neighbor_has_literal =
                    mixed_quote_double_quoted_parts_contain_literal_content(left_inner)
                        || mixed_quote_double_quoted_parts_contain_literal_content(right_inner);
                if neighbor_has_literal
                    && !middle_is_nested
                    && mixed_quote_literal_is_warnable_between_double_quotes(
                        text.as_str(source, middle.span),
                    )
                {
                    spans.push(middle.span);
                }
            }
        }

        let (command_delta, parameter_delta) =
            mixed_quote_shell_fragment_balance_delta_for_part(&word.parts[index], source);
        command_depth += command_delta;
        parameter_depth += parameter_delta;
        command_depth = command_depth.max(0);
        parameter_depth = parameter_depth.max(0);
    }

    for span in mixed_quote_line_join_between_double_quotes_spans(word, source) {
        if !spans.contains(&span) {
            spans.push(span);
        }
    }

    if let Some(span) = mixed_quote_following_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }

    for span in mixed_quote_chained_line_join_between_double_quotes_spans(word, source) {
        if !spans.contains(&span) {
            spans.push(span);
        }
    }

    if let Some(span) = mixed_quote_trailing_line_join_between_double_quotes_span(word, source)
        && !spans.contains(&span)
    {
        spans.push(span);
    }
}

pub(super) fn mixed_quote_double_quoted_parts_contain_literal_content(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => {
            mixed_quote_double_quoted_parts_contain_literal_content(parts)
        }
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

pub(super) fn mixed_quote_literal_is_warnable_between_double_quotes(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    if text == "\"" {
        return true;
    }

    if matches!(text, "\\\n" | "\\\r\n") {
        return true;
    }

    if text == "/,/" {
        return true;
    }

    if text.chars().all(|ch| matches!(ch, '\\' | '"')) && text.contains('\\') {
        return true;
    }

    if text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        if text
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | '{' | '}'))
        {
            return false;
        }

        if mixed_quote_literal_has_shellcheck_skipped_word_operator(text) {
            return false;
        }

        return !text.chars().any(char::is_whitespace);
    }

    if text.chars().all(|ch| ch == ':') {
        return text.len() > 1;
    }

    text.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '+' | '-' | '%' | ':')
    })
}

pub(super) fn mixed_quote_literal_has_shellcheck_skipped_word_operator(text: &str) -> bool {
    text.contains('+') || text.contains('@')
}

pub(super) fn mixed_quote_shell_fragment_balance_delta_for_part(
    part: &WordPartNode,
    source: &str,
) -> (i32, i32) {
    match &part.kind {
        WordPart::CommandSubstitution {
            syntax: CommandSubstitutionSyntax::Backtick,
            ..
        } => {
            let text = part.span.slice(source);
            let body = text
                .strip_prefix('`')
                .and_then(|text| text.strip_suffix('`'))
                .unwrap_or(text);
            mixed_quote_shell_fragment_balance_delta(body, true)
        }
        WordPart::ProcessSubstitution { .. } => {
            mixed_quote_shell_fragment_balance_delta(part.span.slice(source), true)
        }
        WordPart::DoubleQuoted { .. } => {
            let text = part.span.slice(source);
            let body = text
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .unwrap_or(text);
            mixed_quote_shell_fragment_balance_delta(body, false)
        }
        _ => mixed_quote_shell_fragment_balance_delta(part.span.slice(source), false),
    }
}

#[derive(Clone, Copy)]
pub(super) enum MixedQuoteShellParenFrame {
    Command { opened_in_double_quotes: bool },
    Group,
}

pub(super) fn mixed_quote_shell_fragment_balance_delta(
    text: &str,
    allow_top_level_command_comments: bool,
) -> (i32, i32) {
    let mut command_delta = 0i32;
    let mut parameter_delta = 0i32;
    let mut chars = text.chars().peekable();
    let mut escaped = false;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut command_frames = SmallVec::<[MixedQuoteShellParenFrame; 4]>::new();
    let mut parameter_frames = SmallVec::<[bool; 4]>::new();
    let mut previous_char = None;

    while let Some(ch) = chars.next() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                previous_char = Some(ch);
            }
            continue;
        }

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            previous_char = Some(ch);
            continue;
        }

        if escaped {
            escaped = false;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '"' {
            in_double_quotes = !in_double_quotes;
            previous_char = Some(ch);
            continue;
        }

        if ch == '\\' {
            escaped = true;
            previous_char = Some(ch);
            continue;
        }

        let allow_top_level_command_comment =
            allow_top_level_command_comments && parameter_delta == 0;
        if ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_delta,
                allow_top_level_command_comment,
                previous_char,
            )
        {
            in_comment = true;
            continue;
        }

        if ch == '$' {
            match chars.peek().copied() {
                Some('(') => {
                    command_delta += 1;
                    command_frames.push(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes: in_double_quotes,
                    });
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some('{') => {
                    parameter_delta += 1;
                    parameter_frames.push(in_double_quotes);
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        match ch {
            '(' if !in_double_quotes && command_delta > 0 => {
                command_frames.push(MixedQuoteShellParenFrame::Group);
            }
            ')' => match command_frames.last().copied() {
                Some(MixedQuoteShellParenFrame::Group) if !in_double_quotes => {
                    command_frames.pop();
                }
                Some(MixedQuoteShellParenFrame::Command {
                    opened_in_double_quotes,
                }) if !in_double_quotes || opened_in_double_quotes => {
                    command_frames.pop();
                    command_delta -= 1;
                }
                None if !in_double_quotes => command_delta -= 1,
                _ => {}
            },
            '}' => match parameter_frames.last().copied() {
                Some(opened_in_double_quotes) if !in_double_quotes || opened_in_double_quotes => {
                    parameter_frames.pop();
                    parameter_delta -= 1;
                }
                None if !in_double_quotes => parameter_delta -= 1,
                _ => {}
            },
            _ => {}
        }

        if command_delta <= 0 {
            command_frames.clear();
        }
        if parameter_delta <= 0 {
            parameter_frames.clear();
        }

        previous_char = Some(ch);
    }

    (command_delta, parameter_delta)
}

pub(super) fn mixed_quote_shell_comment_can_start(
    command_depth: i32,
    allow_top_level_command_comments: bool,
    previous_char: Option<char>,
) -> bool {
    (command_depth > 0 || allow_top_level_command_comments)
        && previous_char.is_none_or(|ch| {
            ch.is_ascii_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>')
        })
}

pub(super) fn mixed_quote_trailing_line_join_between_double_quotes_span(
    word: &Word,
    source: &str,
) -> Option<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return None;
    }

    let text = word.span.slice(source);
    let (prefix, suffix) = if let Some(prefix) = text.strip_suffix("\\\n") {
        (prefix, "\\\n")
    } else if let Some(prefix) = text.strip_suffix("\\\r\n") {
        (prefix, "\\\r\n")
    } else {
        return None;
    };

    if !mixed_quote_text_ends_with_unescaped_double_quote(prefix)
        || !source[word.span.end.offset..].starts_with('"')
    {
        return None;
    }

    let start = word.span.start.advanced_by(prefix);
    Some(Span::from_positions(start, start.advanced_by(suffix)))
}

pub(super) fn mixed_quote_line_join_between_double_quotes_spans(word: &Word, source: &str) -> Vec<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return Vec::new();
    }

    let text = word.span.slice(source);
    let mut spans = Vec::new();
    let mut byte_offset = 0;

    while byte_offset < text.len() {
        let Some(relative_offset) = text[byte_offset..].find('\\') else {
            break;
        };
        let start_offset = byte_offset + relative_offset;
        let Some(suffix) = text[start_offset..]
            .strip_prefix("\\\r\n\"")
            .map(|_| "\\\r\n")
            .or_else(|| text[start_offset..].strip_prefix("\\\n\"").map(|_| "\\\n"))
        else {
            byte_offset = start_offset + 1;
            continue;
        };

        if mixed_quote_text_ends_with_unescaped_double_quote(&text[..start_offset]) {
            let start = word.span.start.advanced_by(&text[..start_offset]);
            spans.push(Span::from_positions(start, start.advanced_by(suffix)));
        }

        byte_offset = start_offset + suffix.len();
    }

    spans
}

pub(super) fn mixed_quote_following_line_join_between_double_quotes_span(
    word: &Word,
    source: &str,
) -> Option<Span> {
    let suffix = mixed_quote_following_line_join_suffix_after_word(word, source)?;
    Some(Span::from_positions(
        word.span.end,
        word.span.end.advanced_by(suffix),
    ))
}

pub(super) fn mixed_quote_following_line_join_suffix_after_word(
    word: &Word,
    source: &str,
) -> Option<&'static str> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return None;
    }

    let tail = &source[word.span.end.offset..];
    let suffix = tail
        .strip_prefix("\\\r\n\"")
        .map(|_| "\\\r\n")
        .or_else(|| tail.strip_prefix("\\\n\"").map(|_| "\\\n"))?;

    if !mixed_quote_text_ends_with_unescaped_double_quote(word.span.slice(source)) {
        return None;
    }

    Some(suffix)
}

pub(super) fn mixed_quote_chained_line_join_between_double_quotes_spans(
    word: &Word,
    source: &str,
) -> Vec<Span> {
    if !matches!(
        word.parts.first().map(|part| &part.kind),
        Some(WordPart::DoubleQuoted { .. })
    ) {
        return Vec::new();
    }

    let text = word.span.slice(source);
    if !(text.ends_with("\\\n") || text.ends_with("\\\r\n")) {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut cursor = word.span.end.offset;
    while source[cursor..].starts_with('"') {
        let Some(closing_quote_relative) =
            mixed_quote_closing_double_quote_offset(&source[cursor..])
        else {
            break;
        };
        let after_closing_quote = cursor + closing_quote_relative + 1;
        let Some(suffix) = source[after_closing_quote..]
            .strip_prefix("\\\r\n\"")
            .map(|_| "\\\r\n")
            .or_else(|| {
                source[after_closing_quote..]
                    .strip_prefix("\\\n\"")
                    .map(|_| "\\\n")
            })
        else {
            break;
        };

        let start = Position::new().advanced_by(&source[..after_closing_quote]);
        spans.push(Span::from_positions(start, start.advanced_by(suffix)));
        cursor = after_closing_quote + suffix.len();
    }

    spans
}

pub(super) fn mixed_quote_closing_double_quote_offset(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    let (_, first) = chars.next()?;
    if first != '"' {
        return None;
    }

    let mut escaped = false;
    let mut command_depth = 0i32;
    let mut parameter_depth = 0i32;
    let mut command_frames = SmallVec::<[MixedQuoteShellParenFrame; 4]>::new();
    let mut parameter_frames = SmallVec::<[bool; 4]>::new();
    let mut in_backtick_command = false;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut in_comment = false;
    let mut previous_char = Some('"');

    while let Some((offset, ch)) = chars.next() {
        let nested_depth = command_depth > 0 || parameter_depth > 0 || in_backtick_command;

        if in_comment {
            if ch == '\n' {
                in_comment = false;
                previous_char = Some(ch);
            }
            continue;
        }

        if in_single_quotes {
            if ch == '\'' {
                in_single_quotes = false;
            }
            previous_char = Some(ch);
            continue;
        }

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

        if ch == '`' && !in_single_quotes {
            in_backtick_command = !in_backtick_command;
            previous_char = Some(ch);
            continue;
        }

        if nested_depth && ch == '\'' && !in_double_quotes {
            in_single_quotes = true;
            previous_char = Some(ch);
            continue;
        }

        if ch == '"' {
            if !nested_depth {
                return Some(offset);
            }
            in_double_quotes = !in_double_quotes;
            previous_char = Some(ch);
            continue;
        }

        let allow_top_level_command_comment = in_backtick_command && parameter_depth == 0;
        if nested_depth
            && ch == '#'
            && !in_double_quotes
            && mixed_quote_shell_comment_can_start(
                command_depth,
                allow_top_level_command_comment,
                previous_char,
            )
        {
            in_comment = true;
            continue;
        }

        if ch == '$' {
            match chars.peek().copied() {
                Some((_, '(')) => {
                    command_depth += 1;
                    command_frames.push(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes: in_double_quotes,
                    });
                    chars.next();
                    previous_char = Some('(');
                    continue;
                }
                Some((_, '{')) => {
                    parameter_depth += 1;
                    parameter_frames.push(in_double_quotes);
                    chars.next();
                    previous_char = Some('{');
                    continue;
                }
                _ => {}
            }
        }

        if nested_depth {
            match ch {
                '(' if !in_double_quotes && command_depth > 0 => {
                    command_frames.push(MixedQuoteShellParenFrame::Group);
                }
                ')' => match command_frames.last().copied() {
                    Some(MixedQuoteShellParenFrame::Group) if !in_double_quotes => {
                        command_frames.pop();
                    }
                    Some(MixedQuoteShellParenFrame::Command {
                        opened_in_double_quotes,
                    }) if !in_double_quotes || opened_in_double_quotes => {
                        command_frames.pop();
                        command_depth -= 1;
                    }
                    None if !in_double_quotes => command_depth -= 1,
                    _ => {}
                },
                '}' => match parameter_frames.last().copied() {
                    Some(opened_in_double_quotes)
                        if !in_double_quotes || opened_in_double_quotes =>
                    {
                        parameter_frames.pop();
                        parameter_depth -= 1;
                    }
                    None if !in_double_quotes => parameter_depth -= 1,
                    _ => {}
                },
                _ => {}
            }
            command_depth = command_depth.max(0);
            parameter_depth = parameter_depth.max(0);
            if command_depth == 0 {
                command_frames.clear();
            }
            if parameter_depth == 0 {
                parameter_frames.clear();
            }
        }

        previous_char = Some(ch);
    }

    None
}

pub(super) fn mixed_quote_text_ends_with_unescaped_double_quote(text: &str) -> bool {
    let Some(prefix) = text.strip_suffix('"') else {
        return false;
    };

    let backslash_count = prefix.chars().rev().take_while(|ch| *ch == '\\').count();
    backslash_count % 2 == 0
}

pub(super) fn word_occurrence_is_double_quoted_command_substitution_only(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()]);
    let command_substitution_spans = fact_store.word_spans(derived.command_substitution_spans);
    let [command_substitution] = command_substitution_spans else {
        return false;
    };

    if !derived.scalar_expansion_spans.is_empty() || !derived.array_expansion_spans.is_empty() {
        return false;
    }

    let word_text = occurrence_span(nodes, fact).slice(source);
    word_text.len() == command_substitution.slice(source).len() + 2
        && word_text.starts_with('"')
        && word_text.ends_with('"')
        && &word_text[1..word_text.len() - 1] == command_substitution.slice(source)
}

pub(super) fn word_occurrence_is_escaped_double_quoted_dynamic(
    nodes: &[WordNode<'_>],
    fact: &WordOccurrence,
    fact_store: &FactStore<'_>,
    source: &str,
) -> bool {
    let derived = word_node_derived(&nodes[fact.node_id.index()]);
    let word_text = occurrence_span(nodes, fact).slice(source);
    if !word_text.starts_with("\\\"") || !word_text.ends_with("\\\"") {
        return false;
    }

    let inner = &word_text[2..word_text.len() - 2];
    match (
        fact_store.word_spans(derived.scalar_expansion_spans),
        fact_store.word_spans(derived.array_expansion_spans),
        fact_store.word_spans(derived.command_substitution_spans),
    ) {
        ([scalar], [], []) => inner == scalar.slice(source),
        ([], [array], []) => inner == array.slice(source),
        ([], [], [command_substitution]) => inner == command_substitution.slice(source),
        _ => false,
    }
}


pub(super) fn pattern_has_glob_structure(pattern: &Pattern, source: &str) -> bool {
    pattern.parts_with_spans().any(|(part, span)| match part {
        PatternPart::AnyString | PatternPart::AnyChar | PatternPart::CharClass(_) => true,
        PatternPart::Group { .. } => true,
        PatternPart::Literal(text) => literal_text_has_glob_bracket(text.as_str(source, span)),
        PatternPart::Word(word) => word.parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::Literal(text)
                    if literal_text_has_glob_bracket(text.as_str(source, part.span))
            )
        }),
    })
}

pub(super) fn literal_text_has_glob_bracket(text: &str) -> bool {
    text.contains('[') || text.contains(']')
}

pub(super) fn pattern_is_arithmetic_only(pattern: &Pattern) -> bool {
    pattern.parts.iter().all(|part| match &part.kind {
        PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => true,
        PatternPart::Word(word) => word_is_arithmetic_only(word),
        PatternPart::CharClass(_) | PatternPart::Group { .. } => false,
    })
}

pub(super) fn word_is_arithmetic_only(word: &Word) -> bool {
    word.parts.iter().all(word_part_is_arithmetic_only)
}

pub(super) fn word_part_is_arithmetic_only(part: &WordPartNode) -> bool {
    match &part.kind {
        WordPart::Literal(_) | WordPart::SingleQuoted { .. } => true,
        WordPart::ArithmeticExpansion { .. } => true,
        WordPart::DoubleQuoted { parts, .. } => parts.iter().all(word_part_is_arithmetic_only),
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => false,
    }
}

pub(super) fn standalone_variable_name_from_word_parts(parts: &[WordPartNode]) -> Option<&str> {
    let [part] = parts else {
        return None;
    };

    match &part.kind {
        WordPart::Variable(name) => Some(name.as_str()),
        WordPart::Parameter(parameter) => match parameter.bourne() {
            Some(BourneParameterExpansion::Access { reference })
                if reference.subscript.is_none() =>
            {
                Some(reference.name.as_str())
            }
            _ => None,
        },
        WordPart::DoubleQuoted { parts, .. } => standalone_variable_name_from_word_parts(parts),
        WordPart::Literal(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::SingleQuoted { .. }
        | WordPart::ParameterExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ArrayAccess(_)
        | WordPart::ArrayLength(_)
        | WordPart::ArrayIndices(_)
        | WordPart::Substring { .. }
        | WordPart::ArraySlice { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::ZshQualifiedGlob(_) => None,
    }
}

pub(super) fn word_context_supports_operand_class(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::AssignmentValue
            | ExpansionContext::DeclarationAssignmentValue
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::StringTestOperand
            | ExpansionContext::RegexOperand
            | ExpansionContext::CasePattern
            | ExpansionContext::ConditionalPattern
            | ExpansionContext::ParameterPattern
    )
}

pub(super) fn word_has_literal_affixes(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part.kind,
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
        )
    })
}

pub(super) fn word_contains_shell_quoting_literals(word: &Word, source: &str) -> bool {
    word_parts_contain_shell_quoting_literals(&word.parts, source)
}

pub(super) fn word_parts_contain_shell_quoting_literals(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => text_contains_shell_quoting_literals(
            text.as_str(source, part.span),
            ShellQuotingLiteralTextContext::ShellContinuationAware,
        ),
        WordPart::SingleQuoted { value, .. } => text_contains_shell_quoting_literals(
            value.slice(source),
            ShellQuotingLiteralTextContext::LiteralBackslashNewlines,
        ),
        WordPart::DoubleQuoted { parts, .. } => {
            word_parts_contain_shell_quoting_literals(parts, source)
        }
        _ => false,
    })
}

#[derive(Clone, Copy)]
pub(super) enum ShellQuotingLiteralTextContext {
    ShellContinuationAware,
    LiteralBackslashNewlines,
}

pub(super) fn text_contains_shell_quoting_literals(
    text: &str,
    context: ShellQuotingLiteralTextContext,
) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }

        while chars.peek().is_some_and(|next| *next == '\\') {
            chars.next();
        }

        if chars.peek().is_some_and(|next| {
            matches!(next, '"' | '\'')
                || (next.is_whitespace()
                    && (matches!(
                        context,
                        ShellQuotingLiteralTextContext::LiteralBackslashNewlines
                    ) || !matches!(next, '\n' | '\r')))
        }) {
            return true;
        }
    }

    false
}


pub(super) fn word_classification_from_analysis(analysis: ExpansionAnalysis) -> WordClassification {
    WordClassification {
        quote: analysis.quote,
        literalness: analysis.literalness,
        expansion_kind: analysis.expansion_kind(),
        substitution_shape: analysis.substitution_shape,
    }
}

pub(super) fn double_quoted_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_expansion_part_spans(word, &mut spans);
    spans
}

pub(super) fn collect_double_quoted_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_double_quoted_expansion_spans(&word.parts, false, spans);
}

pub(super) fn single_quoted_equivalent_if_plain_double_quoted_word(
    word: &Word,
    source: &str,
) -> Option<String> {
    let [part] = word.parts.as_slice() else {
        return None;
    };
    let WordPart::DoubleQuoted { dollar: false, .. } = &part.kind else {
        return None;
    };

    let text = word.span.slice(source);
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut cooked = String::with_capacity(body.len());
    push_cooked_double_quoted_word_text(body, &mut cooked);

    Some(shell_single_quoted_literal(&cooked))
}

pub(super) fn push_cooked_double_quoted_word_text(text: &str, out: &mut String) {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some(escaped @ ('$' | '"' | '\\' | '`')) => out.push(escaped),
            Some('\n') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
}

pub(super) fn shell_single_quoted_literal(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

pub(super) fn collect_double_quoted_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_double_quoted_expansion_spans(parts, true, spans);
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_)
                if inside_double_quotes =>
            {
                spans.push(part.span)
            }
            WordPart::Literal(_) => {}
            _ => {}
        }
    }
}

pub fn leading_literal_word_prefix(word: &Word, source: &str) -> String {
    let mut prefix = String::new();
    collect_leading_literal_word_parts(&word.parts, source, &mut prefix);
    prefix
}

pub(super) fn collect_leading_literal_word_parts(
    parts: &[WordPartNode],
    source: &str,
    prefix: &mut String,
) -> bool {
    for part in parts {
        if !collect_leading_literal_word_part(part, source, prefix) {
            return false;
        }
    }
    true
}

pub(super) fn collect_leading_literal_word_part(
    part: &WordPartNode,
    source: &str,
    prefix: &mut String,
) -> bool {
    match &part.kind {
        WordPart::Literal(text) => {
            prefix.push_str(text.as_str(source, part.span));
            true
        }
        WordPart::SingleQuoted { value, .. } => {
            prefix.push_str(value.slice(source));
            true
        }
        WordPart::DoubleQuoted { parts, .. } => {
            collect_leading_literal_word_parts(parts, source, prefix)
        }
        _ => false,
    }
}
