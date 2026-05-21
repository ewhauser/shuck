pub(crate) use shuck_ast::raw_shell::{QuoteState, RawShellScanner, shell_comment_can_start};

pub(crate) fn leading_shell_indent(line: &str) -> &str {
    let indent_end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t'))
        .map_or(line.len(), |(index, _)| index);
    &line[..indent_end]
}

pub(crate) struct HeredocStart<'line> {
    pub(crate) delimiter: &'line str,
    pub(crate) strip_tabs: bool,
    pub(crate) operator_end: usize,
}

pub(crate) fn heredoc_start(line: &str) -> Option<HeredocStart<'_>> {
    let marker = line.find("<<")?;
    let after_marker = &line[marker + 2..];
    if after_marker.starts_with('<') {
        return None;
    }
    let (strip_tabs, after_marker) = if let Some(rest) = after_marker.strip_prefix('-') {
        (true, rest)
    } else {
        (false, after_marker)
    };
    let delimiter = after_marker
        .split_whitespace()
        .next()?
        .trim_matches(['\'', '"']);
    (!delimiter.is_empty()).then_some(HeredocStart {
        delimiter,
        strip_tabs,
        operator_end: marker + if strip_tabs { 3 } else { 2 },
    })
}

pub(crate) fn common_indent_prefix<'a>(left: &'a str, right: &str) -> &'a str {
    let len = left
        .as_bytes()
        .iter()
        .zip(right.as_bytes())
        .take_while(|(left, right)| left == right)
        .count();
    &left[..len]
}

pub(crate) fn refine_common_indent(common: &mut Option<String>, indent: &str) -> bool {
    *common = Some(match common.take() {
        Some(previous) => common_indent_prefix(&previous, indent).to_string(),
        None => indent.to_string(),
    });
    common.as_deref() == Some("")
}

pub(crate) fn common_nonempty_shell_indent<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut common: Option<String> = None;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_shell_indent(line);
        if indent.is_empty() || refine_common_indent(&mut common, indent) {
            return String::new();
        }
    }
    common.unwrap_or_default()
}

pub(crate) fn line_without_continuation_backslash(line: &str) -> Option<&str> {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    let prefix = trimmed.strip_suffix('\\')?;
    Some(prefix.trim_end_matches([' ', '\t', '\r']))
}

pub(crate) fn redirect_operator_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start).copied()? {
        b'>' => Some(match bytes.get(start + 1).copied() {
            Some(b'>' | b'|' | b'&') => start + 2,
            _ => start + 1,
        }),
        b'<' => Some(match bytes.get(start + 1).copied() {
            Some(b'<' | b'>' | b'&') => {
                if bytes.get(start + 2) == Some(&b'<') {
                    start + 3
                } else {
                    start + 2
                }
            }
            _ => start + 1,
        }),
        _ => None,
    }
}

pub(crate) fn skip_single_quoted(source: &str, offset: usize, upper: usize) -> usize {
    RawShellScanner::bounded(source, upper).skip_single_quoted_body(offset)
}

pub(crate) fn skip_double_quoted(source: &str, offset: usize, upper: usize) -> usize {
    RawShellScanner::bounded(source, upper).skip_double_quoted_body(offset)
}

pub(crate) fn skip_escaped_or_quoted(
    source: &str,
    offset: usize,
    upper: usize,
    ch: char,
) -> Option<usize> {
    debug_assert_eq!(source[offset..].chars().next(), Some(ch));
    RawShellScanner::bounded(source, upper).skip_escaped_or_quoted_at(offset)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RawShellText<'source> {
    text: &'source str,
}

impl<'source> RawShellText<'source> {
    pub(crate) fn new(text: &'source str) -> Self {
        Self { text }
    }

    pub(crate) fn matching_command_substitution_close(&self, body_start: usize) -> Option<usize> {
        RawShellScanner::new(self.text).matching_command_substitution_close(body_start)
    }

    pub(crate) fn next_command_substitution(&self, index: usize) -> Option<(usize, usize)> {
        RawShellScanner::new(self.text).next_command_substitution(index)
    }

