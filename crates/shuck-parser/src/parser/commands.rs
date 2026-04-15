use super::*;

#[derive(Debug, Clone, Copy)]
enum ForHeaderSurface {
    In {
        in_span: Option<Span>,
    },
    Paren {
        left_paren_span: Span,
        right_paren_span: Span,
    },
}

#[derive(Debug, Clone, Copy)]
struct ZshCaseScanState {
    position: Position,
    paren_depth: usize,
    bracket_depth: usize,
    brace_depth: usize,
    in_single: bool,
    in_double: bool,
    in_backtick: bool,
    escaped: bool,
}

impl ZshCaseScanState {
    fn new(position: Position) -> Self {
        Self {
            position,
            paren_depth: 0,
            bracket_depth: 0,
            brace_depth: 0,
            in_single: false,
            in_double: false,
            in_backtick: false,
            escaped: false,
        }
    }
}

impl<'a> Parser<'a> {
    fn apply_word_command_effects(&mut self, name: &Word, args: &[Word]) {
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

    fn apply_stmt_effects(&mut self, stmt: &Stmt) {
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

    fn apply_stmt_list_effects(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.apply_stmt_effects(stmt);
        }
    }

    fn parse_command_list_required(&mut self) -> Result<Vec<Stmt>> {
        self.parse_command_list()?
            .ok_or_else(|| self.error("expected command"))
    }

    fn skip_command_separators(&mut self) -> Result<()> {
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

    fn is_recovery_separator(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Newline
                | TokenKind::Semicolon
                | TokenKind::Background
                | TokenKind::BackgroundPipe
                | TokenKind::BackgroundBang
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Pipe
                | TokenKind::DoubleSemicolon
                | TokenKind::SemiAmp
                | TokenKind::SemiPipe
                | TokenKind::DoubleSemiAmp
        )
    }

    fn recover_to_command_boundary(&mut self, failed_offset: usize) -> bool {
        let mut advanced = false;

        while let Some(kind) = self.current_token_kind {
            if Self::is_recovery_separator(kind) {
                loop {
                    let Some(kind) = self.current_token_kind else {
                        break;
                    };
                    if !Self::is_recovery_separator(kind) {
                        break;
                    }
                    self.advance();
                    advanced = true;
                }
                break;
            }

            let before_offset = self.current_span.start.offset;
            self.advance();
            advanced = true;

            if self.current_token.is_none() {
                break;
            }

            if self.current_span.start.offset > failed_offset
                && before_offset != self.current_span.start.offset
            {
                continue;
            }
        }

        advanced
    }

    fn parse_impl(&mut self) -> ParseResult {
        let file_span =
            Span::from_positions(Position::new(), Position::new().advanced_by(self.input));
        let mut stmts = Vec::new();
        let mut diagnostics = Vec::new();
        let mut terminal_error = None;

        while self.current_token.is_some() {
            let checkpoint = self.current_span.start.offset;

            if let Err(error) = self.tick() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                terminal_error.get_or_insert(error);
                break;
            }
            if let Err(error) = self.skip_newlines() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                terminal_error.get_or_insert(error);
                break;
            }
            if let Err(error) = self.check_error_token() {
                diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                let recovered = self.recover_to_command_boundary(checkpoint);
                if recovered
                    || (self.current_token.is_some()
                        && self.current_span.start.offset < self.input.len())
                {
                    terminal_error.get_or_insert(error);
                }
                if !recovered && terminal_error.is_some() {
                    break;
                }
                continue;
            }
            if self.current_token.is_none() {
                break;
            }

            let command_start = self.current_span.start.offset;
            match self.parse_command_list_required() {
                Ok(command_stmts) => {
                    self.apply_stmt_list_effects(&command_stmts);
                    stmts.extend(command_stmts);
                }
                Err(error) => {
                    diagnostics.push(self.parse_diagnostic_from_error(error.clone()));
                    let recovered = self.recover_to_command_boundary(command_start);
                    if recovered
                        || (self.current_token.is_some()
                            && self.current_span.start.offset < self.input.len())
                    {
                        terminal_error.get_or_insert(error);
                    }
                    if !recovered && terminal_error.is_some() {
                        break;
                    }
                }
            }
        }

        let mut file = File {
            body: Self::stmt_seq_with_span(file_span, stmts),
            span: file_span,
        };
        self.attach_comments_to_file(&mut file);

        let status = if terminal_error.is_some() {
            ParseStatus::Fatal
        } else if diagnostics.is_empty() {
            ParseStatus::Clean
        } else {
            ParseStatus::Recovered
        };

