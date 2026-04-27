use super::*;

pub fn array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_array_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_array_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_array_expansion_spans(&word.parts, false, false, spans);
}

pub fn all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_all_elements_array_expansion_spans(&word.parts, source, spans);
}

pub fn word_has_all_elements_array_expansion_syntax(word: &Word) -> bool {
    parts_have_all_elements_array_expansion_syntax(&word.parts)
}

pub fn direct_all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_direct_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_direct_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_direct_all_elements_array_expansion_spans(&word.parts, word.span, source, spans);
}

pub fn unquoted_all_elements_array_expansion_part_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_all_elements_array_expansion_part_spans(word, source, &mut spans);
    spans
}

pub fn collect_unquoted_all_elements_array_expansion_part_spans(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_unquoted_all_elements_array_expansion_spans(&word.parts, false, source, spans);
}

pub fn word_all_elements_array_slice_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_slice_spans(&word.parts, false, false, &mut spans);
    spans
}

pub fn word_quoted_all_elements_array_slice_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_all_elements_array_slice_spans(&word.parts, false, true, &mut spans);
    spans
}

pub fn word_has_quoted_all_elements_array_slice(word: &Word) -> bool {
    !word_quoted_all_elements_array_slice_spans(word).is_empty()
}

pub fn word_has_direct_all_elements_array_expansion_in_source(word: &Word, source: &str) -> bool {
    !direct_all_elements_array_expansion_part_spans(word, source).is_empty()
}

pub fn word_all_elements_array_slice_span_in_source(word: &Word, source: &str) -> Option<Span> {
    word_all_elements_array_slice_spans(word)
        .into_iter()
        .find(|span| !span_is_escaped(*span, source))
}

pub fn word_quoted_unindexed_bash_source_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let mut spans = Vec::new();
    collect_quoted_unindexed_bash_source_spans(&word.parts, false, source, &mut spans);
    spans.into_iter().next()
}

pub fn unquoted_array_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_array_expansion_part_spans(word, &mut spans);
    spans
}

pub fn collect_unquoted_array_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_array_expansion_spans(&word.parts, false, true, spans);
}

pub fn word_unquoted_star_parameter_spans(word: &Word, unquoted_array_spans: &[Span]) -> Vec<Span> {
    word.parts_with_spans()
        .filter_map(|(part, span)| {
            (unquoted_array_spans.contains(&span) && part_uses_star_splat(part)).then_some(span)
        })
        .collect()
}

pub fn word_unquoted_star_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_star_splat_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_quoted_star_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_quoted_star_splat_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_positional_at_splat_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_positional_at_splat_spans(&word.parts, &mut spans);
    spans
}

pub fn word_is_pure_positional_at_splat(word: &Word) -> bool {
    parts_are_pure_positional_at_splat(&word.parts)
}

pub fn word_folded_positional_at_splat_span(word: &Word) -> Option<Span> {
    let spans = word_positional_at_splat_spans(word);
    if spans.is_empty() {
        return None;
    }
    if spans.len() == 1 && word_has_single_positional_at_splat_part(word) {
        return None;
    }

    spans.into_iter().next()
}

pub fn word_has_folded_positional_at_splat(word: &Word) -> bool {
    word_folded_positional_at_splat_span(word).is_some()
}

pub fn word_positional_at_splat_span_in_source(word: &Word, source: &str) -> Option<Span> {
    word_positional_at_splat_spans(word)
        .into_iter()
        .find(|span| !span_is_escaped(*span, source))
}

pub fn word_folded_positional_at_splat_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let spans = word_positional_at_splat_spans(word)
        .into_iter()
        .filter(|span| !span_is_escaped(*span, source))
        .collect::<Vec<_>>();
    let first = spans.first().copied()?;

    if spans.len() == 1
        && (word_has_single_positional_at_splat_part(word)
            || positional_at_splat_is_standalone_expansion(word, source))
    {
        return None;
    }

    Some(first)
}

pub fn word_folded_all_elements_array_span_in_source(word: &Word, source: &str) -> Option<Span> {
    let spans = folded_all_elements_array_candidate_spans(word, source)
        .into_iter()
        .filter(|span| !span_is_escaped(*span, source))
        .collect::<Vec<_>>();
    let first = spans.first().copied()?;

    if spans.len() == 1
        && (word_has_single_folded_all_elements_array_part(word)
            || all_elements_array_expansion_is_standalone(word, source))
    {
        return None;
    }

    Some(first)
}

pub(crate) fn folded_all_elements_array_candidate_spans(word: &Word, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_folded_all_elements_array_candidate_spans(&word.parts, source, &mut spans);
    spans
}

pub(crate) fn collect_folded_all_elements_array_candidate_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_folded_all_elements_array_candidate_spans(parts, source, spans)
            }
            WordPart::Parameter(parameter)
                if parameter_uses_replacement_all_elements_array_expansion(parameter) =>
            {
                spans.push(part.span);
            }
            _ if part_uses_direct_all_elements_array_expansion(&part.kind) => {
                if let Some(span) =
                    normalize_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) =
                    normalize_nested_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_array_expansion_spans(
    parts: &[WordPartNode],
    quoted: bool,
    only_unquoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_array_expansion_spans(parts, true, only_unquoted, spans)
            }
            WordPart::Variable(name)
                if matches!(name.as_str(), "@" | "*") && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ArrayAccess(reference)
                if reference.has_array_selector() && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::Parameter(parameter)
                if parameter_is_array_like(parameter) && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                ..
            } if !matches!(operator, ParameterOp::UseReplacement)
                && reference.has_array_selector()
                && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                ..
            } if !matches!(operator, Some(ParameterOp::UseReplacement))
                && reference.has_array_selector()
                && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::Transformation { reference, .. }
                if reference.has_array_selector() && (!quoted || !only_unquoted) =>
            {
                spans.push(part.span);
            }
            WordPart::ArraySlice { .. } | WordPart::ArrayIndices(_)
                if !quoted || !only_unquoted =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_all_elements_array_expansion_spans(parts, source, spans)
            }
            WordPart::Variable(name) if name.as_str() == "@" => {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::ArrayAccess(reference)
                if matches!(
                    reference
                        .subscript
                        .as_ref()
                        .and_then(|subscript| subscript.selector()),
                    Some(SubscriptSelector::At)
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::ArrayIndices(reference)
                if matches!(
                    reference
                        .subscript
                        .as_ref()
                        .and_then(|subscript| subscript.selector()),
                    Some(SubscriptSelector::At)
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::PrefixMatch {
                kind: PrefixMatchKind::At,
                ..
            } => {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) = normalize_all_elements_array_expansion_span(part.span, source) {
                    spans.push(span);
                }
            }
            WordPart::Variable(name) if name.as_str() == "*" => {}
            _ => {}
        }
    }
}

