//! Parser module for shuck
//!
//! Implements a recursive descent parser for bash scripts.

// Parser uses chars().next().unwrap() after validating character presence.
// This is safe because we check bounds before accessing.
#![allow(clippy::unwrap_used)]

mod lexer;

pub use lexer::{Lexer, SpannedToken};

use shuck_ast::{
    ArithmeticForCommand, Assignment, AssignmentValue, CaseCommand, CaseItem, CaseTerminator,
    Command, CommandList, CompoundCommand, CoprocCommand, ForCommand, FunctionDef, IfCommand,
    ListOperator, ParameterOp, Pipeline, Position, Redirect, RedirectKind, Script, SelectCommand,
    SimpleCommand, Span, TimeCommand, Token, UntilCommand, WhileCommand, Word, WordPart,
};

use crate::error::{Error, Result};

/// Default maximum AST depth (matches ExecutionLimits default)
const DEFAULT_MAX_AST_DEPTH: usize = 100;

/// Hard cap on AST depth to prevent stack overflow even if caller misconfigures limits.
/// THREAT[TM-DOS-022]: Protects against deeply nested input attacks where
/// a large max_depth setting allows recursion deep enough to overflow the native stack.
/// This cap cannot be overridden by the caller.
///
/// Set conservatively to avoid stack overflow on tokio's blocking threads (default 2MB
/// stack in debug builds). Each parser recursion level uses ~4-8KB of stack in debug
/// mode. 100 levels × ~8KB = ~800KB, well within 2MB.
/// In release builds this could safely be higher, but we use one value for consistency.
const HARD_MAX_AST_DEPTH: usize = 100;

/// Default maximum parser operations (matches ExecutionLimits default)
const DEFAULT_MAX_PARSER_OPERATIONS: usize = 100_000;

/// Parser for bash scripts.
pub struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    current_token: Option<Token>,
    /// Span of the current token
    current_span: Span,
    /// Lookahead token for function parsing
    peeked_token: Option<SpannedToken>,
    /// Maximum allowed AST nesting depth
    max_depth: usize,
    /// Current nesting depth
    current_depth: usize,
    /// Remaining fuel for parsing operations
    fuel: usize,
    /// Maximum fuel (for error reporting)
    max_fuel: usize,
}

/// A parser diagnostic emitted while recovering from invalid input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub message: String,
    pub span: Span,
}