        ParseResult {
            file,
            diagnostics,
            status,
            terminal_error,
            syntax_facts: std::mem::take(&mut self.syntax_facts),
        }
    }

    /// Parse the input and return the AST, recovery diagnostics, and syntax facts.
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

    fn parse_command_list(&mut self) -> Result<Option<Vec<Stmt>>> {
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

            let terminator = terminator.expect("list terminator should be present");
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
    fn parse_pipeline(&mut self) -> Result<Option<Stmt>> {
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

    fn parse_compound_with_redirects(
        &mut self,
        parser: impl FnOnce(&mut Self) -> Result<CompoundCommand>,
    ) -> Result<Option<Command>> {
        let compound = parser(self)?;
        let redirects = self.parse_trailing_redirects();
        Ok(Some(Command::Compound(Box::new(compound), redirects)))
    }

    fn current_starts_prefix_redirect_compound(&self) -> bool {
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

    fn parse_prefix_redirected_compound_command(&mut self) -> Result<Option<Command>> {
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

    fn classify_flow_control_name(&self, word: &Word) -> Option<FlowControlBuiltinKind> {
        let name = self.single_literal_word_text(word)?;
        match name {
            "break" => Some(FlowControlBuiltinKind::Break),
            "continue" => Some(FlowControlBuiltinKind::Continue),
            "return" => Some(FlowControlBuiltinKind::Return),
            "exit" => Some(FlowControlBuiltinKind::Exit),
            _ => None,
        }
    }

    fn classify_decl_variant_name(&self, word: &Word) -> Option<Name> {
        let name = self.single_literal_word_text(word)?;
        match name {
            "declare" | "local" | "export" | "readonly" | "typeset" => Some(Name::from(name)),
            _ => None,
        }
    }

    fn classify_simple_command(&mut self, command: SimpleCommand) -> Command {
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
            return Command::Decl(DeclClause {
                variant,
                variant_span: name.span,
                operands: self.classify_decl_operands(args),
                redirects,
                assignments,
                span,
            });
        }

        Command::Simple(command)
    }

    fn is_operand_like_double_paren_token(token: &LexedToken<'_>) -> bool {
        match token.kind {
            TokenKind::LiteralWord | TokenKind::QuotedWord => true,
            TokenKind::Word => token.word_string().is_some_and(|text| {
                !text.chars().all(|ch| ch.is_ascii_punctuation())
                    && !Self::word_contains_obvious_arithmetic_punctuation(&text)
            }),
            _ => false,
        }
    }

    fn word_contains_obvious_arithmetic_punctuation(text: &str) -> bool {
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

    fn looks_like_command_style_double_paren(&mut self) -> bool {
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
                        self.restore(checkpoint);
                        return true;
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
                        self.restore(checkpoint);
                        return true;
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

    fn split_current_double_left_paren(&mut self) {
        let (left_span, right_span) = Self::split_double_left_paren(self.current_span);
        self.set_current_kind(TokenKind::LeftParen, left_span);
        self.synthetic_tokens
            .push_front(SyntheticToken::punctuation(
                TokenKind::LeftParen,
                right_span,
            ));
    }

    fn split_current_double_right_paren(&mut self) {
        let (left_span, right_span) = Self::split_double_right_paren(self.current_span);
        self.set_current_kind(TokenKind::RightParen, left_span);
        self.synthetic_tokens
            .push_front(SyntheticToken::punctuation(
                TokenKind::RightParen,
                right_span,
            ));
    }

    /// Parse a single command (simple or compound)
    fn parse_command(&mut self) -> Result<Option<Command>> {
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

    /// Parse an if statement
    fn parse_if(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'if'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let allow_brace_syntax = self.zsh_brace_if_enabled();
        let condition = self.parse_if_condition_until_body_start(allow_brace_syntax)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        let (mut syntax, then_branch, brace_style) =
            if allow_brace_syntax && self.at(TokenKind::LeftBrace) {
                let (then_branch, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty then clause",
                        BraceBodyContext::IfClause,
                    )?;
                self.record_zsh_brace_if_span(left_brace_span);
                (
                    IfSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    },
                    then_branch,
                    true,
                )
            } else {
                let then_span = self.current_span;
                self.expect_keyword(Keyword::Then)?;
                self.skip_newlines()?;

                let then_start = self.current_span.start;
                let then_branch = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
                let then_branch_span = Span::from_positions(then_start, self.current_span.start);

                let then_branch = if then_branch.is_empty() {
                    if self.dialect == ShellDialect::Zsh && self.is_keyword(Keyword::Elif) {
                        Self::stmt_seq_with_span(then_branch_span, Vec::new())
                    } else {
                        self.pop_depth();
                        return Err(self.error("syntax error: empty then clause"));
                    }
                } else {
                    Self::stmt_seq_with_span(then_branch_span, then_branch)
                };

                (
                    IfSyntax::ThenFi {
                        then_span,
                        fi_span: Span::new(),
                    },
                    then_branch,
                    false,
                )
            };

        // Parse elif branches
        let mut elif_branches = Vec::new();
        while self.is_keyword(Keyword::Elif) {
            self.advance(); // consume 'elif'
            self.skip_newlines()?;

            let elif_condition_start = self.current_span.start;
            let elif_condition = self.parse_if_condition_until_body_start(brace_style)?;
            let elif_condition_span =
                Span::from_positions(elif_condition_start, self.current_span.start);
            let elif_condition = Self::stmt_seq_with_span(elif_condition_span, elif_condition);

            let elif_body = if brace_style {
                if !self.at(TokenKind::LeftBrace) {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start elif clause"));
                }
                self.parse_brace_enclosed_stmt_seq(
                    "syntax error: empty elif clause",
                    BraceBodyContext::IfClause,
                )?
                .0
            } else {
                self.expect_keyword(Keyword::Then)?;
                let elif_body_region_start = self.current_span.start;
                self.skip_newlines()?;

                let elif_body_start = self.current_span.start;
                let elif_body = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
                let elif_body_span = Span::from_positions(elif_body_start, self.current_span.start);

                if elif_body.is_empty() {
                    if self.dialect == ShellDialect::Zsh
                        && self.has_recorded_comment_between(
                            elif_body_region_start.offset,
                            self.current_span.start.offset,
                        )
                    {
                        Self::stmt_seq_with_span(
                            Span::from_positions(elif_body_region_start, self.current_span.start),
                            Vec::new(),
                        )
                    } else {
                        self.pop_depth();
                        return Err(self.error("syntax error: empty elif clause"));
                    }
                } else {
                    Self::stmt_seq_with_span(elif_body_span, elif_body)
                }
            };

            elif_branches.push((elif_condition, elif_body));
        }

        // Parse else branch
        let else_branch = if self.is_keyword(Keyword::Else) {
            self.advance(); // consume 'else'
            let else_region_start = self.current_span.start;
            self.skip_newlines()?;
            if brace_style {
                if !self.at(TokenKind::LeftBrace) {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start else clause"));
                }
                Some(
                    self.parse_brace_enclosed_stmt_seq(
                        "syntax error: empty else clause",
                        BraceBodyContext::IfClause,
                    )?
                    .0,
                )
            } else {
                let else_start = self.current_span.start;
                let branch = self.parse_compound_list(Keyword::Fi)?;
                let else_span = Span::from_positions(else_start, self.current_span.start);

                if branch.is_empty() {
                    if self.dialect == ShellDialect::Zsh
                        && self.has_recorded_comment_between(
                            else_region_start.offset,
                            self.current_span.start.offset,
                        )
                    {
                        Some(Self::stmt_seq_with_span(
                            Span::from_positions(else_region_start, self.current_span.start),
                            Vec::new(),
                        ))
                    } else {
                        self.pop_depth();
                        return Err(self.error("syntax error: empty else clause"));
                    }
                } else {
                    Some(Self::stmt_seq_with_span(else_span, branch))
                }
            }
        } else {
            None
        };

        if !brace_style {
            self.expect_keyword(Keyword::Fi)?;
            if let IfSyntax::ThenFi { then_span, .. } = syntax {
                syntax = IfSyntax::ThenFi {
                    then_span,
                    fi_span: self.current_span,
                };
            }
        }

        self.pop_depth();
        Ok(CompoundCommand::If(IfCommand {
            condition,
            then_branch,
            elif_branches,
            else_branch,
            syntax,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse a for loop
    fn parse_for(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'for'
        self.skip_newlines()?;

        // Check for C-style for loop: for ((init; cond; step))
        if self.at(TokenKind::DoubleLeftParen) {
            let result = self.parse_arithmetic_for_inner(start_span);
            self.pop_depth();
            return result;
        }

        let allow_zsh_targets = self.dialect == ShellDialect::Zsh;
        let targets = match self.parse_for_targets(allow_zsh_targets) {
            Ok(targets) => targets,
            Err(error) => {
                self.pop_depth();
                return Err(error);
            }
        };

        if allow_zsh_targets {
            self.skip_newlines()?;
        }

        let (words, header) = if allow_zsh_targets && self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();

            let mut words = Vec::new();
            while !self.at(TokenKind::RightParen) {
                if self.at(TokenKind::Newline) {
                    self.skip_newlines()?;
                    continue;
                }
                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected for word"))?;
                        words.push(word);
                    }
                    Some(_) | None => {
                        self.pop_depth();
                        return Err(self.error("expected ')' after for word list"));
                    }
                }
            }

            let right_paren_span = self.current_span;
            self.advance();
            if self.at(TokenKind::Semicolon) {
                self.advance();
            }
            self.skip_newlines()?;

            (
                Some(words),
                ForHeaderSurface::Paren {
                    left_paren_span,
                    right_paren_span,
                },
            )
        } else if self.is_keyword(Keyword::In) {
            let in_span = self.current_span;
            self.advance();

            let (words, saw_separator) = self.parse_for_word_list_until_body_separator()?;
            if !saw_separator {
                self.pop_depth();
                return Err(self.error("expected ';' or newline before for loop body"));
            }
            (
                Some(words),
                ForHeaderSurface::In {
                    in_span: Some(in_span),
                },
            )
        } else {
            if self.at(TokenKind::Semicolon) {
                self.advance();
            }
            self.skip_newlines()?;
            (None, ForHeaderSurface::In { in_span: None })
        };

        let (body, syntax, end_span) = match header {
            ForHeaderSurface::In { in_span }
                if allow_zsh_targets && self.at(TokenKind::LeftBrace) =>
            {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty for loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    body,
                    ForSyntax::InBrace {
                        in_span,
                        left_brace_span,
                        right_brace_span,
                    },
                    right_brace_span,
                )
            }
            ForHeaderSurface::Paren {
                left_paren_span,
                right_paren_span,
            } if allow_zsh_targets && self.at(TokenKind::LeftBrace) => {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty for loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    body,
                    ForSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    },
                    right_brace_span,
                )
            }
            ForHeaderSurface::In { in_span } => {
                let do_span = if self.is_keyword(Keyword::Do) {
                    self.current_span
                } else {
                    self.pop_depth();
                    return Err(self.error("expected 'do'"));
                };
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() && self.dialect != ShellDialect::Zsh {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty for loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                let body = if body.is_empty() {
                    Self::stmt_seq_with_span(body_span, Vec::new())
                } else {
                    Self::stmt_seq_with_span(body_span, body)
                };
                (
                    body,
                    ForSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    },
                    done_span,
                )
            }
            ForHeaderSurface::Paren {
                left_paren_span,
                right_paren_span,
            } => {
                let do_span = if self.is_keyword(Keyword::Do) {
                    self.current_span
                } else {
                    self.pop_depth();
                    return Err(self.error("expected 'do'"));
                };
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() && self.dialect != ShellDialect::Zsh {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty for loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                let body = if body.is_empty() {
                    Self::stmt_seq_with_span(body_span, Vec::new())
                } else {
                    Self::stmt_seq_with_span(body_span, body)
                };
                (
                    body,
                    ForSyntax::ParenDoDone {
                        left_paren_span,
                        right_paren_span,
                        do_span,
                        done_span,
                    },
                    done_span,
                )
            }
        };

        self.pop_depth();
        Ok(CompoundCommand::For(ForCommand {
            targets,
            words,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    fn parse_for_targets(&mut self, allow_zsh_targets: bool) -> Result<Vec<ForTarget>> {
        let allow_digits = allow_zsh_targets;
        let first_target = self
            .current_for_target(allow_digits)
            .ok_or_else(|| Error::parse("expected variable name in for loop".to_string()))?;
        let first_word = first_target.word.clone();
        self.advance_past_word(&first_word);

        let mut targets = vec![first_target];
        if !allow_zsh_targets {
            return Ok(targets);
        }

        loop {
            if self.current_keyword() == Some(Keyword::In)
                || matches!(
                    self.current_token_kind,
                    Some(TokenKind::LeftParen | TokenKind::Semicolon | TokenKind::Newline)
                )
                || self.at(TokenKind::LeftBrace)
                || self.is_keyword(Keyword::Do)
            {
                break;
            }

            let target = self
                .current_for_target(true)
                .ok_or_else(|| Error::parse("expected variable name in for loop".to_string()))?;
            let word = target.word.clone();
            self.advance_past_word(&word);
            targets.push(target);
        }

        Ok(targets)
    }

    fn current_for_target(&mut self, allow_digits: bool) -> Option<ForTarget> {
        let name = self.current_word_str().and_then(|name| {
            (Self::is_valid_identifier(name)
                || (allow_digits && name.bytes().all(|byte| byte.is_ascii_digit())))
            .then(|| Name::from(name))
        });
        let word = self.current_word()?;
        Some(ForTarget {
            span: word.span,
            word,
            name,
        })
    }

    fn parse_for_word_list_until_body_separator(&mut self) -> Result<(Vec<Word>, bool)> {
        let mut words = Vec::new();
        loop {
            match self.current_token_kind {
                Some(kind)
                    if kind.is_word_like()
                        || (self.dialect == ShellDialect::Zsh
                            && matches!(kind, TokenKind::LeftParen)) =>
                {
                    if self.dialect == ShellDialect::Zsh
                        && self
                            .current_token
                            .as_ref()
                            .is_some_and(|token| !token.flags.is_synthetic())
                    {
                        let start = self.current_span.start;
                        if let Some((text, end)) = self.scan_source_word(start) {
                            let span = Span::from_positions(start, end);
                            let word = self.parse_word_with_context(&text, span, start, true);
                            self.advance_past_word(&word);
                            words.push(word);
                            continue;
                        }
                    }

                    let word = self
                        .take_current_word_and_advance()
                        .ok_or_else(|| self.error("expected for word"))?;
                    words.push(word);
                }
                Some(TokenKind::Semicolon) => {
                    self.advance();
                    self.skip_newlines()?;
                    return Ok((words, true));
                }
                Some(TokenKind::Newline) => {
                    self.skip_newlines()?;
                    return Ok((words, true));
                }
                _ => return Ok((words, false)),
            }
        }
    }

    /// Parse a zsh repeat loop.
    fn parse_repeat(&mut self) -> Result<CompoundCommand> {
        self.ensure_repeat_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'repeat'

        let count = match self.current_token_kind {
            Some(kind) if kind.is_word_like() => self.expect_word()?,
            _ => {
                self.pop_depth();
                return Err(self.error("expected loop count in repeat"));
            }
        };

        let (syntax, body, end_span) = match self.current_token_kind {
            _ if self.is_keyword(Keyword::Do) => {
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            Some(TokenKind::LeftBrace) => {
                let (body, left_brace_span, right_brace_span) = self
                    .parse_brace_enclosed_stmt_seq(
                        "syntax error: empty repeat loop body",
                        BraceBodyContext::Ordinary,
                    )?;
                (
                    RepeatSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    },
                    body,
                    right_brace_span,
                )
            }
            Some(TokenKind::Semicolon) => {
                self.advance();
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            Some(TokenKind::Newline) => {
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::stmt_seq_with_span(body_span, body),
                    done_span,
                )
            }
            _ => {
                let stmt = self.parse_single_stmt_command()?;
                let span = stmt.span;
                (
                    RepeatSyntax::Direct,
                    Self::stmt_seq_with_span(span, vec![stmt]),
                    span,
                )
            }
        };

        self.pop_depth();
        Ok(CompoundCommand::Repeat(RepeatCommand {
            count,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse a zsh foreach loop.
    fn parse_foreach(&mut self) -> Result<CompoundCommand> {
        self.ensure_foreach_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'foreach'

        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
            _ => {
                self.pop_depth();
                return Err(self.error("expected variable name in foreach"));
            }
        };
        self.advance();

        let (words, body, syntax, end_span) = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();

            let mut words = Vec::new();
            while !self.at(TokenKind::RightParen) {
                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                    }
                    Some(_) | None => {
                        self.pop_depth();
                        return Err(self.error("expected ')' after foreach word list"));
                    }
                }
            }
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }

            let right_paren_span = self.current_span;
            self.advance();
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after foreach word list"));
            }

            let (body, left_brace_span, right_brace_span) = self.parse_brace_enclosed_stmt_seq(
                "syntax error: empty foreach loop body",
                BraceBodyContext::Ordinary,
            )?;
            (
                words,
                body,
                ForeachSyntax::ParenBrace {
                    left_paren_span,
                    right_paren_span,
                    left_brace_span,
                    right_brace_span,
                },
                right_brace_span,
            )
        } else if self.is_keyword(Keyword::In) {
            let in_span = self.current_span;
            self.advance();

            let mut words = Vec::new();
            let saw_separator = loop {
                match self.current_token_kind {
                    _ if self.current_keyword() == Some(Keyword::Do) => break false,
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .take_current_word_and_advance()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                    }
                    Some(TokenKind::Semicolon) => {
                        self.advance();
                        break true;
                    }
                    Some(TokenKind::Newline) => {
                        self.skip_newlines()?;
                        break true;
                    }
                    _ => break false,
                }
            };
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }
            if !saw_separator {
                self.pop_depth();
                return Err(self.error("expected ';' or newline before 'do' in foreach"));
            }
            if !self.is_keyword(Keyword::Do) {
                self.pop_depth();
                return Err(self.error("expected 'do' in foreach"));
            }
            let do_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);
            if body.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty foreach loop body"));
            }
            if !self.is_keyword(Keyword::Done) {
                self.pop_depth();
                return Err(self.error("expected 'done'"));
            }
            let done_span = self.current_span;
            self.advance();
            (
                words,
                Self::stmt_seq_with_span(body_span, body),
                ForeachSyntax::InDoDone {
                    in_span,
                    do_span,
                    done_span,
                },
                done_span,
            )
        } else {
            self.pop_depth();
            return Err(self.error("expected '(' or 'in' after foreach variable"));
        };

        self.pop_depth();
        Ok(CompoundCommand::Foreach(ForeachCommand {
            variable,
            variable_span,
            words,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse select loop: select var in list; do body; done
    fn parse_select(&mut self) -> Result<CompoundCommand> {
        self.ensure_select_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'select'
        self.skip_newlines()?;

        // Expect variable name
        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
            _ => {
                self.pop_depth();
                return Err(Error::parse("expected variable name in select".to_string()));
            }
        };
        self.advance();

        // Expect 'in' keyword
        if !self.is_keyword(Keyword::In) {
            self.pop_depth();
            return Err(Error::parse("expected 'in' in select".to_string()));
        }
        self.advance(); // consume 'in'

        // Parse word list until do/newline/;
        let mut words = Vec::new();
        loop {
            match self.current_token_kind {
                _ if self.current_keyword() == Some(Keyword::Do) => break,
                Some(kind) if kind.is_word_like() => {
                    if let Some(word) = self.take_current_word_and_advance() {
                        words.push(word);
                    }
                }
                Some(TokenKind::Newline | TokenKind::Semicolon) => {
                    self.advance();
                    break;
                }
                _ => break,
            }
        }

        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty select loop body"));
        }
        let body = Self::stmt_seq_with_span(body_span, body);

        // Expect 'done'
        self.expect_keyword(Keyword::Done)?;

        self.pop_depth();
        Ok(CompoundCommand::Select(SelectCommand {
            variable,
            variable_span,
            words,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse C-style arithmetic for loop inner: for ((init; cond; step)); do body; done
    /// Note: depth tracking is done by parse_for which calls this
    fn parse_arithmetic_for_inner(&mut self, start_span: Span) -> Result<CompoundCommand> {
        self.ensure_arithmetic_for()?;
        let left_paren_span = self.current_span;
        self.advance(); // consume '(('

        let mut paren_depth = 0_i32;
        let mut segment_start = left_paren_span.end;
        let mut init_span = None;
        let mut first_semicolon_span = None;
        let mut condition_span = None;
        let mut second_semicolon_span = None;

        let right_paren_span = loop {
            match self.current_token_kind {
                Some(TokenKind::DoubleLeftParen) => {
                    paren_depth += 2;
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    paren_depth += 1;
                    self.advance();
                }
                Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                    paren_depth += 1;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if paren_depth == 0 {
                        let right_paren_span = self.current_span;
                        self.advance();
                        break right_paren_span;
                    }
                    if paren_depth == 1 {
                        break self.split_nested_arithmetic_close("arithmetic for header")?;
                    }
                    paren_depth -= 2;
                    self.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    self.advance();
                }
                Some(TokenKind::DoubleSemicolon) if paren_depth == 0 => {
                    let (first_span, second_span) = Self::split_double_semicolon(self.current_span);
                    Self::record_arithmetic_for_separator(
                        first_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    Self::record_arithmetic_for_separator(
                        second_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    self.advance();
                }
                Some(TokenKind::Semicolon) if paren_depth == 0 => {
                    Self::record_arithmetic_for_separator(
                        self.current_span,
                        &mut segment_start,
                        &mut init_span,
                        &mut first_semicolon_span,
                        &mut condition_span,
                        &mut second_semicolon_span,
                    )?;
                    self.advance();
                }
                Some(_) => {
                    self.advance();
                }
                None => {
                    return Err(Error::parse(
                        "unexpected end of input in for loop".to_string(),
                    ));
                }
            }
        };

        let first_semicolon_span = first_semicolon_span
            .ok_or_else(|| Error::parse("expected ';' in arithmetic for header".to_string()))?;
        let second_semicolon_span = second_semicolon_span.ok_or_else(|| {
            Error::parse("expected second ';' in arithmetic for header".to_string())
        })?;
        let step_span = Self::optional_span(segment_start, right_paren_span.start);
        let init_ast =
            self.parse_explicit_arithmetic_span(init_span, "invalid arithmetic for init")?;
        let condition_ast = self
            .parse_explicit_arithmetic_span(condition_span, "invalid arithmetic for condition")?;
        let step_ast =
            self.parse_explicit_arithmetic_span(step_span, "invalid arithmetic for step")?;

        self.skip_newlines()?;

        // Skip optional semicolon after ))
        if self.at(TokenKind::Semicolon) {
            self.advance();
        }
        self.skip_newlines()?;

        let (body, end_span) = if self.at(TokenKind::LeftBrace) {
            let body = self.parse_brace_group(BraceBodyContext::Ordinary)?;
            let span = Self::compound_span(&body);
            (
                Self::stmt_seq_with_span(
                    span,
                    vec![Self::lower_non_sequence_command_to_stmt(Command::Compound(
                        Box::new(body),
                        Vec::new(),
                    ))],
                ),
                self.current_span,
            )
        } else {
            // Expect 'do'
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            // Parse body
            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);

            // Bash requires at least one command in loop body
            if body.is_empty() {
                return Err(self.error("syntax error: empty for loop body"));
            }

            // Expect 'done'
            if !self.is_keyword(Keyword::Done) {
                return Err(self.error("expected 'done'"));
            }
            let done_span = self.current_span;
            self.advance();
            (Self::stmt_seq_with_span(body_span, body), done_span)
        };

        Ok(CompoundCommand::ArithmeticFor(Box::new(
            ArithmeticForCommand {
                left_paren_span,
                init_span,
                init_ast,
                first_semicolon_span,
                condition_span,
                condition_ast,
                second_semicolon_span,
                step_span,
                step_ast,
                right_paren_span,
                body,
                span: start_span.merge(end_span),
            },
        )))
    }

    /// Parse a while loop
    fn parse_while(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'while'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let condition = self.parse_compound_list(Keyword::Do)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() && self.dialect != ShellDialect::Zsh {
            self.pop_depth();
            return Err(self.error("syntax error: empty while loop body"));
        }
        let body = Self::stmt_seq_with_span(body_span, body);

        // Expect 'done'
        self.expect_keyword(Keyword::Done)?;

        self.pop_depth();
        Ok(CompoundCommand::While(WhileCommand {
            condition,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse an until loop
    fn parse_until(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'until'
        self.skip_newlines()?;

        // Parse condition
        let condition_start = self.current_span.start;
        let condition = self.parse_compound_list(Keyword::Do)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::stmt_seq_with_span(condition_span, condition);

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() && self.dialect != ShellDialect::Zsh {
            self.pop_depth();
            return Err(self.error("syntax error: empty until loop body"));
        }
        let body = Self::stmt_seq_with_span(body_span, body);

        // Expect 'done'
        self.expect_keyword(Keyword::Done)?;

        self.pop_depth();
        Ok(CompoundCommand::Until(UntilCommand {
            condition,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse a case statement: case WORD in pattern) commands ;; ... esac
    fn parse_case(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'case'
        self.skip_newlines()?;

        // Get the word to match against
        let word = self.expect_word()?;
        self.skip_newlines()?;

        // Expect 'in'
        self.expect_keyword(Keyword::In)?;
        self.skip_newlines()?;

        // Parse case items
        let mut cases = Vec::new();
        while !self.is_keyword(Keyword::Esac) && self.current_token.is_some() {
            self.skip_newlines()?;
            if self.is_keyword(Keyword::Esac) {
                break;
            }

            let patterns = match self.parse_case_patterns() {
                Ok(patterns) => patterns,
                Err(err) => {
                    self.pop_depth();
                    return Err(err);
                }
            };
            self.skip_newlines()?;

            // Parse commands until ;; or esac
            let body_start = self.current_span.start;
            let mut commands = Vec::new();
            while !self.is_case_terminator()
                && !self.is_keyword(Keyword::Esac)
                && self.current_token.is_some()
            {
                commands.extend(self.parse_command_list_required()?);
                self.skip_newlines()?;
            }

            let (terminator, terminator_span) = self.parse_case_terminator();
            let body_span = Span::from_positions(body_start, self.current_span.start);
            cases.push(CaseItem {
                patterns,
                body: Self::stmt_seq_with_span(body_span, commands),
                terminator,
                terminator_span,
            });
            self.skip_newlines()?;
        }

        // Expect 'esac'
        self.expect_keyword(Keyword::Esac)?;

        self.pop_depth();
        Ok(CompoundCommand::Case(CaseCommand {
            word,
            cases,
            span: start_span.merge(self.current_span),
        }))
    }

    fn parse_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        self.record_zsh_case_group_parts_from_current_case_header();
        if self.dialect == ShellDialect::Zsh {
            self.parse_zsh_case_patterns()
        } else {
            self.parse_posix_case_patterns()
        }
    }

    fn record_zsh_case_group_parts_from_current_case_header(&mut self) {
        let Ok((pattern_spans, _)) = self.scan_zsh_case_pattern_spans() else {
            return;
        };

        for span in pattern_spans {
            let pattern = self.pattern_from_zsh_case_span(span);
            for (index, part) in pattern.parts.iter().enumerate() {
                if matches!(
                    &part.kind,
                    PatternPart::Group {
                        kind: PatternGroupKind::ExactlyOne,
                        ..
                    }
                ) && part.span.slice(self.input).starts_with('(')
                {
                    self.record_zsh_case_group_part(index, part.span);
                }
            }
        }
    }

    fn parse_posix_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        if self.at(TokenKind::LeftParen) {
            self.advance();
        }

        let mut patterns = Vec::new();
        while self.at_word_like() {
            if let Some(word) = self.take_current_word_and_advance() {
                patterns.push(self.pattern_from_word(&word));
            }

            if self.at(TokenKind::Pipe) {
                self.advance();
            } else {
                break;
            }
        }

        if !self.at(TokenKind::RightParen) {
            return Err(self.error("expected ')' after case pattern"));
        }
        self.advance();

        Ok(patterns)
    }

    fn parse_zsh_case_patterns(&mut self) -> Result<Vec<Pattern>> {
        let (pattern_spans, delimiter_span) = self.scan_zsh_case_pattern_spans()?;
        let patterns = pattern_spans
            .into_iter()
            .map(|span| self.pattern_from_zsh_case_span(span))
            .collect::<Vec<_>>();

        while self.current_token.is_some()
            && self.current_span.start.offset < delimiter_span.end.offset
        {
            self.advance();
        }

        Ok(patterns)
    }

    fn scan_zsh_case_pattern_spans(&self) -> Result<(Vec<Span>, Span)> {
        let start = self.current_span.start;
        let Some((spans, delimiter_span)) = self.try_scan_zsh_case_pattern_spans(start) else {
            return Err(self.error("expected ')' after case pattern"));
        };
        if spans.is_empty() {
            return Err(self.error("expected ')' after case pattern"));
        }
        Ok((spans, delimiter_span))
    }

    fn try_scan_zsh_case_pattern_spans(&self, start: Position) -> Option<(Vec<Span>, Span)> {
        if self.input[start.offset..].starts_with('(')
            && let Some(wrapper_close) = self.scan_zsh_case_group_close(start)
            && self.case_wrapper_close_is_arm_delimiter(wrapper_close)
        {
            let inner_start = start.advanced_by("(");
            let inner_span = Span::from_positions(inner_start, wrapper_close.start);
            let patterns = self.split_zsh_case_pattern_alternatives(inner_span)?;
            return Some((patterns, wrapper_close));
        }

        let delimiter_span = self.scan_zsh_case_arm_delimiter(start)?;
        let header_span = Span::from_positions(start, delimiter_span.start);
        let patterns = self.split_zsh_case_pattern_alternatives(header_span)?;
        Some((patterns, delimiter_span))
    }

    fn case_wrapper_close_is_arm_delimiter(&self, close_span: Span) -> bool {
        self.input[close_span.end.offset..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    }

    fn split_zsh_case_pattern_alternatives(&self, span: Span) -> Option<Vec<Span>> {
        let mut state = ZshCaseScanState::new(span.start);
        let mut chars = self.input[span.start.offset..span.end.offset]
            .chars()
            .peekable();
        let mut part_start = span.start;
        let mut parts = Vec::new();

        while let Some(ch) = chars.peek().copied() {
            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0 =>
                {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    state.paren_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth > 0 =>
                {
                    state.paren_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '|' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth == 0 =>
                {
                    let end = state.position;
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    parts.push(
                        self.trim_zsh_case_pattern_span(Span::from_positions(part_start, end))?,
                    );
                    part_start = state.position;
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }
        }

        parts.push(
            self.trim_zsh_case_pattern_span(Span::from_positions(part_start, state.position))?,
        );
        Some(parts)
    }

    fn trim_zsh_case_pattern_span(&self, span: Span) -> Option<Span> {
        let text = span.slice(self.input);
        let trimmed_start = text.len() - text.trim_start_matches(char::is_whitespace).len();
        let trimmed_end = text.trim_end_matches(char::is_whitespace).len();
        if trimmed_end <= trimmed_start {
            return None;
        }
        let start = span.start.advanced_by(&text[..trimmed_start]);
        let end = span.start.advanced_by(&text[..trimmed_end]);
        Some(Span::from_positions(start, end))
    }

    fn scan_zsh_case_group_close(&self, start: Position) -> Option<Span> {
        let mut state = ZshCaseScanState::new(start);
        let mut chars = self.input[start.offset..].chars().peekable();

        if Self::next_word_char_unwrap(&mut chars, &mut state.position) != '(' {
            return None;
        }
        state.paren_depth = 1;

        while let Some(ch) = chars.peek().copied() {
            let ch_start = state.position;

            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    state.paren_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0
                    && state.paren_depth > 0 =>
                {
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    state.paren_depth -= 1;
                    if state.paren_depth == 0 {
                        return Some(Span::from_positions(ch_start, state.position));
                    }
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }
        }

        None
    }

    fn scan_zsh_case_arm_delimiter(&self, start: Position) -> Option<Span> {
        let mut state = ZshCaseScanState::new(start);
        let mut chars = self.input[start.offset..].chars().peekable();

        while let Some(ch) = chars.peek().copied() {
            let ch_start = state.position;

            if state.escaped {
                state.escaped = false;
                Self::next_word_char_unwrap(&mut chars, &mut state.position);
                continue;
            }

            match ch {
                '\\' if !state.in_single => {
                    state.escaped = true;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '\'' if !state.in_double && !state.in_backtick => {
                    state.in_single = !state.in_single;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '"' if !state.in_single && !state.in_backtick => {
                    state.in_double = !state.in_double;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '`' if !state.in_single && !state.in_double => {
                    state.in_backtick = !state.in_backtick;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '[' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.bracket_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ']' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth > 0 =>
                {
                    state.bracket_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '{' if !state.in_single && !state.in_double && !state.in_backtick => {
                    state.brace_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '}' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.brace_depth > 0 =>
                {
                    state.brace_depth -= 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                '(' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    state.paren_depth += 1;
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
                ')' if !state.in_single
                    && !state.in_double
                    && !state.in_backtick
                    && state.bracket_depth == 0
                    && state.brace_depth == 0 =>
                {
                    let _ = Self::next_word_char_unwrap(&mut chars, &mut state.position);
                    if state.paren_depth == 0 {
                        return Some(Span::from_positions(ch_start, state.position));
                    }
                    state.paren_depth -= 1;
                }
                _ => {
                    Self::next_word_char_unwrap(&mut chars, &mut state.position);
                }
            }
        }

        None
    }

    /// Parse a time command: time [-p] [command]
    ///
    /// The time keyword measures execution time of the following command.
    /// Note: Shuck only tracks wall-clock time, not CPU user/sys time.
    fn parse_time(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.advance(); // consume 'time'
        self.skip_newlines()?;

        // Check for -p flag (POSIX format)
        let posix_format = if self.at(TokenKind::Word) && self.current_word_str() == Some("-p") {
            self.advance();
            self.skip_newlines()?;
            true
        } else {
            false
        };

        // Parse the command to time (if any)
        // time with no command is valid in bash (just outputs timing header)
        let command = self.parse_pipeline()?.map(Box::new);

        Ok(CompoundCommand::Time(TimeCommand {
            posix_format,
            command,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse a coproc command: `coproc [NAME] command`
    ///
    /// If the token after `coproc` is a simple word followed by a compound
    /// command (`{`, `(`, `while`, `for`, etc.), it is treated as the coproc
    /// name. Otherwise the command starts immediately and the default name
    /// "COPROC" is used.
    fn parse_coproc(&mut self) -> Result<CompoundCommand> {
        self.ensure_coproc()?;
        let start_span = self.current_span;
        self.advance(); // consume 'coproc'
        self.skip_newlines()?;

        // Determine if next token is a NAME (simple word that is NOT a compound-
        // command keyword and is followed by a compound command start).
        let (name, name_span) = if self.at(TokenKind::Word) {
            let word = self.current_word_str().unwrap().to_string();
            let word_span = self.current_span;
            let is_compound_keyword = matches!(
                word.as_str(),
                "if" | "for" | "while" | "until" | "case" | "select" | "time" | "coproc"
            );
            let next_is_compound_start = matches!(
                self.peek_next_kind(),
                Some(TokenKind::LeftBrace | TokenKind::LeftParen)
            );
            if !is_compound_keyword && next_is_compound_start {
                self.advance(); // consume the NAME
                self.skip_newlines()?;
                (Name::from(word), Some(word_span))
            } else {
                (Name::new_static("COPROC"), None)
            }
        } else {
            (Name::new_static("COPROC"), None)
        };

        // Parse the command body (could be simple, compound, or pipeline)
        let body = self.parse_pipeline()?;
        let body = body.ok_or_else(|| self.error("coproc: missing command"))?;

        Ok(CompoundCommand::Coproc(CoprocCommand {
            name,
            name_span,
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Check if current token is ;; (case terminator)
    fn is_case_terminator(&self) -> bool {
        matches!(
            self.current_token_kind,
            Some(TokenKind::DoubleSemicolon | TokenKind::SemiAmp | TokenKind::DoubleSemiAmp)
        ) || (self.dialect == ShellDialect::Zsh
            && self.current_token_kind == Some(TokenKind::SemiPipe))
    }

    /// Parse case terminator: `;;` (break), `;&` (fallthrough),
    /// `;;&` / `;|` (continue matching)
    fn parse_case_terminator(&mut self) -> (CaseTerminator, Option<Span>) {
        match self.current_token_kind {
            Some(TokenKind::SemiAmp) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::FallThrough, Some(span))
            }
            Some(TokenKind::SemiPipe) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::ContinueMatching, Some(span))
            }
            Some(TokenKind::DoubleSemiAmp) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::Continue, Some(span))
            }
            Some(TokenKind::DoubleSemicolon) => {
                let span = self.current_span;
                self.advance();
                (CaseTerminator::Break, Some(span))
            }
            _ => (CaseTerminator::Break, None),
        }
    }

    /// Parse a subshell (commands in parentheses)
    fn parse_subshell(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        self.advance(); // consume '('
        self.skip_newlines()?;

        let body_start = self.current_span.start;
        let mut commands = Vec::new();
        while !matches!(
            self.current_token_kind,
            Some(TokenKind::RightParen | TokenKind::DoubleRightParen) | None
        ) {
            self.skip_newlines()?;
            if matches!(
                self.current_token_kind,
                Some(TokenKind::RightParen | TokenKind::DoubleRightParen)
            ) {
                break;
            }
            commands.extend(self.parse_command_list_required()?);
        }

        if self.at(TokenKind::DoubleRightParen) {
            // `))` at end of nested subshells: consume as single `)`, leave `)` for parent
            self.set_current_kind(TokenKind::RightParen, self.current_span);
        } else if !self.at(TokenKind::RightParen) {
            self.pop_depth();
            return Err(Error::parse("expected ')' to close subshell".to_string()));
        } else {
            self.advance(); // consume ')'
        }

        self.pop_depth();
        Ok(CompoundCommand::Subshell(Self::stmt_seq_with_span(
            Span::from_positions(body_start, self.current_span.start),
            commands,
        )))
    }

    /// Parse a brace group
    fn parse_brace_group(&mut self, context: BraceBodyContext) -> Result<CompoundCommand> {
        self.push_depth()?;
        let (body, left_brace_span, right_brace_span) =
            self.parse_brace_enclosed_stmt_seq("syntax error: empty brace group", context)?;

        let always_span = self.peek_zsh_always_span();
        if let Some(span) = always_span {
            self.record_zsh_always_span(span);
        }

        let compound = if self.dialect.features().zsh_always && self.is_keyword(Keyword::Always) {
            self.record_zsh_always_span(self.current_span);
            self.advance();
            self.skip_newlines()?;
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after always"));
            }
            let (always_body, _, always_right_brace_span) = self.parse_brace_enclosed_stmt_seq(
                "syntax error: empty always clause",
                BraceBodyContext::Ordinary,
            )?;
            CompoundCommand::Always(AlwaysCommand {
                body,
                always_body,
                span: left_brace_span.merge(always_right_brace_span),
            })
        } else {
            let _ = right_brace_span;
            CompoundCommand::BraceGroup(body)
        };

        self.pop_depth();
        Ok(compound)
    }

    fn parse_brace_enclosed_stmt_seq(
        &mut self,
        empty_error: &str,
        context: BraceBodyContext,
    ) -> Result<(StmtSeq, Span, Span)> {
        let left_brace_span = self.current_span;
        self.advance();
        self.brace_group_depth += 1;
        self.brace_body_stack.push(context);
        self.skip_command_separators()?;

        let body_start = self.current_span.start;
        let mut commands = Vec::new();
        while !matches!(self.current_token_kind, Some(TokenKind::RightBrace) | None) {
            self.skip_command_separators()?;
            if self.at(TokenKind::RightBrace) {
                break;
            }
            commands.extend(self.parse_command_list_required()?);
        }

        if !self.at(TokenKind::RightBrace) {
            self.brace_body_stack.pop();
            self.brace_group_depth -= 1;
            return Err(Error::parse(
                "expected '}' to close brace group".to_string(),
            ));
        }

        if commands.is_empty()
            && !(self.dialect == ShellDialect::Zsh && matches!(context, BraceBodyContext::Function))
        {
            self.brace_body_stack.pop();
            self.brace_group_depth -= 1;
            return Err(self.error(empty_error));
        }

        let right_brace_span = self.current_span;
        self.advance();
        self.brace_body_stack.pop();
        self.brace_group_depth -= 1;
        Ok((
            Self::stmt_seq_with_span(
                Span::from_positions(body_start, right_brace_span.start),
                commands,
            ),
            left_brace_span,
            right_brace_span,
        ))
    }

    fn parse_if_condition_until_body_start(&mut self, allow_brace_body: bool) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::with_capacity(2);

        loop {
            self.skip_newlines()?;

            if !allow_brace_body && !stmts.is_empty() && self.at(TokenKind::LeftBrace) {
                self.record_zsh_brace_if_span(self.current_span);
            }

            if self.at(TokenKind::Semicolon) {
                let checkpoint = self.checkpoint();
                self.advance();
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                let brace_if_span =
                    (!allow_brace_body && !stmts.is_empty() && self.at(TokenKind::LeftBrace))
                        .then_some(self.current_span);
                if self.is_keyword(Keyword::Then)
                    || (allow_brace_body && !stmts.is_empty() && self.at(TokenKind::LeftBrace))
                {
                    if let Some(span) = brace_if_span {
                        self.record_zsh_brace_if_span(span);
                    }
                    break;
                }
                self.restore(checkpoint);
                if let Some(span) = brace_if_span {
                    self.record_zsh_brace_if_span(span);
                }
            }

            if self.is_keyword(Keyword::Then)
                || (allow_brace_body && !stmts.is_empty() && self.at(TokenKind::LeftBrace))
            {
                break;
            }

            if self.current_token.is_none() {
                break;
            }

            let command_stmts = self.parse_command_list_required()?;
            self.apply_stmt_list_effects(&command_stmts);
            stmts.extend(command_stmts);
        }

        Ok(stmts)
    }

    fn peek_zsh_always_span(&mut self) -> Option<Span> {
        if !self.is_keyword(Keyword::Always) {
            return None;
        }

        let always_span = self.current_span;
        let checkpoint = self.checkpoint();
        self.advance();
        let result = match self.skip_newlines() {
            Ok(()) if self.at(TokenKind::LeftBrace) => Some(always_span),
            Ok(()) => None,
            Err(_) => None,
        };
        self.restore(checkpoint);
        result
    }

    fn has_recorded_comment_between(&self, start_offset: usize, end_offset: usize) -> bool {
        self.comments.iter().any(|comment| {
            let comment_start = usize::from(comment.range.start());
            comment_start >= start_offset && comment_start < end_offset
        })
    }

    fn rebase_nested_parse_error(&self, error: Error, base: Position) -> Error {
        let Error::Parse {
            message,
            line,
            column,
        } = error;

        if line == 0 {
            return Error::parse(message);
        }

        let rebased_line = base.line + line.saturating_sub(1);
        let rebased_column = if line == 1 {
            base.column + column.saturating_sub(1)
        } else {
            column
        };

        Error::parse_at(message, rebased_line, rebased_column)
    }

    fn try_parse_compact_function_brace_body(&mut self) -> Result<Option<CompoundCommand>> {
        if self.dialect != ShellDialect::Zsh
            || !self.zsh_short_loops_enabled()
            || !self.at_word_like()
        {
            return Ok(None);
        }

        let Some(body_text) = self.current_source_like_word_text() else {
            return Ok(None);
        };
        let body_text = body_text.into_owned();
        let Some(inner) = body_text
            .strip_prefix('{')
            .and_then(|body| body.strip_suffix('}'))
        else {
            return Ok(None);
        };

        if !inner.is_empty()
            && !inner.chars().any(|ch| {
                matches!(
                    ch,
                    ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' | '$' | '"' | '\'' | '(' | ')'
                )
            })
        {
            return Ok(None);
        }

        let nested_profile = self
            .current_zsh_options()
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let mut nested =
            Parser::with_limits_and_profile(inner, self.max_depth, self.max_fuel, nested_profile);
        nested.aliases = self.aliases.clone();
        nested.expand_aliases = self.expand_aliases;
        nested.expand_next_word = self.expand_next_word;

        let inner_start = self.current_span.start.advanced_by("{");
        let mut output = nested.parse();
        if output.is_err() {
            return Err(self.rebase_nested_parse_error(output.strict_error(), inner_start));
        }
        Self::rebase_stmt_seq(&mut output.file.body, inner_start);
        self.advance();
        Ok(Some(CompoundCommand::BraceGroup(output.file.body)))
    }

    fn should_consume_right_brace_as_literal_argument(
        &mut self,
        next_kind_after_right_brace: Option<TokenKind>,
    ) -> bool {
        if !self.current_token_has_leading_whitespace()
            || next_kind_after_right_brace.is_some_and(Self::is_redirect_kind)
        {
            return false;
        }

        if self.brace_group_depth == 0 {
            return true;
        }

        if self.dialect != ShellDialect::Zsh {
            return true;
        }

        next_kind_after_right_brace == Some(TokenKind::Semicolon)
            && self.current_token_is_tight_to_next_token()
            && self.next_token_after_tight_semicolon_is(TokenKind::RightBrace)
    }

    fn next_token_after_tight_semicolon_is(&mut self, expected: TokenKind) -> bool {
        let checkpoint = self.checkpoint();
        self.advance();
        if !self.at(TokenKind::Semicolon) {
            self.restore(checkpoint);
            return false;
        }
        self.advance();
        let result = self.at(expected);
        self.restore(checkpoint);
        result
    }

    /// Parse arithmetic command ((expression))
    /// Parse [[ conditional expression ]]
    fn parse_conditional(&mut self) -> Result<CompoundCommand> {
        self.ensure_double_bracket()?;
        let left_bracket_span = self.current_span;
        self.advance(); // consume '[['
        self.skip_conditional_newlines();

        let expression = self.parse_conditional_or(false)?;
        self.skip_conditional_newlines();

        let right_bracket_span = match self.current_token_kind {
            Some(TokenKind::DoubleRightBracket) => {
                let span = self.current_span;
                self.advance(); // consume ']]'
                span
            }
            None => {
                return Err(crate::error::Error::parse(
                    "unexpected end of input in [[ ]]".to_string(),
                ));
            }
            _ => return Err(self.error("expected ']]' to close conditional expression")),
        };

        Ok(CompoundCommand::Conditional(ConditionalCommand {
            expression,
            span: left_bracket_span.merge(right_bracket_span),
            left_bracket_span,
            right_bracket_span,
        }))
    }

    fn skip_conditional_newlines(&mut self) {
        while self.at(TokenKind::Newline) {
            self.advance();
        }
    }

    fn parse_conditional_or(&mut self, stop_at_right_paren: bool) -> Result<ConditionalExpr> {
        let mut expr = self.parse_conditional_and(stop_at_right_paren)?;

        loop {
            self.skip_conditional_newlines();
            if !self.at(TokenKind::Or) {
                break;
            }

            let op_span = self.current_span;
            self.advance();
            let right = self.parse_conditional_and(stop_at_right_paren)?;
            expr = ConditionalExpr::Binary(ConditionalBinaryExpr {
                left: Box::new(expr),
                op: ConditionalBinaryOp::Or,
                op_span,
                right: Box::new(right),
            });
        }

        Ok(expr)
    }

    fn parse_conditional_and(&mut self, stop_at_right_paren: bool) -> Result<ConditionalExpr> {
        let mut expr = self.parse_conditional_term(stop_at_right_paren)?;

        loop {
            self.skip_conditional_newlines();
            if !self.at(TokenKind::And) {
                break;
            }

            let op_span = self.current_span;
            self.advance();
            let right = self.parse_conditional_term(stop_at_right_paren)?;
            expr = ConditionalExpr::Binary(ConditionalBinaryExpr {
                left: Box::new(expr),
                op: ConditionalBinaryOp::And,
                op_span,
                right: Box::new(right),
            });
        }

        Ok(expr)
    }

    fn parse_conditional_term(&mut self, stop_at_right_paren: bool) -> Result<ConditionalExpr> {
        self.skip_conditional_newlines();

        if let Some(op) = self.current_conditional_unary_op() {
            let op_span = self.current_span;
            self.advance();
            self.skip_conditional_newlines();

            let expr = if matches!(op, ConditionalUnaryOp::Not) {
                self.parse_conditional_term(stop_at_right_paren)?
            } else {
                if matches!(
                    op,
                    ConditionalUnaryOp::VariableSet | ConditionalUnaryOp::ReferenceVariable
                ) {
                    let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                    self.conditional_var_ref_expr(word)
                } else {
                    let word = self.parse_conditional_operand_word()?;
                    ConditionalExpr::Word(word)
                }
            };

            return Ok(ConditionalExpr::Unary(ConditionalUnaryExpr {
                op,
                op_span,
                expr: Box::new(expr),
            }));
        }

        if self.at(TokenKind::DoubleLeftParen) {
            if self.dialect == ShellDialect::Zsh {
                let left_paren_span = self.current_span;
                if let Some(right_paren_span) = self.scan_arithmetic_command_close(left_paren_span)
                {
                    let span = left_paren_span.merge(right_paren_span);
                    let text = span.slice(self.input).to_string();
                    while self.current_token.is_some()
                        && self.current_span.start.offset < right_paren_span.end.offset
                    {
                        self.advance();
                    }
                    return Ok(ConditionalExpr::Word(
                        self.parse_word_with_context(&text, span, span.start, true),
                    ));
                }
            }
            self.split_current_double_left_paren();
        }

        let left = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();
            let expr = self.parse_conditional_or(true)?;
            self.skip_conditional_newlines();
            if self.at(TokenKind::DoubleRightParen) {
                self.split_current_double_right_paren();
            }
            if !self.at(TokenKind::RightParen) {
                return Err(self.error("expected ')' in conditional expression"));
            }
            let right_paren_span = self.current_span;
            self.advance();
            ConditionalExpr::Parenthesized(ConditionalParenExpr {
                left_paren_span,
                expr: Box::new(expr),
                right_paren_span,
            })
        } else {
            ConditionalExpr::Word(self.parse_conditional_operand_word()?)
        };

        self.skip_conditional_newlines();

        let Some(op) = self.current_conditional_comparison_op() else {
            return Ok(left);
        };

        let op_span = self.current_span;
        self.advance();
        self.skip_conditional_newlines();

        let right = match op {
            ConditionalBinaryOp::RegexMatch => {
                if self.at(TokenKind::LeftBrace) {
                    return Err(self.error("expected conditional operand"));
                }
                ConditionalExpr::Regex(self.collect_conditional_context_word(stop_at_right_paren)?)
            }
            ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe => {
                let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                ConditionalExpr::Pattern(self.pattern_from_conditional_word(&word))
            }
            _ => ConditionalExpr::Word(self.parse_conditional_operand_word()?),
        };

        Ok(ConditionalExpr::Binary(ConditionalBinaryExpr {
            left: Box::new(left),
            op,
            op_span,
            right: Box::new(right),
        }))
    }

    fn parse_conditional_operand_word(&mut self) -> Result<Word> {
        self.skip_conditional_newlines();

        if let Some(word) = self.current_conditional_source_word(false) {
            self.advance_past_word(&word);
            self.restore_conditional_source_delimiter(word.span.end, false);
            return Ok(word);
        }

        if let Some(word) = self.take_current_word_and_advance() {
            return Ok(word);
        }

        let Some(word) = self.current_conditional_literal_word() else {
            return Err(self.error("expected conditional operand"));
        };
        self.advance_past_word(&word);
        Ok(word)
    }

    fn conditional_var_ref_expr(&self, word: Word) -> ConditionalExpr {
        self.parse_var_ref_from_word(&word, SubscriptInterpretation::Contextual)
            .map(Box::new)
            .map(ConditionalExpr::VarRef)
            .unwrap_or(ConditionalExpr::Word(word))
    }

    fn current_conditional_source_word(&mut self, stop_at_right_paren: bool) -> Option<Word> {
        let token = self.current_token.as_ref()?;
        if token.flags.is_synthetic() {
            return None;
        }

        if matches!(
            self.current_token_kind,
            Some(TokenKind::QuotedWord | TokenKind::LiteralWord)
        ) {
            return None;
        }

        let starts_with_paren = matches!(self.current_token_kind, Some(TokenKind::LeftParen));
        let starts_with_zsh_pattern_punct = matches!(
            self.current_token_kind,
            Some(TokenKind::RedirectIn | TokenKind::RedirectOut | TokenKind::RedirectReadWrite)
        ) && self.dialect == ShellDialect::Zsh;

        if !starts_with_paren
            && !starts_with_zsh_pattern_punct
            && !self.current_token_kind.is_some_and(TokenKind::is_word_like)
        {
            return None;
        }

        let start = self.current_span.start;
        let (text, end) =
            self.scan_conditional_source_word(start, stop_at_right_paren, starts_with_paren)?;
        let span = Span::from_positions(start, end);
        Some(self.parse_word_with_context(&text, span, start, true))
    }

    fn scan_conditional_source_word(
        &self,
        start: Position,
        stop_at_right_paren: bool,
        starts_with_paren: bool,
    ) -> Option<(String, Position)> {
        if start.offset >= self.input.len() {
            return None;
        }

        let mut cursor = start;
        let mut text = String::new();
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut bracket_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];
            if !in_single
                && !in_double
                && !in_backtick
                && !escaped
                && paren_depth == 0
                && brace_depth == 0
                && bracket_depth == 0
            {
                if rest.starts_with("]]")
                    || rest.starts_with("&&")
                    || rest.starts_with("||")
                    || (!starts_with_paren && rest.starts_with(')'))
                    || (stop_at_right_paren && rest.starts_with(')'))
                {
                    break;
                }

                let ch = rest.chars().next()?;
                if matches!(ch, ' ' | '\t' | '\n' | ';') {
                    break;
                }
            }

            let ch = self.input[cursor.offset..].chars().next()?;
            cursor.advance(ch);
            text.push(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '`' if !in_single => in_backtick = !in_backtick,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double && bracket_depth > 0 => bracket_depth -= 1,
                _ => {}
            }
        }

        (!text.is_empty()).then_some((text, cursor))
    }

    fn conditional_source_delimiter_after(
        &self,
        end: Position,
        stop_at_right_paren: bool,
    ) -> Option<(TokenKind, Span)> {
        let mut cursor = end;
        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];
            let ch = rest.chars().next()?;
            if matches!(ch, ' ' | '\t') {
                cursor.advance(ch);
                continue;
            }
            break;
        }

        let rest = self.input.get(cursor.offset..)?;
        let (kind, text) = if rest.starts_with("]]") {
            (TokenKind::DoubleRightBracket, "]]")
        } else if rest.starts_with("&&") {
            (TokenKind::And, "&&")
        } else if rest.starts_with("||") {
            (TokenKind::Or, "||")
        } else if stop_at_right_paren && rest.starts_with(')') {
            (TokenKind::RightParen, ")")
        } else {
            return None;
        };

        Some((kind, Span::from_positions(cursor, cursor.advanced_by(text))))
    }

    fn restore_conditional_source_delimiter(&mut self, end: Position, stop_at_right_paren: bool) {
        let Some((kind, span)) = self.conditional_source_delimiter_after(end, stop_at_right_paren)
        else {
            return;
        };

        if self.current_token_kind == Some(kind) && self.current_span == span {
            return;
        }

        if let Some(current_kind) = self.current_token_kind
            && matches!(
                current_kind,
                TokenKind::Newline
                    | TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::RightParen
                    | TokenKind::DoubleRightBracket
            )
        {
            self.synthetic_tokens
                .push_front(SyntheticToken::punctuation(current_kind, self.current_span));
        }

        self.set_current_kind(kind, span);
    }

    fn current_conditional_unary_op(&self) -> Option<ConditionalUnaryOp> {
        if !self.at(TokenKind::Word) {
            return None;
        }
        let word = self.current_word_str()?;

        Some(match word {
            "!" => ConditionalUnaryOp::Not,
            "-e" | "-a" => ConditionalUnaryOp::Exists,
            "-f" => ConditionalUnaryOp::RegularFile,
            "-d" => ConditionalUnaryOp::Directory,
            "-c" => ConditionalUnaryOp::CharacterSpecial,
            "-b" => ConditionalUnaryOp::BlockSpecial,
            "-p" => ConditionalUnaryOp::NamedPipe,
            "-S" => ConditionalUnaryOp::Socket,
            "-L" | "-h" => ConditionalUnaryOp::Symlink,
            "-k" => ConditionalUnaryOp::Sticky,
            "-g" => ConditionalUnaryOp::SetGroupId,
            "-u" => ConditionalUnaryOp::SetUserId,
            "-G" => ConditionalUnaryOp::GroupOwned,
            "-O" => ConditionalUnaryOp::UserOwned,
            "-N" => ConditionalUnaryOp::Modified,
            "-r" => ConditionalUnaryOp::Readable,
            "-w" => ConditionalUnaryOp::Writable,
            "-x" => ConditionalUnaryOp::Executable,
            "-s" => ConditionalUnaryOp::NonEmptyFile,
            "-t" => ConditionalUnaryOp::FdTerminal,
            "-z" => ConditionalUnaryOp::EmptyString,
            "-n" => ConditionalUnaryOp::NonEmptyString,
            "-o" => ConditionalUnaryOp::OptionSet,
            "-v" => ConditionalUnaryOp::VariableSet,
            "-R" => ConditionalUnaryOp::ReferenceVariable,
            _ => return None,
        })
    }

    fn current_conditional_comparison_op(&self) -> Option<ConditionalBinaryOp> {
        match self.current_token_kind? {
            TokenKind::Word => Some(match self.current_word_str()? {
                "=" => ConditionalBinaryOp::PatternEqShort,
                "==" => ConditionalBinaryOp::PatternEq,
                "!=" => ConditionalBinaryOp::PatternNe,
                "=~" => ConditionalBinaryOp::RegexMatch,
                "-nt" => ConditionalBinaryOp::NewerThan,
                "-ot" => ConditionalBinaryOp::OlderThan,
                "-ef" => ConditionalBinaryOp::SameFile,
                "-eq" => ConditionalBinaryOp::ArithmeticEq,
                "-ne" => ConditionalBinaryOp::ArithmeticNe,
                "-le" => ConditionalBinaryOp::ArithmeticLe,
                "-ge" => ConditionalBinaryOp::ArithmeticGe,
                "-lt" => ConditionalBinaryOp::ArithmeticLt,
                "-gt" => ConditionalBinaryOp::ArithmeticGt,
                _ => return None,
            }),
            TokenKind::RedirectIn => Some(ConditionalBinaryOp::LexicalBefore),
            TokenKind::RedirectOut => Some(ConditionalBinaryOp::LexicalAfter),
            _ => None,
        }
    }

    fn collect_conditional_context_word(&mut self, stop_at_right_paren: bool) -> Result<Word> {
        self.skip_conditional_newlines();

        if let Some(word) = self.current_conditional_source_word(stop_at_right_paren) {
            self.advance_past_word(&word);
            self.restore_conditional_source_delimiter(word.span.end, stop_at_right_paren);
            return Ok(word);
        }

        let mut first_word: Option<Word> = None;
        let mut parts = Vec::new();
        let mut start = None;
        let mut end = None;
        let mut previous_end: Option<Position> = None;
        let mut composite = false;
        let mut paren_depth = 0usize;

        loop {
            self.skip_conditional_newlines();

            match self.current_token_kind {
                Some(TokenKind::DoubleRightBracket) => break,
                Some(TokenKind::And) | Some(TokenKind::Or) if paren_depth == 0 => break,
                Some(TokenKind::RightParen) if stop_at_right_paren && paren_depth == 0 => break,
                None => break,
                _ => {}
            }

            if let Some(prev_end) = previous_end
                && prev_end.offset < self.current_span.start.offset
            {
                let gap_span = Span::from_positions(prev_end, self.current_span.start);
                let gap_text = gap_span.slice(self.input);
                if let Some(word) = first_word.take() {
                    parts.extend(word.parts);
                }
                if Self::source_text_needs_quote_preserving_decode(gap_text) {
                    let gap_word = self.decode_word_text_preserving_quotes_if_needed(
                        gap_text,
                        gap_span,
                        gap_span.start,
                        true,
                    );
                    parts.extend(gap_word.parts);
                } else {
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::source()),
                        gap_span,
                    ));
                }
                composite = true;
            }

            match self.current_token_kind {
                Some(TokenKind::Word | TokenKind::LiteralWord | TokenKind::QuotedWord) => {
                    let word = self
                        .take_current_word()
                        .ok_or_else(|| self.error("expected conditional operand"))?;
                    if start.is_none() {
                        start = Some(word.span.start);
                    } else {
                        if let Some(first) = first_word.take() {
                            parts.extend(first.parts);
                        }
                        composite = true;
                    }
                    end = Some(word.span.end);
                    if first_word.is_none() && !composite {
                        first_word = Some(word);
                    } else {
                        parts.extend(word.parts);
                    }
                    previous_end = Some(self.current_span.end);
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("(")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth += 1;
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::DoubleLeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("((")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth += 2;
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned(")")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(1);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("))")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(2);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::Pipe) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("|")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::And) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("&&")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::Or) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::owned("||")),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(TokenKind::RedirectIn)
                | Some(TokenKind::RedirectOut)
                | Some(TokenKind::RedirectReadWrite) => {
                    let literal = self.input
                        [self.current_span.start.offset..self.current_span.end.offset]
                        .to_string();
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(self.literal_text(
                            literal,
                            self.current_span.start,
                            self.current_span.end,
                        )),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                _ => {
                    let literal = self.input
                        [self.current_span.start.offset..self.current_span.end.offset]
                        .to_string();
                    if literal.is_empty() {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    if let Some(word) = first_word.take() {
                        parts.extend(word.parts);
                    }
                    parts.push(WordPartNode::new(
                        WordPart::Literal(self.literal_text(
                            literal,
                            self.current_span.start,
                            self.current_span.end,
                        )),
                        self.current_span,
                    ));
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
            }
        }

        if !composite && let Some(word) = first_word {
            return Ok(word);
        }

        let (start, end) = match (start, end) {
            (Some(start), Some(end)) => (start, end),
            _ => return Err(self.error("expected conditional operand")),
        };

        Ok(self.word_with_parts(parts, Span::from_positions(start, end)))
    }

    fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        self.ensure_arithmetic_command()?;
        let left_paren_span = self.current_span;
        let Some(right_paren_span) = self.scan_arithmetic_command_close(left_paren_span) else {
            return Err(Error::parse(
                "unexpected end of input in arithmetic command".to_string(),
            ));
        };
        while self.current_token.is_some()
            && self.current_span.start.offset < right_paren_span.end.offset
        {
            self.advance();
        }

        let expr_span = Self::optional_span(left_paren_span.end, right_paren_span.start);
        let expr_ast =
            self.parse_explicit_arithmetic_span(expr_span, "invalid arithmetic command")?;
        Ok(CompoundCommand::Arithmetic(ArithmeticCommand {
            span: left_paren_span.merge(right_paren_span),
            left_paren_span,
            expr_span,
            expr_ast,
            right_paren_span,
        }))
    }

    fn scan_arithmetic_command_close(&self, left_paren_span: Span) -> Option<Span> {
        let mut cursor = left_paren_span.end;
        let mut depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while cursor.offset < self.input.len() {
            let rest = &self.input[cursor.offset..];

            if !in_single && !in_double && !in_backtick {
                if rest.starts_with("((") {
                    depth += 2;
                    cursor = cursor.advanced_by("((");
                    continue;
                }

                if rest.starts_with("))") {
                    if depth == 0 {
                        return Some(Span::from_positions(cursor, cursor.advanced_by("))")));
                    }
                    if depth == 1 {
                        cursor.advance(')');
                        return Some(Span::from_positions(cursor, cursor.advanced_by("))")));
                    }
                    depth -= 2;
                    cursor = cursor.advanced_by("))");
                    continue;
                }
            }

            let ch = rest.chars().next()?;
            cursor.advance(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double && !in_backtick => in_single = !in_single,
                '"' if !in_single && !in_backtick => in_double = !in_double,
                '`' if !in_single && !in_double => in_backtick = !in_backtick,
                '(' if !in_single && !in_double && !in_backtick => depth += 1,
                ')' if !in_single && !in_double && !in_backtick && depth > 0 => depth -= 1,
                _ => {}
            }
        }

        None
    }

    fn parse_function_body_command(&mut self, allow_bare_compound: bool) -> Result<Stmt> {
        if let Some(compound) = self.try_parse_compact_function_brace_body()? {
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        let compound = match self.current_keyword() {
            Some(Keyword::If) if allow_bare_compound => self.parse_if()?,
            Some(Keyword::For) if allow_bare_compound => self.parse_for()?,
            Some(Keyword::Repeat) if allow_bare_compound && self.zsh_short_repeat_enabled() => {
                self.parse_repeat()?
            }
            Some(Keyword::Foreach) if allow_bare_compound && self.zsh_short_loops_enabled() => {
                self.parse_foreach()?
            }
            Some(Keyword::While) if allow_bare_compound => self.parse_while()?,
            Some(Keyword::Until) if allow_bare_compound => self.parse_until()?,
            Some(Keyword::Case) if allow_bare_compound => self.parse_case()?,
            Some(Keyword::Select) if allow_bare_compound => self.parse_select()?,
            _ => match self.current_token_kind {
                Some(TokenKind::LeftBrace) => self.parse_brace_group(BraceBodyContext::Function)?,
                Some(TokenKind::LeftParen) => self.parse_subshell()?,
                Some(TokenKind::DoubleLeftBracket) if allow_bare_compound => {
                    self.parse_conditional()?
                }
                Some(TokenKind::DoubleLeftParen) if allow_bare_compound => {
                    if self.looks_like_command_style_double_paren() {
                        self.split_current_double_left_paren();
                        self.parse_subshell()?
                    } else {
                        let checkpoint = self.checkpoint();
                        if let Ok(compound) = self.parse_arithmetic_command() {
                            compound
                        } else {
                            self.restore(checkpoint);
                            self.split_current_double_left_paren();
                            self.parse_subshell()?
                        }
                    }
                }
                _ => {
                    return Err(Error::parse(
                        "expected compound command for function body".to_string(),
                    ));
                }
            },
        };
        let redirects = self.parse_trailing_redirects();
        Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
            Box::new(compound),
            redirects,
        )))
    }

    fn parse_function_header_entry(&mut self) -> Result<FunctionHeaderEntry> {
        let word = self
            .take_current_word_and_advance()
            .ok_or_else(|| self.error("expected function name"))?;
        Ok(self.function_header_entry_from_word(word))
    }

    fn parse_function_keyword_header_entry(&mut self) -> Result<FunctionHeaderEntry> {
        let word = self
            .take_current_word_and_advance()
            .or_else(|| self.take_current_function_keyword_name_and_advance())
            .ok_or_else(|| self.error("expected function name"))?;
        Ok(self.function_header_entry_from_word(word))
    }

    fn take_current_function_keyword_name_and_advance(&mut self) -> Option<Word> {
        let text = match self.current_token_kind? {
            TokenKind::DoubleLeftBracket => "[[",
            TokenKind::DoubleRightBracket => "]]",
            TokenKind::LeftBrace => "{",
            TokenKind::RightBrace => "}",
            _ => return None,
        };
        let word = Word::literal_with_span(text, self.current_span);
        self.advance();
        Some(word)
    }

    fn function_header_entry_from_word(&self, word: Word) -> FunctionHeaderEntry {
        let static_name = self.literal_word_text(&word).map(Name::from);
        FunctionHeaderEntry { word, static_name }
    }

    fn parse_function_parens_span(&mut self) -> Result<Span> {
        if !self.at(TokenKind::LeftParen) {
            return Err(self.error("expected '(' in function definition"));
        }
        let left_paren_span = self.current_span;
        self.advance();

        if !self.at(TokenKind::RightParen) {
            return Err(Error::parse(
                "expected ')' in function definition".to_string(),
            ));
        }
        let right_paren_span = self.current_span;
        self.advance();
        Ok(left_paren_span.merge(right_paren_span))
    }

    fn parse_zsh_function_body_stmt(&mut self) -> Result<Stmt> {
        self.skip_newlines()?;

        if let Some(compound) = self.try_parse_compact_function_brace_body()? {
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        if self.at(TokenKind::LeftBrace) {
            let compound = self.parse_brace_group(BraceBodyContext::Function)?;
            let redirects = self.parse_trailing_redirects();
            return Ok(Self::lower_non_sequence_command_to_stmt(Command::Compound(
                Box::new(compound),
                redirects,
            )));
        }

        self.parse_single_stmt_command()
    }

    fn parse_single_stmt_command(&mut self) -> Result<Stmt> {
        let mut stmt = self
            .parse_pipeline()?
            .ok_or_else(|| self.error("expected command"))?;

        let Some(kind) = self.current_token_kind else {
            return Ok(stmt);
        };
        let operator = match kind {
            TokenKind::And => Some((Some(BinaryOp::And), None, false)),
            TokenKind::Or => Some((Some(BinaryOp::Or), None, false)),
            TokenKind::Semicolon => Some((None, Some(StmtTerminator::Semicolon), true)),
            TokenKind::Background => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Plain)),
                true,
            )),
            TokenKind::BackgroundPipe => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Pipe)),
                true,
            )),
            TokenKind::BackgroundBang => Some((
                None,
                Some(StmtTerminator::Background(BackgroundOperator::Bang)),
                true,
            )),
            _ => None,
        };
        let Some((binary_op, terminator, allow_empty_tail)) = operator else {
            return Ok(stmt);
        };
        let operator_span = self.current_span;
        self.advance();

        if let Some(binary_op) = binary_op {
            self.skip_newlines()?;
            if let Some(right) = self.parse_pipeline()? {
                stmt = Self::binary_stmt(stmt, binary_op, operator_span, right);
            }
            return Ok(stmt);
        }

        if allow_empty_tail
            && matches!(
                self.current_token_kind,
                Some(TokenKind::Semicolon | TokenKind::Newline)
            )
        {
            self.advance();
        }

        stmt.terminator = terminator;
        stmt.terminator_span = Some(operator_span);
        Ok(stmt)
    }

    fn parse_anonymous_function_args(&mut self) -> Result<Vec<Word>> {
        let mut args = Vec::new();
        while self.current_token_kind.is_some_and(TokenKind::is_word_like) {
            let word = self
                .take_current_word_and_advance()
                .ok_or_else(|| self.error("expected anonymous function argument"))?;
            args.push(word);
        }
        Ok(args)
    }

    /// Parse function definition with 'function' keyword: function name { body }
    fn parse_function_keyword(&mut self) -> Result<Command> {
        self.ensure_function_keyword()?;
        let start_span = self.current_span;
        self.advance(); // consume 'function'
        self.skip_newlines()?;

        if self.dialect == ShellDialect::Zsh {
            let mut entries = Vec::new();
            while self.current_token_kind.is_some_and(TokenKind::is_word_like) {
                entries.push(self.parse_function_header_entry()?);
                if self.at(TokenKind::LeftParen) {
                    break;
                }
            }

            let trailing_parens_span = if !entries.is_empty() && self.at(TokenKind::LeftParen) {
                Some(self.parse_function_parens_span()?)
            } else {
                None
            };

            if entries.is_empty() {
                let body = self.parse_zsh_function_body_stmt()?;
                let args = self.parse_anonymous_function_args()?;
                let redirects = self.parse_trailing_redirects();
                let span = start_span.merge(self.current_span);
                return Ok(Command::AnonymousFunction(
                    AnonymousFunctionCommand {
                        surface: AnonymousFunctionSurface::FunctionKeyword {
                            function_keyword_span: start_span,
                        },
                        body: Box::new(body),
                        args,
                        span,
                    },
                    redirects,
                ));
            }

            let body = self.parse_zsh_function_body_stmt()?;
            let span = start_span.merge(self.current_span);
            return Ok(Command::Function(FunctionDef {
                header: FunctionHeader {
                    function_keyword_span: Some(start_span),
                    entries,
                    trailing_parens_span,
                },
                body: Box::new(body),
                span,
            }));
        }

        let entry = self.parse_function_keyword_header_entry()?;
        let saw_newline_after_name = self.skip_newlines_with_flag()?;
        let (trailing_parens_span, allow_bare_compound) = if self.at(TokenKind::LeftParen) {
            let parens_span = self.parse_function_parens_span()?;
            (Some(parens_span), self.skip_newlines_with_flag()?)
        } else {
            (None, saw_newline_after_name)
        };

        let body = self.parse_function_body_command(allow_bare_compound)?;
        let span = start_span.merge(self.current_span);

        Ok(Command::Function(FunctionDef {
            header: FunctionHeader {
                function_keyword_span: Some(start_span),
                entries: vec![entry],
                trailing_parens_span,
            },
            body: Box::new(body),
            span,
        }))
    }

    /// Parse POSIX-style function definition: name() { body }
    fn parse_function_posix(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        let entry = self.parse_function_header_entry()?;
        let trailing_parens_span = self.parse_function_parens_span()?;

        self.finish_parse_function_posix(start_span, entry, trailing_parens_span)
    }

    fn finish_parse_function_posix(
        &mut self,
        start_span: Span,
        entry: FunctionHeaderEntry,
        trailing_parens_span: Span,
    ) -> Result<Command> {
        let body = if self.dialect == ShellDialect::Zsh {
            self.parse_zsh_function_body_stmt()?
        } else {
            self.skip_newlines()?;
            self.parse_function_body_command(true)?
        };

        Ok(Command::Function(FunctionDef {
            header: FunctionHeader {
                function_keyword_span: None,
                entries: vec![entry],
                trailing_parens_span: Some(trailing_parens_span),
            },
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    fn try_parse_zsh_attached_parens_function(&mut self) -> Result<Option<Command>> {
        if self.dialect != ShellDialect::Zsh || !self.at_word_like() {
            return Ok(None);
        }

        let Some(word_text) = self.current_source_like_word_text() else {
            return Ok(None);
        };
        let Some(header_text) = word_text.as_ref().strip_suffix("()") else {
            return Ok(None);
        };
        if header_text.is_empty() || header_text.contains('=') {
            return Ok(None);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if let Err(error) = self.skip_newlines() {
            self.restore(checkpoint);
            return Err(error);
        }
        if !self.at(TokenKind::LeftBrace) {
            self.restore(checkpoint);
            return Ok(None);
        }
        self.restore(checkpoint);

        let start_span = self.current_span;
        let header_span =
            Span::from_positions(start_span.start, start_span.start.advanced_by(header_text));
        let parens_span = Span::from_positions(header_span.end, start_span.end);
        let header_word =
            self.parse_word_with_context(header_text, header_span, header_span.start, true);
        let entry = self.function_header_entry_from_word(header_word);
        self.advance();

        self.finish_parse_function_posix(start_span, entry, parens_span)
            .map(Some)
    }

    fn parse_anonymous_paren_function(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        let parens_span = self.parse_function_parens_span()?;
        let body = self.parse_zsh_function_body_stmt()?;
        let args = self.parse_anonymous_function_args()?;
        let redirects = self.parse_trailing_redirects();
        let span = start_span.merge(self.current_span);
        Ok(Command::AnonymousFunction(
            AnonymousFunctionCommand {
                surface: AnonymousFunctionSurface::Parens { parens_span },
                body: Box::new(body),
                args,
                span,
            },
            redirects,
        ))
    }

    /// Parse commands until a terminating keyword
    fn parse_compound_list(&mut self, terminator: Keyword) -> Result<Vec<Stmt>> {
        self.parse_compound_list_until(KeywordSet::single(terminator))
    }

    /// Parse commands until one of the terminating keywords
    fn parse_compound_list_until(&mut self, terminators: KeywordSet) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::with_capacity(4);

        loop {
            self.skip_command_separators()?;

            // Check for terminators
            if self
                .current_keyword()
                .is_some_and(|keyword| terminators.contains(keyword))
            {
                break;
            }

            if self.current_token.is_none() {
                break;
            }

            let command_stmts = self.parse_command_list_required()?;
            self.apply_stmt_list_effects(&command_stmts);
            stmts.extend(command_stmts);
        }

        Ok(stmts)
    }

    /// Reserved words that cannot start a simple command.
    /// These words are only special in command position, not as arguments.
    /// Check if a word cannot start a command
    fn is_non_command_keyword(keyword: Keyword) -> bool {
        NON_COMMAND_KEYWORDS.contains(keyword)
    }

    /// Check if current token is a specific keyword
    fn is_keyword(&self, keyword: Keyword) -> bool {
        self.current_keyword() == Some(keyword)
    }

    /// Expect a specific keyword
    fn expect_keyword(&mut self, keyword: Keyword) -> Result<()> {
        if self.is_keyword(keyword) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!("expected '{}'", keyword)))
        }
    }
    fn parse_simple_command(&mut self) -> Result<Option<SimpleCommand>> {
        self.tick()?;
        self.skip_newlines()?;
        self.check_error_token()?;
        let start_span = self.current_span;

        let mut assignments = Vec::with_capacity(1);
        let mut words = Vec::with_capacity(4);
        let mut redirects = Vec::with_capacity(1);

        loop {
            self.check_error_token()?;
            let next_kind_after_right_brace = if self.at(TokenKind::RightBrace) {
                self.peek_next_kind()
            } else {
                None
            };
            let right_brace_is_literal_argument = self.at(TokenKind::RightBrace)
                && !words.is_empty()
                && self.should_consume_right_brace_as_literal_argument(next_kind_after_right_brace);
            match self.current_token_kind {
                Some(kind) if kind.is_word_like() => {
                    let is_literal = kind == TokenKind::LiteralWord;
                    let word_text = self.current_source_like_word_text().unwrap();
                    let assignment_shape = (!is_literal && words.is_empty())
                        .then(|| Self::is_assignment(word_text.as_ref()));
                    let assignment_shape = assignment_shape.flatten();

                    // Stop if this word cannot start a command (like 'then', 'fi', etc.)
                    if words.is_empty()
                        && self
                            .current_keyword()
                            .is_some_and(Self::is_non_command_keyword)
                    {
                        break;
                    }

                    // Check for assignment (only before the command name, not for literal words)
                    if words.is_empty()
                        && !is_literal
                        && let Some((assignment, needs_advance)) = self
                            .try_parse_assignment_with_shape(word_text.as_ref(), assignment_shape)
                    {
                        if needs_advance {
                            self.advance();
                        }
                        assignments.push(assignment);
                        continue;
                    }

                    if words.is_empty()
                        && !is_literal
                        && assignment_shape.is_none()
                        && word_text.contains('[')
                        && let Some(assignment) =
                            self.try_parse_split_indexed_assignment_from_text()
                    {
                        assignments.push(assignment);
                        continue;
                    }

                    // Handle compound array assignment in arg position:
                    // declare -a arr=(x y z) → arr=(x y z) as single arg
                    if word_text.ends_with('=') && !words.is_empty() {
                        let original_word = self.current_word_ref().cloned();
                        let saved_span = self.current_span;
                        self.advance();
                        if let Some(word) =
                            self.try_parse_compound_array_arg(word_text.into_owned(), saved_span)
                        {
                            words.push(word);
                            continue;
                        }
                        // Not a compound assignment — treat as regular word
                        if let Some(word) = original_word {
                            words.push(word);
                        }
                        continue;
                    }

                    if let Some(word) = self.take_current_word_and_advance() {
                        words.push(word);
                    }
                }
                Some(TokenKind::LeftParen) if !words.is_empty() => {
                    let Some(word) = self.take_current_word_and_advance() else {
                        break;
                    };
                    words.push(word);
                }
                Some(TokenKind::Newline) => {
                    let next_kind = self.peek_next_kind();
                    let supports_fd_var = next_kind.is_some_and(|kind| {
                        matches!(kind, TokenKind::HereDoc | TokenKind::HereDocStrip)
                            || Self::redirect_supports_fd_var(kind)
                    });
                    if supports_fd_var {
                        let (fd_var, fd_var_span) = self.pop_line_continuation_fd_var(&mut words);
                        if let Some(fd_var) = fd_var {
                            self.advance();
                            if matches!(
                                self.current_token_kind,
                                Some(TokenKind::HereDoc | TokenKind::HereDocStrip)
                            ) {
                                self.parse_heredoc_redirect(
                                    self.current_token_kind == Some(TokenKind::HereDocStrip),
                                    &mut redirects,
                                    Some(fd_var),
                                    fd_var_span,
                                )?;
                                continue;
                            }

                            if self.consume_non_heredoc_redirect(
                                &mut redirects,
                                Some(fd_var),
                                fd_var_span,
                                true,
                            )? {
                                continue;
                            }
                        }
                    }
                    break;
                }
                Some(kind) if Self::is_redirect_kind(kind) => {
                    if matches!(kind, TokenKind::HereDoc | TokenKind::HereDocStrip) {
                        let (fd_var, fd_var_span) = if words
                            .last()
                            .is_some_and(|word| self.word_is_attached_to_current_token(word))
                        {
                            self.pop_fd_var(&mut words)
                        } else {
                            (None, None)
                        };
                        self.parse_heredoc_redirect(
                            kind == TokenKind::HereDocStrip,
                            &mut redirects,
                            fd_var,
                            fd_var_span,
                        )?;
                        continue;
                    }

                    let (fd_var, fd_var_span) = if Self::redirect_supports_fd_var(kind) {
                        if words
                            .last()
                            .is_some_and(|word| self.word_is_attached_to_current_token(word))
                        {
                            self.pop_fd_var(&mut words)
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                    if self.consume_non_heredoc_redirect(
                        &mut redirects,
                        fd_var,
                        fd_var_span,
                        true,
                    )? {
                        continue;
                    }
                    break;
                }
                Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                    let word = self.expect_word()?;
                    words.push(word);
                }
                // `{` can appear as a literal argument outside command position.
                Some(TokenKind::LeftBrace) if !words.is_empty() => {
                    words.push(Word::literal_with_span("{", self.current_span));
                    self.advance();
                }
                // Inside brace groups, a bare `}` can still be a literal
                // argument like `echo }`, but only when it's separated from the
                // preceding token by whitespace and isn't introducing an outer
                // redirect on the brace group itself.
                Some(TokenKind::RightBrace) if right_brace_is_literal_argument => {
                    words.push(Word::literal_with_span("}", self.current_span));
                    self.advance();
                }
                Some(TokenKind::Semicolon)
                | Some(TokenKind::Pipe)
                | Some(TokenKind::And)
                | Some(TokenKind::Or)
                | None => break,
                _ => break,
            }
        }

        // Handle assignment-only or redirect-only commands with no command word.
        if words.is_empty() && (!assignments.is_empty() || !redirects.is_empty()) {
            return Ok(Some(SimpleCommand {
                name: Word::literal(""),
                args: Vec::new(),
                redirects,
                assignments,
                span: start_span.merge(self.current_span),
            }));
        }

        if words.is_empty() {
            return Ok(None);
        }

        let name = words.remove(0);
        let args = words;

        Ok(Some(SimpleCommand {
            name,
            args,
            redirects,
            assignments,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Extract fd-variable name from `{varname}` pattern in the last word.
    /// If the last word is a single literal `{identifier}`, pop it and return the name.
    /// Used for `exec {var}>file` / `exec {var}>&-` syntax.
    fn pop_fd_var(&self, words: &mut Vec<Word>) -> (Option<Name>, Option<Span>) {
        if let Some(last) = words.last()
            && last.parts.len() == 1
            && let WordPart::Literal(ref s) = last.parts[0].kind
            && let Some(span) = last.part_span(0)
            && let text = s.as_str(self.input, span)
            && text.starts_with('{')
            && text.ends_with('}')
            && text.len() > 2
            && text[1..text.len() - 1]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
        {
            let var_name = text[1..text.len() - 1].to_string();
            let start = last.span.start.advanced_by("{");
            let span = Span::from_positions(start, start.advanced_by(&var_name));
            words.pop();
            return (Some(Name::from(var_name)), Some(span));
        }
        (None, None)
    }

    fn word_is_attached_to_current_token(&self, word: &Word) -> bool {
        let start = word.span.end.offset;
        let end = self.current_span.start.offset;
        let input_len = self.input.len();
        start <= end
            && end <= input_len
            && Self::fd_var_gap_allows_attachment(&self.input[start..end])
    }

    fn pop_line_continuation_fd_var(&self, words: &mut Vec<Word>) -> (Option<Name>, Option<Span>) {
        let Some(last) = words.last() else {
            return (None, None);
        };
        let Some(text) = self.single_literal_word_text(last) else {
            return (None, None);
        };
        let Some(fd_text) = text.strip_suffix('\\') else {
            return (None, None);
        };
        let Some((fd_var, fd_var_span)) = Self::fd_var_from_text(fd_text, last.span) else {
            return (None, None);
        };
        words.pop();
        (Some(fd_var), Some(fd_var_span))
    }
}