pub(crate) fn parts_have_all_elements_array_expansion_syntax(parts: &[WordPartNode]) -> bool {
    parts.iter().any(|part| match &part.kind {
        WordPart::SingleQuoted { .. } => false,
        WordPart::DoubleQuoted { parts, .. } => {
            parts_have_all_elements_array_expansion_syntax(parts)
        }
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::At,
            ..
        } => true,
        WordPart::PrefixMatch {
            kind: PrefixMatchKind::Star,
            ..
        } => false,
        WordPart::Parameter(parameter) => {
            parameter_uses_unquoted_all_elements_array_expansion(parameter)
        }
        WordPart::Literal(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ArithmeticExpansion { .. }
        | WordPart::Length(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::IndirectExpansion { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::Transformation { .. }
        | WordPart::Substring { .. }
        | WordPart::ArrayLength(_)
        | WordPart::ZshQualifiedGlob(_) => false,
    })
}

pub(crate) fn collect_unquoted_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    quoted: bool,
    _source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_all_elements_array_expansion_spans(parts, true, _source, spans)
            }
            _ if !quoted && part_uses_unquoted_all_elements_array_expansion(&part.kind) => {
                spans.push(part.span)
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_all_elements_array_slice_spans(
    parts: &[WordPartNode],
    quoted: bool,
    only_quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_all_elements_array_slice_spans(parts, true, only_quoted, spans)
            }
            _ if (!only_quoted || quoted) && part_uses_all_elements_array_slice(&part.kind) => {
                spans.push(part.span)
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_direct_all_elements_array_expansion_spans(
    parts: &[WordPartNode],
    word_span: Span,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        if span_inside_escaped_parameter_template(word_span, part.span, source) {
            continue;
        }
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_direct_all_elements_array_expansion_spans(parts, word_span, source, spans)
            }
            _ if part_uses_direct_all_elements_array_expansion(&part.kind) => {
                if let Some(span) =
                    normalize_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            WordPart::Parameter(parameter)
                if parameter_might_use_all_elements_array_expansion(
                    parameter, part.span, source,
                ) =>
            {
                if let Some(span) =
                    normalize_nested_direct_all_elements_array_expansion_span(part.span, source)
                {
                    spans.push(span);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn span_inside_escaped_parameter_template(
    word_span: Span,
    span: Span,
    source: &str,
) -> bool {
    if span.start.offset < word_span.start.offset || span.start.offset >= word_span.end.offset {
        return false;
    }

    let text = word_span.slice(source);
    let relative_offset = span.start.offset - word_span.start.offset;
    let mut index = 0usize;

    while index < text.len() {
        if text[index..].starts_with("\\${") {
            let dollar_offset = index + '\\'.len_utf8();
            if offset_is_backslash_escaped(word_span.start.offset + dollar_offset, source)
                && let Some(end_offset) = escaped_parameter_template_end(text, dollar_offset)
            {
                let body_start = dollar_offset + "${".len();
                let body_end = end_offset.saturating_sub('}'.len_utf8());
                if relative_offset >= body_start && relative_offset < body_end {
                    return true;
                }
                index = end_offset;
                continue;
            }
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        index += ch.len_utf8();
    }

    false
}

pub(crate) fn escaped_parameter_template_end(text: &str, dollar_offset: usize) -> Option<usize> {
    if dollar_offset >= text.len() || !text[dollar_offset..].starts_with("${") {
        return None;
    }

    let bytes = text.as_bytes();
    let mut index = dollar_offset + "${".len();
    let mut depth = 1usize;
    let mut quote_state = EscapedTemplateQuote::None;

    while index < bytes.len() {
        let byte = bytes[index];
        match quote_state {
            EscapedTemplateQuote::Single => {
                if byte == b'\'' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::Double => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = EscapedTemplateQuote::None;
                }
                index += 1;
                continue;
            }
            EscapedTemplateQuote::None => {}
        }

        match byte {
            b'\\' => {
                index += usize::from(index + 1 < bytes.len()) + 1;
            }
            b'\'' => {
                quote_state = EscapedTemplateQuote::Single;
                index += 1;
            }
            b'"' => {
                quote_state = EscapedTemplateQuote::Double;
                index += 1;
            }
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                depth += 1;
                index += "${".len();
            }
            b'}' => {
                depth -= 1;
                index += '}'.len_utf8();
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index = advance_shell_char(text, index),
        }
    }

    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapedTemplateQuote {
    None,
    Single,
    Double,
}

pub(crate) fn collect_quoted_unindexed_bash_source_spans(
    parts: &[WordPartNode],
    quoted: bool,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_quoted_unindexed_bash_source_spans(parts, true, source, spans)
            }
            WordPart::Variable(name)
                if quoted
                    && name.as_str() == "BASH_SOURCE"
                    && !span_is_escaped(part.span, source) =>
            {
                spans.push(part.span);
            }
            WordPart::Parameter(parameter)
                if quoted
                    && parameter_is_unindexed_bash_source(parameter)
                    && !span_is_escaped(part.span, source) =>
            {
                spans.push(part.span);
            }
            _ => {}
        }
    }
}

pub(crate) fn normalize_all_elements_array_expansion_span(
    span: Span,
    source: &str,
) -> Option<Span> {
    let text = span.slice(source);
    if !span_is_escaped(span, source)
        && (text == "$@" || candidate_is_all_elements_array_expansion(text))
    {
        return Some(span);
    }

    let base_offset = span.start.offset;
    let mut search_from = 0usize;

    while let Some(found) = text[search_from..].find('$') {
        let relative_start = search_from + found;
        let absolute_start = base_offset + relative_start;
        if offset_is_backslash_escaped(absolute_start, source) {
            search_from = relative_start + 1;
            continue;
        }

        let start = position_at_offset(source, absolute_start)?;
        let remainder = &source[absolute_start..];

        if remainder.starts_with("$@") {
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${")
            && let Some(relative_end) = remainder.find('}')
        {
            let candidate = &remainder[..=relative_end];
            if candidate_is_all_elements_array_expansion(candidate) {
                let end = position_at_offset(source, absolute_start + candidate.len())?;
                return Some(Span::from_positions(start, end));
            }
        }

        search_from = relative_start + 1;
    }

    widen_all_elements_array_expansion_span(span, source)
}

pub(crate) fn normalize_direct_all_elements_array_expansion_span(
    span: Span,
    source: &str,
) -> Option<Span> {
    let text = span.slice(source);
    if !span_is_escaped(span, source)
        && (text == "$@" || candidate_is_direct_all_elements_array_expansion(text))
    {
        return Some(span);
    }

    let base_offset = span.start.offset;
    let mut search_from = 0usize;

    while let Some(found) = text[search_from..].find('$') {
        let relative_start = search_from + found;
        let absolute_start = base_offset + relative_start;
        if offset_is_backslash_escaped(absolute_start, source) {
            search_from = relative_start + 1;
            continue;
        }

        let start = position_at_offset(source, absolute_start)?;
        let remainder = &source[absolute_start..];

        if remainder.starts_with("$@") {
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${")
            && let Some(relative_end) = remainder.find('}')
        {
            let candidate = &remainder[..=relative_end];
            if candidate_is_direct_all_elements_array_expansion(candidate) {
                let end = position_at_offset(source, absolute_start + candidate.len())?;
                return Some(Span::from_positions(start, end));
            }
        }

        search_from = relative_start + 1;
    }

    widen_direct_all_elements_array_expansion_span(span, source)
}

pub(crate) fn normalize_nested_direct_all_elements_array_expansion_span(
    span: Span,
    source: &str,
) -> Option<Span> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum QuoteState {
        None,
        Single,
        Double,
    }

    let text = span.slice(source);
    if !text.contains('$') {
        return None;
    }

    let base_offset = span.start.offset;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut nested_braced_depth = 0usize;
    let mut quote_state = QuoteState::None;

    while index < bytes.len() {
        let absolute_start = base_offset + index;
        let byte = bytes[index];

        match quote_state {
            QuoteState::Single if nested_braced_depth > 0 => {
                if byte == b'\'' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::Double if nested_braced_depth > 0 => {
                if byte == b'\\' {
                    index += usize::from(index + 1 < bytes.len()) + 1;
                    continue;
                }
                if byte == b'"' {
                    quote_state = QuoteState::None;
                }
                index += 1;
                continue;
            }
            QuoteState::None if nested_braced_depth > 0 && byte == b'\'' => {
                quote_state = QuoteState::Single;
                index += 1;
                continue;
            }
            QuoteState::None if nested_braced_depth > 0 && byte == b'"' => {
                quote_state = QuoteState::Double;
                index += 1;
                continue;
            }
            QuoteState::None => {}
            QuoteState::Single | QuoteState::Double => {}
        }

        if byte == b'\\' {
            if index + 2 < bytes.len() && bytes[index + 1] == b'$' && bytes[index + 2] == b'{' {
                nested_braced_depth += 1;
                index += 3;
                continue;
            }

            index += usize::from(index + 1 < bytes.len()) + 1;
            continue;
        }

        if byte == b'}' && nested_braced_depth > 0 {
            nested_braced_depth -= 1;
            index += 1;
            continue;
        }

        if byte != b'$' {
            if byte == b'{' && nested_braced_depth > 0 {
                nested_braced_depth += 1;
            }
            index += 1;
            continue;
        }

        if offset_is_backslash_escaped(absolute_start, source) {
            index += 1;
            continue;
        }

        let remainder = &source[absolute_start..];
        if nested_braced_depth == 0 && remainder.starts_with("$@") {
            let start = position_at_offset(source, absolute_start)?;
            let end = position_at_offset(source, absolute_start + "$@".len())?;
            return Some(Span::from_positions(start, end));
        }

        if remainder.starts_with("${") {
            if nested_braced_depth == 0
                && let Some(relative_end) = remainder.find('}')
            {
                let candidate = &remainder[..=relative_end];
                if candidate_is_direct_all_elements_array_expansion(candidate) {
                    let start = position_at_offset(source, absolute_start)?;
                    let end = position_at_offset(source, absolute_start + candidate.len())?;
                    return Some(Span::from_positions(start, end));
                }
            }

            nested_braced_depth += 1;
            index += 2;
            continue;
        }

        index += 1;
    }

    None
}

pub(crate) fn widen_all_elements_array_expansion_span(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    if !text.contains("[@]") {
        return None;
    }

    let start_offset = span.start.offset.checked_sub(2)?;
    if source.as_bytes().get(start_offset..span.start.offset)? != b"${" {
        return None;
    }
    if offset_is_backslash_escaped(start_offset, source) {
        return None;
    }

    let start = position_at_offset(source, start_offset)?;
    let remainder = &source[start_offset..];
    let relative_end = remainder.find('}')?;
    let candidate = &remainder[..=relative_end];
    if !candidate_is_all_elements_array_expansion(candidate) {
        return None;
    }

    let end = position_at_offset(source, start_offset + candidate.len())?;
    Some(Span::from_positions(start, end))
}

pub(crate) fn widen_direct_all_elements_array_expansion_span(
    span: Span,
    source: &str,
) -> Option<Span> {
    let text = span.slice(source);
    if !text.contains("[@]") {
        return None;
    }

    let start_offset = span.start.offset.checked_sub(2)?;
    if source.as_bytes().get(start_offset..span.start.offset)? != b"${" {
        return None;
    }
    if offset_is_backslash_escaped(start_offset, source) {
        return None;
    }

    let start = position_at_offset(source, start_offset)?;
    let remainder = &source[start_offset..];
    let relative_end = remainder.find('}')?;
    let candidate = &remainder[..=relative_end];
    if !candidate_is_direct_all_elements_array_expansion(candidate) {
        return None;
    }

    let end = position_at_offset(source, start_offset + candidate.len())?;
    Some(Span::from_positions(start, end))
}

pub(crate) fn candidate_is_all_elements_array_expansion(candidate: &str) -> bool {
    let Some(inner) = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };

    let (inner, indirect_like) = inner
        .strip_prefix('!')
        .map_or((inner, false), |stripped| (stripped, true));

    let Some(first) = inner.as_bytes().first().copied() else {
        return false;
    };

    if first == b'@' {
        return !indirect_like;
    }

    if !is_name_start(first) {
        return false;
    }

    let bytes = inner.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() && is_name_continue(bytes[index]) {
        index += 1;
    }

    if inner[index..].starts_with("[@]") {
        return true;
    }

    indirect_like && inner[index..].starts_with('@')
}

pub(crate) fn candidate_is_direct_all_elements_array_expansion(candidate: &str) -> bool {
    let Some(mut inner) = candidate
        .strip_prefix("${")
        .and_then(|text| text.strip_suffix('}'))
    else {
        return false;
    };

    if let Some(stripped) = inner.strip_prefix('!') {
        inner = stripped;
    }

    let suffix = if let Some(stripped) = inner.strip_prefix('@') {
        stripped
    } else {
        let Some(first) = inner.as_bytes().first().copied() else {
            return false;
        };
        if !is_name_start(first) {
            return false;
        }

        let bytes = inner.as_bytes();
        let mut index = 1usize;
        while index < bytes.len() && is_name_continue(bytes[index]) {
            index += 1;
        }

        let Some(stripped) = inner[index..].strip_prefix("[@]") else {
            return false;
        };
        stripped
    };

    if suffix.starts_with('+') || suffix.starts_with(":+") {
        return false;
    }

    true
}

pub(crate) fn parameter_is_array_like(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => reference.has_array_selector(),
            BourneParameterExpansion::Indices { .. } => true,
            BourneParameterExpansion::Slice { reference, .. } => reference.has_array_selector(),
            BourneParameterExpansion::Operation {
                reference,
                operator,
                ..
            } => !matches!(operator, ParameterOp::UseReplacement) && reference.has_array_selector(),
            BourneParameterExpansion::Transformation { reference, .. } => {
                reference.has_array_selector()
            }
            _ => false,
        },
        ParameterExpansionSyntax::Zsh(_) => false,
    }
}

pub(crate) fn parameter_might_use_all_elements_array_expansion(
    parameter: &ParameterExpansion,
    span: Span,
    source: &str,
) -> bool {
    if !span.slice(source).contains('@') {
        return false;
    }

    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Length { .. } | BourneParameterExpansion::Indirect { .. } => {
                false
            }
            BourneParameterExpansion::PrefixMatch { kind, .. } => {
                matches!(kind, PrefixMatchKind::At)
            }
            _ => true,
        },
        ParameterExpansionSyntax::Zsh(_) => true,
    }
}

pub(crate) fn parameter_is_scalar_like(parameter: &ParameterExpansion) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference } => !reference.has_array_selector(),
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::PrefixMatch { .. } => true,
            BourneParameterExpansion::Indirect { reference, .. }
            | BourneParameterExpansion::Operation { reference, .. }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                !reference.has_array_selector()
            }
            BourneParameterExpansion::Indices { .. } => false,
            BourneParameterExpansion::Slice { reference, .. } => !reference.has_array_selector(),
        },
        ParameterExpansionSyntax::Zsh(_) => true,
    }
}

