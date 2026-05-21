//! Lightweight lexical scans over raw shell source text.
//!
//! These helpers are for source-preserving consumers that need to skip quoted
//! text, comments, escapes, or balanced substitution-like fragments without
//! rebuilding a parser. They intentionally do not replace the parser for full
//! shell structure.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawQuoteKind {
    Single,
    Double,
    Backtick,
}

/// Stateful quote tracking for raw shell text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuoteState {
    quote: Option<RawQuoteKind>,
    escaped: bool,
}

impl QuoteState {
    /// Returns whether the state is inside a quote that may continue onto later text.
    #[must_use]
    pub fn in_multiline_literal(&self) -> bool {
        self.quote.is_some()
    }

    /// Returns whether the state is currently inside single quotes.
    #[must_use]
    pub fn in_single_quotes(&self) -> bool {
        self.quote == Some(RawQuoteKind::Single)
    }

    /// Returns whether the state is currently inside any tracked quote form.
    #[must_use]
    pub fn in_quotes(&self) -> bool {
        self.quote.is_some()
    }

    /// Consumes one raw shell character and returns whether the character was
    /// handled as quote or escape syntax.
    pub fn consume_raw_char(&mut self, ch: char, include_backticks: bool) -> bool {
        if self.escaped {
            self.escaped = false;
            return true;
        }

        match self.quote {
            Some(RawQuoteKind::Single) => {
                if ch == '\'' {
                    self.quote = None;
                }
                true
            }
            Some(RawQuoteKind::Double) => {
                if ch == '"' {
                    self.quote = None;
                } else if ch == '\\' {
                    self.escaped = true;
                }
                true
            }
            Some(RawQuoteKind::Backtick) if include_backticks => {
                if ch == '`' {
                    self.quote = None;
                } else if ch == '\\' {
                    self.escaped = true;
                }
                true
            }
            _ if ch == '\\' => {
                self.escaped = true;
                true
            }
            _ if ch == '\'' => {
                self.quote = Some(RawQuoteKind::Single);
                true
            }
            _ if ch == '"' => {
                self.quote = Some(RawQuoteKind::Double);
                true
            }
            _ if include_backticks && ch == '`' => {
                self.quote = Some(RawQuoteKind::Backtick);
                true
            }
            _ => false,
        }
    }

    /// Consumes one shell-word character and returns whether the character was
    /// handled as quote or escape syntax.
    pub fn consume_shell_word_char(&mut self, ch: char) -> bool {
        if self.escaped {
            self.escaped = false;
            return true;
        }

        match self.quote {
            Some(RawQuoteKind::Single) => {
                if ch == '\'' {
                    self.quote = None;
                }
                true
            }
            Some(RawQuoteKind::Double) => {
                if ch == '"' {
                    self.quote = None;
                    return true;
                }
                if ch == '\\' {
                    self.escaped = true;
                    return true;
                }
                false
            }
            _ if ch == '\\' => {
                self.escaped = true;
                true
            }
            _ if ch == '\'' => {
                self.quote = Some(RawQuoteKind::Single);
                true
            }
            _ if ch == '"' => {
                self.quote = Some(RawQuoteKind::Double);
                true
            }
            _ => false,
        }
    }

    /// Scans one physical line, stopping before a shell comment.
    pub fn scan_line(&mut self, line: &str) {
        self.escaped = false;
        for (index, ch) in line.char_indices() {
            match self.quote {
                Some(RawQuoteKind::Single) => {
                    if ch == '\'' {
                        self.quote = None;
                    }
                }
                Some(RawQuoteKind::Double) => {
                    if ch == '"' && !self.escaped {
                        self.quote = None;
                    }
                }
                _ => {
                    if ch == '#' && shell_comment_can_start(line, index) {
                        break;
                    }
                    if ch == '\'' || (ch == '"' && !self.escaped) {
                        self.quote = Some(if ch == '\'' {
                            RawQuoteKind::Single
                        } else {
                            RawQuoteKind::Double
                        });
                    }
                }
            }

            self.escaped = ch == '\\' && !self.escaped;
            if ch != '\\' {
                self.escaped = false;
            }
        }
        self.escaped = false;
    }
}

/// Scanner for bounded movement through raw shell source text.
#[derive(Debug, Clone, Copy)]
pub struct RawShellScanner<'source> {
    source: &'source str,
    upper: usize,
}

impl<'source> RawShellScanner<'source> {
    /// Creates a scanner over the whole source string.
    #[must_use]
    pub fn new(source: &'source str) -> Self {
        Self::bounded(source, source.len())
    }

