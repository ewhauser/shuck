use super::*;

impl<'a> Parser<'a> {
    /// Create a new bash-profile parser for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser for the given input and shell dialect.
    ///
    /// This uses [`ShellProfile::native`] for the selected dialect. Use
    /// [`Parser::with_profile`] when zsh option state is known.
    pub fn with_dialect(input: &'a str, dialect: ShellDialect) -> Self {
        Self::with_profile(input, ShellProfile::native(dialect))
    }

    /// Create a new parser for the given input and full shell profile.
    ///
    /// Profiles allow callers to provide parser-visible zsh option state in
    /// addition to the broad shell dialect.
    pub fn with_profile(input: &'a str, shell_profile: ShellProfile) -> Self {
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            shell_profile,
        )
    }

    /// Create a new bash parser with a custom maximum AST depth.
    ///
    /// The requested depth is clamped to the parser's hard safety cap. Hitting
    /// the limit produces a non-clean [`ParseResult`] rather than panicking.
    pub fn with_max_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            max_depth,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new bash parser with a custom fuel limit.
    ///
    /// Fuel bounds the number of parser operations. Exhaustion produces a
    /// terminal parse error in the returned [`ParseResult`].
    pub fn with_fuel(input: &'a str, max_fuel: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            max_fuel,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new bash parser with custom depth and fuel limits.
    ///
    /// `max_depth` is clamped to the parser's hard safety cap to prevent stack
    /// overflow from misconfiguration. `max_fuel` bounds parser operations.
    /// Either limit can produce a non-clean [`ParseResult`].
    pub fn with_limits(input: &'a str, max_depth: usize, max_fuel: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            max_depth,
            max_fuel,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser with custom depth, fuel, and dialect settings.
    ///
    /// This uses [`ShellProfile::native`] for `dialect`; use
    /// [`Parser::with_limits_and_profile`] when explicit zsh option state is
    /// available.
    pub fn with_limits_and_dialect(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        dialect: ShellDialect,
    ) -> Self {
        Self::with_limits_and_profile(input, max_depth, max_fuel, ShellProfile::native(dialect))
    }

    /// Create a new parser with custom depth, fuel, and shell-profile settings.
    ///
    /// This is the most explicit constructor for embedders that need both
    /// resource limits and parser-visible shell option state.
    pub fn with_limits_and_profile(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        shell_profile: ShellProfile,
    ) -> Self {
        Self::with_limits_and_profile_and_benchmarking(
            input,
            max_depth,
            max_fuel,
            shell_profile,
            false,
        )
    }

    pub(super) fn with_limits_and_profile_and_benchmarking(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        shell_profile: ShellProfile,
        benchmark_counters_enabled: bool,
    ) -> Self {
        #[cfg(not(feature = "benchmarking"))]
        let _ = benchmark_counters_enabled;

        let zsh_timeline = (shell_profile.dialect == ShellDialect::Zsh)
            .then(|| ZshOptionTimeline::build(input, &shell_profile))
            .flatten()
            .map(Arc::new);
        let mut lexer = Lexer::with_max_subst_depth_and_profile(
            input,
            max_depth.min(HARD_MAX_AST_DEPTH),
            &shell_profile,
            zsh_timeline.clone(),
        );
        #[cfg(feature = "benchmarking")]
        if benchmark_counters_enabled {
            lexer.enable_benchmark_counters();
        }
        let mut comments = Vec::new();
        let (current_token, current_token_kind, current_keyword, current_span) = loop {
            match lexer.next_lexed_token_with_comments() {
                Some(st) if st.kind == TokenKind::Comment => {
                    comments.push(Comment {
                        range: st.span.to_range(),
                    });
                }
                Some(st) => {
                    break (
                        Some(st.clone()),
                        Some(st.kind),
                        Self::keyword_from_token(&st),
                        st.span,
                    );
                }
                None => break (None, None, None, Span::new()),
            }
        };
        Self {
            input,
            lexer,
            synthetic_tokens: VecDeque::new(),
            alias_replays: Vec::new(),
            current_token,
            current_word_cache: None,
            current_token_kind,
            current_keyword,
            current_span,
            peeked_token: None,
            max_depth: max_depth.min(HARD_MAX_AST_DEPTH),
            current_depth: 0,
            fuel: max_fuel,
            max_fuel,
            source_text_pattern_depth: 0,
            comments,
            aliases: HashMap::new(),
            expand_aliases: false,
            expand_next_word: false,
            brace_group_depth: 0,
            brace_body_stack: Vec::new(),
            syntax_facts: SyntaxFacts::default(),
            dialect: shell_profile.dialect,
            shell_profile,
            zsh_timeline,
            #[cfg(feature = "benchmarking")]
            benchmark_counters: benchmark_counters_enabled.then(ParserBenchmarkCounters::default),
        }
    }

    #[cfg(feature = "benchmarking")]
    pub(super) fn rebuild_with_benchmark_counters(&self) -> Self {
        Self::with_limits_and_profile_and_benchmarking(
            self.input,
            self.max_depth,
            self.max_fuel,
            self.shell_profile.clone(),
            true,
        )
    }

    #[cfg(test)]
    pub(super) fn current_span(&self) -> Span {
        self.current_span
    }

    /// Parse a standalone shell word string.
    ///
    /// This handles shell word constructs such as parameter expansion, command
    /// substitution, arithmetic expansion, and quoting. The returned word is
    /// positioned as if `input` started at the beginning of a file.
    pub fn parse_word_string(input: &str) -> Word {
        let mut parser = Parser::new(input);
        let start = Position::new();
        parser.parse_word_with_context(
            input,
            Span::from_positions(start, start.advanced_by(input)),
            start,
            true,
        )
    }

    /// Classify a contiguous group of already-parsed words as a shell assignment.
    ///
    /// Some shell syntax, such as process substitution inside an array subscript,
    /// can produce multiple AST words while still occupying one contiguous
    /// assignment operand in the source.
    pub fn parse_assignment_word_group(
        source: &str,
        words: &[&Word],
        explicit_array_kind: Option<ArrayKind>,
        subscript_interpretation: SubscriptInterpretation,
    ) -> Option<Assignment> {
        let first = words.first()?;
        let last = words.last()?;
        let span = Span::from_positions(first.span.start, last.span.end);
        let raw = span.slice(source);
        let mut parser = Parser::new(source);
        parser.parse_assignment_from_text(raw, span, explicit_array_kind, subscript_interpretation)
    }

    /// Parse a word string with caller-configured limits and shell dialect.
    pub(super) fn parse_word_string_with_limits_and_dialect(
        input: &str,
        max_depth: usize,
        max_fuel: usize,
        dialect: ShellDialect,
    ) -> Word {
        let mut parser = Parser::with_limits_and_profile(
            input,
            max_depth,
            max_fuel,
            ShellProfile::native(dialect),
        );
        let start = Position::new();
        parser.parse_word_with_context(
            input,
            Span::from_positions(start, start.advanced_by(input)),
            start,
            true,
        )
    }

    /// Parse a fragment against the original source span so part offsets stay
    /// aligned with the surrounding script.
    #[cfg(test)]
    pub(super) fn parse_word_fragment(source: &str, text: &str, span: Span) -> Word {
        Self::parse_word_fragment_with_limits(
            source,
            text,
            span,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    pub(super) fn parse_word_fragment_with_limits(
        source: &str,
        text: &str,
        span: Span,
        max_depth: usize,
        max_fuel: usize,
        shell_profile: ShellProfile,
    ) -> Word {
        let mut parser = Parser::with_limits_and_profile(text, max_depth, max_fuel, shell_profile);
        let source_backed = span.end.offset <= source.len() && span.slice(source) == text;
        let start = Position::new();
        let fragment_span = Span::from_positions(start, start.advanced_by(text));
        let mut word = parser.parse_word_with_context(text, fragment_span, start, source_backed);
        if !source_backed {
            Self::materialize_word_source_backing(&mut word, text);
        }
        Self::rebase_word(&mut word, span.start);
        word.span = span;
        word
    }
}