pub(crate) fn parameter_uses_replacement_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        }
        | BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator, ParameterOp::UseReplacement)
        }
        BourneParameterExpansion::Access { .. }
        | BourneParameterExpansion::Length { .. }
        | BourneParameterExpansion::Indices { .. }
        | BourneParameterExpansion::PrefixMatch { .. }
        | BourneParameterExpansion::Slice { .. }
        | BourneParameterExpansion::Transformation { .. }
        | BourneParameterExpansion::Indirect { operator: None, .. } => false,
    }
}

pub(crate) fn part_uses_star_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "*",
        WordPart::ArrayAccess(reference) => var_ref_uses_star_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_star_splat(parameter),
        WordPart::ParameterExpansion { reference, .. }
        | WordPart::IndirectExpansion { reference, .. }
        | WordPart::Transformation { reference, .. } => var_ref_uses_star_splat(reference),
        _ => false,
    }
}

pub(crate) fn part_uses_all_elements_array_slice(part: &WordPart) -> bool {
    match part {
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_all_elements_array_slice(parameter),
        _ => false,
    }
}

pub(crate) fn part_uses_positional_at_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) => var_ref_uses_positional_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_uses_positional_at_splat(parameter),
        _ => false,
    }
}

pub(crate) fn part_uses_unquoted_all_elements_array_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => {
            parameter_uses_unquoted_all_elements_array_expansion(parameter)
        }
        _ => false,
    }
}

