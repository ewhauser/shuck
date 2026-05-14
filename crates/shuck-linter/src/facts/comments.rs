use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ShebangHeaderFacts {
    pub(crate) indented_shebang_span: Option<Span>,
    pub(crate) indented_shebang_indent_span: Option<Span>,
    pub(crate) space_after_hash_bang_span: Option<Span>,
    pub(crate) space_after_hash_bang_whitespace_span: Option<Span>,
    pub(crate) shebang_not_on_first_line_span: Option<Span>,
    pub(crate) shebang_not_on_first_line_fix_span: Option<Span>,
    pub(crate) shebang_not_on_first_line_preferred_newline: Option<&'static str>,
    pub(crate) missing_shebang_line_span: Option<Span>,
    pub(crate) duplicate_shebang_flag_span: Option<Span>,
    pub(crate) non_absolute_shebang_span: Option<Span>,
    pub(crate) enables_errexit: bool,
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_shebang_header_facts(locator: Locator<'_>) -> ShebangHeaderFacts {
    let source = locator.source();
    let line_index = locator.line_index();
    let Some(first_line) = source_line(source, line_index, 1) else {
        return ShebangHeaderFacts::default();
    };
    let first_line_offset = first_line.offset;
    let first_line_text = first_line.text.trim_end_matches('\r');
    let mut indented_shebang_span = None;
    let mut indented_shebang_indent_span = None;
    let mut space_after_hash_bang_span = None;
    let mut space_after_hash_bang_whitespace_span = None;
    let mut shebang_not_on_first_line_span = None;
    let mut shebang_not_on_first_line_fix_span = None;
    let mut shebang_not_on_first_line_preferred_newline = None;
    let mut last_line_ending =
        (!first_line.line_ending.is_empty()).then_some(first_line.line_ending);

    for (line_index, source_line) in (1..=line_index.line_count()).filter_map(|line_number| {
        source_line(source, line_index, line_number).map(|line| (line_number - 1, line))
    }) {
        let offset = source_line.offset;
        let raw_line = source_line.text;
        let line = raw_line.trim_end_matches('\r');
        let header_like = source_line_is_header_like(line);
        let shebang_candidate = source_line_has_shebang_candidate(line);
        let indented_candidate = source_line_has_leading_whitespace_before_shebang_candidate(line);
        let leading_whitespace_len = source_line_leading_whitespace_len(line);
        let space_after_hash = shebang_space_after_hash_in_line(line);
        let line_number = line_index + 1;

        if indented_shebang_span.is_none() && indented_candidate {
            indented_shebang_span = Some(point_span(line_number, 1, offset));
            indented_shebang_indent_span = leading_whitespace_len
                .filter(|&len| len > 0)
                .map(|len| line_prefix_span(line_number, offset, &line[..len]));
        }
        if space_after_hash_bang_span.is_none()
            && let Some((space_offset, whitespace_len)) = space_after_hash
        {
            space_after_hash_bang_span = Some(point_span(
                line_number,
                space_offset + 1,
                offset + space_offset,
            ));
            space_after_hash_bang_whitespace_span = Some(line_slice_span(
                line_number,
                offset,
                line,
                space_offset,
                whitespace_len,
            ));
        }
        if line_index > 0 && shebang_candidate {
            shebang_not_on_first_line_span = Some(point_span(line_number, 1, offset));
            shebang_not_on_first_line_fix_span = Some(line_with_ending_span(
                line_number,
                offset,
                raw_line,
                source_line.line_ending,
            ));
            shebang_not_on_first_line_preferred_newline = if source_line.line_ending.is_empty() {
                last_line_ending
            } else {
                Some(source_line.line_ending)
            };
        }

        if shebang_candidate || !header_like {
            break;
        }

        if !source_line.line_ending.is_empty() {
            last_line_ending = Some(source_line.line_ending);
        }
    }

    let first_line_shellcheck_shell_directive = first_line_text
        .strip_prefix('#')
        .map(str::trim_start)
        .is_some_and(|comment| {
            comment
                .to_ascii_lowercase()
                .starts_with("shellcheck shell=")
        });
    let missing_shebang_line_span = (!first_line_text.trim_start().starts_with("#!")
        && space_after_hash_bang_span.is_none()
        && shebang_not_on_first_line_span.is_none()
        && !first_line_shellcheck_shell_directive
        && first_line_text.trim_start().starts_with('#'))
    .then(|| line_span(1, first_line_offset, first_line_text));

    let shebang_words = first_line_text
        .strip_prefix("#!")
        .map(parse_shebang_words)
        .unwrap_or_default();

    let duplicate_shebang_flag_span = shebang_duplicate_flag(&shebang_words)
        .map(|_| line_span(1, first_line_offset, first_line_text));

    let non_absolute_shebang_span = shebang_words.first().and_then(|interpreter| {
        if interpreter.starts_with('/') || *interpreter == "/usr/bin/env" {
            return None;
        }
        if has_header_shellcheck_shell_directive(source, line_index) {
            return None;
        }
        Some(line_span(1, first_line_offset, first_line_text))
    });
    let enables_errexit = first_nonempty_source_line(source, line_index)
        .and_then(|(_, line)| line.trim_end_matches('\r').strip_prefix("#!"))
        .map(parse_shebang_words)
        .is_some_and(|words| shebang_enables_errexit(&words));

    ShebangHeaderFacts {
        indented_shebang_span,
        indented_shebang_indent_span,
        space_after_hash_bang_span,
        space_after_hash_bang_whitespace_span,
        shebang_not_on_first_line_span,
        shebang_not_on_first_line_fix_span,
        shebang_not_on_first_line_preferred_newline,
        missing_shebang_line_span,
        duplicate_shebang_flag_span,
        non_absolute_shebang_span,
        enables_errexit,
    }
}

pub(crate) fn source_line_is_header_like(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with('#')
}

pub(crate) fn source_line_has_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.starts_with("#!") || shebang_space_after_hash_in_line(trimmed).is_some()
}