    /// Creates a scanner bounded to the given upper byte offset.
    #[must_use]
    pub fn bounded(source: &'source str, upper: usize) -> Self {
        Self {
            source,
            upper: upper.min(source.len()),
        }
    }

    /// Skips from the first byte after an opening single quote through the close.
    #[must_use]
    pub fn skip_single_quoted_body(&self, mut offset: usize) -> usize {
        while offset < self.upper {
            let Some(ch) = self.source[offset..].chars().next() else {
                break;
            };
            offset += ch.len_utf8();
            if ch == '\'' {
                break;
            }
        }
        offset
    }

    /// Skips from the first byte after an opening double quote through the close.
    #[must_use]
    pub fn skip_double_quoted_body(&self, mut offset: usize) -> usize {
        while offset < self.upper {
            let Some(ch) = self.source[offset..].chars().next() else {
                break;
            };
            offset += ch.len_utf8();
            if ch == '\\' {
                if let Some(escaped) = self.source[offset..].chars().next() {
                    offset += escaped.len_utf8();
                }
            } else if ch == '"' {
                break;
            }
        }
        offset
    }

    /// Skips an escape, single-quoted body, or double-quoted body at `offset`.
    #[must_use]
    pub fn skip_escaped_or_quoted_at(&self, offset: usize) -> Option<usize> {
        let ch = self.source[offset..].chars().next()?;
        let next = offset + ch.len_utf8();
        match ch {
            '\\' => Some(
                self.source[next..self.upper]
                    .chars()
                    .next()
                    .map_or(next, |escaped| next + escaped.len_utf8()),
            ),
            '\'' => Some(self.skip_single_quoted_body(next)),
            '"' => Some(self.skip_double_quoted_body(next)),
            _ => None,
        }
    }

    /// Skips an escape, quote, or comment at `offset`.
    #[must_use]
    pub fn skip_quoted_or_comment_at(&self, offset: usize) -> Option<usize> {
        self.skip_escaped_or_quoted_at(offset).or_else(|| {
            self.comment_starts_at(offset)
                .then(|| self.comment_end_from(offset))
        })
    }

    /// Finds the first unquoted shell comment marker in the given range.
    #[must_use]
    pub fn find_comment(&self, start: usize, upper: usize) -> Option<usize> {
        let mut offset = start.min(self.upper);
        let upper = upper.min(self.upper);
        while offset < upper {
            if let Some(next) = self.skip_escaped_or_quoted_at(offset) {
                offset = next;
                continue;
            }

            let ch = self.source[offset..].chars().next()?;
            if ch == '#' && shell_comment_can_start(self.source, offset) {
                return Some(offset);
            }
            offset += ch.len_utf8();
        }
        None
    }

    /// Returns whether `index` is preceded by an odd number of backslashes.
    #[must_use]
    pub fn is_escaped(&self, index: usize) -> bool {
        offset_is_backslash_escaped(self.source, index)
    }