pub(crate) fn part_uses_direct_all_elements_array_expansion(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) | WordPart::ArrayIndices(reference) => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        WordPart::ArraySlice { reference, .. } => var_ref_uses_all_elements_at_splat(reference),
        WordPart::Parameter(parameter) => {
            parameter_uses_direct_all_elements_array_expansion(parameter)
        }
        _ => false,
    }
}

pub(crate) fn part_is_pure_positional_at_splat(part: &WordPart) -> bool {
    match part {
        WordPart::Variable(name) => name.as_str() == "@",
        WordPart::ArrayAccess(reference) => var_ref_uses_positional_at_splat(reference),
        WordPart::Parameter(parameter) => parameter_is_pure_positional_at_splat(parameter),
        _ => false,
    }
}

pub(crate) fn part_uses_assign_default_operator(part: &WordPart) -> bool {
    match part {
        WordPart::Parameter(parameter) => parameter_uses_assign_default_operator(parameter),
        WordPart::ParameterExpansion { operator, .. }
        | WordPart::IndirectExpansion {
            operator: Some(operator),
            ..
        } => matches!(operator, ParameterOp::AssignDefault),
        _ => false,
    }
}

pub(crate) fn var_ref_uses_star_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "*"
        || matches!(
            reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector()),
            Some(SubscriptSelector::Star)
        )
}

