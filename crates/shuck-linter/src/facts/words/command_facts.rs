pub(super) fn build_function_in_alias_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| alias_definition_word_groups_for_command(fact, source).into_iter())
        .filter_map(|definition_words| function_in_alias_definition_span(definition_words, source))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_alias_definition_expansion_spans(
    commands: &[CommandFact<'_>],
    fact_store: &FactStore<'_>,
    nodes: &[WordNode<'_>],
    occurrences: &[WordOccurrence],
    word_index: &FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    source: &str,
) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| alias_definition_word_groups_for_command(fact, source).into_iter())
        .filter_map(|definition_words| {
            definition_words
                .iter()
                .flat_map(|candidate| {
                    word_index
                        .get(&FactSpan::new(candidate.span))
                        .into_iter()
                        .flat_map(|indices| indices.iter().copied())
                        .map(|id| &occurrences[id.index()])
                        .filter(move |fact| {
                            fact.context
                                == WordFactContext::Expansion(ExpansionContext::CommandArgument)
                                && occurrence_span(nodes, fact) == candidate.span
                        })
                })
                .flat_map(|fact| {
                    let derived = word_node_derived(&nodes[fact.node_id.index()]);
                    fact_store
                        .word_spans(derived.active_expansion_spans)
                        .iter()
                        .copied()
                })
                .min_by_key(|span| (span.start.offset, span.end.offset))
        })
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut spans);
    spans
}

pub(super) fn alias_definition_word_groups_for_command<'a>(
    command: &'a CommandFact<'a>,
    source: &str,
) -> Vec<&'a [&'a Word]> {
    let body_args = command.body_args();
    let mut definition_words = Vec::new();
    let mut index = 0usize;

    while let Some(word) = body_args.get(index).copied() {
        if !word_contains_literal_equals(word, source) {
            index += 1;
            continue;
        }

        let mut last_word = word;
        let mut definition_len = 1usize;
        while word_ends_with_literal_equals(last_word, source)
            && let Some(next_word) = body_args.get(index + definition_len).copied()
            && last_word.span.end.offset == next_word.span.start.offset
        {
            last_word = next_word;
            definition_len += 1;
        }

        definition_words.push(&body_args[index..index + definition_len]);
        index += definition_len;
    }

    definition_words
}

pub(super) fn word_contains_literal_equals(word: &Word, source: &str) -> bool {
    word_chars_outside_expansions(word, source).any(|(_, ch)| ch == '=')
}

pub(super) fn word_ends_with_literal_equals(word: &Word, source: &str) -> bool {
    word_chars_outside_expansions(word, source)
        .last()
        .is_some_and(|(_, ch)| ch == '=')
}

pub(super) fn word_chars_outside_expansions<'a>(
    word: &'a Word,
    source: &'a str,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let text = word.span.slice(source);
    let mut excluded = expansion_part_spans(word);
    excluded.sort_by_key(|span| span.start.offset);
    let mut excluded = excluded.into_iter().peekable();

    text.char_indices().filter(move |(offset, _)| {
        let absolute_offset = word.span.start.offset + offset;
        while matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.end.offset
        ) {
            excluded.next();
        }

        !matches!(
            excluded.peek(),
            Some(span) if absolute_offset >= span.start.offset && absolute_offset < span.end.offset
        )
    })
}

pub(super) fn function_in_alias_definition_span(words: &[&Word], source: &str) -> Option<Span> {
    let definition = static_alias_definition_text(words, source)?;
    let (_, value) = definition.split_once('=').unwrap_or(("", &definition));
    let end = words.last()?.span.end;
    contains_positional_parameter_reference(value)
        .then(|| Span::from_positions(words[0].span.start, end))
}

pub(super) fn static_alias_definition_text(words: &[&Word], source: &str) -> Option<String> {
    let mut text = String::new();
    for word in words {
        text.push_str(&static_word_text(word, source)?);
    }
    Some(text)
}

pub(super) fn contains_positional_parameter_reference(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;

    while let Some(byte) = bytes.get(index).copied() {
        match byte {
            b'\''
                if !in_double_quotes && (in_single_quotes || !is_escaped_dollar(value, index)) =>
            {
                in_single_quotes = !in_single_quotes;
                index += 1;
                continue;
            }
            b'"' if !in_single_quotes && !is_escaped_dollar(value, index) => {
                in_double_quotes = !in_double_quotes;
                index += 1;
                continue;
            }
            b'#' if !in_single_quotes
                && !in_double_quotes
                && !is_escaped_dollar(value, index)
                && starts_comment(value, index) =>
            {
                return false;
            }
            b'$' if !in_single_quotes && !is_escaped_dollar(value, index) => {}
            _ => {
                index += 1;
                continue;
            }
        }

        index += 1;
        let Some(next) = bytes.get(index).copied() else {
            return false;
        };

        if next == b'$' {
            index += 1;
            continue;
        }

        if is_positional_parameter_start(next) {
            return true;
        }

        if next == b'{' && braced_parameter_starts_with_positional(value, index + 1) {
            return true;
        }

        if next == b'{' {
            index += 1;
        }
    }
    false
}