    /// Finds the closing `)` for a command substitution body.
    #[must_use]
    pub fn matching_command_substitution_close(&self, body_start: usize) -> Option<usize> {
        let mut quote = QuoteState::default();
        let mut paren_depth = 0usize;
        let mut index = body_start.min(self.upper);

        while index < self.upper {
            let ch = self.source[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            if quote.consume_raw_char(ch, false) {
                index = next_index;
                continue;
            }

            match ch {
                '(' => paren_depth += 1,
                ')' => {
                    if paren_depth == 0 {
                        return Some(index);
                    }
                    paren_depth -= 1;
                }
                _ => {}
            }

            index = next_index;
        }

        None
    }

    /// Finds the next non-arithmetic `$(` command substitution.
    #[must_use]
    pub fn next_command_substitution(&self, mut index: usize) -> Option<(usize, usize)> {
        let bytes = self.source.as_bytes();
        let mut quote = QuoteState::default();

        while index + 1 < self.upper {
            let ch = self.source[index..].chars().next()?;
            let next_index = index + ch.len_utf8();
            if quote.consume_shell_word_char(ch) {
                index = next_index;
                continue;
            }

            if !quote.in_single_quotes()
                && bytes[index] == b'$'
                && bytes[index + 1] == b'('
                && bytes.get(index + 2).is_none_or(|byte| *byte != b'(')
                && let Some(close_offset) = self.matching_command_substitution_close(index + 2)
            {
                return Some((index, close_offset));
            }
            index = next_index;
        }

        None
    }

    /// Returns whether there is an unclosed substitution before `upper`.
    #[must_use]
    pub fn has_unclosed_substitution_before(&self, upper: usize) -> bool {
        let mut depth = 0usize;
        let mut index = 0usize;
        let upper = upper.min(self.upper);
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;

        while index < upper {
            let Some(ch) = self.source[index..].chars().next() else {
                break;
            };
            let next_index = index + ch.len_utf8();

            if ch == '\\' {
                index = self.source[next_index..upper]
                    .chars()
                    .next()
                    .map_or(next_index, |escaped| next_index + escaped.len_utf8());
                continue;
            }
            if ch == '\'' && !in_double_quotes {
                in_single_quotes = !in_single_quotes;
                index = next_index;
                continue;
            }
            if ch == '"' && !in_single_quotes {
                in_double_quotes = !in_double_quotes;
                index = next_index;
                continue;
            }
            if in_single_quotes {
                index = next_index;
                continue;
            }

            let bytes = self.source.as_bytes();
            match ch {
                '$' | '<' | '>' if bytes.get(index + 1) == Some(&b'(') => {
                    depth += 1;
                    index = index.saturating_add(2);
                    continue;
                }
                ')' if depth > 0 => depth -= 1,
                _ => {}
            }
            index = next_index;
        }

        depth > 0
    }

    /// Finds the end of a shell word starting at `start`.
    #[must_use]
    pub fn shell_word_end(&self, start: usize) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut index = start.min(self.upper);
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while index < self.upper {
            let byte = bytes[index];
            if in_single_quote {
                if byte == b'\'' {
                    in_single_quote = false;
                }
                index += 1;
                continue;
            }

            if byte == b'\\' {
                index = advance_escaped_shell_char(self.source, index).min(self.upper);
                continue;
            }

            if !in_double_quote && byte.is_ascii_whitespace() {
                break;
            }

            match byte {
                b'\'' if !in_double_quote => {
                    in_single_quote = true;
                    index += 1;
                }
                b'"' => {
                    in_double_quote = !in_double_quote;
                    index += 1;
                }
                b'$' if bytes.get(index + 1) == Some(&b'(') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'(', b')')?;
                }
                b'$' if bytes.get(index + 1) == Some(&b'{') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'{', b'}')?;
                }
                b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'(', b')')?;
                }
                b'`' => {
                    index = self.skip_legacy_backtick_construct(index + 1)?;
                }
                _ => index = advance_shell_char(self.source, index).min(self.upper),
            }
        }

        (!in_single_quote && !in_double_quote).then_some(index)
    }

    /// Skips a balanced shell construct and returns the byte after its close.
    #[must_use]
    pub fn skip_balanced_shell_construct(
        &self,
        mut index: usize,
        open: u8,
        close: u8,
    ) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut depth = 1usize;
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while index < self.upper {
            let byte = bytes[index];
            if in_single_quote {
                if byte == b'\'' {
                    in_single_quote = false;
                }
                index += 1;
                continue;
            }

            if byte == b'\\' {
                index = advance_escaped_shell_char(self.source, index).min(self.upper);
                continue;
            }

            match byte {
                b'\'' if !in_double_quote => {
                    in_single_quote = true;
                    index += 1;
                }
                b'"' => {
                    in_double_quote = !in_double_quote;
                    index += 1;
                }
                b'$' if bytes.get(index + 1) == Some(&b'(') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'(', b')')?;
                }
                b'$' if bytes.get(index + 1) == Some(&b'{') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'{', b'}')?;
                }
                b'<' | b'>' if !in_double_quote && bytes.get(index + 1) == Some(&b'(') => {
                    index = self.skip_balanced_shell_construct(index + 2, b'(', b')')?;
                }
                b'`' => {
                    index = self.skip_legacy_backtick_construct(index + 1)?;
                }
                _ if byte == open && !in_double_quote => {
                    depth += 1;
                    index += 1;
                }
                _ if byte == close && !in_double_quote => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => index = advance_shell_char(self.source, index).min(self.upper),
            }
        }

        None
    }

    /// Skips a legacy backtick command substitution and returns the byte after its close.
    #[must_use]
    pub fn skip_legacy_backtick_construct(&self, mut index: usize) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while index < self.upper {
            let byte = bytes[index];
            if in_single_quote {
                if byte == b'\'' {
                    in_single_quote = false;
                }
                index += 1;
                continue;
            }

            if byte == b'\\' {
                index = advance_escaped_shell_char(self.source, index).min(self.upper);
                continue;
            }

            match byte {
                b'\'' if !in_double_quote => {
                    in_single_quote = true;
                    index += 1;
                }
                b'"' => {
                    in_double_quote = !in_double_quote;
                    index += 1;
                }
                b'`' if !in_double_quote => return Some(index + 1),
                _ => index = advance_shell_char(self.source, index).min(self.upper),
            }
        }

        None
    }

    /// Skips a legacy backtick command substitution body during recovery scanning.
    ///
    /// Unlike shell word scanning, this stops at the first unescaped backtick even when
    /// an earlier quote in the partially parsed body is not balanced.
    #[must_use]
    pub fn skip_legacy_backtick_substitution_body(&self, mut index: usize) -> Option<usize> {
        let bytes = self.source.as_bytes();

        while index < self.upper {
            match bytes[index] {
                b'\\' => index = advance_escaped_shell_char(self.source, index).min(self.upper),
                b'`' => return Some(index + 1),
                _ => index = advance_shell_char(self.source, index).min(self.upper),
            }
        }

        None
    }

    fn comment_starts_at(&self, offset: usize) -> bool {
        self.source[offset..self.upper].starts_with('#')
            && shell_comment_can_start(self.source, offset)
    }

    fn comment_end_from(&self, offset: usize) -> usize {
        self.source[offset..self.upper]
            .find('\n')
            .map_or(self.upper, |newline| offset + newline + 1)
    }
}