pub(crate) fn source_line_has_leading_whitespace_before_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.len() != line.len() && source_line_has_shebang_candidate(line)
}

pub(crate) fn source_line_leading_whitespace_len(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    (trimmed.len() != line.len()).then_some(line.len() - trimmed.len())
}

pub(crate) fn shebang_space_after_hash_in_line(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    let leading_whitespace_len = line.len().saturating_sub(trimmed.len());
    let rest = trimmed.strip_prefix('#')?;
    let whitespace_len = rest
        .len()
        .saturating_sub(rest.trim_start_matches(char::is_whitespace).len());
    (whitespace_len > 0 && rest[whitespace_len..].starts_with('!'))
        .then_some((leading_whitespace_len + 1, whitespace_len))
}

pub(crate) fn point_span(line_number: usize, column: usize, offset: usize) -> Span {
    Span::at(Position {
        line: line_number,
        column,
        offset,
    })
}

pub(crate) fn line_prefix_span(line_number: usize, offset: usize, prefix: &str) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(prefix);
    Span::from_positions(start, end)
}

pub(crate) fn line_slice_span(
    line_number: usize,
    line_offset: usize,
    line: &str,
    slice_start: usize,
    slice_len: usize,
) -> Span {
    let line_start = Position {
        line: line_number,
        column: 1,
        offset: line_offset,
    };
    let start = line_start.advanced_by(&line[..slice_start]);
    let end = start.advanced_by(&line[slice_start..slice_start + slice_len]);
    Span::from_positions(start, end)
}

pub(crate) fn line_with_ending_span(
    line_number: usize,
    offset: usize,
    line: &str,
    line_ending: &str,
) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(line).advanced_by(line_ending);
    Span::from_positions(start, end)
}

pub(crate) fn parse_shebang_words(shebang: &str) -> Vec<&str> {
    shebang.split_whitespace().collect()
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceLine<'a> {
    offset: usize,
    text: &'a str,
    line_ending: &'static str,
}

pub(crate) fn source_line<'a>(
    source: &'a str,
    line_index: &LineIndex,
    line: usize,
) -> Option<SourceLine<'a>> {
    let range = line_index.line_range(line, source)?;
    let offset = usize::from(range.start());
    let raw_text = range.slice(source);
    let has_newline = source.as_bytes().get(usize::from(range.end())) == Some(&b'\n');
    let (text, line_ending) = if has_newline {
        if let Some(text) = raw_text.strip_suffix('\r') {
            (text, "\r\n")
        } else {
            (raw_text, "\n")
        }
    } else {
        (raw_text, "")
    };

    Some(SourceLine {
        offset,
        text,
        line_ending,
    })
}