pub(super) fn starts_comment(value: &str, hash: usize) -> bool {
    hash == 0
        || value.as_bytes()[hash - 1].is_ascii_whitespace()
        || matches!(
            value.as_bytes()[hash - 1],
            b';' | b'&' | b'|' | b'(' | b')' | b'{' | b'}'
        )
}

pub(super) fn is_escaped_dollar(value: &str, dollar: usize) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = dollar;
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

pub(super) fn braced_parameter_starts_with_positional(value: &str, index: usize) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.get(index).copied() else {
        return false;
    };

    if is_positional_parameter_start(first) {
        return true;
    }

    matches!(first, b'#' | b'!')
        && bytes
            .get(index + 1)
            .copied()
            .is_some_and(is_positional_parameter_start)
}

pub(super) fn is_positional_parameter_start(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'@' | b'*')
}

pub(super) fn build_echo_backslash_escape_word_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    let mut spans = commands
        .iter()
        .filter(|fact| fact.effective_name_is("echo") && fact.wrappers().is_empty())
        .filter(|fact| !echo_uses_escape_interpreting_flag(fact))
        .flat_map(|fact| fact.body_args().iter().copied())
        .filter(|word| word_contains_echo_backslash_escape(word, source))
        .map(|word| word.span)
        .collect::<Vec<_>>();

    let mut seen = FxHashSet::default();
    spans.retain(|span| seen.insert(FactSpan::new(*span)));
    spans
}

pub(super) fn echo_uses_escape_interpreting_flag(command: &CommandFact<'_>) -> bool {
    command
        .options()
        .echo()
        .is_some_and(|echo| echo.uses_escape_interpreting_flag())
}

pub(super) fn word_contains_echo_backslash_escape(word: &Word, source: &str) -> bool {
    word_parts_contain_echo_backslash_escape(&word.parts, source, false)
}

pub(super) fn word_parts_contain_echo_backslash_escape(
    parts: &[WordPartNode],
    source: &str,
    in_double_quotes: bool,
) -> bool {
    parts
        .iter()
        .enumerate()
        .any(|(index, part)| match &part.kind {
            WordPart::Literal(text) => {
                let core_text = if in_double_quotes {
                    text.as_str(source, part.span)
                } else {
                    part.span.slice(source)
                };
                let rendered_text = text.as_str(source, part.span);
                text_contains_echo_backslash_escape(core_text, echo_escape_is_core_family)
                    || (in_double_quotes
                        && text_contains_echo_backslash_escape(
                            rendered_text,
                            echo_escape_is_quote_like,
                        ))
                    || text_contains_echo_double_backslash(rendered_text)
                    || literal_double_backslash_touches_double_quoted_fragment(
                        parts,
                        index,
                        rendered_text,
                    )
            }
            WordPart::SingleQuoted { value, .. } => {
                text_contains_echo_backslash_escape(value.slice(source), echo_escape_is_core_family)
            }
            WordPart::DoubleQuoted { parts, .. } => {
                word_parts_contain_echo_backslash_escape(parts, source, true)
            }
            _ => false,
        })
}

pub(super) fn echo_escape_is_core_family(byte: u8) -> bool {
    matches!(
        byte,
        b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'x' | b'0'..=b'9'
    )
}

pub(super) fn echo_escape_is_quote_like(byte: u8) -> bool {
    matches!(byte, b'`' | b'\'')
}

pub(super) fn literal_double_backslash_touches_double_quoted_fragment(
    parts: &[WordPartNode],
    index: usize,
    rendered_text: &str,
) -> bool {
    (trailing_backslash_count(rendered_text) >= 2
        && parts
            .get(index + 1)
            .is_some_and(|part| matches!(part.kind, WordPart::DoubleQuoted { .. })))
        || (leading_backslash_count(rendered_text) >= 2
            && index
                .checked_sub(1)
                .and_then(|prev| parts.get(prev))
                .is_some_and(|part| matches!(part.kind, WordPart::DoubleQuoted { .. })))
}

pub(super) fn leading_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .take_while(|byte| **byte == b'\\')
        .count()
}

pub(super) fn trailing_backslash_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
}

pub(super) fn text_contains_echo_double_backslash(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        if index.saturating_sub(run_start) >= 2
            && bytes.get(index).is_some_and(|next| *next != b'"')
        {
            return true;
        }
    }

    false
}

pub(super) fn text_contains_echo_backslash_escape(text: &str, is_sensitive: fn(u8) -> bool) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < bytes.len() && bytes[index] == b'\\' {
            index += 1;
        }

        let Some(&escaped_byte) = bytes.get(index) else {
            continue;
        };

        if index > run_start && is_sensitive(escaped_byte) {
            return true;
        }
    }

    false
}

