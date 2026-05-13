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
