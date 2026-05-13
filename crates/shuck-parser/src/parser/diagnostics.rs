use super::*;

impl<'a> Parser<'a> {
    /// Create a parse error with the current position.
    pub(super) fn error(&self, message: impl Into<String>) -> Error {
        Error::parse_at(
            message,
            self.current_span.start.line,
            self.current_span.start.column,
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
        if self.fuel == 0 {
            let used = self.max_fuel - self.fuel;
            return Err(Error::parse(format!(
                "parser fuel exhausted ({} operations, max {})",
                used, self.max_fuel
            )));
        }
        self.fuel -= 1;
        Ok(())
    }

    /// Push nesting depth and check limit
    pub(super) fn push_depth(&mut self) -> Result<()> {
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            return Err(Error::parse(format!(
                "AST nesting too deep ({} levels, max {})",
                self.current_depth, self.max_depth
            )));
        }
        Ok(())
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