#[derive(Clone, Copy)]
pub(super) struct WordFactLookup<'facts, 'a> {
    pub(super) nodes: &'facts [WordNode<'a>],
    pub(super) occurrences: &'facts [WordOccurrence],
    pub(super) word_index: &'facts FxHashMap<FactSpan, SmallVec<[WordOccurrenceId; 2]>>,
    pub(super) fact_store: &'facts FactStore<'a>,
    pub(super) source: &'a str,
    pub(super) line_index: &'facts LineIndex,
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(super) fn build_echo_to_sed_substitution_spans<'a>(
    commands: CommandFacts<'_, 'a>,
    pipelines: &[PipelineFact<'a>],
    backticks: &[BacktickFragmentFact],
    lookup: WordFactLookup<'_, 'a>,
) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut pipeline_sed_command_ids = FxHashSet::default();

    for pipeline in pipelines {
        if let Some(span) = sc2001_like_pipeline_span(commands, pipeline, backticks, lookup) {
            spans.push(span);
            if let Some(last_segment) = pipeline.last_segment() {
                pipeline_sed_command_ids.insert(last_segment.command_id());
            }
        }
    }

    spans.extend(commands.iter().filter_map(|command| {
        (!pipeline_sed_command_ids.contains(&command.id()))
            .then(|| {
                sc2001_like_here_string_span(command, backticks, lookup.source, lookup.line_index)
            })
            .flatten()
    }));

    sort_and_dedup_spans(&mut spans);
    spans
}

pub(super) fn sc2001_like_pipeline_span<'a>(
    commands: CommandFacts<'_, 'a>,
    pipeline: &PipelineFact<'a>,
    backticks: &[BacktickFragmentFact],
    lookup: WordFactLookup<'_, 'a>,
) -> Option<Span> {
    let [left_segment, right_segment] = pipeline.segments() else {
        return None;
    };

    let left = command_fact_ref(commands, left_segment.command_id());
    let right = command_fact_ref(commands, right_segment.command_id());

    if !command_is_plain_named(left, "echo") || !command_is_plain_named(right, "sed") {
        return None;
    }

    if left
        .options()
        .echo()
        .and_then(|echo| echo.portability_flag_word())
        .is_some()
    {
        return None;
    }

    if !command_has_sc2001_like_sed_script(right, backticks, lookup.source) {
        return None;
    }

    let [argument] = left.body_args() else {
        return None;
    };

    let word_fact = word_occurrence_with_context(
        lookup.nodes,
        lookup.occurrences,
        lookup.word_index,
        argument.span,
        WordFactContext::Expansion(ExpansionContext::CommandArgument),
    )?;

    if occurrence_static_text(lookup.nodes, word_fact, lookup.source).is_some() {
        return None;
    }

    let derived = word_node_derived(&lookup.nodes[word_fact.node_id.index()]);
    if derived.scalar_expansion_spans.is_empty()
        && derived.array_expansion_spans.is_empty()
        && derived.command_substitution_spans.is_empty()
    {
        return None;
    }

    if derived.has_literal_affixes
        && !word_occurrence_is_pure_quoted_dynamic(
            lookup.nodes,
            word_fact,
            lookup.fact_store,
            lookup.source,
        )
    {
        return None;
    }

    if command_is_inside_backtick_fragment(right, backticks)
        && word_occurrence_is_escaped_double_quoted_dynamic(
            lookup.nodes,
            word_fact,
            lookup.fact_store,
            lookup.source,
        )
    {
        return sc2001_like_backtick_pipeline_span(
            commands,
            pipeline,
            right,
            lookup.source,
            lookup.line_index,
        );
    }

    if word_occurrence_is_escaped_double_quoted_dynamic(
        lookup.nodes,
        word_fact,
        lookup.fact_store,
        lookup.source,
    ) {
        return None;
    }

    Some(pipeline_span_with_shellcheck_tail(
        commands,
        pipeline,
        lookup.source,
    ))
}

pub(super) fn sc2001_like_here_string_span(
    command: CommandFactRef<'_, '_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
    line_index: &LineIndex,
) -> Option<Span> {
    if !command_is_plain_named(command, "sed") {
        return None;
    }

    if !command_has_sc2001_like_sed_script(command, backticks, source) {
        return None;
    }

    let mut here_strings = command
        .redirect_facts()
        .iter()
        .filter(|redirect| redirect.redirect().kind == RedirectKind::HereString);
    here_strings.next()?;
    if here_strings.next().is_some() {
        return None;
    }

    if command_is_inside_backtick_fragment(command, backticks) {
        return sc2001_like_backtick_command_span(command, source, line_index);
    }

    command_span_with_redirects_and_shellcheck_tail(command, source)
}

pub(super) fn command_is_plain_named(command: CommandFactRef<'_, '_>, name: &str) -> bool {
    command.effective_name_is(name) && command.wrappers().is_empty()
}

