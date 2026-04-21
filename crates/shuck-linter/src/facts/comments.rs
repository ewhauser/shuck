#[derive(Debug, Clone, Copy, Default)]
struct ShebangHeaderFacts {
    indented_shebang_span: Option<Span>,
    indented_shebang_indent_span: Option<Span>,
    space_after_hash_bang_span: Option<Span>,
    space_after_hash_bang_whitespace_span: Option<Span>,
    shebang_not_on_first_line_span: Option<Span>,
    missing_shebang_line_span: Option<Span>,
    duplicate_shebang_flag_span: Option<Span>,
    non_absolute_shebang_span: Option<Span>,
    enables_errexit: bool,
}

fn build_shebang_header_facts(source: &str) -> ShebangHeaderFacts {
    let mut source_lines = source_lines_with_offsets(source).enumerate();
    let Some((_, (first_line_offset, first_line_text))) = source_lines.next() else {
        return ShebangHeaderFacts::default();
    };
    let first_line = first_line_text.trim_end_matches('\r');
    let mut indented_shebang_span = None;
    let mut indented_shebang_indent_span = None;
    let mut space_after_hash_bang_span = None;
    let mut space_after_hash_bang_whitespace_span = None;
    let mut shebang_not_on_first_line_span = None;

    for (line_index, (offset, raw_line)) in
        std::iter::once((0, (first_line_offset, first_line_text))).chain(source_lines)
    {
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
        }

        if shebang_candidate || !header_like {
            break;
        }
    }

    let first_line_shellcheck_shell_directive = first_line
        .strip_prefix('#')
        .map(str::trim_start)
        .is_some_and(|comment| {
            comment
                .to_ascii_lowercase()
                .starts_with("shellcheck shell=")
        });
    let missing_shebang_line_span = (!first_line.trim_start().starts_with("#!")
        && space_after_hash_bang_span.is_none()
        && shebang_not_on_first_line_span.is_none()
        && !first_line_shellcheck_shell_directive
        && first_line.trim_start().starts_with('#'))
    .then(|| line_span(1, first_line_offset, first_line));

    let shebang_words = first_line
        .strip_prefix("#!")
        .map(parse_shebang_words)
        .unwrap_or_default();

    let duplicate_shebang_flag_span =
        shebang_duplicate_flag(&shebang_words).map(|_| line_span(1, first_line_offset, first_line));

    let non_absolute_shebang_span = shebang_words.first().and_then(|interpreter| {
        if interpreter.starts_with('/') || *interpreter == "/usr/bin/env" {
            return None;
        }
        if has_header_shellcheck_shell_directive(source) {
            return None;
        }
        Some(line_span(1, first_line_offset, first_line))
    });
    let enables_errexit = first_nonempty_source_line(source)
        .and_then(|(_, line)| line.trim_end_matches('\r').strip_prefix("#!"))
        .map(parse_shebang_words)
        .is_some_and(|words| shebang_enables_errexit(&words));

    ShebangHeaderFacts {
        indented_shebang_span,
        indented_shebang_indent_span,
        space_after_hash_bang_span,
        space_after_hash_bang_whitespace_span,
        shebang_not_on_first_line_span,
        missing_shebang_line_span,
        duplicate_shebang_flag_span,
        non_absolute_shebang_span,
        enables_errexit,
    }
}

fn source_line_is_header_like(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn source_line_has_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.starts_with("#!") || shebang_space_after_hash_in_line(trimmed).is_some()
}

fn source_line_has_leading_whitespace_before_shebang_candidate(line: &str) -> bool {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    trimmed.len() != line.len() && source_line_has_shebang_candidate(line)
}

fn source_line_leading_whitespace_len(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    (trimmed.len() != line.len()).then_some(line.len() - trimmed.len())
}

fn shebang_space_after_hash_in_line(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start_matches(char::is_whitespace);
    let leading_whitespace_len = line.len().saturating_sub(trimmed.len());
    let rest = trimmed.strip_prefix('#')?;
    let whitespace_len = rest
        .len()
        .saturating_sub(rest.trim_start_matches(char::is_whitespace).len());
    (whitespace_len > 0 && rest[whitespace_len..].starts_with('!'))
        .then_some((leading_whitespace_len + 1, whitespace_len))
}

fn point_span(line_number: usize, column: usize, offset: usize) -> Span {
    Span::at(Position {
        line: line_number,
        column,
        offset,
    })
}

fn line_prefix_span(line_number: usize, offset: usize, prefix: &str) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(prefix);
    Span::from_positions(start, end)
}

fn line_slice_span(
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

fn parse_shebang_words(shebang: &str) -> Vec<&str> {
    shebang.split_whitespace().collect()
}

fn source_lines_with_offsets(source: &str) -> impl Iterator<Item = (usize, &str)> + '_ {
    source
        .split_inclusive('\n')
        .scan(0usize, |offset, raw_line| {
            let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
            let line_offset = *offset;
            *offset += raw_line.len();
            Some((line_offset, line))
        })
}

fn first_nonempty_source_line(source: &str) -> Option<(usize, &str)> {
    source_lines_with_offsets(source).find(|(_, line)| !line.trim().is_empty())
}

fn shebang_duplicate_flag<'a>(shebang_words: &[&'a str]) -> Option<&'a str> {
    let mut seen = FxHashSet::default();

    shebang_words
        .iter()
        .copied()
        .skip(1)
        .find(|word| word.starts_with('-') && !seen.insert(*word))
}

fn shebang_enables_errexit(shebang_words: &[&str]) -> bool {
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

fn shebang_short_option_cluster_enables_errexit(word: &str) -> bool {
    let Some(flags) = word.strip_prefix('-') else {
        return false;
    };

    if word == "-" || word == "--" || word.starts_with("--") {
        return false;
    }

    flags.chars().all(|char| char.is_ascii_alphabetic()) && flags.contains('e')
}

fn line_span(line_number: usize, offset: usize, line: &str) -> Span {
    let start = Position {
        line: line_number,
        column: 1,
        offset,
    };
    let end = start.advanced_by(line);
    Span::from_positions(start, end)
}

fn build_commented_continuation_comment_spans(source: &str, indexer: &Indexer) -> Vec<Span> {
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

fn build_trailing_directive_comment_spans(
    file: &File,
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
            if shellcheck_directive_can_apply_to_following_command(source, file, comment.range) {
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

fn case_item_label_comment(
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

fn has_header_shellcheck_shell_directive(source: &str) -> bool {
    for line in source.lines().skip(1) {
        let trimmed = line.trim_start();
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