    pub(crate) fn has_unclosed_command_substitution_open(&self) -> bool {
        let bytes = self.text.as_bytes();
        let mut quote = QuoteState::default();
        let mut index = 0usize;

        while index + 1 < self.text.len() {
            let Some(ch) = self.text[index..].chars().next() else {
                break;
            };
            let next_index = index + ch.len_utf8();
            if quote.consume_shell_word_char(ch) {
                index = next_index;
                continue;
            }
            if !quote.in_single_quotes()
                && bytes[index] == b'$'
                && bytes[index + 1] == b'('
                && bytes.get(index + 2).is_none_or(|byte| *byte != b'(')
            {
                let Some(close_offset) = self.matching_command_substitution_close(index + 2) else {
                    return true;
                };
                index = close_offset + 1;
                continue;
            }
            index = next_index;
        }

        false
    }

    pub(crate) fn normalize_pipeline_continuations(&self) -> Option<String> {
        let trailing = normalize_raw_trailing_pipe_continuations(self.text);
        let leading =
            normalize_raw_leading_pipe_continuations(trailing.as_deref().unwrap_or(self.text));
        leading.or(trailing)
    }

    pub(crate) fn line_ends_with_continuation_operator(&self) -> bool {
        line_ends_with_raw_continuation_operator(self.text)
    }

    pub(crate) fn line_without_trailing_pipe_continuation(&self) -> Option<&'source str> {
        let prefix = line_without_continuation_backslash(self.text)?;
        RawShellText::new(prefix)
            .line_ends_with_continuation_operator()
            .then_some(prefix)
    }

    pub(crate) fn leading_pipe_continuation(
        &self,
    ) -> Option<(&'source str, &'static str, &'source str)> {
        leading_pipe_continuation(self.text)
    }

    pub(crate) fn trim_command_substitution_open_padding(&self) -> String {
        let mut rendered = String::with_capacity(self.text.len());
        let mut cursor = 0usize;
        let mut index = 0usize;
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;
        let mut escaped = false;

        while index < self.text.len() {
            let Some(ch) = self.text[index..].chars().next() else {
                break;
            };
            let next_index = index + ch.len_utf8();
            if escaped {
                escaped = false;
            } else if ch == '\\' && !in_single_quotes {
                escaped = true;
            } else if ch == '\'' && !in_double_quotes {
                in_single_quotes = !in_single_quotes;
            } else if ch == '"' && !in_single_quotes {
                in_double_quotes = !in_double_quotes;
            } else if !in_single_quotes && self.text[index..].starts_with("$(") {
                rendered.push_str(&self.text[cursor..index + 2]);
                index += 2;
                while self.text[index..].starts_with([' ', '\t']) {
                    index += 1;
                }
                cursor = index;
                continue;
            }
            index = next_index;
        }
        rendered.push_str(&self.text[cursor..]);
        rendered
    }

    pub(crate) fn starts_with_redirect_continuation(&self) -> bool {
        let trimmed = self.text.trim_start_matches([' ', '\t']);
        let bytes = trimmed.as_bytes();
        let mut index = 0;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        match bytes.get(index) {
            Some(b'<' | b'>') => true,
            Some(b'&') => bytes.get(index + 1) == Some(&b'>'),
            _ => false,
        }
    }

    pub(crate) fn continuation_inside_unclosed_substitution(&self) -> bool {
        let Some(before_continuation) = self.text.strip_suffix('\\') else {
            return false;
        };

        RawShellScanner::new(before_continuation)
            .has_unclosed_substitution_before(before_continuation.len())
    }

    pub(crate) fn quoted_backslash_continuation(&self) -> bool {
        let mut chars = self.text.chars().peekable();
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;
        while let Some(ch) = chars.next() {
            if ch == '\'' && !in_double_quotes {
                in_single_quotes = !in_single_quotes;
                continue;
            }
            if ch == '"' && !in_single_quotes {
                in_double_quotes = !in_double_quotes;
                continue;
            }
            if ch == '\\' {
                let mut probe = chars.clone();
                let escaped_newline = match probe.next() {
                    Some('\n') => true,
                    Some('\r') => probe.next().is_some_and(|next| next == '\n'),
                    _ => false,
                };
                if escaped_newline {
                    return in_single_quotes || in_double_quotes;
                }
                if !in_single_quotes {
                    chars.next();
                }
            }
        }
        false
    }
}