pub(super) fn sc2001_like_backtick_pipeline_span(
    commands: CommandFacts<'_, '_>,
    pipeline: &PipelineFact<'_>,
    sed_command: CommandFactRef<'_, '_>,
    source: &str,
    line_index: &LineIndex,
) -> Option<Span> {
    let first_segment = pipeline.first_segment()?;
    let first = command_fact_ref(commands, first_segment.command_id());
    let start = first.body_name_word()?.span.start;
    let end = sc2001_like_backtick_sed_script_end(sed_command.body_args(), source, line_index)?;
    Some(Span::from_positions(start, end))
}

pub(super) fn sc2001_like_backtick_command_span(
    command: CommandFactRef<'_, '_>,
    source: &str,
    line_index: &LineIndex,
) -> Option<Span> {
    let start = command.body_name_word()?.span.start;
    let end = sc2001_like_backtick_sed_script_end(command.body_args(), source, line_index)?;
    Some(Span::from_positions(start, end))
}

pub(super) fn sc2001_like_backtick_sed_script_end(
    args: &[&Word],
    source: &str,
    line_index: &LineIndex,
) -> Option<Position> {
    let script_words = match args {
        [flag, words @ ..] if static_word_text(flag, source).as_deref() == Some("-e") => words,
        _ => args,
    };

    let raw_script_end = match script_words {
        [script] => backtick_sed_script_content_end_offset(
            script.span.slice(source),
            script.span.end.offset,
        )?,
        [first, .., last]
            if first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"") =>
        {
            last.span.end.offset.checked_sub(2)?
        }
        _ => return None,
    };

    let trim_chars = sc2001_like_backtick_sed_script_trim_chars(script_words, source)?;
    let end_offset = rewind_offset_by_chars(source, raw_script_end, trim_chars)?;
    Locator::new(source, line_index).position_at_offset(end_offset)
}

pub(super) fn backtick_sed_script_content_end_offset(text: &str, end_offset: usize) -> Option<usize> {
    if text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\"") {
        end_offset.checked_sub(2)
    } else if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        end_offset.checked_sub(1)
    } else {
        Some(end_offset)
    }
}

pub(super) fn sc2001_like_backtick_sed_script_trim_chars(
    script_words: &[&Word],
    source: &str,
) -> Option<usize> {
    let uses_backtick_escaped_double_quotes =
        backtick_sed_script_uses_escaped_double_quotes(script_words, source);
    let text = sed_script_text(
        script_words,
        source,
        SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes,
    )?;
    let text = text.as_ref();

    let remainder = text.strip_prefix('s')?;
    let delimiter = remainder.chars().next()?;
    let delimiter_len = delimiter.len_utf8();

    let pattern_start = 1 + delimiter_len;
    let (pattern_end, _) = find_sed_substitution_section(text, pattern_start, delimiter)?;
    let replacement_start = pattern_end + delimiter_len;
    let (replacement_end, _) = find_sed_substitution_section(text, replacement_start, delimiter)?;
    let pattern = &text[pattern_start..pattern_end];
    let replacement = &text[replacement_start..replacement_end];
    let flags = &text[replacement_end + delimiter_len..];

    let mut trim_chars = if flags.is_empty() {
        if uses_backtick_escaped_double_quotes && replacement_start == replacement_end {
            2
        } else {
            1
        }
    } else {
        flags.chars().count()
    };

    // ShellCheck trims one additional character for these legacy backtick sed sites
    // when the match pattern itself ends with an escaped dollar.
    if pattern.ends_with(r"\$") {
        trim_chars += 1;
        if replacement.starts_with(r"\\") {
            trim_chars += 1;
        }
    }

    Some(trim_chars)
}

pub(super) fn backtick_sed_script_uses_escaped_double_quotes(script_words: &[&Word], source: &str) -> bool {
    match script_words {
        [script] => {
            let text = script.span.slice(source);
            text.len() >= 4 && text.starts_with("\\\"") && text.ends_with("\\\"")
        }
        [first, .., last] => {
            first.span.slice(source).starts_with("\\\"")
                && last.span.slice(source).ends_with("\\\"")
        }
        _ => false,
    }
}

pub(super) fn rewind_offset_by_chars(source: &str, mut offset: usize, count: usize) -> Option<usize> {
    if offset > source.len() || !source.is_char_boundary(offset) {
        return None;
    }

    for _ in 0..count {
        let prefix = source.get(..offset)?;
        let (_, ch) = prefix.char_indices().next_back()?;
        offset = offset.checked_sub(ch.len_utf8())?;
    }

    Some(offset)
}

pub(super) fn command_has_sc2001_like_sed_script(
    command: CommandFactRef<'_, '_>,
    backticks: &[BacktickFragmentFact],
    source: &str,
) -> bool {
    command
        .options()
        .sed()
        .is_some_and(|sed| sed.has_single_substitution_script())
        || (command_is_inside_backtick_fragment(command, backticks)
            && sed_has_single_substitution_script(
                command.body_args(),
                source,
                SedScriptQuoteMode::AllowBacktickEscapedDoubleQuotes,
            ))
}

