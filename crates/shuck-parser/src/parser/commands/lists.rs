use super::*;

impl<'a> Parser<'a> {
    pub(super) fn apply_word_command_effects(&mut self, name: &Word, args: &[Word]) {
        let Some(name) = self.literal_word_text(name) else {
            return;
        };

        match name.as_str() {
            "shopt" => {
                let mut toggle = None;
                for arg in args {
                    let Some(arg) = self.literal_word_text(arg) else {
                        continue;
                    };
                    match arg.as_str() {
                        "-s" => toggle = Some(true),
                        "-u" => toggle = Some(false),
                        "expand_aliases" => {
                            if let Some(toggle) = toggle {
                                self.expand_aliases = toggle;
                            }
                        }
                        _ => {}
                    }
                }
            }
            "alias" => {
                for arg in args {
                    let Some(arg) = self.literal_word_text(arg) else {
                        continue;
                    };
                    if arg == "--" {
                        continue;
                    }
                    let Some((alias_name, value)) = arg.split_once('=') else {
                        continue;
                    };
                    self.aliases
                        .insert(alias_name.to_string(), self.compile_alias_definition(value));
                }
            }
            "unalias" => {
                for arg in args {
                    let Some(arg) = self.literal_word_text(arg) else {
                        continue;
                    };
                    match arg.as_str() {
                        "--" => {}
                        "-a" => self.aliases.clear(),
                        _ => {
                            self.aliases.remove(arg.as_str());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_stmt_effects(&mut self, stmt: &Stmt) {
        match &stmt.command {
            AstCommand::Simple(simple) => {
                self.apply_word_command_effects(&simple.name, &simple.args)
            }
            AstCommand::Binary(binary) if matches!(binary.op, BinaryOp::And | BinaryOp::Or) => {
                self.apply_stmt_effects(&binary.left);
                self.apply_stmt_effects(&binary.right);
            }
            _ => {}
        }
    }

    pub(in crate::parser) fn apply_stmt_list_effects(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.apply_stmt_effects(stmt);
        }
    }

    pub(in crate::parser) fn parse_command_list_required(&mut self) -> Result<Vec<Stmt>> {
        self.parse_command_list()?
            .ok_or_else(|| self.error("expected command"))
    }

    pub(super) fn skip_command_separators(&mut self) -> Result<()> {
        loop {
            self.skip_newlines()?;
            if self.at(TokenKind::Semicolon) {
                self.advance();
                continue;
            }
            break;
        }
        Ok(())
    }

    /// Parse the configured input.
    ///
    /// The returned [`ParseResult`] contains the best AST the parser could
    /// produce, plus recovery diagnostics and syntax facts. Use
    /// [`ParseResult::is_ok`] when a caller needs to reject recovered parses.
    pub fn parse(mut self) -> ParseResult {
        self.parse_impl()
    }

    #[cfg(feature = "benchmarking")]
    #[doc(hidden)]
    pub fn parse_with_benchmark_counters(self) -> (ParseResult, ParserBenchmarkCounters) {
        let mut parser = self.rebuild_with_benchmark_counters();
        let output = parser.parse_impl();
        (output, parser.finish_benchmark_counters())
    }

    pub(super) fn parse_command_list(&mut self) -> Result<Option<Vec<Stmt>>> {
        self.tick()?;
        let mut current = match self.parse_pipeline()? {
            Some(stmt) => stmt,
            None => return Ok(None),
        };

        let mut stmts = Vec::with_capacity(2);

        loop {
            let (op, terminator, allow_empty_tail) = match self.current_token_kind {
                Some(TokenKind::And) => (Some(BinaryOp::And), None, false),
                Some(TokenKind::Or) => (Some(BinaryOp::Or), None, false),
                Some(TokenKind::Semicolon) => (None, Some(StmtTerminator::Semicolon), true),
                Some(TokenKind::Background) => (
                    None,
                    Some(StmtTerminator::Background(BackgroundOperator::Plain)),
                    true,
                ),
                Some(TokenKind::BackgroundPipe) => (
                    None,
                    Some(StmtTerminator::Background(BackgroundOperator::Pipe)),
                    true,
                ),
                Some(TokenKind::BackgroundBang) => (
                    None,
                    Some(StmtTerminator::Background(BackgroundOperator::Bang)),
                    true,
                ),
                _ => break,
            };
            let operator_span = self.current_span;
            self.advance();

            self.skip_newlines()?;
            if allow_empty_tail && self.current_token.is_none() {
                current.terminator = terminator;
                current.terminator_span = Some(operator_span);
                stmts.push(current);
                return Ok(Some(stmts));
            }

            if let Some(binary_op) = op {
                if let Some(right) = self.parse_pipeline()? {
                    current = Self::binary_stmt(current, binary_op, operator_span, right);
                } else {
                    break;
                }
                continue;
            }

            let Some(terminator) = terminator else {
                unreachable!("list terminator should be present");
            };
            if let Some(next) = self.parse_pipeline()? {
                current.terminator = Some(terminator);
                current.terminator_span = Some(operator_span);
                stmts.push(current);
                current = next;
            } else if allow_empty_tail {
                if self
                    .current_keyword()
                    .is_some_and(Self::is_non_command_keyword)
                {
                    if matches!(terminator, StmtTerminator::Background(_)) {
                        current.terminator = Some(terminator);
                        current.terminator_span = Some(operator_span);
                        stmts.push(current);
                        return Ok(Some(stmts));
                    }
                    break;
                }
                if matches!(
                    self.current_token_kind,
                    Some(TokenKind::Semicolon | TokenKind::Newline)
                ) {
                    self.advance();
                }
                current.terminator = Some(terminator);
                current.terminator_span = Some(operator_span);
                stmts.push(current);
                return Ok(Some(stmts));
            } else {
                break;
            }
        }

        stmts.push(current);
        Ok(Some(stmts))
    }

    /// Parse a pipeline (commands connected by |)
    ///
    /// Handles `!` pipeline negation: `! cmd | cmd2` negates the exit code.
    pub(super) fn parse_pipeline(&mut self) -> Result<Option<Stmt>> {
        let start_span = self.current_span;

        // Check for pipeline negation: `! command`
        let negated = self.at(TokenKind::Word) && self.current_word_str() == Some("!");
        if negated {
            self.advance();
        }

        let mut stmt = match self.parse_command()? {
            Some(cmd) => Self::lower_non_sequence_command_to_stmt(cmd),
            None => {
                if negated {
                    return Err(self.error("expected command after !"));
                }
                return Ok(None);
            }
        };

        let mut saw_pipe = false;
        while self.at_in_set(PIPE_OPERATOR_TOKENS) {
            saw_pipe = true;
            let op = if self.at(TokenKind::PipeBoth) {
                BinaryOp::PipeAll
            } else {
                BinaryOp::Pipe
            };
            let operator_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            if let Some(cmd) = self.parse_command()? {
                let right = Self::lower_non_sequence_command_to_stmt(cmd);
                stmt = Self::binary_stmt(stmt, op, operator_span, right);
            } else {
                return Err(self.error("expected command after |"));
            }
        }

        if negated || saw_pipe {
            stmt.negated = negated;
            stmt.span = start_span.merge(self.current_span);
        }
        Ok(Some(stmt))
    }

    pub(super) fn parse_compound_with_redirects(
        &mut self,
        parser: impl FnOnce(&mut Self) -> Result<CompoundCommand>,
    ) -> Result<Option<Command>> {
        let compound = parser(self)?;
        let redirects = self.parse_trailing_redirects();
        Ok(Some(Command::Compound(Box::new(compound), redirects)))
    }

    pub(super) fn current_starts_prefix_redirect_compound(&self) -> bool {
        match self.current_keyword() {
            Some(Keyword::If)
            | Some(Keyword::While)
            | Some(Keyword::Until)
            | Some(Keyword::Case)
            | Some(Keyword::Select)
            | Some(Keyword::Time)
            | Some(Keyword::Coproc) => true,
            Some(Keyword::For) => self.dialect == ShellDialect::Zsh,
            Some(Keyword::Repeat) => self.zsh_short_repeat_enabled(),
            Some(Keyword::Foreach) => self.zsh_short_loops_enabled(),
            Some(Keyword::Function) => false,
            None => matches!(
                self.current_token_kind,
                Some(
                    TokenKind::DoubleLeftBracket
                        | TokenKind::DoubleLeftParen
                        | TokenKind::LeftParen
                        | TokenKind::LeftBrace
                )
            ),
            _ => false,
        }
    }

    pub(super) fn parse_prefix_redirected_compound_command(&mut self) -> Result<Option<Command>> {
        if !self.current_token_kind.is_some_and(Self::is_redirect_kind) {
            return Ok(None);
        }

        let checkpoint = self.checkpoint();
        let mut redirects = self.parse_trailing_redirects();
        if redirects.is_empty() || !self.current_starts_prefix_redirect_compound() {
            self.restore(checkpoint);
            return Ok(None);
        }

        let Some(mut command) = self.parse_command()? else {
            self.restore(checkpoint);
            return Ok(None);
        };

        match &mut command {
            Command::Compound(_, trailing) => {
                redirects.append(trailing);
                *trailing = redirects;
                Ok(Some(command))
            }
            _ => {
                self.restore(checkpoint);
                Ok(None)
            }
        }
    }

    pub(super) fn classify_flow_control_name(&self, word: &Word) -> Option<FlowControlBuiltinKind> {
        let name = self.single_literal_word_text(word)?;
        match name {
            "break" => Some(FlowControlBuiltinKind::Break),
            "continue" => Some(FlowControlBuiltinKind::Continue),
            "return" => Some(FlowControlBuiltinKind::Return),
            "exit" => Some(FlowControlBuiltinKind::Exit),
            _ => None,
        }
    }

    pub(super) fn classify_decl_variant_name(&self, word: &Word) -> Option<Name> {
        let name = self.single_literal_word_text(word)?;
        match name {
            "declare" | "local" | "export" | "readonly" | "typeset" => Some(Name::from(name)),
            "integer" if self.dialect == ShellDialect::Zsh => Some(Name::from(name)),
            _ => None,
        }
    }

    pub(super) fn classify_simple_command(&mut self, command: SimpleCommand) -> Command {
        let kind = self.classify_flow_control_name(&command.name);

        if let Some(kind) = kind {
            let SimpleCommand {
                args,
                redirects,
                assignments,
                span,
                ..
            } = command;
            let mut args = args.into_iter();

            return match kind {
                FlowControlBuiltinKind::Break => {
                    Command::Builtin(BuiltinCommand::Break(BreakCommand {
                        depth: args.next(),
                        extra_args: args.collect(),
                        redirects,
                        assignments,
                        span,
                    }))
                }
                FlowControlBuiltinKind::Continue => {
                    Command::Builtin(BuiltinCommand::Continue(ContinueCommand {
                        depth: args.next(),
                        extra_args: args.collect(),
                        redirects,
                        assignments,
                        span,
                    }))
                }
                FlowControlBuiltinKind::Return => {
                    Command::Builtin(BuiltinCommand::Return(ReturnCommand {
                        code: args.next(),
                        extra_args: args.collect(),
                        redirects,
                        assignments,
                        span,
                    }))
                }
                FlowControlBuiltinKind::Exit => {
                    Command::Builtin(BuiltinCommand::Exit(ExitCommand {
                        code: args.next(),
                        extra_args: args.collect(),
                        redirects,
                        assignments,
                        span,
                    }))
                }
            };
        }

        if let Some(variant) = self.classify_decl_variant_name(&command.name) {
            let SimpleCommand {
                name,
                args,
                redirects,
                assignments,
                span,
            } = command;
            return Command::Decl(Box::new(DeclClause {
                variant,
                variant_span: name.span,
                operands: self.classify_decl_operands(args),
                redirects,
                assignments,
                span,
            }));
        }

        Command::Simple(command)
    }

    pub(super) fn is_operand_like_double_paren_token(token: &LexedToken<'_>) -> bool {
        match token.kind {
            TokenKind::LiteralWord | TokenKind::QuotedWord => true,
            TokenKind::Word => token.word_string().is_some_and(|text| {
                !text.chars().all(|ch| ch.is_ascii_punctuation())
                    && !Self::word_contains_obvious_arithmetic_punctuation(&text)
            }),
            _ => false,
        }
    }

    pub(super) fn word_contains_obvious_arithmetic_punctuation(text: &str) -> bool {
        text.chars().any(|ch| {
            matches!(
                ch,
                ',' | '='
                    | '+'
                    | '*'
                    | '/'
                    | '%'
                    | '<'
                    | '>'
                    | '&'
                    | '|'
                    | '^'
                    | '!'
                    | '?'
                    | ':'
                    | '['
                    | ']'
            )
        })
    }

    pub(super) fn suspicious_double_paren_is_command_style(
        &mut self,
        checkpoint: &ParserCheckpoint<'a>,
    ) -> bool {
        self.restore(checkpoint.clone());
        let parses_as_arithmetic = self.parse_arithmetic_command().is_ok();
        self.restore(checkpoint.clone());
        !parses_as_arithmetic
    }

    pub(super) fn looks_like_command_style_double_paren(&mut self) -> bool {
        if self.current_token_kind != Some(TokenKind::DoubleLeftParen) {
            return false;
        }

        let checkpoint = self.checkpoint();
        self.advance();
        let mut paren_depth = 0_i32;
        let mut previous_top_level_operand = false;

        loop {
            match self.current_token_kind {
                Some(TokenKind::DoubleLeftParen) => {
                    paren_depth += 2;
                    previous_top_level_operand = false;
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    paren_depth += 1;
                    previous_top_level_operand = false;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if paren_depth == 0 {
                        self.restore(checkpoint);
                        return false;
                    }
                    if paren_depth == 1 {
                        self.restore(checkpoint);
                        return false;
                    }
                    paren_depth -= 2;
                    previous_top_level_operand = false;
                    self.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth == 0 {
                        return self.suspicious_double_paren_is_command_style(&checkpoint);
                    }
                    paren_depth -= 1;
                    previous_top_level_operand = false;
                    self.advance();
                }
                Some(TokenKind::Newline) | Some(TokenKind::Semicolon) if paren_depth == 0 => {
                    previous_top_level_operand = false;
                    self.advance();
                }
                Some(TokenKind::Comment) if self.dialect == ShellDialect::Zsh => {
                    self.restore(checkpoint);
                    return false;
                }
                Some(_)
                    if paren_depth == 0
                        && self
                            .current_token
                            .as_ref()
                            .is_some_and(Self::is_operand_like_double_paren_token) =>
                {
                    if previous_top_level_operand {
                        return self.suspicious_double_paren_is_command_style(&checkpoint);
                    }
                    previous_top_level_operand = true;
                    self.advance();
                }
                Some(_) => {
                    previous_top_level_operand = false;
                    self.advance();
                }
                None => {
                    self.restore(checkpoint);
                    return false;
                }
            }
        }
    }

    pub(super) fn split_current_double_left_paren(&mut self) {
        let (left_span, right_span) = Self::split_double_left_paren(self.current_span);
        self.set_current_kind(TokenKind::LeftParen, left_span);
        self.synthetic_tokens
            .push_front(SyntheticToken::punctuation(
                TokenKind::LeftParen,
                right_span,
            ));
    }

    pub(in crate::parser) fn split_current_double_right_paren(&mut self) {
        let (left_span, right_span) = Self::split_double_right_paren(self.current_span);
        self.set_current_kind(TokenKind::RightParen, left_span);
        self.synthetic_tokens
            .push_front(SyntheticToken::punctuation(
                TokenKind::RightParen,
                right_span,
            ));
    }

    /// Parse a single command (simple or compound)
    pub(super) fn parse_command(&mut self) -> Result<Option<Command>> {
        self.skip_newlines()?;
        self.check_error_token()?;
        self.maybe_expand_current_alias_chain();
        self.check_error_token()?;

        if !self.zsh_short_repeat_enabled() && self.looks_like_disabled_repeat_loop()? {
            self.ensure_repeat_loop()?;
        }
        if !self.zsh_short_loops_enabled() && self.looks_like_disabled_foreach_loop()? {
            self.ensure_foreach_loop()?;
        }

        if let Some(command) = self.parse_prefix_redirected_compound_command()? {
            return Ok(Some(command));
        }

        if let Some(command) = self.try_parse_zsh_attached_parens_function()? {
            return Ok(Some(command));
        }

        // Check for compound commands and function keyword
        match self.current_keyword() {
            Some(Keyword::If) => return self.parse_compound_with_redirects(|s| s.parse_if()),
            Some(Keyword::For) => return self.parse_compound_with_redirects(|s| s.parse_for()),
            Some(Keyword::Repeat) if self.zsh_short_repeat_enabled() => {
                return self.parse_compound_with_redirects(|s| s.parse_repeat());
            }
            Some(Keyword::Foreach) if self.zsh_short_loops_enabled() => {
                return self.parse_compound_with_redirects(|s| s.parse_foreach());
            }
            Some(Keyword::While) => {
                return self.parse_compound_with_redirects(|s| s.parse_while());
            }
            Some(Keyword::Until) => {
                return self.parse_compound_with_redirects(|s| s.parse_until());
            }
            Some(Keyword::Case) => return self.parse_compound_with_redirects(|s| s.parse_case()),
            Some(Keyword::Select) => {
                return self.parse_compound_with_redirects(|s| s.parse_select());
            }
            Some(Keyword::Time) => return self.parse_compound_with_redirects(|s| s.parse_time()),
            Some(Keyword::Coproc) => {
                return self.parse_compound_with_redirects(|s| s.parse_coproc());
            }
            Some(Keyword::Function) => return self.parse_function_keyword().map(Some),
            _ => {}
        }

        if self.at(TokenKind::Word)
            && let Some(word) = self.current_source_like_word_text()
            && self.peek_next_is(TokenKind::LeftParen)
        {
            let checkpoint = self.checkpoint();
            self.advance();
            self.advance();
            let is_right_paren = self.at(TokenKind::RightParen);
            self.restore(checkpoint);
            if is_right_paren {
                // Check for POSIX-style function: name() { body }
                // Exclude obvious assignment-like heads such as `a[(1+2)*3]=9`.
                if !word.contains('=') && !word.contains('[') {
                    return self.parse_function_posix().map(Some);
                }
            } else if word.contains('$') && !word.contains('=') {
                return Err(self.error("unexpected '(' after command word"));
            }
        }

        // Check for conditional expression [[ ... ]]
        if self.at(TokenKind::DoubleLeftBracket) {
            return self.parse_compound_with_redirects(|s| s.parse_conditional());
        }

        // Check for arithmetic command ((expression))
        if self.at(TokenKind::DoubleLeftParen) {
            if self.looks_like_command_style_double_paren() {
                self.split_current_double_left_paren();
                return self.parse_compound_with_redirects(|s| s.parse_subshell());
            }

            let checkpoint = self.checkpoint();
            if let Ok(compound) = self.parse_arithmetic_command() {
                let redirects = self.parse_trailing_redirects();
                return Ok(Some(Command::Compound(Box::new(compound), redirects)));
            }
            self.restore(checkpoint);

            self.split_current_double_left_paren();
            return self.parse_compound_with_redirects(|s| s.parse_subshell());
        }

        if self.dialect == ShellDialect::Zsh && self.at(TokenKind::LeftParen) {
            let checkpoint = self.checkpoint();
            self.advance();
            let is_right_paren = self.at(TokenKind::RightParen);
            self.restore(checkpoint);
            if is_right_paren {
                return self.parse_anonymous_paren_function().map(Some);
            }
        }

        // Check for subshell
        if self.at(TokenKind::LeftParen) {
            return self.parse_compound_with_redirects(|s| s.parse_subshell());
        }

        // Check for brace group
        if self.at(TokenKind::LeftBrace) {
            return self.parse_compound_with_redirects(|s| {
                s.parse_brace_group(BraceBodyContext::Ordinary)
            });
        }

        // Default to simple command
        match self.parse_simple_command()? {
            Some(cmd) => Ok(Some(self.classify_simple_command(cmd))),
            None => Ok(None),
        }
    }
}
