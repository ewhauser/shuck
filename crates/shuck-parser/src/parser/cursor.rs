use super::*;

impl<'a> Parser<'a> {
    pub(super) fn checkpoint(&self) -> ParserCheckpoint<'a> {
        ParserCheckpoint {
            lexer: self.lexer.clone(),
            synthetic_tokens: self.synthetic_tokens.clone(),
            alias_replays: self.alias_replays.clone(),
            current_token: self.current_token.clone(),
            current_token_kind: self.current_token_kind,
            current_keyword: self.current_keyword,
            current_span: self.current_span,
            peeked_token: self.peeked_token.clone(),
            current_depth: self.current_depth,
            fuel: self.fuel,
            source_text_pattern_depth: self.source_text_pattern_depth,
            comments_len: self.comments.len(),
            expand_next_word: self.expand_next_word,
            brace_group_depth: self.brace_group_depth,
            brace_body_stack_len: self.brace_body_stack.len(),
            syntax_facts_zsh_brace_if_spans_len: self.syntax_facts.zsh_brace_if_spans.len(),
            syntax_facts_zsh_always_spans_len: self.syntax_facts.zsh_always_spans.len(),
            syntax_facts_zsh_case_group_parts_len: self.syntax_facts.zsh_case_group_parts.len(),
            #[cfg(feature = "benchmarking")]
            benchmark_counters: self.benchmark_counters,
        }
    }

    pub(super) fn restore(&mut self, checkpoint: ParserCheckpoint<'a>) {
        self.lexer = checkpoint.lexer;
        self.synthetic_tokens = checkpoint.synthetic_tokens;
        self.alias_replays = checkpoint.alias_replays;
        self.current_token = checkpoint.current_token;
        // This is a cache over current_token/current_span, so rebuilding it is equivalent.
        self.current_word_cache = None;
        self.current_token_kind = checkpoint.current_token_kind;
        self.current_keyword = checkpoint.current_keyword;
        self.current_span = checkpoint.current_span;
        self.peeked_token = checkpoint.peeked_token;
        self.current_depth = checkpoint.current_depth;
        self.fuel = checkpoint.fuel;
        self.source_text_pattern_depth = checkpoint.source_text_pattern_depth;
        self.comments.truncate(checkpoint.comments_len);
        self.expand_next_word = checkpoint.expand_next_word;
        self.brace_group_depth = checkpoint.brace_group_depth;
        self.brace_body_stack
            .truncate(checkpoint.brace_body_stack_len);
        self.syntax_facts
            .zsh_brace_if_spans
            .truncate(checkpoint.syntax_facts_zsh_brace_if_spans_len);
        self.syntax_facts
            .zsh_always_spans
            .truncate(checkpoint.syntax_facts_zsh_always_spans_len);
        self.syntax_facts
            .zsh_case_group_parts
            .truncate(checkpoint.syntax_facts_zsh_case_group_parts_len);
        #[cfg(feature = "benchmarking")]
        {
            self.benchmark_counters = checkpoint.benchmark_counters;
        }
    }

    pub(super) fn set_current_spanned(&mut self, token: LexedToken<'a>) {
        #[cfg(feature = "benchmarking")]
        self.maybe_record_set_current_spanned_call();
        let span = token.span;
        self.current_token_kind = Some(token.kind);
        self.current_keyword = Self::keyword_from_token(&token);
        self.current_token = Some(token);
        self.current_word_cache = None;
        self.current_span = span;
    }

    pub(super) fn set_current_kind(&mut self, kind: TokenKind, span: Span) {
        self.current_token_kind = Some(kind);
        self.current_keyword = None;
        self.current_token = Some(LexedToken::punctuation(kind).with_span(span));
        self.current_word_cache = None;
        self.current_span = span;
    }

    pub(super) fn clear_current_token(&mut self) {
        self.current_token = None;
        self.current_word_cache = None;
        self.current_token_kind = None;
        self.current_keyword = None;
    }

    pub(super) fn skip_raw_to_offset(&mut self, offset: usize) {
        let offset = offset.min(self.input.len());
        self.lexer = Lexer::with_max_subst_depth_and_profile(
            self.input,
            self.max_depth,
            &self.shell_profile,
            self.zsh_timeline.clone(),
        );
        self.lexer.consume_source_bytes(offset);
        self.synthetic_tokens.clear();
        self.alias_replays.clear();
        self.peeked_token = None;
        self.expand_next_word = false;
        self.advance_raw();
    }

    pub(super) fn raw_recovery_separator_offset(&self, start: usize) -> Option<usize> {
        let mut state = RawRecoveryScanState::default();
        for (relative, ch) in self.input[start.min(self.input.len())..].char_indices() {
            let offset = start + relative;
            if state.in_comment {
                if ch == '\n' {
                    return Some(offset);
                }
                continue;
            }
            if state.update_quoted_state(ch) {
                continue;
            }
            if state.is_plain_top_level() && matches!(ch, ';' | '\n') {
                return Some(offset);
            }
            state.update_raw_syntax(ch);
        }
        None
    }

    pub(super) fn raw_conditional_close_offset(&self, start: usize) -> Option<usize> {
        let mut state = RawRecoveryScanState::default();
        let start = start.min(self.input.len());
        for (relative, ch) in self.input[start..].char_indices() {
            let offset = start + relative;
            if state.in_comment {
                if ch == '\n' {
                    state.in_comment = false;
                    state.at_command_start = true;
                    state.at_token_start = true;
                }
                continue;
            }
            if state.update_quoted_state(ch) {
                continue;
            }
            if state.is_plain_top_level() && self.input[offset..].starts_with("]]") {
                return Some(offset + "]]".len());
            }
            state.update_raw_syntax(ch);
        }
        None
    }

    pub(super) fn raw_case_end_offset(&self, start: usize) -> Option<usize> {
        let mut state = RawRecoveryScanState::default();
        let start = start.min(self.input.len());
        for (relative, ch) in self.input[start..].char_indices() {
            let offset = start + relative;
            if state.in_comment {
                if ch == '\n' {
                    state.in_comment = false;
                    state.at_command_start = true;
                    state.at_token_start = true;
                }
                continue;
            }
            if state.update_quoted_state(ch) {
                continue;
            }
            if state.is_plain_top_level() && state.at_command_start {
                if raw_keyword_starts_at(self.input, offset, "case") {
                    state.case_depth += 1;
                    state.at_command_start = false;
                    state.at_token_start = false;
                    continue;
                }
                if raw_keyword_starts_at(self.input, offset, "esac") {
                    if state.case_depth == 0 {
                        return Some(offset + "esac".len());
                    }
                    state.case_depth -= 1;
                    state.at_command_start = false;
                    state.at_token_start = false;
                    continue;
                }
            }
            state.update_raw_syntax(ch);
        }
        None
    }

    pub(super) fn next_pending_token(&mut self) -> Option<LexedToken<'a>> {
        if let Some(token) = self.synthetic_tokens.pop_front() {
            return Some(token.materialize());
        }

        loop {
            let replay = self.alias_replays.last_mut()?;
            if let Some(token) = replay.next_token() {
                return Some(token);
            }
            self.alias_replays.pop();
        }
    }

    pub(super) fn next_spanned_token_with_comments(&mut self) -> Option<LexedToken<'a>> {
        self.next_pending_token()
            .or_else(|| self.lexer.next_lexed_token_with_comments())
    }

    pub(super) fn compile_alias_definition(&self, value: &str) -> AliasDefinition {
        let source = Arc::<str>::from(value.to_string());
        let mut lexer = Lexer::with_max_subst_depth(source.as_ref(), self.max_depth);
        let mut tokens = Vec::new();

        while let Some(token) = lexer.next_lexed_token_with_comments() {
            tokens.push(token.into_shared(&source));
        }

        AliasDefinition {
            tokens: tokens.into(),
            expands_next_word: value.chars().last().is_some_and(char::is_whitespace),
        }
    }

    pub(super) fn maybe_expand_current_alias_chain(&mut self) {
        if !self.expand_aliases {
            self.expand_next_word = false;
            return;
        }

        let mut seen = HashSet::new();
        let mut expands_next_word = false;

        loop {
            if self.current_token_kind != Some(TokenKind::Word) {
                break;
            }
            let Some(name) = self.current_token.as_ref().and_then(LexedToken::word_text) else {
                break;
            };
            if self.current_source_word_starts_posix_function_header(name) {
                break;
            }
            let Some(alias) = self.aliases.get(name).cloned() else {
                break;
            };
            if !seen.insert(name.to_string()) {
                break;
            }

            expands_next_word = alias.expands_next_word;
            self.peeked_token = None;
            self.alias_replays
                .push(AliasReplay::new(&alias, self.current_span.start));
            self.advance_raw();
        }

        self.expand_next_word = expands_next_word;
    }

    pub(super) fn current_source_word_starts_posix_function_header(&self, name: &str) -> bool {
        if name.contains('=') || name.contains('[') {
            return false;
        }

        if self
            .current_token
            .as_ref()
            .is_some_and(|token| token.flags.is_synthetic())
        {
            return false;
        }

        let Some(tail) = self.input.get(self.current_span.end.offset..) else {
            return false;
        };
        let tail = tail.trim_start_matches([' ', '\t']);
        let Some(after_left) = tail.strip_prefix('(') else {
            return false;
        };
        let after_left = after_left.trim_start_matches([' ', '\t']);
        after_left.starts_with(')')
    }

    pub(super) fn next_word_char(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
    ) -> Option<char> {
        let ch = chars.next()?;
        cursor.advance(ch);
        Some(ch)
    }

    pub(super) fn next_word_char_unwrap(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
    ) -> char {
        let Some(ch) = Self::next_word_char(chars, cursor) else {
            unreachable!("word parser should only consume characters that were already peeked");
        };
        ch
    }

    pub(super) fn consume_word_char_if(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        expected: char,
    ) -> bool {
        if chars.peek() == Some(&expected) {
            Self::next_word_char_unwrap(chars, cursor);
            true
        } else {
            false
        }
    }

    pub(super) fn read_word_while<F>(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        mut predicate: F,
    ) -> String
    where
        F: FnMut(char) -> bool,
    {
        let mut text = String::new();
        while let Some(&ch) = chars.peek() {
            if !predicate(ch) {
                break;
            }
            text.push(Self::next_word_char_unwrap(chars, cursor));
        }
        text
    }

    pub(super) fn rebase_redirects(redirects: &mut [Redirect], base: Position) {
        for redirect in redirects {
            redirect.span = redirect.span.rebased(base);
            redirect.fd_var_span = redirect.fd_var_span.map(|span| span.rebased(base));
            match &mut redirect.target {
                RedirectTarget::Word(word) => Self::rebase_word(word, base),
                RedirectTarget::Heredoc(heredoc) => {
                    heredoc.delimiter.span = heredoc.delimiter.span.rebased(base);
                    Self::rebase_word(&mut heredoc.delimiter.raw, base);
                    Self::rebase_heredoc_body(&mut heredoc.body, base);
                }
            }
        }
    }

    pub(super) fn rebase_assignments(assignments: &mut [Assignment], base: Position) {
        for assignment in assignments {
            assignment.span = assignment.span.rebased(base);
            Self::rebase_var_ref(&mut assignment.target, base);
            match &mut assignment.value {
                AssignmentValue::Scalar(word) => Self::rebase_word(word, base),
                AssignmentValue::Compound(array) => Self::rebase_array_expr(array, base),
            }
        }
    }
}