pub(super) fn command_is_inside_backtick_fragment(
    command: CommandFactRef<'_, '_>,
    backticks: &[BacktickFragmentFact],
) -> bool {
    let span = command.span();
    backticks.iter().any(|fragment| {
        let fragment_span = fragment.span();
        fragment_span.start.offset <= span.start.offset
            && fragment_span.end.offset >= span.end.offset
    })
}


pub(super) fn parse_wait_command(args: &[&Word], source: &str) -> WaitCommandFacts {
    let mut option_spans = Vec::new();
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            break;
        }

        if text.starts_with('-') && text != "-" {
            option_spans.push(word.span);
            index += 1;
            if wait_option_consumes_argument(&text) {
                index += 1;
            }
            continue;
        }

        break;
    }

    WaitCommandFacts {
        option_spans: option_spans.into_boxed_slice(),
    }
}

pub(super) fn wait_option_consumes_argument(text: &str) -> bool {
    let Some(flags) = text.strip_prefix('-') else {
        return false;
    };
    let Some(p_index) = flags.find('p') else {
        return false;
    };

    p_index + 1 == flags.len()
}

pub(super) fn parse_mapfile_command(
    args: &[&Word],
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
) -> MapfileCommandFacts {
    let mut input_fd = Some(0);
    let mut index = 0;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" || !text.starts_with('-') || text == "-" || text.starts_with("--") {
            break;
        }

        let flags = &text[1..];
        let mut recognized = true;

        for (offset, flag) in flags.char_indices() {
            if !matches!(flag, 't' | 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's') {
                recognized = false;
                break;
            }

            if !mapfile_option_takes_argument(flag) {
                continue;
            }

            let remainder = &flags[offset + flag.len_utf8()..];
            let argument = if remainder.is_empty() {
                index += 1;
                args.get(index)
                    .and_then(|next| static_word_text(next, source))
            } else {
                Some(remainder.into())
            };

            if flag == 'u' {
                input_fd = argument.and_then(|value| value.parse::<i32>().ok());
            }

            break;
        }

        if !recognized {
            break;
        }

        index += 1;
    }

    if args
        .get(index)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("--")
    {
        index += 1;
    }

    let target_name_uses = args
        .get(index)
        .filter(|word| !word_starts_with_literal_dash(word, source))
        .map(|word| comparable_read_target_name_uses(word, semantic, source))
        .unwrap_or_default();

    MapfileCommandFacts {
        input_fd,
        target_name_uses,
    }
}

pub(super) fn mapfile_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'u' | 'C' | 'c' | 'd' | 'n' | 'O' | 's')
}

pub(super) fn parse_expr_command(args: &[&Word], source: &str) -> Option<ExprCommandFacts> {
    let (string_helper_kind, string_helper_span) = expr_string_helper(args, source)
        .map_or((None, None), |(kind, span)| (Some(kind), Some(span)));

    Some(ExprCommandFacts {
        uses_arithmetic_operator: !expr_uses_string_form(args, source),
        string_helper_kind,
        string_helper_span,
    })
}

pub(super) fn expr_uses_string_form(args: &[&Word], source: &str) -> bool {
    matches!(
        args.first()
            .and_then(|word| static_word_text(word, source))
            .as_deref(),
        Some("length" | "index" | "match" | "substr")
    ) || args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        .is_some_and(|text| matches!(text, ":" | "=" | "!=" | "<" | ">" | "<=" | ">=" | "=="))
}

pub(super) fn expr_string_helper(args: &[&Word], source: &str) -> Option<(ExprStringHelperKind, Span)> {
    let word = args.first()?;
    let kind = match static_word_text(word, source).as_deref() {
        Some("length") => ExprStringHelperKind::Length,
        Some("index") => ExprStringHelperKind::Index,
        Some("match") => ExprStringHelperKind::Match,
        Some("substr") => ExprStringHelperKind::Substr,
        _ => return None,
    };

    Some((kind, word.span))
}

pub(super) fn parse_exit_command<'a>(command: &'a Command, source: &str) -> Option<ExitCommandFacts<'a>> {
    let Command::Builtin(BuiltinCommand::Exit(exit)) = command else {
        return None;
    };
    let Some(status_word) = exit.code.as_ref() else {
        return Some(ExitCommandFacts {
            status_word: None,
            is_numeric_literal: false,
            status_is_static: false,
            status_has_literal_content: false,
        });
    };
    let status_text = static_word_text(status_word, source);

    Some(ExitCommandFacts {
        status_word: Some(status_word),
        is_numeric_literal: status_text.as_deref().is_some_and(|text| {
            !text.is_empty() && text.chars().all(|character| character.is_ascii_digit())
        }),
        status_is_static: status_text.is_some(),
        status_has_literal_content: word_contains_literal_content(status_word, source),
    })
}