pub(crate) fn normalize_raw_pipeline_continuations(text: &str) -> Option<String> {
    RawShellText::new(text).normalize_pipeline_continuations()
}

pub(crate) fn line_ends_with_raw_continuation_operator(line: &str) -> bool {
    let code = trailing_comment_start(line)
        .map(|comment_start| &line[..comment_start])
        .unwrap_or(line);
    let trimmed = code.trim_end_matches([' ', '\t', '\r']);
    line_ends_with_pipeline_operator(trimmed) || trimmed.ends_with("&&") || trimmed.ends_with("||")
}

pub(crate) fn matching_raw_command_substitution_close(
    raw: &str,
    body_start: usize,
) -> Option<usize> {
    RawShellText::new(raw).matching_command_substitution_close(body_start)
}

fn line_ends_with_pipeline_operator(line: &str) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    trimmed.ends_with("|&") || (trimmed.ends_with('|') && !trimmed.ends_with("||"))
}

fn normalize_raw_trailing_pipe_continuations(text: &str) -> Option<String> {
    let mut lines = text
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut changed = false;

    for line in &mut lines {
        let Some(prefix) = RawShellText::new(line)
            .line_without_trailing_pipe_continuation()
            .map(str::to_string)
        else {
            continue;
        };
        *line = prefix;
        changed = true;
    }

    changed.then(|| lines.join("\n"))
}

fn normalize_raw_leading_pipe_continuations(text: &str) -> Option<String> {
    let mut lines = text
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut changed = false;

    for index in 0..lines.len().saturating_sub(1) {
        let Some(prefix) = line_without_continuation_backslash(&lines[index]).map(str::to_string)
        else {
            continue;
        };
        let Some((indent, operator, rest)) = RawShellText::new(&lines[index + 1])
            .leading_pipe_continuation()
            .map(|(indent, operator, rest)| {
                (
                    indent.to_string(),
                    operator,
                    rest.trim_start_matches([' ', '\t', '\r']).to_string(),
                )
            })
        else {
            continue;
        };

        lines[index] = format!("{prefix} {operator}");
        lines[index + 1] = format!("{indent}{rest}");
        changed = true;
    }

    changed.then(|| lines.join("\n"))
}

fn leading_pipe_continuation(line: &str) -> Option<(&str, &'static str, &str)> {
    let content_start = line
        .char_indices()
        .find_map(|(index, ch)| (!matches!(ch, ' ' | '\t')).then_some(index))
        .unwrap_or(line.len());
    let indent = &line[..content_start];
    let rest = &line[content_start..];
    if let Some(remainder) = rest.strip_prefix("|&") {
        Some((indent, "|&", remainder))
    } else if let Some(remainder) = rest.strip_prefix("||") {
        Some((indent, "||", remainder))
    } else if let Some(remainder) = rest.strip_prefix("&&") {
        Some((indent, "&&", remainder))
    } else {
        rest.strip_prefix('|')
            .map(|remainder| (indent, "|", remainder))
    }
}

fn trailing_comment_start(line: &str) -> Option<usize> {
    RawShellScanner::new(line).find_comment(0, line.len())
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedHeredocTail {
    pub(crate) delimiter: String,
    pub(crate) strip_tabs: bool,
    pub(crate) command_indent: String,
}

impl RenderedHeredocTail {
    pub(crate) fn body_line<'a>(&self, line: &'a str) -> &'a str {
        if self.strip_tabs {
            line
        } else if let Some(stripped) = line.strip_prefix(&self.command_indent)
            && stripped == self.delimiter
        {
            stripped
        } else {
            line
        }
    }

    pub(crate) fn closes(&self, line: &str) -> bool {
        let line = self.body_line(line);
        if self.strip_tabs {
            line.trim_start_matches('\t') == self.delimiter
        } else {
            line == self.delimiter
        }
    }
}