pub(crate) fn first_nonempty_source_line<'a>(
    source: &'a str,
    line_index: &LineIndex,
) -> Option<(usize, &'a str)> {
    (1..=line_index.line_count())
        .filter_map(|line_number| source_line(source, line_index, line_number))
        .find(|line| !line.text.trim().is_empty())
        .map(|line| (line.offset, line.text))
}

pub(crate) fn shebang_duplicate_flag<'a>(shebang_words: &[&'a str]) -> Option<&'a str> {
    let mut seen = FxHashSet::default();

    shebang_words
        .iter()
        .copied()
        .skip(1)
        .find(|word| word.starts_with('-') && !seen.insert(*word))
}

pub(crate) fn shebang_enables_errexit(shebang_words: &[&str]) -> bool {
    let mut words = shebang_words.iter().copied().peekable();
    while let Some(word) = words.next() {
        if shebang_short_option_cluster_enables_errexit(word) {
            return true;
        }
        if word == "-o" && matches!(words.peek(), Some(&"errexit")) {
            return true;
        }
        if word == "-oerrexit" {
            return true;
        }
    }

    false
}

pub(crate) fn shebang_short_option_cluster_enables_errexit(word: &str) -> bool {
    let Some(flags) = word.strip_prefix('-') else {
        return false;
    };

    if word == "-" || word == "--" || word.starts_with("--") {
        return false;
    }

    flags.chars().all(|char| char.is_ascii_alphabetic()) && flags.contains('e')
}

pub(crate) fn line_span(line_number: usize, offset: usize, line: &str) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(line);
    Span::from_positions(start, end)
}

#[cfg_attr(shuck_profiling, inline(never))]
pub(crate) fn build_commented_continuation_comment_spans(
    source: &str,
    indexer: &Indexer,
) -> Vec<Span> {
    let line_index = indexer.line_index();
    let comment_index = indexer.comment_index();

    indexer
        .continuation_line_starts()
        .iter()
        .filter_map(|&line_start_offset| {
            let line = line_index.line_number(line_start_offset);
            let comment = comment_index
                .comments_on_line(line)
                .iter()
                .find(|comment| comment.is_own_line)?;
            let line_start = usize::from(line_index.line_start(line)?);
            let line_end = usize::from(line_index.line_range(line, source)?.end());
            let comment_start = usize::from(comment.range.start());
            if comment_start < line_start || comment_start >= line_end || line_end > source.len() {
                return None;
            }
            let comment_text = &source[comment_start..line_end];
            let trimmed_comment_text = comment_text.trim_end_matches([' ', '\t', '\r']);
            if !trimmed_comment_text.ends_with('\\') {
                return None;
            }
            let caret_offset = comment_start + trimmed_comment_text.len();

            let line_start_position = Position {
                line,
                column: 1,
                offset: line_start,
            };
            let caret = line_start_position.advanced_by(&source[line_start..caret_offset]);
            Some(Span::at(caret))
        })
        .collect()
}

pub(crate) fn build_escaped_dash_command_name_spans(
    source: &str,
    line_index: &LineIndex,
    region_index: &RegionIndex,
    command_name_offsets: &[usize],
) -> Vec<Span> {
    (1..=line_index.line_count())
        .filter_map(|line_number| {
            let source_line = source_line(source, line_index, line_number)?;
            let line = source_line.text;
            let marker_start = line
                .len()
                .saturating_sub(line.trim_start_matches(char::is_whitespace).len());
            let rest = &line[marker_start..];
            if !rest.starts_with("\\-") {
                return None;
            }
            if previous_line_ends_with_unescaped_continuation(source, line_index, line_number) {
                return None;
            }

            let marker_offset = source_line.offset + marker_start;
            if command_name_offsets.binary_search(&marker_offset).is_err() {
                return None;
            }

            let marker_text_size = TextSize::from(marker_offset as u32);
            if region_index.is_heredoc(marker_text_size) || region_index.is_quoted(marker_text_size)
            {
                return None;
            }

            let marker_len = rest
                .find(escaped_dash_command_name_token_delimiter)
                .unwrap_or(rest.len());
            Some(line_slice_span(
                line_number,
                source_line.offset,
                line,
                marker_start,
                marker_len,
            ))
        })
        .collect()
}