pub(super) fn word_contains_literal_content(word: &Word, source: &str) -> bool {
    word_parts_contain_literal_content(&word.parts, source)
}

pub(super) fn word_parts_contain_literal_content(parts: &[WordPartNode], source: &str) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::Literal(text) => !text.as_str(source, part.span).is_empty(),
        WordPart::SingleQuoted { value, .. } => !value.slice(source).is_empty(),
        WordPart::DoubleQuoted { parts, .. } => word_parts_contain_literal_content(parts, source),
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

pub(super) fn detect_sudo_family_invoker(
    command: &Command,
    normalized: &NormalizedCommand<'_>,
    source: &str,
) -> Option<SudoFamilyInvoker> {
    let Command::Simple(command) = command else {
        return None;
    };
    let body_start = normalized.body_span.start.offset;
    let scan_all_words = normalized.body_words.is_empty();

    std::iter::once(&command.name)
        .chain(command.args.iter())
        // Unresolved sudo-family wrappers intentionally keep the wrapper marker
        // even when there is no statically known inner command.
        .take_while(|word| scan_all_words || word.span.start.offset < body_start)
        .filter_map(|word| static_word_text(word, source))
        .map(|word| word.strip_prefix('\\').unwrap_or(word.as_ref()).to_owned())
        .filter_map(|word| match word.as_str() {
            "sudo" => Some(SudoFamilyInvoker::Sudo),
            "doas" => Some(SudoFamilyInvoker::Doas),
            "run0" => Some(SudoFamilyInvoker::Run0),
            _ => None,
        })
        .last()
}

pub(super) fn single_quoted_literal_exempt_argument(
    command_name: Option<&str>,
    shell_dialect: shuck_parser::ShellDialect,
    args: &[Word],
    arg_index: usize,
    body_arg_start: usize,
    word: &Word,
    source: &str,
) -> bool {
    let Some(command_name) = command_name else {
        return false;
    };

    let Some(body_args) = args.get(body_arg_start..) else {
        return false;
    };
    let Some(relative_arg_index) = arg_index.checked_sub(body_arg_start) else {
        return false;
    };

    match command_name {
        "alias" => static_word_text(word, source).is_some_and(|text| text.contains('=')),
        "eval" => true,
        "git filter-branch" | "mumps -run %XCMD" | "mumps -run LOOP%XCMD" => true,
        "docker" | "podman" | "oc" => {
            container_shell_command_argument_index(body_args, source) == Some(relative_arg_index)
                || format_option_argument_index(body_args, source) == Some(relative_arg_index)
                || format_option_value_word(body_args, relative_arg_index, source)
        }
        "dpkg-query" => {
            dpkg_query_format_argument_index(body_args, source) == Some(relative_arg_index)
                || dpkg_query_format_option_value_word(body_args, relative_arg_index, source)
        }
        "jq" => jq_literal_argument_index(body_args, source).contains(&relative_arg_index),
        "rename" => rename_program_argument_index(body_args, source) == Some(relative_arg_index),
        "rg" => rg_pattern_argument_index(body_args, source) == Some(relative_arg_index),
        "sched" if shell_dialect == shuck_parser::ShellDialect::Zsh => {
            sched_command_argument_index(body_args, source) == Some(relative_arg_index)
        }
        "sh" | "bash" | "dash" | "ksh" | "zsh" => {
            shell_command_argument_index(body_args, source) == Some(relative_arg_index)
        }
        "ssh" => ssh_remote_command_argument_index(body_args, source).is_some_and(|index| {
            relative_arg_index >= index
                && static_word_text(word, source).is_some_and(|text| text.as_ref() != "-t")
        }),
        "unset" => true,
        "xprop" => xprop_value_argument_index(body_args, source) == Some(relative_arg_index),
        "zstyle" if shell_dialect == shuck_parser::ShellDialect::Zsh => {
            zstyle_eval_argument_index(body_args, source) == Some(relative_arg_index)
        }
        _ if command_name.ends_with("awk") => {
            awk_literal_argument_index(body_args, source).contains(&relative_arg_index)
        }
        _ if command_name.starts_with("perl") => {
            perl_program_argument_index(body_args, source).contains(&relative_arg_index)
        }
        _ => false,
    }
}

fn sched_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let time = static_word_text(&pair[0], source)?;
        let starts_like_delay = time
            .as_ref()
            .strip_prefix('+')
            .is_some_and(|tail| tail.chars().next().is_some_and(|ch| ch.is_ascii_digit()));
        let starts_like_absolute_time = time
            .as_ref()
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit());
        (starts_like_delay || starts_like_absolute_time).then_some(index + 1)
    })
}

fn zstyle_eval_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(4).enumerate().find_map(|(index, window)| {
        let flag = static_word_text(&window[0], source)?;
        (flag.as_ref() == "-e").then_some(index + 3)
    })
}

