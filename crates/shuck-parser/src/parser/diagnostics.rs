use super::*;

impl<'a> Parser<'a> {
    /// Create a parse error with the current position.
    pub(super) fn error(&self, message: impl Into<String>) -> Error {
        Error::parse_at(
            message,
            self.current_span.start.line(),
            self.current_span.start.column(),
        )
    }

    pub(super) fn ensure_feature(
        &self,
        enabled: bool,
        feature: &str,
        unsupported_message: &str,
    ) -> Result<()> {
        if enabled {
            Ok(())
        } else {
            Err(self.error(format!("{feature} {unsupported_message}")))
        }
    }

    pub(super) fn ensure_double_bracket(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().double_bracket,
            "[[ ]] conditionals",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_arithmetic_for(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().arithmetic_for,
            "c-style for loops",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_coproc(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().coproc_keyword,
            "coprocess commands",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_arithmetic_command(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().arithmetic_command,
            "arithmetic commands",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_select_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().select_loop,
            "select loops",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_repeat_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.zsh_short_repeat_enabled(),
            "repeat loops",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_foreach_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.zsh_short_loops_enabled(),
            "foreach loops",
            "are not available in this shell mode",
        )
    }

    pub(super) fn ensure_function_keyword(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().function_keyword,
            "function keyword definitions",
            "are not available in this shell mode",
        )
    }

    /// Consume one unit of fuel, returning an error if exhausted
    pub(super) fn tick(&mut self) -> Result<()> {
        self.budget.tick()
    }

    /// Push nesting depth and check limit
    pub(super) fn push_depth(&mut self) -> Result<()> {
        let depth = self.current_depth + 1;
        if depth > self.max_depth {
            return Err(self.budget.depth_error(depth, self.max_depth));
        }
        self.budget.check_stack()?;
        self.current_depth = depth;
        Ok(())
    }

    /// Run `parse` one nesting level deeper, restoring the depth afterwards
    /// whether or not it succeeds.
    pub(super) fn with_nested_depth<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let depth = self.current_depth;
        let result = self.push_depth().and_then(|()| parse(self));
        self.current_depth = depth;
        result
    }

    /// Add a link to the operator chain in `chain`, failing once the links
    /// open across all enclosing chains exceed [`MAX_OPERATOR_CHAIN_LEN`].
    pub(super) fn extend_operator_chain(&self, chain: &mut ChainLinks, what: &str) -> Result<()> {
        chain.extend(&self.budget).map_err(|links| {
            self.budget.record(self.error(format!(
                "{what} too long ({links} chained operators, max {MAX_OPERATOR_CHAIN_LEN})"
            )))
        })
    }

    /// Limits for a parser nested one level below the current depth.
    pub(super) fn nested_limits(&self) -> Result<NestedLimits> {
        NestedLimits::below(self.current_depth, self.max_depth, &self.budget)
    }

    /// Pop nesting depth
    pub(super) fn pop_depth(&mut self) {
        if self.current_depth > 0 {
            self.current_depth -= 1;
        }
    }

    /// Check if current token is an error token and return the error if so
    pub(super) fn check_error_token(&self) -> Result<()> {
        if self.current_token_kind == Some(TokenKind::Error) {
            let msg = self
                .current_token
                .as_ref()
                .and_then(LexedToken::error_kind)
                .map(|kind| kind.message())
                .unwrap_or("unknown lexer error");
            return Err(self.error(format!("syntax error: {}", msg)));
        }
        Ok(())
    }

    pub(super) fn parse_diagnostic_from_error(&self, error: Error) -> ParseDiagnostic {
        let Error::Parse { message, .. } = error;
        ParseDiagnostic {
            message,
            span: self.current_span,
        }
    }
}