/// The result of a recovered parse: a partial script plus parse diagnostics.
#[derive(Debug, Clone)]
pub struct RecoveredParse {
    pub script: Script,
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_limits(input, DEFAULT_MAX_AST_DEPTH, DEFAULT_MAX_PARSER_OPERATIONS)
    }

    /// Create a new parser with a custom maximum AST depth.
    pub fn with_max_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_limits(input, max_depth, DEFAULT_MAX_PARSER_OPERATIONS)
    }

    /// Create a new parser with a custom fuel limit.
    pub fn with_fuel(input: &'a str, max_fuel: usize) -> Self {
        Self::with_limits(input, DEFAULT_MAX_AST_DEPTH, max_fuel)
    }

    /// Create a new parser with custom depth and fuel limits.
    ///
    /// THREAT[TM-DOS-022]: `max_depth` is clamped to `HARD_MAX_AST_DEPTH` (500)
    /// to prevent stack overflow from misconfiguration. Even if the caller passes
    /// `max_depth = 1_000_000`, the parser will cap it at 500.
    pub fn with_limits(input: &'a str, max_depth: usize, max_fuel: usize) -> Self {
        let mut lexer = Lexer::with_max_subst_depth(input, max_depth.min(HARD_MAX_AST_DEPTH));
        let spanned = lexer.next_spanned_token();
        let (current_token, current_span) = match spanned {
            Some(st) => (Some(st.token), st.span),
            None => (None, Span::new()),
        };
        Self {
            input,
            lexer,
            current_token,
            current_span,
            peeked_token: None,
            max_depth: max_depth.min(HARD_MAX_AST_DEPTH),
            current_depth: 0,
            fuel: max_fuel,
            max_fuel,
        }
    }

    /// Get the current token's span.
    pub fn current_span(&self) -> Span {
        self.current_span
    }

    /// Parse a string as a word (handling $var, $((expr)), ${...}, etc.).
    /// Used by the interpreter to expand operands in parameter expansions lazily.
    pub fn parse_word_string(input: &str) -> Word {
        let parser = Parser::new(input);
        let start = Position::new();
        parser.parse_word_with_context(
            input.to_string(),
            Span::from_positions(start, start.advanced_by(input)),
            start,
        )
    }

    /// THREAT[TM-DOS-050]: Parse a word string with caller-configured limits.
    /// Prevents bypass of parser limits in parameter expansion contexts.
    pub fn parse_word_string_with_limits(input: &str, max_depth: usize, max_fuel: usize) -> Word {
        let parser = Parser::with_limits(input, max_depth, max_fuel);
        let start = Position::new();
        parser.parse_word_with_context(
            input.to_string(),
            Span::from_positions(start, start.advanced_by(input)),
            start,
        )
    }

    fn word_from_token(&self, token: &Token, span: Span) -> Option<Word> {
        match token {
            Token::Word(w) => {
                Some(self.parse_word_with_context(w.clone(), span, span.start))
            }
            Token::QuotedWord(w) => {
                let mut word =
                    self.parse_word_with_context(w.clone(), span, span.start.advanced_by("\""));
                word.quoted = true;
                Some(word)
            }
            Token::LiteralWord(w) => Some(Word::quoted_literal_with_span(w.clone(), span)),
            _ => None,
        }
    }

    fn current_word_to_word(&self) -> Option<Word> {
        self.current_token
            .as_ref()
            .and_then(|token| self.word_from_token(token, self.current_span))
    }

    fn nested_commands_from_source(&self, source: &str, base: Position) -> Vec<Command> {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let inner_parser = Parser::with_limits(source, remaining_depth, self.fuel);
        match inner_parser.parse() {
            Ok(mut script) => {
                Self::rebase_commands(&mut script.commands, base);
                script.commands
            }
            Err(_) => Vec::new(),
        }
    }

    fn nested_commands_from_current_input(&self, start: Position, end: Position) -> Vec<Command> {
        if start.offset > end.offset || end.offset > self.input.len() {
            return Vec::new();
        }
        let source = &self.input[start.offset..end.offset];
        self.nested_commands_from_source(source, start)
    }

    fn merge_optional_span(primary: Span, other: Span) -> Span {
        if other == Span::new() {
            primary
        } else {
            primary.merge(other)
        }
    }

    fn redirect_span(operator_span: Span, target: &Word) -> Span {
        Self::merge_optional_span(operator_span, target.span)
    }

    fn rebase_commands(commands: &mut [Command], base: Position) {
        for command in commands {
            Self::rebase_command(command, base);
        }
    }

    fn rebase_command(command: &mut Command, base: Position) {
        match command {
            Command::Simple(simple) => {
                simple.span = simple.span.rebased(base);
                Self::rebase_word(&mut simple.name, base);
                Self::rebase_words(&mut simple.args, base);
                Self::rebase_redirects(&mut simple.redirects, base);
                Self::rebase_assignments(&mut simple.assignments, base);
            }
            Command::Pipeline(pipeline) => {
                pipeline.span = pipeline.span.rebased(base);
                Self::rebase_commands(&mut pipeline.commands, base);
            }
            Command::List(list) => {
                list.span = list.span.rebased(base);
                Self::rebase_command(&mut list.first, base);
                for (_, command) in &mut list.rest {
                    Self::rebase_command(command, base);
                }
            }
            Command::Compound(compound, redirects) => {
                Self::rebase_compound(compound, base);
                Self::rebase_redirects(redirects, base);
            }
            Command::Function(function) => {
                function.span = function.span.rebased(base);
                Self::rebase_command(&mut function.body, base);
            }
        }
    }

    fn rebase_compound(compound: &mut CompoundCommand, base: Position) {
        match compound {
            CompoundCommand::If(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_commands(&mut command.condition, base);
                Self::rebase_commands(&mut command.then_branch, base);
                for (condition, body) in &mut command.elif_branches {
                    Self::rebase_commands(condition, base);
                    Self::rebase_commands(body, base);
                }
                if let Some(else_branch) = &mut command.else_branch {
                    Self::rebase_commands(else_branch, base);
                }
            }
            CompoundCommand::For(command) => {
                command.span = command.span.rebased(base);
                if let Some(words) = &mut command.words {
                    Self::rebase_words(words, base);
                }
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::ArithmeticFor(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::While(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_commands(&mut command.condition, base);
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::Until(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_commands(&mut command.condition, base);
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::Case(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_word(&mut command.word, base);
                for case in &mut command.cases {
                    Self::rebase_words(&mut case.patterns, base);
                    Self::rebase_commands(&mut case.commands, base);
                }
            }
            CompoundCommand::Select(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::rebase_commands(commands, base);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                command.span = command.span.rebased(base);
                if let Some(inner) = &mut command.command {
                    Self::rebase_command(inner, base);
                }
            }
            CompoundCommand::Conditional(words) => {
                Self::rebase_words(words, base);
            }
            CompoundCommand::Coproc(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_command(&mut command.body, base);
            }
        }
    }

    fn rebase_words(words: &mut [Word], base: Position) {
        for word in words {
            Self::rebase_word(word, base);
        }
    }

    fn rebase_word(word: &mut Word, base: Position) {
        word.span = word.span.rebased(base);
        for span in &mut word.part_spans {
            *span = span.rebased(base);
        }
        for part in &mut word.parts {
            match part {
                WordPart::CommandSubstitution(commands)
                | WordPart::ProcessSubstitution { commands, .. } => {
                    Self::rebase_commands(commands, base);
                }
                _ => {}
            }
        }
    }

    fn push_word_part(
        parts: &mut Vec<WordPart>,
        part_spans: &mut Vec<Span>,
        part: WordPart,
        start: Position,
        end: Position,
    ) {
        parts.push(part);
        part_spans.push(Span::from_positions(start, end));
    }

    fn flush_literal_part(
        parts: &mut Vec<WordPart>,
        part_spans: &mut Vec<Span>,
        current: &mut String,
        current_start: Position,
        end: Position,
    ) {
        if !current.is_empty() {
            Self::push_word_part(
                parts,
                part_spans,
                WordPart::Literal(std::mem::take(current)),
                current_start,
                end,
            );
        }
    }

    fn next_word_char(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
    ) -> Option<char> {
        let ch = chars.next()?;
        cursor.advance(ch);
        Some(ch)
    }

    fn next_word_char_unwrap(
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
    ) -> char {
        Self::next_word_char(chars, cursor).unwrap()
    }

    fn consume_word_char_if(
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

    fn read_word_while<F>(
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

    fn rebase_redirects(redirects: &mut [Redirect], base: Position) {
        for redirect in redirects {
            redirect.span = redirect.span.rebased(base);
            Self::rebase_word(&mut redirect.target, base);
        }
    }

    fn rebase_assignments(assignments: &mut [Assignment], base: Position) {
        for assignment in assignments {
            assignment.span = assignment.span.rebased(base);
            match &mut assignment.value {
                AssignmentValue::Scalar(word) => Self::rebase_word(word, base),
                AssignmentValue::Array(words) => Self::rebase_words(words, base),
            }
        }
    }

    /// Create a parse error with the current position.
    fn error(&self, message: impl Into<String>) -> Error {
        Error::parse_at(
            message,
            self.current_span.start.line,
            self.current_span.start.column,
        )
    }

    /// Consume one unit of fuel, returning an error if exhausted
    fn tick(&mut self) -> Result<()> {
        if self.fuel == 0 {
            let used = self.max_fuel;
            return Err(Error::parse(format!(
                "parser fuel exhausted ({} operations, max {})",
                used, self.max_fuel
            )));
        }
        self.fuel -= 1;
        Ok(())
    }

    /// Push nesting depth and check limit
    fn push_depth(&mut self) -> Result<()> {
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
    fn pop_depth(&mut self) {
        if self.current_depth > 0 {
            self.current_depth -= 1;
        }
    }

    /// Check if current token is an error token and return the error if so
    fn check_error_token(&self) -> Result<()> {
        if let Some(Token::Error(msg)) = &self.current_token {
            return Err(self.error(format!("syntax error: {}", msg)));
        }
        Ok(())
    }

    fn parse_diagnostic_from_error(&self, error: Error) -> ParseDiagnostic {
        let Error::Parse { message, .. } = error;
        ParseDiagnostic {
            message,
            span: self.current_span,
        }
    }

    fn parse_command_list_required(&mut self) -> Result<Command> {
        self.parse_command_list()?
            .ok_or_else(|| self.error("expected command"))
    }

    fn is_recovery_separator(token: &Token) -> bool {
        matches!(
            token,
            Token::Newline
                | Token::Semicolon
                | Token::Background
                | Token::And
                | Token::Or
                | Token::Pipe
                | Token::DoubleSemicolon
                | Token::SemiAmp
                | Token::DoubleSemiAmp
        )
    }

    fn recover_to_command_boundary(&mut self, failed_offset: usize) -> bool {
        let mut advanced = false;

        while let Some(token) = &self.current_token {
            if Self::is_recovery_separator(token) {
                loop {
                    let Some(token) = &self.current_token else {
                        break;
                    };
                    if !Self::is_recovery_separator(token) {
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

    /// Parse the input and return the AST.
    pub fn parse(mut self) -> Result<Script> {
        // Check if the very first token is an error
        self.check_error_token()?;

        let start_span = self.current_span;
        let mut commands = Vec::new();

        while self.current_token.is_some() {
            self.tick()?;
            self.skip_newlines()?;
            self.check_error_token()?;
            if self.current_token.is_none() {
                break;
            }
            commands.push(self.parse_command_list_required()?);
        }

        let end_span = self.current_span;
        Ok(Script {
            commands,
            span: start_span.merge(end_span),
        })
    }

    /// Parse the input while recovering at top-level command boundaries.
    pub fn parse_recovered(mut self) -> RecoveredParse {
        let start_span = self.current_span;
        let mut commands = Vec::new();
        let mut diagnostics = Vec::new();

        while self.current_token.is_some() {
            let checkpoint = self.current_span.start.offset;

            if let Err(error) = self.tick() {
                diagnostics.push(self.parse_diagnostic_from_error(error));
                break;
            }
            if let Err(error) = self.skip_newlines() {
                diagnostics.push(self.parse_diagnostic_from_error(error));
                break;
            }
            if let Err(error) = self.check_error_token() {
                diagnostics.push(self.parse_diagnostic_from_error(error));
                if !self.recover_to_command_boundary(checkpoint) {
                    break;
                }
                continue;
            }
            if self.current_token.is_none() {
                break;
            }

            let command_start = self.current_span.start.offset;
            match self.parse_command_list_required() {
                Ok(command) => commands.push(command),
                Err(error) => {
                    diagnostics.push(self.parse_diagnostic_from_error(error));
                    if !self.recover_to_command_boundary(command_start) {
                        break;
                    }
                }
            }
        }

        let end_span = self.current_span;
        RecoveredParse {
            script: Script {
                commands,
                span: start_span.merge(end_span),
            },
            diagnostics,
        }
    }

    fn advance(&mut self) {
        if let Some(peeked) = self.peeked_token.take() {
            self.current_token = Some(peeked.token);
            self.current_span = peeked.span;
        } else {
            match self.lexer.next_spanned_token() {
                Some(st) => {
                    self.current_token = Some(st.token);
                    self.current_span = st.span;
                }
                None => {
                    self.current_token = None;
                    // Keep the last span for error reporting
                }
            }
        }
    }

    /// Peek at the next token without consuming the current one
    fn peek_next(&mut self) -> Option<&Token> {
        if self.peeked_token.is_none() {
            self.peeked_token = self.lexer.next_spanned_token();
        }
        self.peeked_token.as_ref().map(|st| &st.token)
    }

    fn skip_newlines(&mut self) -> Result<()> {
        while matches!(self.current_token, Some(Token::Newline)) {
            self.tick()?;
            self.advance();
        }
        Ok(())
    }

    /// Parse a command list (commands connected by && or ||)
    fn parse_command_list(&mut self) -> Result<Option<Command>> {
        self.tick()?;
        let start_span = self.current_span;
        let first = match self.parse_pipeline()? {
            Some(cmd) => cmd,
            None => return Ok(None),
        };

        let mut rest = Vec::new();

        loop {
            let op = match &self.current_token {
                Some(Token::And) => {
                    self.advance();
                    ListOperator::And
                }
                Some(Token::Or) => {
                    self.advance();
                    ListOperator::Or
                }
                Some(Token::Semicolon) => {
                    self.advance();
                    self.skip_newlines()?;
                    // Check if there's more to parse
                    if self.current_token.is_none()
                        || matches!(self.current_token, Some(Token::Newline))
                    {
                        break;
                    }
                    ListOperator::Semicolon
                }
                Some(Token::Background) => {
                    self.advance();
                    self.skip_newlines()?;
                    // Check if there's more to parse after &
                    if self.current_token.is_none()
                        || matches!(self.current_token, Some(Token::Newline))
                    {
                        // Just & at end - return as background
                        rest.push((
                            ListOperator::Background,
                            Command::Simple(SimpleCommand {
                                name: Word::literal(""),
                                args: vec![],
                                redirects: vec![],
                                assignments: vec![],
                                span: self.current_span,
                            }),
                        ));
                        break;
                    }
                    ListOperator::Background
                }
                _ => break,
            };

            self.skip_newlines()?;

            if let Some(cmd) = self.parse_pipeline()? {
                rest.push((op, cmd));
            } else {
                break;
            }
        }

        if rest.is_empty() {
            Ok(Some(first))
        } else {
            Ok(Some(Command::List(CommandList {
                first: Box::new(first),
                rest,
                span: start_span.merge(self.current_span),
            })))
        }
    }

    /// Parse a pipeline (commands connected by |)
    ///
    /// Handles `!` pipeline negation: `! cmd | cmd2` negates the exit code.
    fn parse_pipeline(&mut self) -> Result<Option<Command>> {
        let start_span = self.current_span;

        // Check for pipeline negation: `! command`
        let negated = match &self.current_token {
            Some(Token::Word(w)) if w == "!" => {
                self.advance();
                true
            }
            _ => false,
        };

        let first = match self.parse_command()? {
            Some(cmd) => cmd,
            None => {
                if negated {
                    return Err(self.error("expected command after !"));
                }
                return Ok(None);
            }
        };

        let mut commands = vec![first];

        while matches!(self.current_token, Some(Token::Pipe)) {
            self.advance();
            self.skip_newlines()?;

            if let Some(cmd) = self.parse_command()? {
                commands.push(cmd);
            } else {
                return Err(self.error("expected command after |"));
            }
        }

        if commands.len() == 1 && !negated {
            Ok(Some(commands.remove(0)))
        } else {
            Ok(Some(Command::Pipeline(Pipeline {
                negated,
                commands,
                span: start_span.merge(self.current_span),
            })))
        }
    }

    /// Parse redirections that follow a compound command (>, >>, 2>, etc.)
    fn parse_trailing_redirects(&mut self) -> Vec<Redirect> {
        let mut redirects = Vec::new();
        loop {
            match &self.current_token {
                Some(Token::RedirectOut) | Some(Token::Clobber) => {
                    let operator_span = self.current_span;
                    let kind = if matches!(&self.current_token, Some(Token::Clobber)) {
                        RedirectKind::Clobber
                    } else {
                        RedirectKind::Output
                    };
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::RedirectAppend) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind: RedirectKind::Append,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::RedirectIn) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind: RedirectKind::Input,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::RedirectBoth) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind: RedirectKind::OutputBoth,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::DupOutput) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(1),
                            fd_var: None,
                            kind: RedirectKind::DupOutput,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::RedirectFd(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(fd),
                            fd_var: None,
                            kind: RedirectKind::Output,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::RedirectFdAppend(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(fd),
                            fd_var: None,
                            kind: RedirectKind::Append,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::DupFd(src_fd, dst_fd)) => {
                    let src_fd = *src_fd;
                    let dst_fd = *dst_fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(src_fd),
                        fd_var: None,
                        kind: RedirectKind::DupOutput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Some(Token::DupInput) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(0),
                            fd_var: None,
                            kind: RedirectKind::DupInput,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::DupFdIn(src_fd, dst_fd)) => {
                    let src_fd = *src_fd;
                    let dst_fd = *dst_fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(src_fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Some(Token::DupFdClose(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal("-"),
                    });
                }
                Some(Token::RedirectFdIn(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(fd),
                            fd_var: None,
                            kind: RedirectKind::Input,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::HereString) => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind: RedirectKind::HereString,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::HereDoc) | Some(Token::HereDocStrip) => {
                    let operator_span = self.current_span;
                    let strip_tabs =
                        matches!(self.current_token, Some(Token::HereDocStrip));
                    self.advance();
                    let (delimiter, quoted) = match &self.current_token {
                        Some(Token::Word(w)) => (w.clone(), false),
                        Some(Token::LiteralWord(w)) => (w.clone(), true),
                        Some(Token::QuotedWord(w)) => (w.clone(), true),
                        _ => break,
                    };
                    let content = self.lexer.read_heredoc(&delimiter);
                    let content = if strip_tabs {
                        let had_trailing_newline = content.ends_with('\n');
                        let mut stripped: String = content
                            .lines()
                            .map(|l| l.trim_start_matches('\t'))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if had_trailing_newline {
                            stripped.push('\n');
                        }
                        stripped
                    } else {
                        content
                    };
                    self.advance();
                    let target = if quoted {
                        Word::quoted_literal(content)
                    } else {
                        self.parse_word(content)
                    };
                    let kind = if strip_tabs {
                        RedirectKind::HereDocStrip
                    } else {
                        RedirectKind::HereDoc
                    };
                    redirects.push(Redirect {
                        fd: None,
                        fd_var: None,
                        kind,
                        span: operator_span,
                        target,
                    });
                    // Rest-of-line tokens re-injected by lexer; break so callers
                    // can see pipes/semicolons.
                    break;
                }
                _ => break,
            }
        }
        redirects
    }

    /// Parse a compound command and any trailing redirections
    fn parse_compound_with_redirects(
        &mut self,
        parser: impl FnOnce(&mut Self) -> Result<CompoundCommand>,
    ) -> Result<Option<Command>> {
        let compound = parser(self)?;
        let redirects = self.parse_trailing_redirects();
        Ok(Some(Command::Compound(compound, redirects)))
    }

    /// Parse a single command (simple or compound)
    fn parse_command(&mut self) -> Result<Option<Command>> {
        self.skip_newlines()?;
        self.check_error_token()?;

        // Check for compound commands and function keyword
        if let Some(Token::Word(w)) = &self.current_token {
            let word = w.clone();
            match word.as_str() {
                "if" => return self.parse_compound_with_redirects(|s| s.parse_if()),
                "for" => return self.parse_compound_with_redirects(|s| s.parse_for()),
                "while" => return self.parse_compound_with_redirects(|s| s.parse_while()),
                "until" => return self.parse_compound_with_redirects(|s| s.parse_until()),
                "case" => return self.parse_compound_with_redirects(|s| s.parse_case()),
                "select" => return self.parse_compound_with_redirects(|s| s.parse_select()),
                "time" => return self.parse_compound_with_redirects(|s| s.parse_time()),
                "coproc" => return self.parse_compound_with_redirects(|s| s.parse_coproc()),
                "function" => return self.parse_function_keyword().map(Some),
                _ => {
                    // Check for POSIX-style function: name() { body }
                    // Don't match if word contains '=' (that's an assignment like arr=(a b c))
                    if !word.contains('=')
                        && matches!(self.peek_next(), Some(Token::LeftParen))
                    {
                        return self.parse_function_posix().map(Some);
                    }
                }
            }
        }

        // Check for conditional expression [[ ... ]]
        if matches!(self.current_token, Some(Token::DoubleLeftBracket)) {
            return self.parse_compound_with_redirects(|s| s.parse_conditional());
        }

        // Check for arithmetic command ((expression))
        if matches!(self.current_token, Some(Token::DoubleLeftParen)) {
            return self.parse_compound_with_redirects(|s| s.parse_arithmetic_command());
        }

        // Check for subshell
        if matches!(self.current_token, Some(Token::LeftParen)) {
            return self.parse_compound_with_redirects(|s| s.parse_subshell());
        }

        // Check for brace group
        if matches!(self.current_token, Some(Token::LeftBrace)) {
            return self.parse_compound_with_redirects(|s| s.parse_brace_group());
        }

        // Default to simple command
        match self.parse_simple_command()? {
            Some(cmd) => Ok(Some(Command::Simple(cmd))),
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
        let condition = self.parse_compound_list("then")?;

        // Expect 'then'
        self.expect_keyword("then")?;
        self.skip_newlines()?;

        // Parse then branch
        let then_branch = self.parse_compound_list_until(&["elif", "else", "fi"])?;

        // Bash requires at least one command in then branch
        if then_branch.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty then clause"));
        }

        // Parse elif branches
        let mut elif_branches = Vec::new();
        while self.is_keyword("elif") {
            self.advance(); // consume 'elif'
            self.skip_newlines()?;

            let elif_condition = self.parse_compound_list("then")?;
            self.expect_keyword("then")?;
            self.skip_newlines()?;

            let elif_body = self.parse_compound_list_until(&["elif", "else", "fi"])?;

            // Bash requires at least one command in elif branch
            if elif_body.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty elif clause"));
            }

            elif_branches.push((elif_condition, elif_body));
        }

        // Parse else branch
        let else_branch = if self.is_keyword("else") {
            self.advance(); // consume 'else'
            self.skip_newlines()?;
            let branch = self.parse_compound_list("fi")?;

            // Bash requires at least one command in else branch
            if branch.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty else clause"));
            }

            Some(branch)
        } else {
            None
        };

        // Expect 'fi'
        self.expect_keyword("fi")?;

        self.pop_depth();
        Ok(CompoundCommand::If(IfCommand {
            condition,
            then_branch,
            elif_branches,
            else_branch,
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
        if matches!(self.current_token, Some(Token::DoubleLeftParen)) {
            let result = self.parse_arithmetic_for_inner(start_span);
            self.pop_depth();
            return result;
        }

        // Expect variable name
        let variable = match &self.current_token {
            Some(Token::Word(w))
            | Some(Token::LiteralWord(w))
            | Some(Token::QuotedWord(w)) => w.clone(),
            _ => {
                self.pop_depth();
                return Err(Error::parse(
                    "expected variable name in for loop".to_string(),
                ));
            }
        };
        self.advance();

        // Check for 'in' keyword
        let words = if self.is_keyword("in") {
            self.advance(); // consume 'in'

            // Parse word list until do/newline/;
            let mut words = Vec::new();
            loop {
                match &self.current_token {
                    Some(Token::Word(w)) if w == "do" => break,
                    Some(Token::Word(_))
                    | Some(Token::LiteralWord(_))
                    | Some(Token::QuotedWord(_)) => {
                        if let Some(word) = self.current_word_to_word() {
                            words.push(word);
                        }
                        self.advance();
                    }
                    Some(Token::Newline) | Some(Token::Semicolon) => {
                        self.advance();
                        break;
                    }
                    _ => break,
                }
            }
            Some(words)
        } else {
            // for var; do ... (iterates over positional params)
            // Consume optional semicolon before 'do'
            if matches!(self.current_token, Some(Token::Semicolon)) {
                self.advance();
            }
            None
        };

        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list("done")?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty for loop body"));
        }

        // Expect 'done'
        self.expect_keyword("done")?;

        self.pop_depth();
        Ok(CompoundCommand::For(ForCommand {
            variable,
            words,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse select loop: select var in list; do body; done
    fn parse_select(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'select'
        self.skip_newlines()?;

        // Expect variable name
        let variable = match &self.current_token {
            Some(Token::Word(w))
            | Some(Token::LiteralWord(w))
            | Some(Token::QuotedWord(w)) => w.clone(),
            _ => {
                self.pop_depth();
                return Err(Error::parse("expected variable name in select".to_string()));
            }
        };
        self.advance();

        // Expect 'in' keyword
        if !self.is_keyword("in") {
            self.pop_depth();
            return Err(Error::parse("expected 'in' in select".to_string()));
        }
        self.advance(); // consume 'in'

        // Parse word list until do/newline/;
        let mut words = Vec::new();
        loop {
            match &self.current_token {
                Some(Token::Word(w)) if w == "do" => break,
                Some(Token::Word(_))
                | Some(Token::LiteralWord(_))
                | Some(Token::QuotedWord(_)) => {
                    if let Some(word) = self.current_word_to_word() {
                        words.push(word);
                    }
                    self.advance();
                }
                Some(Token::Newline) | Some(Token::Semicolon) => {
                    self.advance();
                    break;
                }
                _ => break,
            }
        }

        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list("done")?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty select loop body"));
        }

        // Expect 'done'
        self.expect_keyword("done")?;

        self.pop_depth();
        Ok(CompoundCommand::Select(SelectCommand {
            variable,
            words,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse C-style arithmetic for loop inner: for ((init; cond; step)); do body; done
    /// Note: depth tracking is done by parse_for which calls this
    fn parse_arithmetic_for_inner(&mut self, start_span: Span) -> Result<CompoundCommand> {
        self.advance(); // consume '(('

        // Read the three expressions separated by semicolons
        let mut parts: Vec<String> = Vec::new();
        let mut current_expr = String::new();
        let mut paren_depth = 0;

        loop {
            match &self.current_token {
                Some(Token::DoubleRightParen) => {
                    // End of the (( )) section
                    parts.push(current_expr.trim().to_string());
                    self.advance();
                    break;
                }
                Some(Token::LeftParen) => {
                    paren_depth += 1;
                    current_expr.push('(');
                    self.advance();
                }
                Some(Token::RightParen) => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                        current_expr.push(')');
                        self.advance();
                    } else {
                        // Unexpected - probably error
                        self.advance();
                    }
                }
                Some(Token::Semicolon) => {
                    if paren_depth == 0 {
                        // Separator between init, cond, step
                        parts.push(current_expr.trim().to_string());
                        current_expr.clear();
                    } else {
                        current_expr.push(';');
                    }
                    self.advance();
                }
                Some(Token::Word(w))
                | Some(Token::LiteralWord(w))
                | Some(Token::QuotedWord(w)) => {
                    // Don't add space when joining operator pairs like < + =3 → <=3
                    let skip_space = current_expr.ends_with('<')
                        || current_expr.ends_with('>')
                        || current_expr.ends_with(' ')
                        || current_expr.ends_with('(')
                        || current_expr.is_empty();
                    if !skip_space {
                        current_expr.push(' ');
                    }
                    current_expr.push_str(w);
                    self.advance();
                }
                Some(Token::Newline) => {
                    self.advance();
                }
                // Handle operators that are normally special tokens but valid in arithmetic
                Some(Token::RedirectIn) => {
                    current_expr.push('<');
                    self.advance();
                }
                Some(Token::RedirectOut) => {
                    current_expr.push('>');
                    self.advance();
                }
                Some(Token::And) => {
                    current_expr.push_str("&&");
                    self.advance();
                }
                Some(Token::Or) => {
                    current_expr.push_str("||");
                    self.advance();
                }
                Some(Token::Pipe) => {
                    current_expr.push('|');
                    self.advance();
                }
                Some(Token::Background) => {
                    current_expr.push('&');
                    self.advance();
                }
                None => {
                    return Err(Error::parse(
                        "unexpected end of input in for loop".to_string(),
                    ));
                }
                _ => {
                    self.advance();
                }
            }
        }

        // Ensure we have exactly 3 parts
        while parts.len() < 3 {
            parts.push(String::new());
        }

        let init = parts.first().cloned().unwrap_or_default();
        let condition = parts.get(1).cloned().unwrap_or_default();
        let step = parts.get(2).cloned().unwrap_or_default();

        self.skip_newlines()?;

        // Skip optional semicolon after ))
        if matches!(self.current_token, Some(Token::Semicolon)) {
            self.advance();
        }
        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list("done")?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            return Err(self.error("syntax error: empty for loop body"));
        }

        // Expect 'done'
        self.expect_keyword("done")?;

        Ok(CompoundCommand::ArithmeticFor(ArithmeticForCommand {
            init,
            condition,
            step,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse a while loop
    fn parse_while(&mut self) -> Result<CompoundCommand> {
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'while'
        self.skip_newlines()?;

        // Parse condition
        let condition = self.parse_compound_list("do")?;

        // Expect 'do'
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list("done")?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty while loop body"));
        }

        // Expect 'done'
        self.expect_keyword("done")?;

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
        let condition = self.parse_compound_list("do")?;

        // Expect 'do'
        self.expect_keyword("do")?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list("done")?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty until loop body"));
        }

        // Expect 'done'
        self.expect_keyword("done")?;

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
        self.expect_keyword("in")?;
        self.skip_newlines()?;

        // Parse case items
        let mut cases = Vec::new();
        while !self.is_keyword("esac") && self.current_token.is_some() {
            self.skip_newlines()?;
            if self.is_keyword("esac") {
                break;
            }

            // Parse patterns (pattern1 | pattern2 | ...)
            // Optional leading (
            if matches!(self.current_token, Some(Token::LeftParen)) {
                self.advance();
            }

            let mut patterns = Vec::new();
            while matches!(
                &self.current_token,
                Some(Token::Word(_))
                    | Some(Token::LiteralWord(_))
                    | Some(Token::QuotedWord(_))
            ) {
                let w = match &self.current_token {
                    Some(Token::Word(w))
                    | Some(Token::LiteralWord(w))
                    | Some(Token::QuotedWord(w)) => w.clone(),
                    _ => unreachable!(),
                };
                patterns.push(self.parse_word(w));
                self.advance();

                // Check for | between patterns
                if matches!(self.current_token, Some(Token::Pipe)) {
                    self.advance();
                } else {
                    break;
                }
            }

            // Expect )
            if !matches!(self.current_token, Some(Token::RightParen)) {
                self.pop_depth();
                return Err(self.error("expected ')' after case pattern"));
            }
            self.advance();
            self.skip_newlines()?;

            // Parse commands until ;; or esac
            let mut commands = Vec::new();
            while !self.is_case_terminator()
                && !self.is_keyword("esac")
                && self.current_token.is_some()
            {
                commands.push(self.parse_command_list_required()?);
                self.skip_newlines()?;
            }

            let terminator = self.parse_case_terminator();
            cases.push(CaseItem {
                patterns,
                commands,
                terminator,
            });
            self.skip_newlines()?;
        }

        // Expect 'esac'
        self.expect_keyword("esac")?;

        self.pop_depth();
        Ok(CompoundCommand::Case(CaseCommand {
            word,
            cases,
            span: start_span.merge(self.current_span),
        }))
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
        let posix_format = if let Some(Token::Word(w)) = &self.current_token {
            if w == "-p" {
                self.advance();
                self.skip_newlines()?;
                true
            } else {
                false
            }
        } else {
            false
        };

        // Parse the command to time (if any)
        // time with no command is valid in bash (just outputs timing header)
        let command = self.parse_pipeline()?;

        Ok(CompoundCommand::Time(TimeCommand {
            posix_format,
            command: command.map(Box::new),
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
        let start_span = self.current_span;
        self.advance(); // consume 'coproc'
        self.skip_newlines()?;

        // Determine if next token is a NAME (simple word that is NOT a compound-
        // command keyword and is followed by a compound command start).
        let (name, consumed_name) = if let Some(Token::Word(w)) = &self.current_token {
            let word = w.clone();
            let is_compound_keyword = matches!(
                word.as_str(),
                "if" | "for" | "while" | "until" | "case" | "select" | "time" | "coproc"
            );
            let next_is_compound_start = matches!(
                self.peek_next(),
                Some(Token::LeftBrace) | Some(Token::LeftParen)
            );
            if !is_compound_keyword && next_is_compound_start {
                self.advance(); // consume the NAME
                self.skip_newlines()?;
                (word, true)
            } else {
                ("COPROC".to_string(), false)
            }
        } else {
            ("COPROC".to_string(), false)
        };

        let _ = consumed_name;

        // Parse the command body (could be simple, compound, or pipeline)
        let body = self.parse_pipeline()?;
        let body = body.ok_or_else(|| self.error("coproc: missing command"))?;

        Ok(CompoundCommand::Coproc(CoprocCommand {
            name,
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Check if current token is ;; (case terminator)
    fn is_case_terminator(&self) -> bool {
        matches!(
            self.current_token,
            Some(Token::DoubleSemicolon)
                | Some(Token::SemiAmp)
                | Some(Token::DoubleSemiAmp)
        )
    }

    /// Parse case terminator: `;;` (break), `;&` (fallthrough), `;;&` (continue matching)
    fn parse_case_terminator(&mut self) -> CaseTerminator {
        match self.current_token {
            Some(Token::SemiAmp) => {
                self.advance();
                CaseTerminator::FallThrough
            }
            Some(Token::DoubleSemiAmp) => {
                self.advance();
                CaseTerminator::Continue
            }
            Some(Token::DoubleSemicolon) => {
                self.advance();
                CaseTerminator::Break
            }
            _ => CaseTerminator::Break,
        }
    }

    /// Parse a subshell (commands in parentheses)
    fn parse_subshell(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        self.advance(); // consume '('
        self.skip_newlines()?;

        let mut commands = Vec::new();
        while !matches!(
            self.current_token,
            Some(Token::RightParen) | Some(Token::DoubleRightParen) | None
        ) {
            self.skip_newlines()?;
            if matches!(
                self.current_token,
                Some(Token::RightParen) | Some(Token::DoubleRightParen)
            ) {
                break;
            }
            commands.push(self.parse_command_list_required()?);
        }

        if matches!(self.current_token, Some(Token::DoubleRightParen)) {
            // `))` at end of nested subshells: consume as single `)`, leave `)` for parent
            self.current_token = Some(Token::RightParen);
        } else if !matches!(self.current_token, Some(Token::RightParen)) {
            self.pop_depth();
            return Err(Error::parse("expected ')' to close subshell".to_string()));
        } else {
            self.advance(); // consume ')'
        }

        self.pop_depth();
        Ok(CompoundCommand::Subshell(commands))
    }

    /// Parse a brace group
    fn parse_brace_group(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        self.advance(); // consume '{'
        self.skip_newlines()?;

        let mut commands = Vec::new();
        while !matches!(self.current_token, Some(Token::RightBrace) | None) {
            self.skip_newlines()?;
            if matches!(self.current_token, Some(Token::RightBrace)) {
                break;
            }
            commands.push(self.parse_command_list_required()?);
        }

        if !matches!(self.current_token, Some(Token::RightBrace)) {
            self.pop_depth();
            return Err(Error::parse(
                "expected '}' to close brace group".to_string(),
            ));
        }

        // Bash requires at least one command in a brace group
        if commands.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty brace group"));
        }

        self.advance(); // consume '}'

        self.pop_depth();
        Ok(CompoundCommand::BraceGroup(commands))
    }

    /// Parse arithmetic command ((expression))
    /// Parse [[ conditional expression ]]
    fn parse_conditional(&mut self) -> Result<CompoundCommand> {
        self.advance(); // consume '[['

        let mut words = Vec::new();
        let mut saw_regex_op = false;

        loop {
            match &self.current_token {
                Some(Token::DoubleRightBracket) => {
                    self.advance(); // consume ']]'
                    break;
                }
                Some(Token::Word(w))
                | Some(Token::LiteralWord(w))
                | Some(Token::QuotedWord(w)) => {
                    let w_clone = w.clone();
                    let is_quoted =
                        matches!(self.current_token, Some(Token::QuotedWord(_)));

                    // After =~, handle regex pattern.
                    // If the pattern contains $ (variable reference), parse it as a
                    // normal word so variables expand. Otherwise collect as literal
                    // regex to preserve parens, backslashes, etc.
                    if saw_regex_op {
                        if w_clone.contains('$') && !is_quoted {
                            // Variable reference — parse normally for expansion
                            let parsed = self.parse_word(w_clone);
                            words.push(parsed);
                            self.advance();
                        } else {
                            let pattern = self.collect_conditional_regex_pattern(&w_clone);
                            words.push(Word::literal(&pattern));
                        }
                        saw_regex_op = false;
                        continue;
                    }

                    if w_clone == "=~" {
                        saw_regex_op = true;
                    }

                    if let Some(word) = self.current_word_to_word() {
                        words.push(word);
                    } else if is_quoted {
                        let mut parsed = self.parse_word(w_clone);
                        parsed.quoted = true;
                        words.push(parsed);
                    }
                    self.advance();
                }
                // Operators that the lexer tokenizes separately
                Some(Token::And) => {
                    words.push(Word::literal("&&"));
                    self.advance();
                }
                Some(Token::Or) => {
                    words.push(Word::literal("||"));
                    self.advance();
                }
                Some(Token::LeftParen) => {
                    if saw_regex_op {
                        // Regex pattern starts with '(' — collect it
                        let pattern = self.collect_conditional_regex_pattern("(");
                        words.push(Word::literal(&pattern));
                        saw_regex_op = false;
                        continue;
                    }
                    words.push(Word::literal("("));
                    self.advance();
                }
                Some(Token::RightParen) => {
                    words.push(Word::literal(")"));
                    self.advance();
                }
                None => {
                    return Err(crate::error::Error::parse(
                        "unexpected end of input in [[ ]]".to_string(),
                    ));
                }
                _ => {
                    // Skip unknown tokens
                    self.advance();
                }
            }
        }

        Ok(CompoundCommand::Conditional(words))
    }

    /// Collect a regex pattern after =~ in [[ ]], handling parens and special chars.
    fn collect_conditional_regex_pattern(&mut self, first_word: &str) -> String {
        let mut pattern = first_word.to_string();
        self.advance(); // consume the first word

        // Concatenate adjacent tokens that are part of the regex pattern
        loop {
            match &self.current_token {
                Some(Token::DoubleRightBracket) => break,
                Some(Token::And) | Some(Token::Or) => break,
                Some(Token::LeftParen) => {
                    pattern.push('(');
                    self.advance();
                }
                Some(Token::RightParen) => {
                    pattern.push(')');
                    self.advance();
                }
                Some(Token::Word(w))
                | Some(Token::LiteralWord(w))
                | Some(Token::QuotedWord(w)) => {
                    pattern.push_str(w);
                    self.advance();
                }
                _ => break,
            }
        }

        pattern
    }

    /// Check if current token starts with `=` (e.g., Word("=5") from `>=5`).
    /// If so, return the rest of the word after `=`.
    fn current_token_starts_with_eq(&self) -> Option<String> {
        match &self.current_token {
            Some(Token::Assignment) => Some(String::new()),
            Some(Token::Word(w)) | Some(Token::LiteralWord(w)) => {
                w.strip_prefix('=').map(|rest| rest.to_string())
            }
            _ => None,
        }
    }

    fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        self.advance(); // consume '(('

        // Read expression until we find ))
        let mut expr = String::new();
        let mut depth = 1;

        loop {
            match &self.current_token {
                Some(Token::DoubleLeftParen) => {
                    depth += 1;
                    expr.push_str("((");
                    self.advance();
                }
                Some(Token::DoubleRightParen) => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance(); // consume '))'
                        break;
                    }
                    expr.push_str("))");
                    self.advance();
                }
                Some(Token::LeftParen) => {
                    expr.push('(');
                    self.advance();
                }
                Some(Token::RightParen) => {
                    expr.push(')');
                    self.advance();
                }
                Some(Token::Word(w))
                | Some(Token::LiteralWord(w))
                | Some(Token::QuotedWord(w)) => {
                    if !expr.is_empty() && !expr.ends_with(' ') && !expr.ends_with('(') {
                        expr.push(' ');
                    }
                    expr.push_str(w);
                    self.advance();
                }
                Some(Token::Semicolon) => {
                    expr.push(';');
                    self.advance();
                }
                Some(Token::Newline) => {
                    self.advance();
                }
                // Handle operators that are normally special tokens but valid in arithmetic
                Some(Token::RedirectIn) => {
                    self.advance();
                    // Check if next token starts with '=' to form '<='
                    if let Some(rest) = self.current_token_starts_with_eq() {
                        expr.push_str("<=");
                        if !rest.is_empty() {
                            expr.push_str(&rest);
                        }
                        self.advance();
                    } else {
                        expr.push('<');
                    }
                }
                Some(Token::RedirectOut) => {
                    self.advance();
                    // Check if next token starts with '=' to form '>='
                    if let Some(rest) = self.current_token_starts_with_eq() {
                        expr.push_str(">=");
                        if !rest.is_empty() {
                            expr.push_str(&rest);
                        }
                        self.advance();
                    } else {
                        expr.push('>');
                    }
                }
                Some(Token::And) => {
                    expr.push_str("&&");
                    self.advance();
                }
                Some(Token::Or) => {
                    expr.push_str("||");
                    self.advance();
                }
                Some(Token::Pipe) => {
                    expr.push('|');
                    self.advance();
                }
                Some(Token::Background) => {
                    expr.push('&');
                    self.advance();
                }
                Some(Token::Assignment) => {
                    expr.push('=');
                    self.advance();
                }
                // In arithmetic context, N> is a number followed by >, not a fd redirect
                Some(Token::RedirectFd(fd)) => {
                    let fd = *fd;
                    self.advance();
                    if let Some(rest) = self.current_token_starts_with_eq() {
                        // N>= → number >= ...
                        expr.push_str(&format!("{}>=", fd));
                        if !rest.is_empty() {
                            expr.push_str(&rest);
                        }
                        self.advance();
                    } else {
                        expr.push_str(&format!("{}>", fd));
                    }
                }
                Some(Token::RedirectFdAppend(fd)) => {
                    // N>> in arithmetic is N >> (right shift)
                    let fd = *fd;
                    expr.push_str(&format!("{}>>", fd));
                    self.advance();
                }
                Some(Token::RedirectFdIn(fd)) => {
                    let fd = *fd;
                    self.advance();
                    if let Some(rest) = self.current_token_starts_with_eq() {
                        expr.push_str(&format!("{}<=", fd));
                        if !rest.is_empty() {
                            expr.push_str(&rest);
                        }
                        self.advance();
                    } else {
                        expr.push_str(&format!("{}<", fd));
                    }
                }
                Some(Token::RedirectAppend) => {
                    // >> in arithmetic is right shift
                    expr.push_str(">>");
                    self.advance();
                }
                None => {
                    return Err(Error::parse(
                        "unexpected end of input in arithmetic command".to_string(),
                    ));
                }
                _ => {
                    self.advance();
                }
            }
        }

        Ok(CompoundCommand::Arithmetic(expr.trim().to_string()))
    }

    /// Parse function definition with 'function' keyword: function name { body }
    fn parse_function_keyword(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        self.advance(); // consume 'function'
        self.skip_newlines()?;

        // Get function name
        let name = match &self.current_token {
            Some(Token::Word(w)) => w.clone(),
            _ => return Err(self.error("expected function name")),
        };
        self.advance();
        self.skip_newlines()?;

        // Optional () after name
        if matches!(self.current_token, Some(Token::LeftParen)) {
            self.advance(); // consume '('
            if !matches!(self.current_token, Some(Token::RightParen)) {
                return Err(Error::parse(
                    "expected ')' in function definition".to_string(),
                ));
            }
            self.advance(); // consume ')'
            self.skip_newlines()?;
        }

        // Expect { for body
        if !matches!(self.current_token, Some(Token::LeftBrace)) {
            return Err(Error::parse("expected '{' for function body".to_string()));
        }

        // Parse body as brace group
        let body = self.parse_brace_group()?;

        Ok(Command::Function(FunctionDef {
            name,
            body: Box::new(Command::Compound(body, Vec::new())),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse POSIX-style function definition: name() { body }
    fn parse_function_posix(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        // Get function name
        let name = match &self.current_token {
            Some(Token::Word(w)) => w.clone(),
            _ => return Err(self.error("expected function name")),
        };
        self.advance();

        // Consume ()
        if !matches!(self.current_token, Some(Token::LeftParen)) {
            return Err(self.error("expected '(' in function definition"));
        }
        self.advance(); // consume '('

        if !matches!(self.current_token, Some(Token::RightParen)) {
            return Err(self.error("expected ')' in function definition"));
        }
        self.advance(); // consume ')'
        self.skip_newlines()?;

        // Expect { for body
        if !matches!(self.current_token, Some(Token::LeftBrace)) {
            return Err(self.error("expected '{' for function body"));
        }

        // Parse body as brace group
        let body = self.parse_brace_group()?;

        Ok(Command::Function(FunctionDef {
            name,
            body: Box::new(Command::Compound(body, Vec::new())),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse commands until a terminating keyword
    fn parse_compound_list(&mut self, terminator: &str) -> Result<Vec<Command>> {
        self.parse_compound_list_until(&[terminator])
    }

    /// Parse commands until one of the terminating keywords
    fn parse_compound_list_until(&mut self, terminators: &[&str]) -> Result<Vec<Command>> {
        let mut commands = Vec::new();

        loop {
            self.skip_newlines()?;

            // Check for terminators
            if let Some(Token::Word(w)) = &self.current_token
                && terminators.contains(&w.as_str())
            {
                break;
            }

            if self.current_token.is_none() {
                break;
            }

            commands.push(self.parse_command_list_required()?);
        }

        Ok(commands)
    }

    /// Reserved words that cannot start a simple command.
    /// These words are only special in command position, not as arguments.
    const NON_COMMAND_WORDS: &'static [&'static str] =
        &["then", "else", "elif", "fi", "do", "done", "esac", "in"];

    /// Check if a word cannot start a command
    fn is_non_command_word(word: &str) -> bool {
        Self::NON_COMMAND_WORDS.contains(&word)
    }

    /// Check if current token is a specific keyword
    fn is_keyword(&self, keyword: &str) -> bool {
        matches!(&self.current_token, Some(Token::Word(w)) if w == keyword)
    }

    /// Expect a specific keyword
    fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        if self.is_keyword(keyword) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!("expected '{}'", keyword)))
        }
    }

    /// Strip surrounding quotes from a string value
    fn strip_quotes(s: &str) -> &str {
        if s.len() >= 2
            && ((s.starts_with('"') && s.ends_with('"'))
                || (s.starts_with('\'') && s.ends_with('\'')))
        {
            return &s[1..s.len() - 1];
        }
        s
    }

    /// Check if a word is an assignment (NAME=value, NAME+=value, or NAME[index]=value)
    /// Returns (name, optional_index, value, is_append)
    fn is_assignment(word: &str) -> Option<(&str, Option<&str>, &str, bool)> {
        // Check for += append operator first
        let (eq_pos, is_append) = if let Some(pos) = word.find("+=") {
            (pos, true)
        } else if let Some(pos) = word.find('=') {
            (pos, false)
        } else {
            return None;
        };

        let lhs = &word[..eq_pos];
        let value = &word[eq_pos + if is_append { 2 } else { 1 }..];

        // Check for array subscript: name[index]
        if let Some(bracket_pos) = lhs.find('[') {
            let name = &lhs[..bracket_pos];
            // Validate name
            if name.is_empty() {
                return None;
            }
            let mut chars = name.chars();
            let first = chars.next().unwrap();
            if !first.is_ascii_alphabetic() && first != '_' {
                return None;
            }
            for c in chars {
                if !c.is_ascii_alphanumeric() && c != '_' {
                    return None;
                }
            }
            // Extract index (everything between [ and ])
            if lhs.ends_with(']') {
                let index = &lhs[bracket_pos + 1..lhs.len() - 1];
                return Some((name, Some(index), value, is_append));
            }
        } else {
            // Name must be valid identifier: starts with letter or _, followed by alnum or _
            if lhs.is_empty() {
                return None;
            }
            let mut chars = lhs.chars();
            let first = chars.next().unwrap();
            if !first.is_ascii_alphabetic() && first != '_' {
                return None;
            }
            for c in chars {
                if !c.is_ascii_alphanumeric() && c != '_' {
                    return None;
                }
            }
            return Some((lhs, None, value, is_append));
        }
        None
    }

    /// Parse a simple command with redirections
    /// Collect array elements between `(` and `)` tokens into a `Vec<Word>`.
    fn collect_array_elements(&mut self) -> (Vec<Word>, Span) {
        let mut elements = Vec::new();
        let mut closing_span = Span::new();
        loop {
            match &self.current_token {
                Some(Token::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(Token::Word(_))
                | Some(Token::LiteralWord(_))
                | Some(Token::QuotedWord(_)) => {
                    if let Some(word) = self.current_word_to_word() {
                        elements.push(word);
                    }
                    self.advance();
                }
                None => break,
                _ => {
                    self.advance();
                }
            }
        }
        (elements, closing_span)
    }

    /// Parse the value side of an assignment (`VAR=value`).
    /// Returns `Some((Assignment, needs_advance))` if the current word is an assignment.
    /// The bool indicates whether the caller must call `self.advance()` afterward.
    fn try_parse_assignment(&mut self, w: &str) -> Option<(Assignment, bool)> {
        let (name, index, value, is_append) = Self::is_assignment(w)?;
        let assignment_span = self.current_span;
        let value_start_offset = if let Some(pos) = w.find("+=") {
            pos + 2
        } else {
            w.find('=')? + 1
        };
        let value_start = assignment_span.start.advanced_by(&w[..value_start_offset]);
        let value_span = Span::from_positions(value_start, assignment_span.end);
        let name = name.to_string();
        let index = index.map(|s| s.to_string());
        let value_str = value.to_string();

        // Array literal in the token itself: arr=(a b c)
        if value_str.starts_with('(') && value_str.ends_with(')') {
            let inner = &value_str[1..value_str.len() - 1];
            let elements: Vec<Word> = inner
                .split_whitespace()
                .map(|s| self.parse_word(s.to_string()))
                .collect();
            return Some((
                Assignment {
                    name,
                    index,
                    value: AssignmentValue::Array(elements),
                    append: is_append,
                    span: assignment_span,
                },
                true,
            ));
        }

        // Empty value — check for arr=(...) syntax with separate tokens
        if value_str.is_empty() {
            self.advance();
            if matches!(self.current_token, Some(Token::LeftParen)) {
                let open_paren_span = self.current_span;
                self.advance(); // consume '('
                let (elements, close_span) = self.collect_array_elements();
                return Some((
                    Assignment {
                        name,
                        index,
                        value: AssignmentValue::Array(elements),
                        append: is_append,
                        span: Self::merge_optional_span(
                            assignment_span,
                            Self::merge_optional_span(open_paren_span, close_span),
                        ),
                    },
                    false,
                ));
            }
            // Empty assignment: VAR=
            return Some((
                Assignment {
                    name,
                    index,
                    value: AssignmentValue::Scalar(Word::literal_with_span("", value_span)),
                    append: is_append,
                    span: assignment_span,
                },
                false,
            ));
        }

        // Quoted or plain scalar value
        let value_word = if value_str.starts_with('"') && value_str.ends_with('"') {
            let inner = Self::strip_quotes(&value_str);
            let mut w = self.parse_word_with_context(
                inner.to_string(),
                value_span,
                value_start.advanced_by("\""),
            );
            w.quoted = true;
            w
        } else if value_str.starts_with('\'') && value_str.ends_with('\'') {
            let inner = Self::strip_quotes(&value_str);
            Word::quoted_literal_with_span(inner.to_string(), value_span)
        } else {
            self.parse_word_with_context(value_str, value_span, value_start)
        };
        Some((
            Assignment {
                name,
                index,
                value: AssignmentValue::Scalar(value_word),
                append: is_append,
                span: assignment_span,
            },
            true,
        ))
    }

    /// Parse a compound array argument in arg position (e.g. `declare -a arr=(x y z)`).
    /// Called when the current word ends with `=` and the next token is `(`.
    /// Returns the compound word if successful, or `None` if not a compound assignment.
    fn try_parse_compound_array_arg(&mut self, saved_w: String) -> Option<Word> {
        if !matches!(self.current_token, Some(Token::LeftParen)) {
            return None;
        }
        self.advance(); // consume '('
        let mut compound = saved_w;
        compound.push('(');
        loop {
            match &self.current_token {
                Some(Token::RightParen) => {
                    compound.push(')');
                    self.advance();
                    break;
                }
                Some(Token::Word(elem))
                | Some(Token::LiteralWord(elem))
                | Some(Token::QuotedWord(elem)) => {
                    if !compound.ends_with('(') {
                        compound.push(' ');
                    }
                    compound.push_str(elem);
                    self.advance();
                }
                None => break,
                _ => {
                    self.advance();
                }
            }
        }
        Some(self.parse_word(compound))
    }

    /// Parse a heredoc redirect (`<<` or `<<-`) and any trailing redirects on the same line.
    fn parse_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
    ) -> Result<()> {
        let operator_span = self.current_span;
        self.advance();
        // Get the delimiter word and track if it was quoted
        let (delimiter, quoted) = match &self.current_token {
            Some(Token::Word(w)) => (w.clone(), false),
            Some(Token::LiteralWord(w)) => (w.clone(), true),
            Some(Token::QuotedWord(w)) => (w.clone(), true),
            _ => return Err(Error::parse("expected delimiter after <<".to_string())),
        };

        let content = self.lexer.read_heredoc(&delimiter);

        // Strip leading tabs for <<-
        let content = if strip_tabs {
            let had_trailing_newline = content.ends_with('\n');
            let mut stripped: String = content
                .lines()
                .map(|l: &str| l.trim_start_matches('\t'))
                .collect::<Vec<_>>()
                .join("\n");
            if had_trailing_newline {
                stripped.push('\n');
            }
            stripped
        } else {
            content
        };

        let target = if quoted {
            Word::quoted_literal(content)
        } else {
            self.parse_word(content)
        };

        let kind = if strip_tabs {
            RedirectKind::HereDocStrip
        } else {
            RedirectKind::HereDoc
        };

        redirects.push(Redirect {
            fd: None,
            fd_var: None,
            kind,
            span: operator_span,
            target,
        });

        // Advance so re-injected rest-of-line tokens are picked up
        self.advance();

        // Consume any trailing redirects on the same line (e.g. `cat <<EOF > file`)
        self.collect_trailing_redirects(redirects);
        Ok(())
    }

    /// Consume redirect tokens that follow a heredoc on the same line.
    fn collect_trailing_redirects(&mut self, redirects: &mut Vec<Redirect>) {
        while let Some(tok) = &self.current_token {
            match tok {
                Token::RedirectOut | Token::Clobber => {
                    let operator_span = self.current_span;
                    let kind = if matches!(&self.current_token, Some(Token::Clobber)) {
                        RedirectKind::Clobber
                    } else {
                        RedirectKind::Output
                    };
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Token::RedirectAppend => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: None,
                            fd_var: None,
                            kind: RedirectKind::Append,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Token::RedirectFd(fd) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(fd),
                            fd_var: None,
                            kind: RedirectKind::Output,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Token::DupInput => {
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(0),
                            fd_var: None,
                            kind: RedirectKind::DupInput,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Token::DupFdIn(src_fd, dst_fd) => {
                    let src_fd = *src_fd;
                    let dst_fd = *dst_fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(src_fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Token::DupFdClose(fd) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal("-"),
                    });
                }
                Token::RedirectFdIn(fd) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    if let Ok(target) = self.expect_word() {
                        redirects.push(Redirect {
                            fd: Some(fd),
                            fd_var: None,
                            kind: RedirectKind::Input,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                _ => break,
            }
        }
    }

    /// Extract fd-variable name from `{varname}` pattern in the last word.
    /// If the last word is a single literal `{identifier}`, pop it and return the name.
    /// Used for `exec {var}>file` / `exec {var}>&-` syntax.
    fn pop_fd_var(words: &mut Vec<Word>) -> Option<String> {
        if let Some(last) = words.last()
            && last.parts.len() == 1
            && let WordPart::Literal(ref s) = last.parts[0]
            && s.starts_with('{')
            && s.ends_with('}')
            && s.len() > 2
            && s[1..s.len() - 1]
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_')
        {
            let var_name = s[1..s.len() - 1].to_string();
            words.pop();
            return Some(var_name);
        }
        None
    }

    fn parse_simple_command(&mut self) -> Result<Option<SimpleCommand>> {
        self.tick()?;
        self.skip_newlines()?;
        self.check_error_token()?;
        let start_span = self.current_span;

        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirects = Vec::new();

        loop {
            match &self.current_token {
                Some(Token::Word(w))
                | Some(Token::LiteralWord(w))
                | Some(Token::QuotedWord(w)) => {
                    let is_literal =
                        matches!(&self.current_token, Some(Token::LiteralWord(_)));
                    // Clone early to release borrow on self.current_token
                    let w = w.clone();

                    // Stop if this word cannot start a command (like 'then', 'fi', etc.)
                    if words.is_empty() && Self::is_non_command_word(&w) {
                        break;
                    }

                    // Check for assignment (only before the command name, not for literal words)
                    if words.is_empty()
                        && !is_literal
                        && let Some((assignment, needs_advance)) = self.try_parse_assignment(&w)
                    {
                        if needs_advance {
                            self.advance();
                        }
                        assignments.push(assignment);
                        continue;
                    }

                    // Handle compound array assignment in arg position:
                    // declare -a arr=(x y z) → arr=(x y z) as single arg
                    if w.ends_with('=') && !words.is_empty() {
                        self.advance();
                        if let Some(word) = self.try_parse_compound_array_arg(w.clone()) {
                            words.push(word);
                            continue;
                        }
                        // Not a compound assignment — treat as regular word
                        if let Some(word) = self.current_word_to_word() {
                            words.push(word);
                        }
                        continue;
                    }

                    if let Some(word) = self.current_word_to_word() {
                        words.push(word);
                    }
                    self.advance();
                }
                Some(Token::RedirectOut) | Some(Token::Clobber) => {
                    let operator_span = self.current_span;
                    let kind = if matches!(&self.current_token, Some(Token::Clobber)) {
                        RedirectKind::Clobber
                    } else {
                        RedirectKind::Output
                    };
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        kind,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectAppend) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectIn) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::HereString) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        kind: RedirectKind::HereString,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::HereDoc) | Some(Token::HereDocStrip) => {
                    let strip_tabs =
                        matches!(self.current_token, Some(Token::HereDocStrip));
                    self.parse_heredoc_redirect(strip_tabs, &mut redirects)?;
                    break;
                }
                Some(Token::ProcessSubIn) | Some(Token::ProcessSubOut) => {
                    let word = self.expect_word()?;
                    words.push(word);
                }
                Some(Token::RedirectBoth) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        kind: RedirectKind::OutputBoth,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::DupOutput) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(1) },
                        fd_var,
                        kind: RedirectKind::DupOutput,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectFd(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::Output,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectFdAppend(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::DupFd(src_fd, dst_fd)) => {
                    let src_fd = *src_fd;
                    let dst_fd = *dst_fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(src_fd),
                        fd_var: None,
                        kind: RedirectKind::DupOutput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Some(Token::DupInput) => {
                    let operator_span = self.current_span;
                    let fd_var = Self::pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(0) },
                        fd_var,
                        kind: RedirectKind::DupInput,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::DupFdIn(src_fd, dst_fd)) => {
                    let src_fd = *src_fd;
                    let dst_fd = *dst_fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(src_fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Some(Token::DupFdClose(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::DupInput,
                        span: operator_span,
                        target: Word::literal("-"),
                    });
                }
                Some(Token::RedirectFdIn(fd)) => {
                    let fd = *fd;
                    let operator_span = self.current_span;
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                // { and } as arguments (not in command position) are literal words
                Some(Token::LeftBrace) | Some(Token::RightBrace)
                    if !words.is_empty() =>
                {
                    let sym = if matches!(self.current_token, Some(Token::LeftBrace)) {
                        "{"
                    } else {
                        "}"
                    };
                    words.push(Word::literal_with_span(sym, self.current_span));
                    self.advance();
                }
                Some(Token::Newline)
                | Some(Token::Semicolon)
                | Some(Token::Pipe)
                | Some(Token::And)
                | Some(Token::Or)
                | None => break,
                _ => break,
            }
        }

        // Handle assignment-only commands (VAR=value with no command)
        if words.is_empty() && !assignments.is_empty() {
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

    /// Expect a word token and return it as a Word
    fn expect_word(&mut self) -> Result<Word> {
        match &self.current_token {
            Some(Token::Word(_))
            | Some(Token::LiteralWord(_))
            | Some(Token::QuotedWord(_)) => {
                let word = self
                    .current_word_to_word()
                    .ok_or_else(|| self.error("expected word"))?;
                self.advance();
                Ok(word)
            }
            Some(Token::ProcessSubIn) | Some(Token::ProcessSubOut) => {
                // Process substitution <(cmd) or >(cmd)
                let is_input = matches!(self.current_token, Some(Token::ProcessSubIn));
                let process_span = self.current_span;
                self.advance();

                // Walk tokens until the matching closing paren, then reparse the original
                // source slice so nested command spans remain absolute.
                let mut depth = 1;
                let close_span = loop {
                    match &self.current_token {
                        Some(Token::LeftParen) => {
                            depth += 1;
                            self.advance();
                        }
                        Some(Token::RightParen) => {
                            depth -= 1;
                            if depth == 0 {
                                let close_span = self.current_span;
                                self.advance();
                                break close_span;
                            }
                            self.advance();
                        }
                        None => {
                            return Err(Error::parse(
                                "unexpected end of input in process substitution".to_string(),
                            ));
                        }
                        _ => self.advance(),
                    }
                };

                let inner_start = process_span.end;
                let commands =
                    self.nested_commands_from_current_input(inner_start, close_span.start);

                Ok(Word {
                    parts: vec![WordPart::ProcessSubstitution { commands, is_input }],
                    part_spans: vec![process_span.merge(close_span)],
                    quoted: false,
                    span: process_span.merge(close_span),
                })
            }
            _ => Err(self.error("expected word")),
        }
    }

    // Helper methods for word handling - kept for potential future use
    #[allow(dead_code)]
    /// Check if current token is a word (Word, LiteralWord, or QuotedWord)
    fn is_current_word(&self) -> bool {
        matches!(
            &self.current_token,
            Some(Token::Word(_))
                | Some(Token::LiteralWord(_))
                | Some(Token::QuotedWord(_))
        )
    }

    #[allow(dead_code)]
    /// Get the string content if current token is a word
    fn current_word_str(&self) -> Option<String> {
        match &self.current_token {
            Some(Token::Word(w))
            | Some(Token::LiteralWord(w))
            | Some(Token::QuotedWord(w)) => Some(w.clone()),
            _ => None,
        }
    }

    /// Parse a word string into a Word with proper parts (variables, literals)
    fn parse_word(&self, s: String) -> Word {
        self.parse_word_with_context(s, Span::new(), Position::new())
    }

    fn parse_word_with_context(&self, s: String, span: Span, base: Position) -> Word {
        let mut parts = Vec::new();
        let mut part_spans = Vec::new();
        let mut chars = s.chars().peekable();
        let mut current = String::new();
        let mut current_start = base;
        let mut cursor = base;

        while chars.peek().is_some() {
            let part_start = cursor;
            let ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);

            if ch == '\x00' {
                if current.is_empty() {
                    current_start = part_start;
                }
                if let Some(literal_ch) = Self::next_word_char(&mut chars, &mut cursor) {
                    current.push(literal_ch);
                }
                continue;
            }

            if ch != '$' {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push(ch);
                continue;
            }

            Self::flush_literal_part(
                &mut parts,
                &mut part_spans,
                &mut current,
                current_start,
                part_start,
            );

            if chars.peek() == Some(&'\'') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let mut ansi = String::new();
                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if c == '\'' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(esc) = Self::next_word_char(&mut chars, &mut cursor) {
                            match esc {
                                'n' => ansi.push('\n'),
                                't' => ansi.push('\t'),
                                'r' => ansi.push('\r'),
                                'a' => ansi.push('\x07'),
                                'b' => ansi.push('\x08'),
                                'e' | 'E' => ansi.push('\x1B'),
                                '\\' => ansi.push('\\'),
                                '\'' => ansi.push('\''),
                                _ => {
                                    ansi.push('\\');
                                    ansi.push(esc);
                                }
                            }
                        }
                    } else {
                        ansi.push(c);
                    }
                }
                Self::push_word_part(
                    &mut parts,
                    &mut part_spans,
                    WordPart::Literal(ansi),
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'(') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                if chars.peek() == Some(&'(') {
                    Self::next_word_char_unwrap(&mut chars, &mut cursor);
                    let mut expr = String::new();
                    let mut depth = 2;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if c == '(' {
                            depth += 1;
                            expr.push(c);
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            expr.push(c);
                        } else {
                            expr.push(c);
                        }
                    }
                    if expr.ends_with(')') {
                        expr.pop();
                    }
                    Self::push_word_part(
                        &mut parts,
                        &mut part_spans,
                        WordPart::ArithmeticExpansion(expr),
                        part_start,
                        cursor,
                    );
                } else {
                    let mut cmd_str = String::new();
                    let mut depth = 1;
                    let inner_start = cursor;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if c == '(' {
                            depth += 1;
                            cmd_str.push(c);
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            cmd_str.push(c);
                        } else {
                            cmd_str.push(c);
                        }
                    }
                    Self::push_word_part(
                        &mut parts,
                        &mut part_spans,
                        WordPart::CommandSubstitution(
                            self.nested_commands_from_source(&cmd_str, inner_start),
                        ),
                        part_start,
                        cursor,
                    );
                }
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'{') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);

                if Self::consume_word_char_if(&mut chars, &mut cursor, '#') {
                    let var_name =
                        Self::read_word_while(&mut chars, &mut cursor, |c| c != '}' && c != '[');
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let index = Self::read_word_while(&mut chars, &mut cursor, |c| c != ']');
                        Self::consume_word_char_if(&mut chars, &mut cursor, ']');
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = if index == "@" || index == "*" {
                            WordPart::ArrayLength(var_name)
                        } else {
                            WordPart::Length(format!("{}[{}]", var_name, index))
                        };
                        Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    } else {
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::Length(var_name),
                            part_start,
                            cursor,
                        );
                    }
                    current_start = cursor;
                    continue;
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '!') {
                    let var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                        !matches!(c, '}' | '[' | '*' | '@' | ':' | '-' | '=' | '+' | '?')
                    });

                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let index = Self::read_word_while(&mut chars, &mut cursor, |c| c != ']');
                        Self::consume_word_char_if(&mut chars, &mut cursor, ']');
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = if index == "@" || index == "*" {
                            WordPart::ArrayIndices(var_name)
                        } else {
                            WordPart::Variable(format!("!{}[{}]", var_name, index))
                        };
                        Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::IndirectExpansion {
                                name: var_name,
                                operator: None,
                                operand: String::new(),
                                colon_variant: false,
                            },
                            part_start,
                            cursor,
                        );
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, ':') {
                        let operator = match chars.peek().copied() {
                            Some('-') => Some(ParameterOp::UseDefault),
                            Some('=') => Some(ParameterOp::AssignDefault),
                            Some('+') => Some(ParameterOp::UseReplacement),
                            Some('?') => Some(ParameterOp::Error),
                            _ => None,
                        };
                        if let Some(operator) = operator {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand = self.read_brace_operand(&mut chars, &mut cursor);
                            Self::push_word_part(
                                &mut parts,
                                &mut part_spans,
                                WordPart::IndirectExpansion {
                                    name: var_name,
                                    operator: Some(operator),
                                    operand,
                                    colon_variant: true,
                                },
                                part_start,
                                cursor,
                            );
                        } else {
                            let mut suffix = String::new();
                            while let Some(&c) = chars.peek() {
                                if c == '}' {
                                    Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                    break;
                                }
                                suffix.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                            }
                            Self::push_word_part(
                                &mut parts,
                                &mut part_spans,
                                WordPart::Variable(format!("!{}{}", var_name, suffix)),
                                part_start,
                                cursor,
                            );
                        }
                    } else if matches!(
                        chars.peek(),
                        Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?')
                    ) {
                        let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                        let operand = self.read_brace_operand(&mut chars, &mut cursor);
                        let operator = match op_char {
                            '-' => ParameterOp::UseDefault,
                            '=' => ParameterOp::AssignDefault,
                            '+' => ParameterOp::UseReplacement,
                            '?' => ParameterOp::Error,
                            _ => unreachable!(),
                        };
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::IndirectExpansion {
                                name: var_name,
                                operator: Some(operator),
                                operand,
                                colon_variant: false,
                            },
                            part_start,
                            cursor,
                        );
                    } else {
                        let mut suffix = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '}' {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                break;
                            }
                            suffix.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                        }
                        let part = if suffix.ends_with('*') || suffix.ends_with('@') {
                            WordPart::PrefixMatch(format!(
                                "{}{}",
                                var_name,
                                &suffix[..suffix.len() - 1]
                            ))
                        } else {
                            WordPart::Variable(format!("!{}{}", var_name, suffix))
                        };
                        Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    }

                    current_start = cursor;
                    continue;
                }

                let mut var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                    c.is_ascii_alphanumeric() || c == '_'
                });

                if var_name.is_empty()
                    && let Some(&c) = chars.peek()
                    && matches!(c, '@' | '*')
                {
                    var_name.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                    let mut index = String::new();
                    let mut bracket_depth: i32 = 0;
                    let mut brace_depth: i32 = 0;
                    while let Some(&c) = chars.peek() {
                        if c == ']' && bracket_depth == 0 && brace_depth == 0 {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            break;
                        }
                        match c {
                            '[' => bracket_depth += 1,
                            ']' => bracket_depth -= 1,
                            '$' => {
                                index.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                                if chars.peek() == Some(&'{') {
                                    brace_depth += 1;
                                    index
                                        .push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                                    continue;
                                }
                                continue;
                            }
                            '{' => brace_depth += 1,
                            '}' => {
                                if brace_depth > 0 {
                                    brace_depth -= 1;
                                }
                            }
                            _ => {}
                        }
                        index.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                    }

                    if index.len() >= 2
                        && ((index.starts_with('"') && index.ends_with('"'))
                            || (index.starts_with('\'') && index.ends_with('\'')))
                    {
                        index = index[1..index.len() - 1].to_string();
                    }

                    let part = if let Some(next_c) = chars.peek().copied() {
                        if next_c == ':' {
                            let mut lookahead = chars.clone();
                            lookahead.next();
                            let is_param_op = matches!(
                                lookahead.peek(),
                                Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?')
                            );
                            if is_param_op {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let arr_name = format!("{}[{}]", var_name, index);
                                let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let operand = self.read_brace_operand(&mut chars, &mut cursor);
                                let operator = match op_char {
                                    '-' => ParameterOp::UseDefault,
                                    '=' => ParameterOp::AssignDefault,
                                    '+' => ParameterOp::UseReplacement,
                                    '?' => ParameterOp::Error,
                                    _ => unreachable!(),
                                };
                                WordPart::ParameterExpansion {
                                    name: arr_name,
                                    operator,
                                    operand,
                                    colon_variant: true,
                                }
                            } else {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let offset = Self::read_word_while(&mut chars, &mut cursor, |c| {
                                    c != ':' && c != '}'
                                });
                                let length =
                                    if Self::consume_word_char_if(&mut chars, &mut cursor, ':') {
                                        Some(Self::read_word_while(&mut chars, &mut cursor, |c| {
                                            c != '}'
                                        }))
                                    } else {
                                        None
                                    };
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::ArraySlice {
                                    name: var_name,
                                    offset,
                                    length,
                                }
                            }
                        } else if matches!(next_c, '-' | '+' | '=' | '?') {
                            let arr_name = format!("{}[{}]", var_name, index);
                            let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand = self.read_brace_operand(&mut chars, &mut cursor);
                            let operator = match op_char {
                                '-' => ParameterOp::UseDefault,
                                '=' => ParameterOp::AssignDefault,
                                '+' => ParameterOp::UseReplacement,
                                '?' => ParameterOp::Error,
                                _ => unreachable!(),
                            };
                            WordPart::ParameterExpansion {
                                name: arr_name,
                                operator,
                                operand,
                                colon_variant: false,
                            }
                        } else {
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ArrayAccess {
                                name: var_name,
                                index,
                            }
                        }
                    } else {
                        WordPart::ArrayAccess {
                            name: var_name,
                            index,
                        }
                    };

                    Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                let part = if let Some(c) = chars.peek().copied() {
                    match c {
                        ':' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match chars.peek() {
                                Some(&'-') | Some(&'=') | Some(&'+') | Some(&'?') => {
                                    let op_char =
                                        Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                    let operand = self.read_brace_operand(&mut chars, &mut cursor);
                                    let operator = match op_char {
                                        '-' => ParameterOp::UseDefault,
                                        '=' => ParameterOp::AssignDefault,
                                        '+' => ParameterOp::UseReplacement,
                                        '?' => ParameterOp::Error,
                                        _ => unreachable!(),
                                    };
                                    WordPart::ParameterExpansion {
                                        name: var_name,
                                        operator,
                                        operand,
                                        colon_variant: true,
                                    }
                                }
                                _ => {
                                    let offset =
                                        Self::read_word_while(&mut chars, &mut cursor, |ch| {
                                            ch != ':' && ch != '}'
                                        });
                                    let length =
                                        if Self::consume_word_char_if(&mut chars, &mut cursor, ':')
                                        {
                                            Some(Self::read_word_while(
                                                &mut chars,
                                                &mut cursor,
                                                |ch| ch != '}',
                                            ))
                                        } else {
                                            None
                                        };
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                    WordPart::Substring {
                                        name: var_name,
                                        offset,
                                        length,
                                    }
                                }
                            }
                        }
                        '-' | '=' | '+' | '?' => {
                            let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand = self.read_brace_operand(&mut chars, &mut cursor);
                            let operator = match op_char {
                                '-' => ParameterOp::UseDefault,
                                '=' => ParameterOp::AssignDefault,
                                '+' => ParameterOp::UseReplacement,
                                '?' => ParameterOp::Error,
                                _ => unreachable!(),
                            };
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand,
                                colon_variant: false,
                            }
                        }
                        '#' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '#') {
                                    ParameterOp::RemovePrefixLong
                                } else {
                                    ParameterOp::RemovePrefixShort
                                };
                            let operand = self.read_brace_operand(&mut chars, &mut cursor);
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand,
                                colon_variant: false,
                            }
                        }
                        '%' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '%') {
                                    ParameterOp::RemoveSuffixLong
                                } else {
                                    ParameterOp::RemoveSuffixShort
                                };
                            let operand = self.read_brace_operand(&mut chars, &mut cursor);
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand,
                                colon_variant: false,
                            }
                        }
                        '/' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let replace_all =
                                Self::consume_word_char_if(&mut chars, &mut cursor, '/');
                            let mut pattern = String::new();
                            while let Some(&ch) = chars.peek() {
                                if ch == '/' || ch == '}' {
                                    break;
                                }
                                if ch == '\\' {
                                    Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                    if let Some(&next) = chars.peek()
                                        && next == '/'
                                    {
                                        pattern.push(Self::next_word_char_unwrap(
                                            &mut chars,
                                            &mut cursor,
                                        ));
                                        continue;
                                    }
                                    pattern.push('\\');
                                    continue;
                                }
                                pattern.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                            }
                            let replacement =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '/') {
                                    Self::read_word_while(&mut chars, &mut cursor, |ch| ch != '}')
                                } else {
                                    String::new()
                                };
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            let operator = if replace_all {
                                ParameterOp::ReplaceAll {
                                    pattern,
                                    replacement,
                                }
                            } else {
                                ParameterOp::ReplaceFirst {
                                    pattern,
                                    replacement,
                                }
                            };
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand: String::new(),
                                colon_variant: false,
                            }
                        }
                        '^' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '^') {
                                    ParameterOp::UpperAll
                                } else {
                                    ParameterOp::UpperFirst
                                };
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand: String::new(),
                                colon_variant: false,
                            }
                        }
                        ',' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operator =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, ',') {
                                    ParameterOp::LowerAll
                                } else {
                                    ParameterOp::LowerFirst
                                };
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ParameterExpansion {
                                name: var_name,
                                operator,
                                operand: String::new(),
                                colon_variant: false,
                            }
                        }
                        '@' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            if chars.peek().is_some() {
                                let operator = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Transformation {
                                    name: var_name,
                                    operator,
                                }
                            } else {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Variable(var_name)
                            }
                        }
                        '}' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            WordPart::Variable(var_name)
                        }
                        _ => {
                            while let Some(&next) = chars.peek() {
                                let consumed = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                if next == '}' || consumed == '}' {
                                    break;
                                }
                            }
                            WordPart::Variable(var_name)
                        }
                    }
                } else {
                    WordPart::Variable(var_name)
                };

                Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                current_start = cursor;
                continue;
            }

            if let Some(&c) = chars.peek() {
                if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-') || c.is_ascii_digit() {
                    let name = Self::next_word_char_unwrap(&mut chars, &mut cursor).to_string();
                    Self::push_word_part(
                        &mut parts,
                        &mut part_spans,
                        WordPart::Variable(name),
                        part_start,
                        cursor,
                    );
                    current_start = cursor;
                } else {
                    let var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                        c.is_ascii_alphanumeric() || c == '_'
                    });
                    if !var_name.is_empty() {
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::Variable(var_name),
                            part_start,
                            cursor,
                        );
                        current_start = cursor;
                    } else {
                        if current.is_empty() {
                            current_start = part_start;
                        }
                        current.push('$');
                    }
                }
            } else {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push('$');
            }
        }

        Self::flush_literal_part(
            &mut parts,
            &mut part_spans,
            &mut current,
            current_start,
            cursor,
        );

        if parts.is_empty() {
            Self::push_word_part(
                &mut parts,
                &mut part_spans,
                WordPart::Literal(String::new()),
                base,
                cursor,
            );
        }

        Word {
            parts,
            part_spans,
            quoted: false,
            span,
        }
    }

    /// Read operand for brace expansion (everything until closing brace)
    fn read_brace_operand(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
    ) -> String {
        let mut operand = String::new();
        let mut depth = 1;
        while let Some(&c) = chars.peek() {
            if c == '{' {
                depth += 1;
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    Self::next_word_char_unwrap(chars, cursor);
                    break;
                }
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            } else {
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            }
        }
        operand
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_command() {
        let parser = Parser::new("echo hello");
        let script = parser.parse().unwrap();

        assert_eq!(script.commands.len(), 1);

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.to_string(), "echo");
            assert_eq!(cmd.args.len(), 1);
            assert_eq!(cmd.args[0].to_string(), "hello");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_multiple_args() {
        let parser = Parser::new("echo hello world");
        let script = parser.parse().unwrap();

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.to_string(), "echo");
            assert_eq!(cmd.args.len(), 2);
            assert_eq!(cmd.args[0].to_string(), "hello");
            assert_eq!(cmd.args[1].to_string(), "world");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_variable() {
        let parser = Parser::new("echo $HOME");
        let script = parser.parse().unwrap();

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.args.len(), 1);
            assert_eq!(cmd.args[0].parts.len(), 1);
            assert!(matches!(&cmd.args[0].parts[0], WordPart::Variable(v) if v == "HOME"));
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_unexpected_top_level_token_errors_in_strict_mode() {
        let error = Parser::new("echo ok\n)\necho later\n").parse().unwrap_err();

        let Error::Parse {
            message,
            line,
            column,
        } = error;
        assert_eq!(message, "expected command");
        assert_eq!(line, 2);
        assert_eq!(column, 1);
    }

    #[test]
    fn test_parse_recovered_skips_invalid_command_and_continues() {
        let recovered = Parser::new("echo one\ncat >\necho two\n").parse_recovered();

        assert_eq!(recovered.script.commands.len(), 2);
        assert_eq!(recovered.diagnostics.len(), 1);
        assert_eq!(recovered.diagnostics[0].message, "expected word");
        assert_eq!(recovered.diagnostics[0].span.start.line, 2);

        let Command::Simple(first) = &recovered.script.commands[0] else {
            panic!("expected first command to be simple");
        };
        assert_eq!(first.name.to_string(), "echo");
        assert_eq!(first.args[0].to_string(), "one");

        let Command::Simple(second) = &recovered.script.commands[1] else {
            panic!("expected second command to be simple");
        };
        assert_eq!(second.name.to_string(), "echo");
        assert_eq!(second.args[0].to_string(), "two");
    }

    #[test]
    fn test_parse_pipeline() {
        let parser = Parser::new("echo hello | cat");
        let script = parser.parse().unwrap();

        assert_eq!(script.commands.len(), 1);
        assert!(matches!(&script.commands[0], Command::Pipeline(_)));

        if let Command::Pipeline(pipeline) = &script.commands[0] {
            assert_eq!(pipeline.commands.len(), 2);
        }
    }

    #[test]
    fn test_parse_redirect_out() {
        let parser = Parser::new("echo hello > /tmp/out");
        let script = parser.parse().unwrap();

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.redirects.len(), 1);
            assert_eq!(cmd.redirects[0].kind, RedirectKind::Output);
            assert_eq!(cmd.redirects[0].target.to_string(), "/tmp/out");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_redirect_append() {
        let parser = Parser::new("echo hello >> /tmp/out");
        let script = parser.parse().unwrap();

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.redirects.len(), 1);
            assert_eq!(cmd.redirects[0].kind, RedirectKind::Append);
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_redirect_in() {
        let parser = Parser::new("cat < /tmp/in");
        let script = parser.parse().unwrap();

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.redirects.len(), 1);
            assert_eq!(cmd.redirects[0].kind, RedirectKind::Input);
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_command_list_and() {
        let parser = Parser::new("true && echo success");
        let script = parser.parse().unwrap();

        assert!(matches!(&script.commands[0], Command::List(_)));
    }

    #[test]
    fn test_parse_command_list_or() {
        let parser = Parser::new("false || echo fallback");
        let script = parser.parse().unwrap();

        assert!(matches!(&script.commands[0], Command::List(_)));
    }

    #[test]
    fn test_heredoc_pipe() {
        let parser = Parser::new("cat <<EOF | sort\nc\na\nb\nEOF\n");
        let script = parser.parse().unwrap();
        assert!(
            matches!(&script.commands[0], Command::Pipeline(_)),
            "heredoc with pipe should parse as Pipeline"
        );
    }

    #[test]
    fn test_heredoc_multiple_on_line() {
        let input = "while cat <<E1 && cat <<E2; do cat <<E3; break; done\n1\nE1\n2\nE2\n3\nE3\n";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap();
        assert_eq!(script.commands.len(), 1);
        if let Command::Compound(comp, _) = &script.commands[0] {
            if let CompoundCommand::While(w) = comp {
                assert!(
                    !w.condition.is_empty(),
                    "while condition should be non-empty"
                );
                assert!(!w.body.is_empty(), "while body should be non-empty");
            } else {
                panic!("expected While compound command");
            }
        } else {
            panic!("expected Compound command");
        }
    }

    #[test]
    fn test_empty_function_body_rejected() {
        let parser = Parser::new("f() { }");
        assert!(
            parser.parse().is_err(),
            "empty function body should be rejected"
        );
    }

    #[test]
    fn test_empty_while_body_rejected() {
        let parser = Parser::new("while true; do\ndone");
        assert!(
            parser.parse().is_err(),
            "empty while body should be rejected"
        );
    }

    #[test]
    fn test_empty_for_body_rejected() {
        let parser = Parser::new("for i in 1 2 3; do\ndone");
        assert!(parser.parse().is_err(), "empty for body should be rejected");
    }

    #[test]
    fn test_empty_if_then_rejected() {
        let parser = Parser::new("if true; then\nfi");
        assert!(
            parser.parse().is_err(),
            "empty then clause should be rejected"
        );
    }

    #[test]
    fn test_empty_else_rejected() {
        let parser = Parser::new("if false; then echo yes; else\nfi");
        assert!(
            parser.parse().is_err(),
            "empty else clause should be rejected"
        );
    }

    #[test]
    fn test_unterminated_single_quote_rejected() {
        let parser = Parser::new("echo 'unterminated");
        assert!(
            parser.parse().is_err(),
            "unterminated single quote should be rejected"
        );
    }

    #[test]
    fn test_unterminated_double_quote_rejected() {
        let parser = Parser::new("echo \"unterminated");
        assert!(
            parser.parse().is_err(),
            "unterminated double quote should be rejected"
        );
    }

    #[test]
    fn test_nonempty_function_body_accepted() {
        let parser = Parser::new("f() { echo hi; }");
        assert!(
            parser.parse().is_ok(),
            "non-empty function body should be accepted"
        );
    }

    #[test]
    fn test_nonempty_while_body_accepted() {
        let parser = Parser::new("while true; do echo hi; done");
        assert!(
            parser.parse().is_ok(),
            "non-empty while body should be accepted"
        );
    }

    /// Issue #600: Subscript reader must handle nested ${...} containing brackets.
    #[test]
    fn test_nested_expansion_in_array_subscript() {
        // ${arr[$RANDOM % ${#arr[@]}]} must parse without error.
        // The subscript contains ${#arr[@]} which has its own [ and ].
        let parser = Parser::new("echo ${arr[$RANDOM % ${#arr[@]}]}");
        let script = parser.parse().unwrap();
        assert_eq!(script.commands.len(), 1);
        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.to_string(), "echo");
            assert_eq!(cmd.args.len(), 1);
            // The arg should contain an ArrayAccess with the full nested index
            let arg = &cmd.args[0];
            let has_array_access = arg.parts.iter().any(|p| {
                matches!(
                    p,
                    WordPart::ArrayAccess { name, index }
                    if name == "arr" && index.contains("${#arr[@]}")
                )
            });
            assert!(
                has_array_access,
                "expected ArrayAccess with nested index, got: {:?}",
                arg.parts
            );
        } else {
            panic!("expected simple command");
        }
    }

    /// Assignment with nested subscript must parse (previously caused fuel exhaustion).
    #[test]
    fn test_assignment_nested_subscript_parses() {
        let parser = Parser::new("x=${arr[$RANDOM % ${#arr[@]}]}");
        assert!(
            parser.parse().is_ok(),
            "assignment with nested subscript should parse"
        );
    }

    #[test]
    fn test_leaf_spans_track_words_assignments_and_redirects() {
        let script = Parser::new("foo=bar echo hi > out\n").parse().unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        assert_eq!(command.assignments[0].span.start.line, 1);
        assert_eq!(command.assignments[0].span.start.column, 1);
        assert_eq!(command.name.span.start.column, 9);
        assert_eq!(command.args[0].span.start.column, 14);
        assert_eq!(command.redirects[0].span.start.column, 17);
        assert_eq!(command.redirects[0].target.span.start.column, 19);
    }

    #[test]
    fn test_word_part_spans_track_mixed_expansions() {
        let input = "echo pre${name:-fallback}$(printf hi)$((1+2))post\n";
        let script = Parser::new(input).parse().unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let slices: Vec<&str> = word
            .part_spans
            .iter()
            .map(|span| &input[span.start.offset..span.end.offset])
            .collect();

        assert_eq!(
            slices,
            vec![
                "pre",
                "${name:-fallback}",
                "$(printf hi)",
                "$((1+2))",
                "post"
            ]
        );
    }

    #[test]
    fn test_word_part_spans_track_quoted_expansions() {
        let input = "echo \"x$HOME$(pwd)y\"\n";
        let script = Parser::new(input).parse().unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let slices: Vec<&str> = word
            .part_spans
            .iter()
            .map(|span| &input[span.start.offset..span.end.offset])
            .collect();

        assert_eq!(slices, vec!["x", "$HOME", "$(pwd)", "y"]);
    }

    #[test]
    fn test_word_part_spans_track_nested_array_expansions() {
        let input = "echo ${arr[$RANDOM % ${#arr[@]}]}\n";
        let script = Parser::new(input).parse().unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.part_spans.len(), 1);
        assert_eq!(
            &input[word.part_spans[0].start.offset..word.part_spans[0].end.offset],
            "${arr[$RANDOM % ${#arr[@]}]}"
        );
    }

    #[test]
    fn test_command_substitution_spans_are_absolute() {
        let script = Parser::new("out=$(\n  printf '%s\\n' $x\n)\n")
            .parse()
            .unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        let WordPart::CommandSubstitution(commands) = &word.parts[0] else {
            panic!("expected command substitution");
        };
        let Command::Simple(inner) = &commands[0] else {
            panic!("expected simple command in substitution");
        };

        assert_eq!(inner.name.span.start.line, 2);
        assert_eq!(inner.name.span.start.column, 3);
        assert_eq!(inner.args[0].span.start.line, 2);
        assert_eq!(inner.args[1].span.start.column, 17);
    }

    #[test]
    fn test_process_substitution_spans_are_absolute() {
        let script = Parser::new("cat <(\n  printf '%s\\n' $x\n)\n")
            .parse()
            .unwrap();

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let WordPart::ProcessSubstitution { commands, is_input } = &command.args[0].parts[0] else {
            panic!("expected process substitution");
        };
        assert!(*is_input);

        let Command::Simple(inner) = &commands[0] else {
            panic!("expected simple command in process substitution");
        };
        assert_eq!(inner.name.span.start.line, 2);
        assert_eq!(inner.name.span.start.column, 3);
        assert_eq!(inner.args[1].span.start.column, 17);
    }
}