struct RawRecoveryScanState {
    in_single: bool,
    in_double: bool,
    in_backtick: bool,
    escaped: bool,
    in_comment: bool,
    at_command_start: bool,
    at_token_start: bool,
    pending_dollar: bool,
    subst_paren_depth: usize,
    parameter_brace_depth: usize,
    case_depth: usize,
}

impl Default for RawRecoveryScanState {
    fn default() -> Self {
        Self {
            in_single: false,
            in_double: false,
            in_backtick: false,
            escaped: false,
            in_comment: false,
            at_command_start: true,
            at_token_start: true,
            pending_dollar: false,
            subst_paren_depth: 0,
            parameter_brace_depth: 0,
            case_depth: 0,
        }
    }
}

impl RawRecoveryScanState {
    fn is_plain(&self) -> bool {
        !self.in_single && !self.in_double && !self.in_backtick && !self.escaped
    }

    fn is_plain_top_level(&self) -> bool {
        self.is_plain() && self.subst_paren_depth == 0 && self.parameter_brace_depth == 0
    }

    fn update_quoted_state(&mut self, ch: char) -> bool {
        if self.escaped {
            self.escaped = false;
            self.at_command_start = false;
            self.at_token_start = false;
            return true;
        }

        match ch {
            '\\' if !self.in_single => {
                self.escaped = true;
                self.at_command_start = false;
                self.at_token_start = false;
                true
            }
            '\'' if !self.in_double && !self.in_backtick => {
                self.in_single = !self.in_single;
                self.at_command_start = false;
                self.at_token_start = false;
                true
            }
            '"' if !self.in_single && !self.in_backtick => {
                self.in_double = !self.in_double;
                self.at_command_start = false;
                self.at_token_start = false;
                true
            }
            '`' if !self.in_single && !self.in_double => {
                self.in_backtick = !self.in_backtick;
                self.at_command_start = false;
                self.at_token_start = false;
                true
            }
            _ => false,
        }
    }