pub(crate) fn var_ref_uses_all_elements_at_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "@"
        || matches!(
            reference
                .subscript
                .as_ref()
                .and_then(|subscript| subscript.selector()),
            Some(SubscriptSelector::At)
        )
}

pub(crate) fn parameter_uses_all_elements_array_slice(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Slice { reference, .. }
            if var_ref_uses_all_elements_at_splat(reference)
    )
}

pub(crate) fn parameter_uses_unquoted_all_elements_array_expansion(
    parameter: &ParameterExpansion,
) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        } => {
            !matches!(operator, ParameterOp::UseReplacement)
                && var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        _ => false,
    }
}

pub(crate) fn parameter_uses_direct_all_elements_array_expansion(
    parameter: &ParameterExpansion,
) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            ..
        } => {
            !matches!(operator, ParameterOp::UseReplacement)
                && var_ref_uses_all_elements_at_splat(reference)
        }
        BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_all_elements_at_splat(reference)
        }
        _ => false,
    }
}

pub(crate) fn parameter_uses_replacement_all_elements_array_expansion(
    parameter: &ParameterExpansion,
) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Operation {
            reference,
            operator: ParameterOp::UseReplacement,
            ..
        } if var_ref_uses_all_elements_at_splat(reference)
    )
}

pub(crate) fn parameter_is_unindexed_bash_source(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    matches!(
        syntax,
        BourneParameterExpansion::Access { reference }
            if reference.name.as_str() == "BASH_SOURCE" && reference.subscript.is_none()
    )
}

pub(crate) fn parameter_uses_star_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. }
        | BourneParameterExpansion::Operation { reference, .. }
        | BourneParameterExpansion::Transformation { reference, .. } => {
            var_ref_uses_star_splat(reference)
        }
        _ => false,
    }
}

pub(crate) fn var_ref_uses_positional_at_splat(reference: &VarRef) -> bool {
    reference.name.as_str() == "@"
}

pub(crate) fn parameter_uses_positional_at_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. }
        | BourneParameterExpansion::Operation { reference, .. } => {
            var_ref_uses_positional_at_splat(reference)
        }
        _ => false,
    }
}

pub(crate) fn parameter_is_pure_positional_at_splat(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Slice { reference, .. } => {
            var_ref_uses_positional_at_splat(reference)
        }
        _ => false,
    }
}

pub(crate) fn parameter_uses_assign_default_operator(parameter: &ParameterExpansion) -> bool {
    let ParameterExpansionSyntax::Bourne(syntax) = &parameter.syntax else {
        return false;
    };

    match syntax {
        BourneParameterExpansion::Operation { operator, .. } => {
            matches!(operator, ParameterOp::AssignDefault)
        }
        BourneParameterExpansion::Indirect {
            operator: Some(operator),
            ..
        } => matches!(operator, ParameterOp::AssignDefault),
        _ => false,
    }
}

pub(crate) fn collect_unquoted_star_splat_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_star_splat_spans(parts, true, spans);
            }
            _ if !quoted && part_uses_star_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

pub(crate) fn collect_quoted_star_splat_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_quoted_star_splat_spans(parts, true, spans);
            }
            _ if quoted && part_uses_star_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

pub(crate) fn collect_positional_at_splat_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => collect_positional_at_splat_spans(parts, spans),
            _ if part_uses_positional_at_splat(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

pub(crate) fn parts_are_pure_positional_at_splat(parts: &[WordPartNode]) -> bool {
    let mut saw_splat = false;

    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => return false,
            WordPart::DoubleQuoted { parts, .. } => {
                if !parts_are_pure_positional_at_splat(parts) {
                    return false;
                }
                saw_splat = true;
            }
            _ if part_is_pure_positional_at_splat(&part.kind) => saw_splat = true,
            _ => return false,
        }
    }

    saw_splat
}