/// Returns whether a shell comment can start at `offset`.
#[must_use]
pub fn shell_comment_can_start(source: &str, offset: usize) -> bool {
    source[..offset]
        .chars()
        .next_back()
        .is_none_or(|ch| ch == '\n' || ch.is_whitespace() || matches!(ch, ';' | '&' | '|'))
}

/// Advances one UTF-8 character from `index`.
#[must_use]
pub fn advance_shell_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(index + 1, |ch| index + ch.len_utf8())
}

/// Advances one shell-escaped character pair from `index`.
#[must_use]
pub fn advance_escaped_shell_char(text: &str, index: usize) -> usize {
    let next = advance_shell_char(text, index);
    if next < text.len() {
        advance_shell_char(text, next)
    } else {
        next
    }
}

/// Returns whether `index` is preceded by an odd number of backslashes.
#[must_use]
pub fn offset_is_backslash_escaped(source: &str, offset: usize) -> bool {
    byte_is_backslash_escaped(source.as_bytes(), offset)
}

/// Returns whether `index` is preceded by an odd number of backslashes.
#[must_use]
pub fn byte_is_backslash_escaped(bytes: &[u8], index: usize) -> bool {
    let mut cursor = index.min(bytes.len());
    let mut backslashes = 0usize;

    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }

    backslashes % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::RawShellScanner;

    #[test]
    fn finds_comments_outside_quotes() {
        let source = "echo '# no' \"# no\" value#no # yes";
        let scanner = RawShellScanner::new(source);
        assert_eq!(scanner.find_comment(0, source.len()), Some(28));
    }

    #[test]
    fn matches_nested_command_substitution_close() {
        let raw = "$(echo \"$(date)\"; (cd x)) tail";
        let scanner = RawShellScanner::new(raw);
        assert_eq!(
            scanner.matching_command_substitution_close(2),
            Some("$(echo \"$(date)\"; (cd x))".len() - 1),
        );
    }

    #[test]
    fn tracks_unclosed_process_substitution() {
        let raw = "diff <(sort a";
        let scanner = RawShellScanner::new(raw);
        assert!(scanner.has_unclosed_substitution_before(raw.len()));
    }

    #[test]
    fn finds_next_command_substitution_outside_single_quotes() {
        let raw = "echo '$(nope)' $(date) $((1 + 2))";
        let scanner = RawShellScanner::new(raw);
        assert_eq!(scanner.next_command_substitution(0), Some((15, 21)));
    }

    #[test]
    fn legacy_backtick_substitution_body_stops_at_first_unescaped_backtick() {
        let raw = "`echo \"`\" tail`";
        let scanner = RawShellScanner::new(raw);
        assert_eq!(
            scanner.skip_legacy_backtick_substitution_body(1),
            Some("`echo \"`".len()),
        );
    }

    #[test]
    fn legacy_backtick_construct_keeps_quote_aware_word_scan_behavior() {
        let raw = "`echo \"`\" tail`";
        let scanner = RawShellScanner::new(raw);
        assert_eq!(scanner.skip_legacy_backtick_construct(1), Some(raw.len()));
    }
}