    fn update_raw_syntax(&mut self, ch: char) {
        if !self.is_plain() {
            return;
        }

        if self.pending_dollar {
            self.pending_dollar = false;
            match ch {
                '(' => self.subst_paren_depth += 1,
                '{' => self.parameter_brace_depth += 1,
                _ => {}
            }
            self.at_command_start = false;
            self.at_token_start = false;
            return;
        }

        if ch == '$' {
            self.pending_dollar = true;
            self.at_command_start = false;
            self.at_token_start = false;
            return;
        }

        if self.subst_paren_depth > 0 {
            match ch {
                '(' => self.subst_paren_depth += 1,
                ')' => self.subst_paren_depth = self.subst_paren_depth.saturating_sub(1),
                _ => {}
            }
            self.at_command_start = false;
            self.at_token_start = false;
            return;
        }

        if self.parameter_brace_depth > 0 {
            match ch {
                '{' => self.parameter_brace_depth += 1,
                '}' => self.parameter_brace_depth = self.parameter_brace_depth.saturating_sub(1),
                _ => {}
            }
            self.at_command_start = false;
            self.at_token_start = false;
            return;
        }

        match ch {
            ' ' | '\t' | '\r' => self.at_token_start = true,
            '#' if self.at_token_start => self.in_comment = true,
            '\n' | ';' | '&' | '|' => {
                self.at_command_start = true;
                self.at_token_start = true;
            }
            _ => {
                self.at_command_start = false;
                self.at_token_start = false;
            }
        }
    }
}

fn raw_keyword_starts_at(input: &str, offset: usize, keyword: &str) -> bool {
    input[offset..].starts_with(keyword)
        && input[..offset]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_shell_name_char(ch))
        && input[offset + keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !is_shell_name_char(ch))
}

fn is_shell_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