pub(crate) fn rendered_shell_text_has_heredoc_tail(text: &str) -> bool {
    text.lines()
        .any(|line| rendered_heredoc_tail_start(line).is_some())
}

pub(crate) fn rendered_heredoc_tail_start(line: &str) -> Option<RenderedHeredocTail> {
    let start = heredoc_start(line)?;
    Some(RenderedHeredocTail {
        delimiter: start.delimiter.to_string(),
        strip_tabs: start.strip_tabs,
        command_indent: leading_shell_indent(line).to_string(),
    })
}

pub(crate) fn normalize_rendered_heredoc_start_spacing(line: &str) -> Option<String> {
    let operator_end = heredoc_start(line)?.operator_end;
    let target_start = line[operator_end..]
        .char_indices()
        .find_map(|(index, ch)| (!matches!(ch, ' ' | '\t' | '\r')).then_some(operator_end + index))
        .unwrap_or(line.len());
    if target_start == operator_end || target_start == line.len() {
        return None;
    }

    let mut normalized = String::with_capacity(line.len());
    normalized.push_str(&line[..operator_end]);
    normalized.push_str(&line[target_start..]);
    Some(normalized)
}

#[derive(Clone, Copy)]
pub(crate) enum CommandSubstitutionPipelineContinuation {
    None,
    Comment,
    StructuralPipe { line_started_in_quote: bool },
}

#[derive(Default)]
pub(crate) struct RawLineQuoteState {
    single_quoted: bool,
    double_quoted: bool,
    escaped: bool,
}

impl RawLineQuoteState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn in_quote(&self) -> bool {
        self.single_quoted || self.double_quoted
    }

    fn operator_is_unquoted_at(&mut self, line: &str, operator_offset: usize) -> bool {
        for (offset, ch) in line.char_indices() {
            if offset == operator_offset {
                return !self.in_quote() && !self.escaped;
            }

            self.consume(ch);
        }

        false
    }

    fn consume(&mut self, ch: char) {
        if self.escaped {
            self.escaped = false;
            return;
        }

        match ch {
            '\\' if !self.single_quoted => self.escaped = true,
            '\'' if !self.double_quoted => {
                self.single_quoted = !self.single_quoted;
            }
            '"' if !self.single_quoted => {
                self.double_quoted = !self.double_quoted;
            }
            _ => {}
        }
    }
}

pub(crate) fn command_substitution_pipeline_stage_continuation(
    line: &str,
    was_pipeline_stage: bool,
    quote_state: &mut RawLineQuoteState,
) -> CommandSubstitutionPipelineContinuation {
    let content = line.trim_end_matches(['\r', '\n']);
    let scan_start = command_substitution_context_start(content).unwrap_or(0);
    if scan_start > 0 {
        quote_state.reset();
    }
    let content = &content[scan_start..];

    if was_pipeline_stage
        && !quote_state.in_quote()
        && content.trim_start_matches([' ', '\t']).starts_with('#')
    {
        return CommandSubstitutionPipelineContinuation::Comment;
    }
    let line_started_in_quote = quote_state.in_quote();
    if rendered_line_ends_with_command_substitution_continuation_in_quote_state(
        content,
        quote_state,
    ) {
        CommandSubstitutionPipelineContinuation::StructuralPipe {
            line_started_in_quote,
        }
    } else {
        CommandSubstitutionPipelineContinuation::None
    }
}

pub(crate) fn rendered_line_opens_command_substitution_pipeline(line: &str) -> bool {
    if !rendered_line_ends_with_structural_pipe_continuation(line) {
        return false;
    }

    command_substitution_context_start(line).is_some()
        && line
            .bytes()
            .take_while(|byte| matches!(*byte, b' ' | b'\t'))
            .any(|byte| byte == b' ')
}

pub(crate) fn rendered_line_ends_with_structural_pipe_continuation(line: &str) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    let Some((pipe_offset, scan_end)) = final_pipe_operator_bounds(trimmed) else {
        return false;
    };
    let scan_start = command_substitution_context_start(&trimmed[..pipe_offset]).unwrap_or(0);

    final_pipe_operator_is_unquoted(&trimmed[scan_start..scan_end])
}

