use super::*;

impl<'a> Lexer<'a> {
    /// Read here document content until the delimiter line is found
    pub(in crate::parser) fn read_heredoc(
        &mut self,
        delimiter: &str,
        strip_tabs: bool,
    ) -> HeredocRead {
        let mut content = String::with_capacity(64);
        let mut current_line = String::with_capacity(64);

        // Save rest of current line (after the delimiter token on the command line).
        // For `cat <<EOF | sort`, this captures ` | sort` so the parser can
        // tokenize the pipe and subsequent command after the heredoc body.
        //
        // Quoted strings may span multiple lines (e.g., `cat <<EOF; echo "two\nthree"`),
        // so we track quoting state and continue across newlines until quotes close.
        let mut rest_of_line = String::with_capacity(32);
        let rest_of_line_start = self.current_position();
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        let mut in_comment = false;
        let mut saw_non_whitespace_tail = false;
        let mut consecutive_backslashes = 0usize;
        let mut previous_tail_char = None;
        while let Some(ch) = self.peek_char() {
            self.advance();
            if in_comment {
                if ch == '\n' {
                    break;
                }
                rest_of_line.push(ch);
                previous_tail_char = Some(ch);
                continue;
            }
            if ch == '#'
                && !in_single_quote
                && !in_double_quote
                && self.comments_enabled()
                && heredoc_tail_hash_starts_comment(previous_tail_char)
            {
                in_comment = true;
                rest_of_line.push(ch);
                previous_tail_char = Some(ch);
                consecutive_backslashes = 0;
                continue;
            }
            let backslash_continues_line = ch == '\\'
                && !in_single_quote
                && self.peek_char() == Some('\n')
                && (saw_non_whitespace_tail || self.heredoc_tail_line_join_stays_in_tail())
                && consecutive_backslashes.is_multiple_of(2);
            if backslash_continues_line {
                rest_of_line.push(ch);
                rest_of_line.push('\n');
                self.advance();
                consecutive_backslashes = 0;
                continue;
            }
            if ch == '\n' && !in_double_quote && !in_single_quote {
                break;
            }
            if ch == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
            } else if ch == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
            } else if ch == '\\' && in_double_quote {
                // Escaped char inside double quotes — skip the next char too
                rest_of_line.push(ch);
                if let Some(next) = self.peek_char() {
                    rest_of_line.push(next);
                    self.advance();
                }
                continue;
            }
            rest_of_line.push(ch);
            if !ch.is_whitespace() {
                saw_non_whitespace_tail = true;
            }
            if ch == '\\' && !in_single_quote {
                consecutive_backslashes += 1;
            } else {
                consecutive_backslashes = 0;
            }
            previous_tail_char = Some(ch);
        }

        // If we just drained a heredoc replay buffer (for example when multiple
        // heredocs share one command line), resume tracking from the true cursor
        // position before we measure the body span.
        self.sync_offset_to_cursor();
        let content_start = self.current_position();
        let mut current_line_start = content_start;
        let content_end;

        // Read lines until we find the delimiter
        loop {
            if self.reinject_buf.is_empty() {
                // When the body reading drains a reinject buffer (from a
                // previous heredoc on the same command line), the virtual
                // offset drifts away from the cursor. Snap it back before
                // any source-based work so spans and `post_heredoc_offset`
                // stay within bounds.
                self.sync_offset_to_cursor();
                let rest = self.cursor.rest();
                if rest.is_empty() {
                    content_end = self.current_position();
                    break;
                }

                let line_len = self.cursor.find_byte(b'\n').unwrap_or(rest.len());
                let line = &rest[..line_len];
                let has_newline = line_len < rest.len();

                if heredoc_line_matches_delimiter(line, delimiter, strip_tabs) {
                    content_end = current_line_start;
                    self.consume_source_bytes(line_len);
                    if has_newline {
                        self.consume_ascii_chars(1);
                    }
                    break;
                }

                content.push_str(line);
                self.consume_source_bytes(line_len);

                if has_newline {
                    self.consume_ascii_chars(1);
                    content.push('\n');
                    current_line_start = self.current_position();
                    continue;
                }

                content_end = self.current_position();
                break;
            }

            match self.peek_char() {
                Some('\n') => {
                    self.advance();
                    // Check if current line matches delimiter
                    if heredoc_line_matches_delimiter(&current_line, delimiter, strip_tabs) {
                        content_end = current_line_start;
                        break;
                    }
                    content.push_str(&current_line);
                    content.push('\n');
                    current_line.clear();
                    current_line_start = self.current_position();
                }
                Some(ch) => {
                    current_line.push(ch);
                    self.advance();
                }
                None => {
                    // End of input - check last line
                    if heredoc_line_matches_delimiter(&current_line, delimiter, strip_tabs) {
                        content_end = current_line_start;
                        break;
                    }
                    if !current_line.is_empty() {
                        content.push_str(&current_line);
                    }
                    content_end = self.current_position();
                    break;
                }
            }
        }

        // Re-inject the command-line tail so subsequent same-line tokens (pipes,
        // redirects, command words, additional heredocs) stay visible to the
        // parser. Always replay a terminating newline so parsing stops before
        // tokens that originally lived on later source lines, like `}` or `do`.
        let post_heredoc_offset = self.offset;
        self.offset = rest_of_line_start.offset;
        for ch in rest_of_line.chars() {
            self.reinject_buf.push_back(ch);
        }
        self.reinject_buf.push_back('\n');
        self.reinject_resume_offset = Some(post_heredoc_offset);

        HeredocRead {
            content,
            content_span: Span::from_positions(content_start, content_end),
        }
    }

    pub(in crate::parser) fn heredoc_tail_line_join_stays_in_tail(&mut self) -> bool {
        let mut chars = self.cursor.rest().chars();
        if chars.next() != Some('\n') {
            return false;
        }

        for ch in chars {
            if matches!(ch, ' ' | '\t') {
                continue;
            }
            if ch == '\n' {
                return false;
            }
            return matches!(ch, '|' | '&' | ';' | '<' | '>')
                || (ch == '#' && self.comments_enabled());
        }

        false
    }
}

pub(in crate::parser) fn heredoc_line_matches_delimiter(
    line: &str,
    delimiter: &str,
    strip_tabs: bool,
) -> bool {
    let line = if strip_tabs {
        line.trim_start_matches('\t')
    } else {
        line
    };

    if line == delimiter {
        return true;
    }

    let Some(trailing) = line.strip_prefix(delimiter) else {
        return false;
    };

    trailing.chars().all(|ch| matches!(ch, ' ' | '\t'))
}

fn heredoc_tail_hash_starts_comment(previous_tail_char: Option<char>) -> bool {
    previous_tail_char.is_none_or(|prev| {
        prev.is_whitespace() || matches!(prev, ';' | '|' | '&' | '<' | '>' | ')')
    })
}