fn escaped_dash_command_name_token_delimiter(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>')
}

fn previous_line_ends_with_unescaped_continuation(
    source: &str,
    line_index: &LineIndex,
    line_number: usize,
) -> bool {
    let Some(previous_line_number) = line_number.checked_sub(1) else {
        return false;
    };
    if previous_line_number == 0 {
        return false;
    }

    let Some(previous_line) = source_line(source, line_index, previous_line_number) else {
        return false;
    };
    let trimmed = previous_line.text.trim_end_matches([' ', '\t', '\r']);
    let backslash_count = trimmed
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    backslash_count % 2 == 1
}

pub(crate) fn build_comment_double_quote_nesting_spans(
    source: &str,
    indexer: &Indexer,
) -> Vec<Span> {
    let line_index = indexer.line_index();

    (1..=line_index.line_count())
        .filter_map(|line_number| {
            let source_line = source_line(source, line_index, line_number)?;
            let line = source_line.text.trim_end_matches('\r');
            let comment_start = line.find('#')?;
            let comment_offset = source_line.offset + comment_start;
            if indexer
                .region_index()
                .is_heredoc(TextSize::from(comment_offset as u32))
            {
                return None;
            }
            if !line[..comment_start].trim().is_empty() {
                return None;
            }

            let comment_text = &line[comment_start..];
            let parameter_start = comment_text.find("$0")?;
            let quoted_positional = comment_text[parameter_start..]
                .find("\"$@\"")
                .map(|offset| parameter_start + offset)?;
            let expansion_start = comment_start + quoted_positional + 1;
            Some(line_slice_span(
                line_number,
                source_line.offset,
                line,
                expansion_start,
                2,
            ))
        })
        .collect()
}

pub(crate) fn build_trailing_directive_comment_spans(
    directive_attachment_facts: &crate::suppression::DirectiveAttachmentFacts,
    case_items: &[CaseItemFact<'_>],
    source: &str,
    indexer: &Indexer,
) -> Vec<Span> {
    let line_index = indexer.line_index();

    indexer
        .comment_index()
        .comments()
        .iter()
        .filter_map(|comment| {
            if comment.is_own_line {
                return None;
            }

            let line = line_index.line_number(comment.range.start());
            let line_start = usize::from(line_index.line_start(line)?);
            let line_end = usize::from(line_index.line_range(line, source)?.end());
            let comment_start = usize::from(comment.range.start());
            let comment_end = usize::from(comment.range.end())
                .min(line_end)
                .min(source.len());
            if comment_start < line_start || comment_start >= comment_end {
                return None;
            }
            let comment_text = &source[comment_start..comment_end];
            if !is_inline_shellcheck_directive(comment_text) {
                return None;
            }
            if case_item_label_comment(case_items, line, comment_start) {
                return None;
            }
            if directive_attachment_facts.can_apply_to_following_command(comment.range) {
                return None;
            }

            let line_start_position = Position {
                line,
                column: 1,
                offset: line_start,
            };
            let start = line_start_position.advanced_by(&source[line_start..comment_start]);
            let end = start.advanced_by("#");
            Some(Span::from_positions(start, end))
        })
        .collect()
}

pub(crate) fn case_item_label_comment(
    case_items: &[CaseItemFact<'_>],
    line: usize,
    comment_start: usize,
) -> bool {
    case_items.iter().any(|case_item| {
        let Some(pattern) = case_item.item().patterns.last() else {
            return false;
        };

        if pattern.span.end.line != line || comment_start < pattern.span.end.offset {
            return false;
        }

        let Some(stmt) = case_item.item().body.first() else {
            return true;
        };

        stmt.span.start.line != line
    })
}

pub(crate) fn has_header_shellcheck_shell_directive(source: &str, line_index: &LineIndex) -> bool {
    for line_number in 2..=line_index.line_count() {
        let Some(source_line) = source_line(source, line_index, line_number) else {
            continue;
        };
        let trimmed = source_line.text.trim_end_matches('\r').trim_start();
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            let body = comment.trim_start().to_ascii_lowercase();
            if body.starts_with("shellcheck shell=") {
                return true;
            }
            continue;
        }
        break;
    }

    false
}