pub(crate) fn word_has_single_positional_at_splat_part(word: &Word) -> bool {
    parts_have_single_positional_at_splat(&word.parts)
}

pub(crate) fn parts_have_single_positional_at_splat(parts: &[WordPartNode]) -> bool {
    let [part] = parts else {
        return false;
    };

    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => parts_have_single_positional_at_splat(parts),
        WordPart::SingleQuoted { .. } => false,
        _ => part_uses_positional_at_splat(&part.kind),
    }
}

pub(crate) fn word_has_single_folded_all_elements_array_part(word: &Word) -> bool {
    parts_have_single_folded_all_elements_array_part(&word.parts)
}

pub(crate) fn parts_have_single_folded_all_elements_array_part(parts: &[WordPartNode]) -> bool {
    let [part] = parts else {
        return false;
    };

    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            parts_have_single_folded_all_elements_array_part(parts)
        }
        WordPart::SingleQuoted { .. } => false,
        WordPart::Parameter(parameter) => {
            part_uses_direct_all_elements_array_expansion(&part.kind)
                || parameter_uses_replacement_all_elements_array_expansion(parameter)
        }
        _ => part_uses_direct_all_elements_array_expansion(&part.kind),
    }
}

pub(crate) fn positional_at_splat_is_standalone_expansion(word: &Word, source: &str) -> bool {
    let text = word.span.slice(source);
    let body = if word.is_fully_double_quoted() {
        let Some(unquoted) = text
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        unquoted
    } else {
        text
    };

    if body == "$@" || body == "${@}" {
        return true;
    }

    if !body.starts_with("${@") || !body.ends_with('}') {
        return false;
    }
    true
}