pub(crate) fn command_substitution_context_start(line: &str) -> Option<usize> {
    line.rfind("$(")
        .or_else(|| line.rfind("<("))
        .or_else(|| line.rfind(">("))
        .map(|offset| offset.saturating_add(2))
}

fn rendered_line_ends_with_command_substitution_continuation_in_quote_state(
    line: &str,
    quote_state: &mut RawLineQuoteState,
) -> bool {
    let trimmed = line.trim_end_matches([' ', '\t', '\r']);
    let operator_offset =
        final_command_substitution_continuation_operator_bounds(trimmed).map(|(offset, _)| offset);
    let Some(operator_offset) = operator_offset else {
        for ch in trimmed.chars() {
            quote_state.consume(ch);
        }
        quote_state.escaped = false;
        return false;
    };

    let operator_is_unquoted = quote_state.operator_is_unquoted_at(trimmed, operator_offset);
    quote_state.escaped = false;
    operator_is_unquoted
}

fn final_pipe_operator_bounds(line: &str) -> Option<(usize, usize)> {
    if line.ends_with("|&") {
        Some((line.len().saturating_sub(2), line.len()))
    } else if line.ends_with('|') && !line.ends_with("||") {
        Some((line.len().saturating_sub(1), line.len()))
    } else {
        None
    }
}

fn final_command_substitution_continuation_operator_bounds(line: &str) -> Option<(usize, usize)> {
    if line.ends_with("||") || line.ends_with("&&") || line.ends_with("|&") {
        Some((line.len().saturating_sub(2), line.len()))
    } else if line.ends_with('|') {
        Some((line.len().saturating_sub(1), line.len()))
    } else {
        None
    }
}

fn final_pipe_operator_is_unquoted(text: &str) -> bool {
    let Some((pipe_offset, _)) =
        final_pipe_operator_bounds(text.trim_end_matches([' ', '\t', '\r']))
    else {
        return false;
    };
    let mut quote_state = RawLineQuoteState::default();
    quote_state.operator_is_unquoted_at(text, pipe_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_continuation_normalization_moves_leading_operator() {
        assert_eq!(
            normalize_raw_pipeline_continuations("echo a \\\n  | grep a"),
            Some("echo a |\n  grep a".to_string())
        );
    }

    #[test]
    fn raw_scanner_finds_shell_comments_outside_quotes() {
        let source = "echo '# no' \"# no\" value#no # yes";
        let scanner = RawShellScanner::new(source);

        assert_eq!(scanner.find_comment(0, source.len()), Some(28));
    }

    #[test]
    fn raw_scanner_matches_nested_command_substitutions() {
        let raw = "$(echo \"$(date +%s)\" '(')";
        let scanner = RawShellScanner::new(raw);

        assert_eq!(
            scanner.matching_command_substitution_close(2),
            Some(raw.len() - 1)
        );
    }

    #[test]
    fn raw_scanner_finds_unclosed_process_substitution() {
        let raw = "cat <(printf '%s\\n' \"$(value)\"";
        let scanner = RawShellScanner::new(raw);

        assert!(scanner.has_unclosed_substitution_before(raw.len()));
    }

    #[test]
    fn raw_scanner_skips_escaped_command_substitutions() {
        let raw = r#"echo \$(skip) "$(keep)" '$(skip)'"#;
        let scanner = RawShellScanner::new(raw);

        assert_eq!(scanner.next_command_substitution(0), Some((15, 21)));
    }

    #[test]
    fn unclosed_command_substitution_ignores_quoted_open() {
        assert!(!RawShellText::new("'$(").has_unclosed_command_substitution_open());
        assert!(RawShellText::new("value=$(echo").has_unclosed_command_substitution_open());
    }

    #[test]
    fn rendered_pipeline_continuation_ignores_quoted_pipe() {
        assert!(!rendered_line_ends_with_structural_pipe_continuation(
            "printf '%s|'"
        ));
        assert!(rendered_line_ends_with_structural_pipe_continuation(
            "printf '%s' |"
        ));
    }
}