pub(super) fn single_quoted_literal_exempt_here_string(command_name: Option<&str>) -> bool {
    matches!(command_name, Some("sh" | "bash" | "dash" | "ksh" | "zsh"))
}

pub(super) fn shell_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        shell_flag_contains_command_string(flag.as_ref()).then_some(index + 1)
    })
}

pub(super) fn awk_literal_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            let raw = args[index].span.slice(source);
            if raw.starts_with("-F") {
                index += 1;
                continue;
            }
            result.push(index);
            index += 1;
            continue;
        };
        match text.as_ref() {
            "--" => {
                result.extend(index + 1..args.len());
                break;
            }
            "-F" | "-f" | "--field-separator" | "--file" => index += 2,
            "-v" | "--assign" => {
                if args.get(index + 1).is_some() {
                    result.push(index + 1);
                }
                index += 2;
            }
            _ if text.starts_with("--assign=") => {
                result.push(index);
                index += 1;
            }
            _ if text.starts_with("-F") && text.len() > 2 => index += 1,
            _ if text.starts_with("--field-separator=") || text.starts_with("--file=") => {
                index += 1;
            }
            _ if text.starts_with('-') && text != "-" => {
                if short_option_cluster_contains_flag(text.as_ref(), 'F')
                    || short_option_cluster_contains_flag(text.as_ref(), 'f')
                {
                    index += 2;
                } else {
                    if short_option_cluster_contains_flag(text.as_ref(), 'v')
                        && args.get(index + 1).is_some()
                    {
                        result.push(index + 1);
                    }
                    index += 1;
                }
            }
            _ => {
                result.push(index);
                index += 1;
            }
        }
    }
    result
}

pub(super) fn jq_literal_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            result.push(index);
            break;
        };
        match text.as_ref() {
            "--" => {
                if args.get(index + 1).is_some() {
                    result.push(index + 1);
                }
                break;
            }
            "-f" | "--from-file" => return result,
            "-L" | "--slurpfile" | "--rawfile" => index += 2,
            "--arg" | "--argjson" => {
                if args.get(index + 2).is_some() {
                    result.push(index + 2);
                }
                index += 3;
            }
            _ if text.starts_with("--from-file=") => return result,
            _ if text.starts_with("-L") && text.len() > 2 => index += 1,
            _ if text.starts_with('-') && text != "-" => index += 1,
            _ => {
                result.push(index);
                break;
            }
        }
    }
    result
}

pub(super) fn perl_program_argument_index(args: &[Word], source: &str) -> Vec<usize> {
    args.windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            let flag = static_word_text(&pair[0], source)?;
            perl_option_takes_program_argument(flag.as_ref()).then_some(index + 1)
        })
        .collect()
}

pub(super) fn perl_option_takes_program_argument(option: &str) -> bool {
    matches!(option, "-e" | "-E")
        || (option.starts_with('-')
            && !option.starts_with("--")
            && option.chars().any(|character| matches!(character, 'e' | 'E')))
}

pub(super) fn rename_program_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.iter()
        .position(|word| static_word_text(word, source).is_none_or(|text| !text.starts_with('-')))
}

pub(super) fn ssh_remote_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let mut index = 0usize;
    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            return args.get(index + 1).map(|_| index + 1);
        };

        if text == "--" {
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        index += 1;
        if ssh_option_consumes_next_argument(text.as_ref())? {
            args.get(index)?;
            index += 1;
        }
    }

    args.get(index)?;
    args.get(index + 1).map(|_| index + 1)
}

pub(super) fn rg_pattern_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let mut index = 0usize;
    while index < args.len() {
        let text = static_word_text(&args[index], source)?;
        match text.as_ref() {
            "--" => return args.get(index + 1).map(|_| index + 1),
            "-e" | "--regexp" => return args.get(index + 1).map(|_| index + 1),
            "-f" | "--file" => return None,
            _ if text.starts_with("--regexp=") => return Some(index),
            _ if text.starts_with("--file=") => return None,
            _ if text.starts_with('-') && text != "-" => {
                index += if rg_option_consumes_next_argument(text.as_ref()) {
                    2
                } else {
                    1
                };
            }
            _ => return Some(index),
        }
    }
    None
}

pub(super) fn rg_option_consumes_next_argument(option: &str) -> bool {
    matches!(
        option,
        "-A" | "--after-context"
            | "-B"
            | "--before-context"
            | "-C"
            | "--context"
            | "-g"
            | "--glob"
            | "--iglob"
            | "-m"
            | "--max-count"
            | "-t"
            | "--type"
            | "-T"
            | "--type-not"
            | "--sort"
            | "--sort-files"
            | "--threads"
    )
}

