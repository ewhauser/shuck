use super::*;

impl<'a> Lexer<'a> {
    /// Create a new bash-profile lexer for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_max_subst_depth_and_profile(
            input,
            DEFAULT_MAX_SUBST_DEPTH,
            &ShellProfile::native(super::ShellDialect::Bash),
            None,
        )
    }

    /// Create a new lexer with a custom max substitution nesting depth.
    /// Limits recursion in read_command_subst_into().
    pub(in crate::parser) fn with_max_subst_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_max_subst_depth_and_profile(
            input,
            max_depth,
            &ShellProfile::native(super::ShellDialect::Bash),
            None,
        )
    }

    /// Create a new lexer using the provided shell profile.
    #[cfg(test)]
    pub(in crate::parser) fn with_profile(input: &'a str, shell_profile: &ShellProfile) -> Self {
        let zsh_timeline = (shell_profile.dialect == super::ShellDialect::Zsh)
            .then(|| ZshOptionTimeline::build(input, shell_profile))
            .flatten()
            .map(Arc::new);
        Self::with_max_subst_depth_and_profile(
            input,
            DEFAULT_MAX_SUBST_DEPTH,
            shell_profile,
            zsh_timeline,
        )
    }

    pub(crate) fn with_max_subst_depth_and_profile(
        input: &'a str,
        max_depth: usize,
        shell_profile: &ShellProfile,
        zsh_timeline: Option<Arc<ZshOptionTimeline>>,
    ) -> Self {
        Self {
            input,
            offset: 0,
            cursor: Cursor::new(input),
            position_map: PositionMap::new(input),
            reinject_buf: VecDeque::new(),
            reinject_resume_offset: None,
            max_subst_depth: max_depth,
            initial_zsh_options: shell_profile.zsh_options().cloned(),
            zsh_timeline,
            zsh_timeline_index: 0,
            #[cfg(feature = "benchmarking")]
            benchmark_counters: None,
        }
    }

    pub(in crate::parser) fn position_at_offset(&self, offset: usize) -> Position {
        self.position_map.position_uncached(offset)
    }

    pub(in crate::parser) fn current_position(&mut self) -> Position {
        #[cfg(feature = "benchmarking")]
        self.maybe_record_current_position_call();
        self.position_map.position(self.offset)
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn enable_benchmark_counters(&mut self) {
        self.benchmark_counters = Some(LexerBenchmarkCounters::default());
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn benchmark_counters(&self) -> LexerBenchmarkCounters {
        self.benchmark_counters.unwrap_or_default()
    }

    #[cfg(feature = "benchmarking")]
    pub(in crate::parser) fn maybe_record_current_position_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.current_position_calls += 1;
        }
    }

    pub(in crate::parser) fn sync_offset_to_cursor(&mut self) {
        if self.reinject_buf.is_empty()
            && let Some(offset) = self.reinject_resume_offset.take()
        {
            self.offset = offset;
        }
    }

    /// Get the next token kind from the input.
    ///
    /// This skips whitespace and line comments, matching
    /// [`Lexer::next_lexed_token`]. It is useful for callers that only need the
    /// token stream shape.
    pub fn next_token_kind(&mut self) -> Option<TokenKind> {
        self.next_lexed_token().map(|token| token.kind)
    }

    pub(in crate::parser) fn peek_char(&mut self) -> Option<char> {
        self.sync_offset_to_cursor();
        if let Some(&ch) = self.reinject_buf.front() {
            Some(ch)
        } else {
            self.cursor.first()
        }
    }

    pub(in crate::parser) fn advance(&mut self) -> Option<char> {
        self.sync_offset_to_cursor();
        let ch = if !self.reinject_buf.is_empty() {
            self.reinject_buf.pop_front()
        } else {
            self.cursor.bump()
        };
        if let Some(c) = ch {
            self.offset += c.len_utf8();
        }
        ch
    }

    pub(in crate::parser) fn lookahead_chars(&self) -> impl Iterator<Item = char> + '_ {
        self.reinject_buf
            .iter()
            .copied()
            .chain(self.cursor.rest().chars())
    }

    pub(in crate::parser) fn second_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.second(),
            1 => self.cursor.first(),
            _ => self.reinject_buf.get(1).copied(),
        }
    }

    pub(in crate::parser) fn third_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.third(),
            1 => self.cursor.second(),
            2 => self.cursor.first(),
            _ => self.reinject_buf.get(2).copied(),
        }
    }

    pub(in crate::parser) fn fourth_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.rest().chars().nth(3),
            1 => self.cursor.third(),
            2 => self.cursor.second(),
            3 => self.cursor.first(),
            _ => self.reinject_buf.get(3).copied(),
        }
    }

    pub(in crate::parser) fn consume_source_bytes(&mut self, byte_len: usize) {
        debug_assert!(self.reinject_buf.is_empty());
        self.sync_offset_to_cursor();
        self.offset += byte_len;
        self.cursor.skip_bytes(byte_len);
    }

    pub(in crate::parser) fn advance_scanned_source_bytes(&mut self, byte_len: usize) {
        debug_assert!(self.reinject_buf.is_empty());
        self.offset += byte_len;
    }

    pub(in crate::parser) fn consume_ascii_chars(&mut self, count: usize) {
        if self.reinject_buf.is_empty() {
            self.consume_source_bytes(count);
            return;
        }

        for _ in 0..count {
            self.advance();
        }
    }

    pub(in crate::parser) fn source_horizontal_whitespace_len(&self) -> usize {
        self.cursor
            .rest()
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count()
    }

    pub(in crate::parser) fn source_ascii_plain_word_len(&self) -> usize {
        self.cursor
            .rest()
            .as_bytes()
            .iter()
            .take_while(|byte| Self::is_ascii_plain_word_byte(**byte))
            .count()
    }

    pub(in crate::parser) fn find_double_quote_special(source: &str) -> Option<usize> {
        source
            .as_bytes()
            .iter()
            .position(|byte| matches!(*byte, b'"' | b'\\' | b'$' | b'`'))
    }

    pub(in crate::parser) fn ensure_capture_from_source(
        &self,
        capture: &mut Option<String>,
        start: Position,
        end: Position,
    ) {
        if capture.is_none() {
            *capture = Some(self.input[start.offset..end.offset].to_string());
        }
    }

    pub(in crate::parser) fn push_capture_char(capture: &mut Option<String>, ch: char) {
        if let Some(text) = capture.as_mut() {
            text.push(ch);
        }
    }

    pub(in crate::parser) fn push_capture_str(capture: &mut Option<String>, text: &str) {
        if let Some(current) = capture.as_mut() {
            current.push_str(text);
        }
    }

    pub(in crate::parser) fn current_zsh_options(&mut self) -> Option<&ZshOptionState> {
        if let Some(timeline) = self.zsh_timeline.as_ref() {
            while self.zsh_timeline_index < timeline.entries.len()
                && timeline.entries[self.zsh_timeline_index].offset <= self.offset
            {
                self.zsh_timeline_index += 1;
            }
            return if self.zsh_timeline_index == 0 {
                self.initial_zsh_options.as_ref()
            } else {
                Some(&timeline.entries[self.zsh_timeline_index - 1].state)
            };
        }

        self.initial_zsh_options.as_ref()
    }

    pub(in crate::parser) fn comments_enabled(&mut self) -> bool {
        !self
            .current_zsh_options()
            .is_some_and(|options| options.interactive_comments.is_definitely_off())
    }

    pub(in crate::parser) fn rc_quotes_enabled(&mut self) -> bool {
        self.current_zsh_options()
            .is_some_and(|options| options.rc_quotes.is_definitely_on())
    }

    pub(in crate::parser) fn ignore_braces_enabled(&mut self) -> bool {
        self.current_zsh_options()
            .is_some_and(|options| options.ignore_braces.is_definitely_on())
    }

    pub(in crate::parser) fn ignore_close_braces_enabled(&mut self) -> bool {
        self.current_zsh_options().is_some_and(|options| {
            options.ignore_braces.is_definitely_on()
                || options.ignore_close_braces.is_definitely_on()
        })
    }

    pub(in crate::parser) fn brace_ccl_enabled(&mut self) -> bool {
        self.current_zsh_options()
            .is_some_and(|options| options.brace_ccl.is_definitely_on())
    }

    pub(in crate::parser) fn should_treat_hash_as_word_char(&mut self) -> bool {
        if !self.comments_enabled() {
            return true;
        }
        self.reinject_buf.is_empty()
            && (self
                .input
                .get(..self.offset)
                .and_then(|prefix| prefix.chars().next_back())
                .is_some_and(|prev| {
                    !prev.is_whitespace() && !matches!(prev, ';' | '|' | '&' | '<' | '>')
                })
                || self.is_inside_unclosed_double_paren_on_line())
    }

    pub(in crate::parser) fn current_word_text<'b>(
        &'b self,
        start: Position,
        capture: &'b Option<String>,
    ) -> &'b str {
        capture
            .as_deref()
            .unwrap_or(&self.input[start.offset..self.offset])
    }

    pub(in crate::parser) fn current_word_surface_is_single_char(
        &self,
        start: Position,
        capture: &Option<String>,
        target: char,
    ) -> bool {
        let text = self.current_word_text(start, capture);
        if !text.contains('\x00') {
            let mut encoded = [0; 4];
            return text == target.encode_utf8(&mut encoded);
        }

        let mut chars = text.chars().filter(|&ch| ch != '\x00');
        matches!((chars.next(), chars.next()), (Some(ch), None) if ch == target)
    }

    pub(in crate::parser) fn current_word_surface_last_char<'b>(
        &'b self,
        start: Position,
        capture: &'b Option<String>,
    ) -> Option<char> {
        self.current_word_text(start, capture)
            .chars()
            .rev()
            .find(|&ch| ch != '\x00')
    }

    pub(in crate::parser) fn current_word_surface_ends_with_char(
        &self,
        start: Position,
        capture: &Option<String>,
        target: char,
    ) -> bool {
        self.current_word_surface_last_char(start, capture) == Some(target)
    }

    pub(in crate::parser) fn current_word_surface_ends_with_extglob_prefix(
        &self,
        start: Position,
        capture: &Option<String>,
    ) -> bool {
        self.current_word_surface_last_char(start, capture)
            .is_some_and(|ch| matches!(ch, '@' | '?' | '*' | '+' | '!'))
    }

    pub(in crate::parser) fn current_word_surface_can_take_zsh_glob_modifier_suffix(
        &mut self,
        start: Position,
        capture: &Option<String>,
    ) -> bool {
        if self.current_zsh_options().is_none() || self.peek_char() != Some('(') {
            return false;
        }

        let text = self.current_word_text(start, capture);
        if !text.contains('/') {
            return false;
        }

        let mut chars = self.lookahead_chars();
        matches!((chars.next(), chars.next()), (Some('('), Some(':')))
    }
}