pub(crate) fn all_elements_array_expansion_is_standalone(word: &Word, source: &str) -> bool {
    if word.parts.len() != 1 {
        return false;
    }

    let text = word.span.slice(source);
    let body = if word.is_fully_double_quoted() {
        let Some(unquoted) = text
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            return false;
        };
        unquoted
    } else {
        text
    };

    folded_all_elements_array_candidate_spans(word, source)
        .first()
        .is_some_and(|span| span.slice(source) == body)
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::{
        all_elements_array_expansion_part_spans, array_expansion_part_spans,
        unquoted_all_elements_array_expansion_part_spans, unquoted_array_expansion_part_spans,
        word_all_elements_array_slice_span_in_source, word_all_elements_array_slice_spans,
        word_folded_all_elements_array_span_in_source, word_folded_positional_at_splat_span,
        word_folded_positional_at_splat_span_in_source,
        word_has_direct_all_elements_array_expansion_in_source,
        word_has_folded_positional_at_splat, word_has_quoted_all_elements_array_slice,
        word_is_pure_positional_at_splat, word_positional_at_splat_span_in_source,
        word_positional_at_splat_spans, word_quoted_all_elements_array_slice_spans,
        word_quoted_star_splat_spans, word_quoted_unindexed_bash_source_span_in_source,
        word_unquoted_star_parameter_spans, word_unquoted_star_splat_spans,
    };
    use crate::facts::word_spans::scalar_expansion_part_spans;

    #[test]
    fn array_expansion_spans_only_return_array_like_parts() {
        let source = "printf '%s\\n' ${arr[@]} ${arr[@]+fallback} ${arr[*]:-fallback} ${arr[*]@Q} ${arr[0]}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .skip(1)
            .flat_map(|word| array_expansion_part_spans(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(
            spans,
            vec!["${arr[@]}", "${arr[*]:-fallback}", "${arr[*]@Q}"]
        );
    }

    #[test]
    fn selector_helpers_distinguish_splats_from_indexed_and_quoted_keys() {
        let source = "printf '%s\\n' ${arr[@]} ${arr[*]} ${arr[0]} ${assoc[\"key\"]}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(command.args.len(), 5);
        assert_eq!(
            array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert_eq!(
            array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[*]}"]
        );
        assert!(array_expansion_part_spans(&command.args[3], source).is_empty());
        assert!(array_expansion_part_spans(&command.args[4], source).is_empty());

        assert!(scalar_expansion_part_spans(&command.args[1], source).is_empty());
        assert!(scalar_expansion_part_spans(&command.args[2], source).is_empty());
        assert_eq!(
            scalar_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}"]
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[4], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${assoc[\"key\"]}"]
        );
    }

    #[test]
    fn all_elements_array_expansion_spans_only_return_at_style_parts() {
        let source =
            "printf '%s\\n' $@ $* \"$@\" \"$*\" ${arr[@]} ${arr[*]} ${arr[@]:1:2} ${arr[*]:1:2}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[2], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[4], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[6], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[7], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:1:2}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[8], source).is_empty());
    }

    #[test]
    fn all_elements_array_expansion_spans_normalize_parser_misalignment() {
        let source = "\
#!/bin/bash
shims=(a)
eval \\
\"conda_shim() {
  case \\\"\\${1##*/}\\\" in
    ${shims[@]}
    *) return 1;;
  esac
}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[1].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = all_elements_array_expansion_part_spans(&command.args[0], source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].slice(source), "${shims[@]}");
        assert_eq!(spans[0].start.column, 5);
        assert_eq!(spans[0].end.column, 16);
    }

    #[test]
    fn all_elements_array_expansion_spans_ignore_escaped_literal_expansions() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\" \\\"\\$@\\\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(all_elements_array_expansion_part_spans(&command.args[2], source).is_empty());
    }

    #[test]
    fn all_elements_array_expansion_spans_track_safe_quoted_name_fanout() {
        let source = "\
printf '%s\\n' ${#arr[@]} ${!arr[@]} ${!cfg@} ${name:-safe[@]} ${arr[@]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(all_elements_array_expansion_part_spans(&command.args[1], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!arr[@]}"]
        );
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!cfg@}"]
        );
        assert!(all_elements_array_expansion_part_spans(&command.args[4], source).is_empty());
        assert_eq!(
            all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
    }

    #[test]
    fn unquoted_all_elements_array_expansion_spans_only_return_unquoted_at_style_parts() {
        let source = "printf '%s\\n' $@ $* \"$@\" \"$*\" ${arr[@]} ${arr[*]} ${arr[@]:1:2} ${arr[*]:1:2} ${!arr[@]} ${arr[@]/#/#} ${arr[@]@Q} ${arr[@]:-fallback} ${arr[@]:+fallback} ${1+\"$@\"}\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[2], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[3], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[4], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[5], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[6], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[7], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:1:2}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[8], source).is_empty()
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[9], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${!arr[@]}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[10], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]/#/#}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[11], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]@Q}"]
        );
        assert_eq!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[12], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]:-fallback}"]
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[13], source).is_empty()
        );
        assert!(
            unquoted_all_elements_array_expansion_part_spans(&command.args[14], source).is_empty()
        );
    }

    #[test]
    fn positional_parameters_are_treated_like_array_splats() {
        let source = "printf '%s\\n' $@ $* \"$@\" \"$*\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            array_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
        assert_eq!(
            array_expansion_part_spans(&command.args[2], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$*"]
        );
    }

    #[test]
    fn word_all_elements_array_slice_spans_track_at_selector_slice_forms_only() {
        let source = "\
printf '%s\\n' ${@:2} ${@:2:3} ${arr[@]:1} ${arr[@]:1:2} ${arr[*]:1} ${*:2} ${arr[0]:1} ${@} ${arr[@]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_all_elements_array_slice_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${@:2}", "${@:2:3}", "${arr[@]:1}", "${arr[@]:1:2}"]
        );
    }

    #[test]
    fn word_quoted_all_elements_array_slice_spans_track_only_quoted_forms() {
        let source = "\
printf '%s\\n' \"${@:2}\" \"x${@:2}y\" \"${arr[@]:1}\" \"${arr[@]:1:2}\" ${@:2} \"${arr[*]:1}\" \"${*:2}\" \"\\${@:2}\" \"${@:-fallback}\" \"${@}\" \"${arr[@]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_quoted_all_elements_array_slice_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${@:2}", "${@:2}", "${arr[@]:1}", "${arr[@]:1:2}"]
        );
        assert!(word_has_quoted_all_elements_array_slice(&command.args[1]));
        assert!(!word_has_quoted_all_elements_array_slice(&command.args[5]));
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_nested_or_scalar_operator_uses() {
        let source = "\
printf '%s\\n' \"$@\" \"${arr[@]}\" \"${arr[@]:1}\" \"${arr[@]:-fallback}\" \"${@:+ok}\" \"${arr[@]:+ok}\" \"${target=\"$@\"}\" \"$(echo \"$@\")\" \"${arr[*]}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(
            matches,
            vec![true, true, true, true, false, false, false, false, false]
        );
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_handles_backslash_parity() {
        let source = "\
printf '%s\\n' \"\\$@\" \"\\\\$@\" \"\\${@:2}\" \"\\\\${@:2}\" \"\\${arr[@]}\" \"\\\\${arr[@]}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true, false, true, false, true]);
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_escaped_parameter_nesting() {
        let source = "\
printf '%s\\n' \"\\${1+'\\\"$@\\\"'}\" \"$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true]);
    }

    #[test]
    fn word_has_direct_all_elements_array_expansion_ignores_quoted_braces_in_escaped_text() {
        let source = "\
printf '%s\\n' \"\\${1+'} \\\"$@\\\"'}\" \"$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let matches = command
            .args
            .iter()
            .skip(1)
            .map(|word| word_has_direct_all_elements_array_expansion_in_source(word, source))
            .collect::<Vec<_>>();

        assert_eq!(matches, vec![false, true]);
    }

    #[test]
    fn word_all_elements_array_slice_span_in_source_ignores_escaped_markers() {
        let source = "printf '%s\\n' \"\\${arr[@]:1}\" \"${arr[@]:1}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_all_elements_array_slice_span_in_source(&command.args[1], source).is_none());
        assert_eq!(
            word_all_elements_array_slice_span_in_source(&command.args[2], source)
                .expect("expected array slice span")
                .slice(source),
            "${arr[@]:1}"
        );
    }

    #[test]
    fn word_quoted_unindexed_bash_source_span_in_source_tracks_scalar_forms() {
        let source = "\
printf '%s\\n' \"$BASH_SOURCE\" \"${BASH_SOURCE}\" \"$(dirname \"$BASH_SOURCE\")\" \"${BASH_SOURCE[0]}\" \"\\$BASH_SOURCE\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[1], source)
                .expect("expected BASH_SOURCE span")
                .slice(source),
            "$BASH_SOURCE"
        );
        assert_eq!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[2], source)
                .expect("expected BASH_SOURCE span")
                .slice(source),
            "${BASH_SOURCE}"
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[3], source).is_none()
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[4], source).is_none()
        );
        assert!(
            word_quoted_unindexed_bash_source_span_in_source(&command.args[5], source).is_none()
        );
    }

    #[test]
    fn word_unquoted_star_splat_spans_tracks_star_selector_forms_only() {
        let source = "\
printf '%s\\n' $* ${*} ${*:1} ${arr[*]} ${arr[*]:1:2} ${arr[*]:-fallback} ${arr[*]@Q} ${!arr[*]} ${arr[@]} ${arr[@]:1} ${arr[0]} \"$*\" \"${arr[*]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_unquoted_star_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "$*",
                "${*}",
                "${*:1}",
                "${arr[*]}",
                "${arr[*]:1:2}",
                "${arr[*]:-fallback}",
                "${arr[*]@Q}"
            ]
        );
    }

    #[test]
    fn word_unquoted_star_parameter_spans_tracks_star_selector_forms_only() {
        let source = "\
printf '%s\\n' $* ${arr[*]} ${arr[*]:1:2} ${arr[*]:-fallback} ${arr[*]@Q} ${!arr[*]} ${arr[@]} ${arr[@]:1} ${arr[0]} \"$*\" \"${arr[*]}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(|word| {
                let unquoted_array_spans = unquoted_array_expansion_part_spans(word, source);
                word_unquoted_star_parameter_spans(word, &unquoted_array_spans)
            })
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                "$*",
                "${arr[*]}",
                "${arr[*]:1:2}",
                "${arr[*]:-fallback}",
                "${arr[*]@Q}"
            ]
        );
    }

    #[test]
    fn word_quoted_star_splat_spans_tracks_double_quoted_star_selector_forms_only() {
        let source = "\
printf '%s\\n' \"$*\" \"${*}\" \"${*:1}\" \"${arr[*]}\" \"${arr[*]:1:2}\" \"${!arr[*]}\" \"${arr[@]}\" \"${arr[@]:1}\" \"$@\" ${arr[*]}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_quoted_star_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["$*", "${*}", "${*:1}", "${arr[*]}", "${arr[*]:1:2}"]
        );
    }

    #[test]
    fn word_positional_at_splat_spans_tracks_positional_forms_only() {
        let source = "\
printf '%s\\n' $@ ${@} ${@:1:2} \"${@}\" \"x$@y\" ${array[@]} ${array[@]:1} $* \"${*}\" ${!@}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_positional_at_splat_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$@", "${@}", "${@:1:2}", "${@}", "$@"]);
    }

    #[test]
    fn word_is_pure_positional_at_splat_rejects_mixed_words() {
        let source = "\
printf '%s\\n' \"$@\" ${@} \"${@:1}\" \"$@$@\" \"prefix$@suffix\" ${array[@]} \"$*\" \"$1\" \"${@:-fallback}\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let pure = command
            .args
            .iter()
            .map(word_is_pure_positional_at_splat)
            .collect::<Vec<_>>();

        assert_eq!(
            pure,
            vec![
                false, true, true, true, true, false, false, false, false, false
            ]
        );
    }

    #[test]
    fn word_folded_positional_at_splat_span_tracks_only_folding_forms() {
        let source = "\
printf '%s\\n' \"$@\" \"${@}\" \"${@:1}\" \"$@$@\" \"$@\"\"$@\" \"x$@y\" x$@y ${@} ${@:1} ${@:-fallback}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let folded = command
            .args
            .iter()
            .filter_map(word_folded_positional_at_splat_span)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(folded, vec!["$@", "$@", "$@", "$@"]);
        assert!(!word_has_folded_positional_at_splat(&command.args[1]));
        assert!(word_has_folded_positional_at_splat(&command.args[4]));
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_ignores_standalone_expansions() {
        let source = "\
exec \"$@\" \"${@}\" \"${@:1}\" \"${@:-fallback}\" \"${@:${args_offset}}\" \"${@//-I\\/usr\\/include/-I${XBPS_CROSS_BASE}\\/usr\\/include}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(command.args.iter().all(|word| {
            word_folded_positional_at_splat_span_in_source(word, source).is_none()
        }));
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_ignores_escaped_positional_markers() {
        let source = "eval command \"\\$@\" \"x\\$@y\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert!(word_folded_positional_at_splat_span_in_source(&command.args[0], source).is_none());
        assert!(word_folded_positional_at_splat_span_in_source(&command.args[1], source).is_none());
    }

    #[test]
    fn word_folded_positional_at_splat_span_in_source_tracks_unescaped_splats_after_escaped_literals()
     {
        let source = "echo \"gvm_pkgset_use: \\$@   => $@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_folded_positional_at_splat_span_in_source(&command.args[0], source)
                .expect("expected folded positional span")
                .slice(source),
            "$@"
        );
    }

    #[test]
    fn word_folded_all_elements_array_span_in_source_tracks_array_splats_in_larger_words() {
        let source = "\
printf '%s\\n' \"${arr[@]}\" \"x${arr[@]}\" \"x${!arr[@]}\" \"x${arr[@]:1}\" \"x${arr[@]/a/b}\" \"x${arr[*]}\" \"\\${arr[@]}\" \"$@\" \"x$@\" \"${arr[@]+ ${arr[*]}}\" \"x${arr[@]+ ${arr[*]}}\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let folded = command
            .args
            .iter()
            .filter_map(|word| word_folded_all_elements_array_span_in_source(word, source))
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            folded,
            vec![
                "${arr[@]}",
                "${!arr[@]}",
                "${arr[@]:1}",
                "${arr[@]/a/b}",
                "$@",
                "${arr[@]+ ${arr[*]}}"
            ]
        );
    }

    #[test]
    fn word_positional_at_splat_span_in_source_tracks_operation_forms() {
        let source = "printf '%s\\n' \"${@:-fallback}\" \"$@\" \"\\$@\"\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            word_positional_at_splat_span_in_source(&command.args[1], source)
                .expect("expected positional span")
                .slice(source),
            "${@:-fallback}"
        );
        assert_eq!(
            word_positional_at_splat_span_in_source(&command.args[2], source)
                .expect("expected positional span")
                .slice(source),
            "$@"
        );
        assert!(word_positional_at_splat_span_in_source(&command.args[3], source).is_none());
    }
}