pub(super) fn container_shell_command_argument_index(args: &[Word], source: &str) -> Option<usize> {
    let run_index = args
        .iter()
        .position(|word| static_word_text(word, source).as_deref() == Some("run"))?;
    let mut index = run_index + 1;
    let mut entrypoint_shell = None;

    while index < args.len() {
        let Some(text) = static_word_text(&args[index], source) else {
            break;
        };

        match text.as_ref() {
            "--" => {
                index += 1;
                break;
            }
            "--entrypoint" => {
                entrypoint_shell = args
                    .get(index + 1)
                    .and_then(|word| static_word_text(word, source))
                    .filter(|value| shell_command_name(value.as_ref()))
                    .map(|_| ());
                index += 2;
            }
            _ if text.starts_with("--entrypoint=") => {
                entrypoint_shell = shell_command_name(&text["--entrypoint=".len()..]).then_some(());
                index += 1;
            }
            _ if text.starts_with('-') && text != "-" => {
                index += if container_run_option_consumes_next_argument(text.as_ref()) {
                    2
                } else {
                    1
                };
            }
            _ => break,
        }
    }

    args.get(index)?;

    if entrypoint_shell.is_some() {
        return shell_command_argument_index(args.get(index + 1..).unwrap_or_default(), source)
            .map(|relative| index + 1 + relative);
    }

    let shell_index = (index + 1..args.len()).find(|candidate| {
        static_word_text(&args[*candidate], source).is_some_and(|text| shell_command_name(&text))
    })?;
    shell_command_argument_index(args.get(shell_index + 1..).unwrap_or_default(), source)
        .map(|relative| shell_index + 1 + relative)
}

pub(super) fn shell_command_name(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "dash" | "ksh" | "zsh")
}

pub(super) fn container_run_option_consumes_next_argument(option: &str) -> bool {
    matches!(
        option,
        "-a" | "--attach"
            | "--add-host"
            | "--annotation"
            | "--blkio-weight"
            | "--blkio-weight-device"
            | "-c"
            | "--cpu-shares"
            | "--cpus"
            | "--cpuset-cpus"
            | "--cpuset-mems"
            | "--device"
            | "--dns"
            | "--dns-option"
            | "--dns-search"
            | "-e"
            | "--env"
            | "--env-file"
            | "--expose"
            | "--gpus"
            | "-h"
            | "--hostname"
            | "--ip"
            | "--ip6"
            | "-l"
            | "--label"
            | "--label-file"
            | "--log-driver"
            | "--log-opt"
            | "--mount"
            | "--name"
            | "--network"
            | "--network-alias"
            | "-p"
            | "--publish"
            | "--pull"
            | "--restart"
            | "--stop-signal"
            | "--stop-timeout"
            | "--ulimit"
            | "-u"
            | "--user"
            | "--userns"
            | "-v"
            | "--volume"
            | "--volumes-from"
            | "-w"
            | "--workdir"
    )
}

pub(super) fn format_option_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        matches!(flag.as_ref(), "-f" | "--format" | "--template").then_some(index + 1)
    })
}

pub(super) fn format_option_value_word(args: &[Word], arg_index: usize, source: &str) -> bool {
    static_word_text(&args[arg_index], source).is_some_and(|text| {
        matches!(
            text.as_ref(),
            _ if text.starts_with("--format=") || text.starts_with("--template=")
        )
    })
}

pub(super) fn dpkg_query_format_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(2).enumerate().find_map(|(index, pair)| {
        let flag = static_word_text(&pair[0], source)?;
        matches!(flag.as_ref(), "-f" | "--showformat").then_some(index + 1)
    })
}

pub(super) fn dpkg_query_format_option_value_word(args: &[Word], arg_index: usize, source: &str) -> bool {
    static_word_text(&args[arg_index], source).is_some_and(|text| {
        text.starts_with("-f=")
            || text.starts_with("--showformat=")
            || (text.starts_with("-f") && text.len() > 2)
    })
}

pub(super) fn xprop_value_argument_index(args: &[Word], source: &str) -> Option<usize> {
    args.windows(3).enumerate().find_map(|(index, triple)| {
        let flag = static_word_text(&triple[0], source)?;
        (flag.as_ref() == "-set").then_some(index + 2)
    })
}

pub(super) fn trap_action_word<'a>(command: &'a Command, source: &str) -> Option<&'a Word> {
    let Command::Simple(command) = command else {
        return None;
    };

    trap_action_word_from_simple_command(command, source)
}

pub(super) fn trap_action_word_from_simple_command<'a>(
    command: &'a SimpleCommand,
    source: &str,
) -> Option<&'a Word> {
    if static_word_text(&command.name, source).as_deref() != Some("trap") {
        return None;
    }

    let mut start = 0usize;

    if let Some(first) = command
        .args
        .first()
        .and_then(|word| static_word_text(word, source))
    {
        match first.as_ref() {
            "-p" | "-l" => return None,
            "--" => start = 1,
            _ => {}
        }
    }

    let action = command.args.get(start)?;
    command.args.get(start + 1)?;
    Some(action)
}
