//! Parser module for shuck
//!
//! Implements a recursive descent parser for bash scripts.

// Parser uses chars().next().unwrap() after validating character presence.
// This is safe because we check bounds before accessing.
#![allow(clippy::unwrap_used)]

mod lexer;

use std::collections::{HashMap, HashSet, VecDeque};

pub use lexer::{HeredocRead, Lexer, SpannedToken};

use shuck_ast::{
    ArithmeticCommand, ArithmeticForCommand, Assignment, AssignmentValue, BreakCommand,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandList, Comment,
    CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand,
    ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp,
    ContinueCommand, CoprocCommand, DeclClause, DeclName, DeclOperand, ExitCommand, ForCommand,
    FunctionDef, IfCommand, ListOperator, LiteralText, Name, ParameterOp, Pipeline, Position,
    Redirect, RedirectKind, ReturnCommand, Script, SelectCommand, SimpleCommand, SourceText, Span,
    TextSize, TimeCommand, Token, UntilCommand, WhileCommand, Word, WordPart,
};

use crate::error::{Error, Result};

/// Default maximum AST depth (matches ExecutionLimits default)
const DEFAULT_MAX_AST_DEPTH: usize = 100;

/// Hard cap on AST depth to prevent stack overflow even if caller misconfigures limits.
/// Protects against deeply nested input attacks where
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

/// The result of a successful parse: a script plus collected comments.
#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub script: Script,
    pub comments: Vec<Comment>,
}

/// Parser for bash scripts.
pub struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    synthetic_tokens: VecDeque<SpannedToken>,
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
    /// Comments collected during parsing.
    comments: Vec<Comment>,
    /// Known aliases declared earlier in the current parse stream.
    aliases: HashMap<String, AliasDefinition>,
    /// Whether alias expansion is currently enabled.
    expand_aliases: bool,
    /// Whether the next fetched word is eligible for alias expansion because
    /// the previous alias expansion ended with trailing whitespace.
    expand_next_word: bool,
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
    pub comments: Vec<Comment>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

#[derive(Debug, Clone)]
struct AliasDefinition {
    value: String,
    expands_next_word: bool,
}

#[derive(Debug, Clone, Copy)]
enum FlowControlBuiltinKind {
    Break,
    Continue,
    Return,
    Exit,
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
    /// `max_depth` is clamped to `HARD_MAX_AST_DEPTH` (500)
    /// to prevent stack overflow from misconfiguration. Even if the caller passes
    /// `max_depth = 1_000_000`, the parser will cap it at 500.
    pub fn with_limits(input: &'a str, max_depth: usize, max_fuel: usize) -> Self {
        let mut lexer = Lexer::with_max_subst_depth(input, max_depth.min(HARD_MAX_AST_DEPTH));
        let mut comments = Vec::new();
        let (current_token, current_span) = loop {
            match lexer.next_spanned_token_with_comments() {
                Some(st) if matches!(st.token, Token::Comment(_)) => {
                    comments.push(Comment {
                        range: st.span.to_range(),
                    });
                }
                Some(st) => break (Some(st.token), st.span),
                None => break (None, Span::new()),
            }
        };
        Self {
            input,
            lexer,
            synthetic_tokens: VecDeque::new(),
            current_token,
            current_span,
            peeked_token: None,
            max_depth: max_depth.min(HARD_MAX_AST_DEPTH),
            current_depth: 0,
            fuel: max_fuel,
            max_fuel,
            comments,
            aliases: HashMap::new(),
            expand_aliases: false,
            expand_next_word: false,
        }
    }

    /// Get the current token's span.
    pub fn current_span(&self) -> Span {
        self.current_span
    }

    /// Parse a string as a word (handling $var, $((expr)), ${...}, etc.).
    /// Used by the interpreter to expand operands in parameter expansions lazily.
    pub fn parse_word_string(input: &str) -> Word {
        let mut parser = Parser::new(input);
        let start = Position::new();
        parser.parse_word_with_context(
            input,
            Span::from_positions(start, start.advanced_by(input)),
            start,
        )
    }

    /// Parse a word string with caller-configured limits.
    /// Prevents bypass of parser limits in parameter expansion contexts.
    pub fn parse_word_string_with_limits(input: &str, max_depth: usize, max_fuel: usize) -> Word {
        let mut parser = Parser::with_limits(input, max_depth, max_fuel);
        let start = Position::new();
        parser.parse_word_with_context(
            input,
            Span::from_positions(start, start.advanced_by(input)),
            start,
        )
    }

    fn word_from_token(&mut self, token: &Token, span: Span) -> Option<Word> {
        match token {
            Token::Word(w) => Some(self.parse_word_with_context(w, span, span.start)),
            Token::QuotedWord(w) => {
                let mut word = self.parse_word_with_context(w, span, span.start.advanced_by("\""));
                word.quoted = true;
                Some(word)
            }
            Token::LiteralWord(w) => Some(Word::quoted_literal_with_span(w.clone(), span)),
            _ => None,
        }
    }

    fn current_word_to_word(&mut self) -> Option<Word> {
        let token = self.current_token.take()?;
        let word = self.word_from_token(&token, self.current_span);
        self.current_token = Some(token);
        word
    }

    fn current_name_token(&self) -> Option<(Name, Span)> {
        match &self.current_token {
            Some(Token::Word(w)) | Some(Token::LiteralWord(w)) | Some(Token::QuotedWord(w)) => {
                Some((Name::from(w.as_str()), self.current_span))
            }
            _ => None,
        }
    }

    fn nested_commands_from_source(&mut self, source: &str, base: Position) -> Vec<Command> {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let inner_parser = Parser::with_limits(source, remaining_depth, self.fuel);
        match inner_parser.parse() {
            Ok(mut output) => {
                let base_offset = TextSize::new(base.offset as u32);
                for comment in &mut output.comments {
                    comment.range = comment.range.offset_by(base_offset);
                }
                self.comments.extend(output.comments);
                Self::rebase_commands(&mut output.script.commands, base);
                output.script.commands
            }
            Err(_) => Vec::new(),
        }
    }

    fn nested_commands_from_current_input(
        &mut self,
        start: Position,
        end: Position,
    ) -> Vec<Command> {
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

    fn optional_span(start: Position, end: Position) -> Option<Span> {
        (start.offset < end.offset).then(|| Span::from_positions(start, end))
    }

    fn split_nested_arithmetic_close(&mut self, context: &'static str) -> Result<Span> {
        let right_paren_start = self.current_span.start.advanced_by(")");
        self.advance();

        match self.current_token {
            Some(Token::RightParen) => {
                let right_paren_span =
                    Span::from_positions(right_paren_start, self.current_span.end);
                self.advance();
                Ok(right_paren_span)
            }
            _ => Err(Error::parse(format!(
                "expected ')' after '))' in {context}"
            ))),
        }
    }

    fn split_double_semicolon(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by(";");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    fn record_arithmetic_for_separator(
        semicolon_span: Span,
        segment_start: &mut Position,
        init_span: &mut Option<Span>,
        first_semicolon_span: &mut Option<Span>,
        condition_span: &mut Option<Span>,
        second_semicolon_span: &mut Option<Span>,
    ) -> Result<()> {
        if first_semicolon_span.is_none() {
            *init_span = Self::optional_span(*segment_start, semicolon_span.start);
            *first_semicolon_span = Some(semicolon_span);
            *segment_start = semicolon_span.end;
            return Ok(());
        }

        if second_semicolon_span.is_none() {
            *condition_span = Self::optional_span(*segment_start, semicolon_span.start);
            *second_semicolon_span = Some(semicolon_span);
            *segment_start = semicolon_span.end;
            return Ok(());
        }

        Err(Error::parse(
            "unexpected ';' in arithmetic for header".to_string(),
        ))
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
            Command::Builtin(builtin) => {
                Self::rebase_builtin(builtin, base);
            }
            Command::Decl(decl) => {
                Self::rebase_decl(decl, base);
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
                function.name_span = function.name_span.rebased(base);
                Self::rebase_command(&mut function.body, base);
            }
        }
    }

    fn rebase_builtin(builtin: &mut BuiltinCommand, base: Position) {
        match builtin {
            BuiltinCommand::Break(command) => {
                command.span = command.span.rebased(base);
                if let Some(depth) = &mut command.depth {
                    Self::rebase_word(depth, base);
                }
                Self::rebase_words(&mut command.extra_args, base);
                Self::rebase_redirects(&mut command.redirects, base);
                Self::rebase_assignments(&mut command.assignments, base);
            }
            BuiltinCommand::Continue(command) => {
                command.span = command.span.rebased(base);
                if let Some(depth) = &mut command.depth {
                    Self::rebase_word(depth, base);
                }
                Self::rebase_words(&mut command.extra_args, base);
                Self::rebase_redirects(&mut command.redirects, base);
                Self::rebase_assignments(&mut command.assignments, base);
            }
            BuiltinCommand::Return(command) => {
                command.span = command.span.rebased(base);
                if let Some(code) = &mut command.code {
                    Self::rebase_word(code, base);
                }
                Self::rebase_words(&mut command.extra_args, base);
                Self::rebase_redirects(&mut command.redirects, base);
                Self::rebase_assignments(&mut command.assignments, base);
            }
            BuiltinCommand::Exit(command) => {
                command.span = command.span.rebased(base);
                if let Some(code) = &mut command.code {
                    Self::rebase_word(code, base);
                }
                Self::rebase_words(&mut command.extra_args, base);
                Self::rebase_redirects(&mut command.redirects, base);
                Self::rebase_assignments(&mut command.assignments, base);
            }
        }
    }

    fn rebase_decl(decl: &mut DeclClause, base: Position) {
        decl.span = decl.span.rebased(base);
        decl.variant_span = decl.variant_span.rebased(base);
        Self::rebase_redirects(&mut decl.redirects, base);
        Self::rebase_assignments(&mut decl.assignments, base);
        for operand in &mut decl.operands {
            match operand {
                DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                    Self::rebase_word(word, base);
                }
                DeclOperand::Name(name) => {
                    name.span = name.span.rebased(base);
                    name.name_span = name.name_span.rebased(base);
                    if let Some(index) = &mut name.index {
                        index.rebased(base);
                    }
                }
                DeclOperand::Assignment(assignment) => {
                    Self::rebase_assignments(std::slice::from_mut(assignment), base);
                }
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
                command.variable_span = command.variable_span.rebased(base);
                if let Some(words) = &mut command.words {
                    Self::rebase_words(words, base);
                }
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::ArithmeticFor(command) => {
                command.span = command.span.rebased(base);
                command.left_paren_span = command.left_paren_span.rebased(base);
                command.init_span = command.init_span.map(|span| span.rebased(base));
                command.first_semicolon_span = command.first_semicolon_span.rebased(base);
                command.condition_span = command.condition_span.map(|span| span.rebased(base));
                command.second_semicolon_span = command.second_semicolon_span.rebased(base);
                command.step_span = command.step_span.map(|span| span.rebased(base));
                command.right_paren_span = command.right_paren_span.rebased(base);
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
                command.variable_span = command.variable_span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                Self::rebase_commands(&mut command.body, base);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::rebase_commands(commands, base);
            }
            CompoundCommand::Arithmetic(command) => {
                command.span = command.span.rebased(base);
                command.left_paren_span = command.left_paren_span.rebased(base);
                command.expr_span = command.expr_span.map(|span| span.rebased(base));
                command.right_paren_span = command.right_paren_span.rebased(base);
            }
            CompoundCommand::Time(command) => {
                command.span = command.span.rebased(base);
                if let Some(inner) = &mut command.command {
                    Self::rebase_command(inner, base);
                }
            }
            CompoundCommand::Conditional(command) => {
                command.span = command.span.rebased(base);
                command.left_bracket_span = command.left_bracket_span.rebased(base);
                command.right_bracket_span = command.right_bracket_span.rebased(base);
                Self::rebase_conditional_expr(&mut command.expression, base);
            }
            CompoundCommand::Coproc(command) => {
                command.span = command.span.rebased(base);
                command.name_span = command.name_span.map(|span| span.rebased(base));
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
                WordPart::ParameterExpansion { operand, .. } => {
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                }
                WordPart::ArrayAccess { index, .. } => index.rebased(base),
                WordPart::Substring { offset, length, .. }
                | WordPart::ArraySlice { offset, length, .. } => {
                    offset.rebased(base);
                    if let Some(length) = length {
                        length.rebased(base);
                    }
                }
                WordPart::IndirectExpansion { operand, .. } => {
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                }
                WordPart::ArithmeticExpansion(expression) => expression.rebased(base),
                WordPart::Literal(_)
                | WordPart::Variable(_)
                | WordPart::Length(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::PrefixMatch(_)
                | WordPart::Transformation { .. } => {}
                WordPart::CommandSubstitution(commands)
                | WordPart::ProcessSubstitution { commands, .. } => {
                    Self::rebase_commands(commands, base);
                }
            }
        }
    }

    fn rebase_conditional_expr(expr: &mut ConditionalExpr, base: Position) {
        match expr {
            ConditionalExpr::Binary(binary) => {
                binary.op_span = binary.op_span.rebased(base);
                Self::rebase_conditional_expr(&mut binary.left, base);
                Self::rebase_conditional_expr(&mut binary.right, base);
            }
            ConditionalExpr::Unary(unary) => {
                unary.op_span = unary.op_span.rebased(base);
                Self::rebase_conditional_expr(&mut unary.expr, base);
            }
            ConditionalExpr::Parenthesized(paren) => {
                paren.left_paren_span = paren.left_paren_span.rebased(base);
                paren.right_paren_span = paren.right_paren_span.rebased(base);
                Self::rebase_conditional_expr(&mut paren.expr, base);
            }
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => {
                Self::rebase_word(word, base);
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
        &self,
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
                WordPart::Literal(self.literal_text(std::mem::take(current), current_start, end)),
                current_start,
                end,
            );
        }
    }

    fn literal_text(&self, text: String, start: Position, end: Position) -> LiteralText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, &text) {
            LiteralText::source()
        } else {
            LiteralText::owned(text)
        }
    }

    fn source_text(&self, text: String, start: Position, end: Position) -> SourceText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, &text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text)
        }
    }

    fn empty_source_text(&self, pos: Position) -> SourceText {
        SourceText::source(Span::from_positions(pos, pos))
    }

    fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == text
    }

    fn single_literal_word_text<'b>(&'b self, word: &'b Word) -> Option<&'b str> {
        if word.quoted || word.parts.len() != 1 {
            return None;
        }
        let WordPart::Literal(text) = &word.parts[0] else {
            return None;
        };
        Some(text.as_str(self.input, word.part_span(0)?))
    }

    fn literal_word_text(&self, word: &Word) -> Option<String> {
        let mut text = String::new();

        for (part, span) in word.parts_with_spans() {
            let WordPart::Literal(literal) = part else {
                return None;
            };
            text.push_str(literal.as_str(self.input, span));
        }

        Some(text)
    }

    fn next_spanned_token_with_comments(&mut self) -> Option<SpannedToken> {
        self.synthetic_tokens
            .pop_front()
            .or_else(|| self.lexer.next_spanned_token_with_comments())
    }

    fn queue_synthetic_tokens(&mut self, source: &str, base: Position) {
        let mut lexer = Lexer::with_max_subst_depth(source, self.max_depth);
        let mut queued = Vec::new();

        while let Some(token) = lexer.next_spanned_token_with_comments() {
            queued.push(SpannedToken {
                token: token.token,
                span: token.span.rebased(base),
            });
        }

        for token in queued.into_iter().rev() {
            self.synthetic_tokens.push_front(token);
        }
    }

    fn maybe_expand_current_alias_chain(&mut self) {
        if !self.expand_aliases {
            self.expand_next_word = false;
            return;
        }

        let mut seen = HashSet::new();
        let mut expands_next_word = false;

        loop {
            let Some(Token::Word(name)) = &self.current_token else {
                break;
            };
            let Some(alias) = self.aliases.get(name).cloned() else {
                break;
            };
            if !seen.insert(name.clone()) {
                break;
            }

            expands_next_word = alias.expands_next_word;
            self.peeked_token = None;
            self.queue_synthetic_tokens(&alias.value, self.current_span.start);
            self.advance_raw();
        }

        self.expand_next_word = expands_next_word;
    }

    fn apply_simple_command_effects(&mut self, command: &SimpleCommand) {
        let Some(name) = self.literal_word_text(&command.name) else {
            return;
        };

        match name.as_str() {
            "shopt" => {
                let mut toggle = None;
                for arg in &command.args {
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
                for arg in &command.args {
                    let Some(arg) = self.literal_word_text(arg) else {
                        continue;
                    };
                    if arg == "--" {
                        continue;
                    }
                    let Some((alias_name, value)) = arg.split_once('=') else {
                        continue;
                    };
                    self.aliases.insert(
                        alias_name.to_string(),
                        AliasDefinition {
                            value: value.to_string(),
                            expands_next_word: value
                                .chars()
                                .last()
                                .is_some_and(char::is_whitespace),
                        },
                    );
                }
            }
            "unalias" => {
                for arg in &command.args {
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

    fn apply_command_effects(&mut self, command: &Command) {
        match command {
            Command::Simple(simple) => self.apply_simple_command_effects(simple),
            Command::List(list) => {
                self.apply_command_effects(&list.first);
                for (_, command) in &list.rest {
                    self.apply_command_effects(command);
                }
            }
            _ => {}
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

    fn read_source_text_while<F>(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        predicate: F,
    ) -> SourceText
    where
        F: FnMut(char) -> bool,
    {
        let start = *cursor;
        let text = Self::read_word_while(chars, cursor, predicate);
        self.source_text(text, start, *cursor)
    }

    fn rebase_redirects(redirects: &mut [Redirect], base: Position) {
        for redirect in redirects {
            redirect.span = redirect.span.rebased(base);
            redirect.fd_var_span = redirect.fd_var_span.map(|span| span.rebased(base));
            Self::rebase_word(&mut redirect.target, base);
        }
    }

    fn rebase_assignments(assignments: &mut [Assignment], base: Position) {
        for assignment in assignments {
            assignment.span = assignment.span.rebased(base);
            assignment.name_span = assignment.name_span.rebased(base);
            if let Some(index) = &mut assignment.index {
                index.rebased(base);
            }
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

    /// Parse the input and return the AST with collected comments.
    pub fn parse(mut self) -> Result<ParseOutput> {
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
            let command = self.parse_command_list_required()?;
            self.apply_command_effects(&command);
            commands.push(command);
        }

        let end_span = self.current_span;
        Ok(ParseOutput {
            script: Script {
                commands,
                span: start_span.merge(end_span),
            },
            comments: self.comments,
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
                Ok(command) => {
                    self.apply_command_effects(&command);
                    commands.push(command);
                }
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
            comments: self.comments,
            diagnostics,
        }
    }

    fn advance_raw(&mut self) {
        if let Some(peeked) = self.peeked_token.take() {
            self.current_token = Some(peeked.token);
            self.current_span = peeked.span;
        } else {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if matches!(st.token, Token::Comment(_)) => {
                        self.comments.push(Comment {
                            range: st.span.to_range(),
                        });
                    }
                    Some(st) => {
                        self.current_token = Some(st.token);
                        self.current_span = st.span;
                        break;
                    }
                    None => {
                        self.current_token = None;
                        // Keep the last span for error reporting
                        break;
                    }
                }
            }
        }
    }

    fn advance(&mut self) {
        let should_expand = std::mem::take(&mut self.expand_next_word);
        self.advance_raw();
        if should_expand {
            self.maybe_expand_current_alias_chain();
        }
    }

    /// Peek at the next token without consuming the current one
    fn peek_next(&mut self) -> Option<&Token> {
        if self.peeked_token.is_none() {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if matches!(st.token, Token::Comment(_)) => {
                        self.comments.push(Comment {
                            range: st.span.to_range(),
                        });
                    }
                    other => {
                        self.peeked_token = other;
                        break;
                    }
                }
            }
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                        fd_var_span: None,
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
                            fd_var_span: None,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
                            kind: RedirectKind::HereString,
                            span: Self::redirect_span(operator_span, &target),
                            target,
                        });
                    }
                }
                Some(Token::HereDoc) | Some(Token::HereDocStrip) => {
                    let operator_span = self.current_span;
                    let strip_tabs = matches!(self.current_token, Some(Token::HereDocStrip));
                    self.advance();
                    let (delimiter, quoted) = match &self.current_token {
                        Some(Token::Word(w)) => (w.clone(), false),
                        Some(Token::LiteralWord(w)) => (w.clone(), true),
                        Some(Token::QuotedWord(w)) => (w.clone(), true),
                        _ => break,
                    };
                    let heredoc = self.lexer.read_heredoc(&delimiter);
                    let content_span = heredoc.content_span;
                    let content = heredoc.content;
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
                    self.advance();
                    let target = if quoted {
                        Word::quoted_literal_with_span(content, content_span)
                    } else {
                        self.parse_word_with_context(&content, content_span, content_span.start)
                    };
                    let kind = if strip_tabs {
                        RedirectKind::HereDocStrip
                    } else {
                        RedirectKind::HereDoc
                    };
                    redirects.push(Redirect {
                        fd: None,
                        fd_var: None,
                        fd_var_span: None,
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
                operands: args
                    .into_iter()
                    .map(|word| self.classify_decl_operand(word))
                    .collect(),
                redirects,
                assignments,
                span,
            });
        }

        Command::Simple(command)
    }

    /// Parse a single command (simple or compound)
    fn parse_command(&mut self) -> Result<Option<Command>> {
        self.skip_newlines()?;
        self.check_error_token()?;
        self.maybe_expand_current_alias_chain();
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
                    if !word.contains('=') && matches!(self.peek_next(), Some(Token::LeftParen)) {
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
        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
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
            variable_span,
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
        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
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
                Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_)) => {
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
            variable_span,
            words,
            body,
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse C-style arithmetic for loop inner: for ((init; cond; step)); do body; done
    /// Note: depth tracking is done by parse_for which calls this
    fn parse_arithmetic_for_inner(&mut self, start_span: Span) -> Result<CompoundCommand> {
        let left_paren_span = self.current_span;
        self.advance(); // consume '(('

        let mut paren_depth = 0_i32;
        let mut segment_start = left_paren_span.end;
        let mut init_span = None;
        let mut first_semicolon_span = None;
        let mut condition_span = None;
        let mut second_semicolon_span = None;

        let right_paren_span = loop {
            match &self.current_token {
                Some(Token::DoubleLeftParen) | Some(Token::LeftParen) => {
                    paren_depth += 1;
                    self.advance();
                }
                Some(Token::DoubleRightParen) => {
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
                Some(Token::RightParen) => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    self.advance();
                }
                Some(Token::DoubleSemicolon) if paren_depth == 0 => {
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
                Some(Token::Semicolon) if paren_depth == 0 => {
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
        if !self.is_keyword("done") {
            return Err(self.error("expected 'done'"));
        }
        let done_span = self.current_span;
        self.advance();

        Ok(CompoundCommand::ArithmeticFor(ArithmeticForCommand {
            left_paren_span,
            init_span,
            first_semicolon_span,
            condition_span,
            second_semicolon_span,
            step_span,
            right_paren_span,
            body,
            span: start_span.merge(done_span),
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
                Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_))
            ) {
                let w = match &self.current_token {
                    Some(Token::Word(w))
                    | Some(Token::LiteralWord(w))
                    | Some(Token::QuotedWord(w)) => w.clone(),
                    _ => unreachable!(),
                };
                patterns.push(self.parse_word(&w));
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
        let (name, name_span) = if let Some(Token::Word(w)) = &self.current_token {
            let word = w.clone();
            let word_span = self.current_span;
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
            self.current_token,
            Some(Token::DoubleSemicolon) | Some(Token::SemiAmp) | Some(Token::DoubleSemiAmp)
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
        let left_bracket_span = self.current_span;
        self.advance(); // consume '[['
        self.skip_conditional_newlines();

        let expression = self.parse_conditional_or(false)?;
        self.skip_conditional_newlines();

        let right_bracket_span = match self.current_token {
            Some(Token::DoubleRightBracket) => {
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
        while matches!(self.current_token, Some(Token::Newline)) {
            self.advance();
        }
    }

    fn parse_conditional_or(&mut self, stop_at_right_paren: bool) -> Result<ConditionalExpr> {
        let mut expr = self.parse_conditional_and(stop_at_right_paren)?;

        loop {
            self.skip_conditional_newlines();
            if !matches!(self.current_token, Some(Token::Or)) {
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
            if !matches!(self.current_token, Some(Token::And)) {
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
                ConditionalExpr::Word(self.parse_conditional_operand_word()?)
            };

            return Ok(ConditionalExpr::Unary(ConditionalUnaryExpr {
                op,
                op_span,
                expr: Box::new(expr),
            }));
        }

        let left = if matches!(self.current_token, Some(Token::LeftParen)) {
            let left_paren_span = self.current_span;
            self.advance();
            let expr = self.parse_conditional_or(true)?;
            self.skip_conditional_newlines();
            if !matches!(self.current_token, Some(Token::RightParen)) {
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
                ConditionalExpr::Regex(self.collect_conditional_context_word(stop_at_right_paren)?)
            }
            ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe => ConditionalExpr::Pattern(
                self.collect_conditional_context_word(stop_at_right_paren)?,
            ),
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

        let Some(word) = self.current_word_to_word() else {
            return Err(self.error("expected conditional operand"));
        };
        self.advance();
        Ok(word)
    }

    fn current_conditional_unary_op(&self) -> Option<ConditionalUnaryOp> {
        let Token::Word(word) = self.current_token.as_ref()? else {
            return None;
        };

        Some(match word.as_str() {
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
        match self.current_token.as_ref()? {
            Token::Word(word) => Some(match word.as_str() {
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
            Token::RedirectIn => Some(ConditionalBinaryOp::LexicalBefore),
            Token::RedirectOut => Some(ConditionalBinaryOp::LexicalAfter),
            _ => None,
        }
    }

    fn collect_conditional_context_word(&mut self, stop_at_right_paren: bool) -> Result<Word> {
        self.skip_conditional_newlines();

        let mut first_word: Option<Word> = None;
        let mut parts = Vec::new();
        let mut part_spans = Vec::new();
        let mut start = None;
        let mut end = None;
        let mut previous_end: Option<Position> = None;
        let mut composite = false;
        let mut paren_depth = 0usize;

        loop {
            self.skip_conditional_newlines();

            match self.current_token.as_ref() {
                Some(Token::DoubleRightBracket) => break,
                Some(Token::And) | Some(Token::Or) if paren_depth == 0 => break,
                Some(Token::RightParen) if stop_at_right_paren && paren_depth == 0 => break,
                None => break,
                _ => {}
            }

            if let Some(prev_end) = previous_end
                && prev_end.offset < self.current_span.start.offset
            {
                let gap_span = Span::from_positions(prev_end, self.current_span.start);
                let gap = self.input[prev_end.offset..self.current_span.start.offset].to_string();
                if !gap.is_empty() {
                    parts.push(WordPart::Literal(self.literal_text(
                        gap,
                        gap_span.start,
                        gap_span.end,
                    )));
                    part_spans.push(gap_span);
                    composite = true;
                }
            }

            match self.current_token.as_ref() {
                Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_)) => {
                    let word = self
                        .current_word_to_word()
                        .ok_or_else(|| self.error("expected conditional operand"))?;
                    if start.is_none() {
                        start = Some(word.span.start);
                    } else {
                        composite = true;
                    }
                    end = Some(word.span.end);
                    if first_word.is_none() && !composite {
                        first_word = Some(word.clone());
                    }
                    parts.extend(word.parts.clone());
                    part_spans.extend(word.part_spans.clone());
                    previous_end = Some(self.current_span.end);
                    self.advance();
                }
                Some(Token::LeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("(")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    paren_depth += 1;
                    composite = true;
                    self.advance();
                }
                Some(Token::DoubleLeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("((")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    paren_depth += 2;
                    composite = true;
                    self.advance();
                }
                Some(Token::RightParen) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned(")")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(1);
                    composite = true;
                    self.advance();
                }
                Some(Token::DoubleRightParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("))")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    paren_depth = paren_depth.saturating_sub(2);
                    composite = true;
                    self.advance();
                }
                Some(Token::Pipe) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("|")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(Token::And) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("&&")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(Token::Or) => {
                    if paren_depth == 0 {
                        break;
                    }
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(LiteralText::owned("||")));
                    part_spans.push(self.current_span);
                    previous_end = Some(self.current_span.end);
                    composite = true;
                    self.advance();
                }
                Some(Token::RedirectIn) | Some(Token::RedirectOut) => {
                    let literal = self.input
                        [self.current_span.start.offset..self.current_span.end.offset]
                        .to_string();
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
                    parts.push(WordPart::Literal(self.literal_text(
                        literal,
                        self.current_span.start,
                        self.current_span.end,
                    )));
                    part_spans.push(self.current_span);
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
                    parts.push(WordPart::Literal(self.literal_text(
                        literal,
                        self.current_span.start,
                        self.current_span.end,
                    )));
                    part_spans.push(self.current_span);
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

        Ok(Word {
            parts,
            part_spans,
            quoted: false,
            span: Span::from_positions(start, end),
        })
    }

    fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        let left_paren_span = self.current_span;
        self.advance(); // consume '(('

        let mut depth = 0_i32;
        let right_paren_span = loop {
            match &self.current_token {
                Some(Token::DoubleLeftParen) | Some(Token::LeftParen) => {
                    depth += 1;
                    self.advance();
                }
                Some(Token::DoubleRightParen) => {
                    if depth == 0 {
                        let right_paren_span = self.current_span;
                        self.advance();
                        break right_paren_span;
                    }
                    if depth == 1 {
                        break self.split_nested_arithmetic_close("arithmetic command")?;
                    }
                    depth -= 2;
                    self.advance();
                }
                Some(Token::RightParen) => {
                    if depth > 0 {
                        depth -= 1;
                    }
                    self.advance();
                }
                Some(_) => {
                    self.advance();
                }
                None => {
                    return Err(Error::parse(
                        "unexpected end of input in arithmetic command".to_string(),
                    ));
                }
            }
        };

        Ok(CompoundCommand::Arithmetic(ArithmeticCommand {
            span: left_paren_span.merge(right_paren_span),
            left_paren_span,
            expr_span: Self::optional_span(left_paren_span.end, right_paren_span.start),
            right_paren_span,
        }))
    }

    /// Parse function definition with 'function' keyword: function name { body }
    fn parse_function_keyword(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        self.advance(); // consume 'function'
        self.skip_newlines()?;

        // Get function name
        let (name, name_span) = match &self.current_token {
            Some(Token::Word(w)) => (Name::from(w), self.current_span),
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
            name_span,
            body: Box::new(Command::Compound(body, Vec::new())),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse POSIX-style function definition: name() { body }
    fn parse_function_posix(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        // Get function name
        let (name, name_span) = match &self.current_token {
            Some(Token::Word(w)) => (Name::from(w), self.current_span),
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
            name_span,
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

            let command = self.parse_command_list_required()?;
            self.apply_command_effects(&command);
            commands.push(command);
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
                Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_)) => {
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

    fn parse_array_words_from_text(&mut self, inner: &str, base: Position) -> Vec<Word> {
        let mut lexer =
            Lexer::with_max_subst_depth(inner, self.max_depth.saturating_sub(self.current_depth));
        let mut elements = Vec::new();

        while let Some(spanned) = lexer.next_spanned_token() {
            match &spanned.token {
                Token::Word(_) | Token::LiteralWord(_) | Token::QuotedWord(_) => {
                    let span = spanned.span.rebased(base);
                    if let Some(word) = self.word_from_token(&spanned.token, span) {
                        elements.push(word);
                    }
                }
                _ => {}
            }
        }

        elements
    }

    fn parse_assignment_from_text(&mut self, w: &str, assignment_span: Span) -> Option<Assignment> {
        let (name, index, value, is_append) = Self::is_assignment(w)?;
        let name_span = Span::from_positions(
            assignment_span.start,
            assignment_span.start.advanced_by(name),
        );
        let index_span = index.map(|index| {
            let start = name_span.end.advanced_by("[");
            Span::from_positions(start, start.advanced_by(index))
        });
        let value_start_offset = if let Some(pos) = w.find("+=") {
            pos + 2
        } else {
            w.find('=')? + 1
        };
        let value_start = assignment_span.start.advanced_by(&w[..value_start_offset]);
        let value_span = Span::from_positions(value_start, assignment_span.end);
        let name = Name::from(name);
        let index = index
            .zip(index_span)
            .map(|(index, span)| self.source_text(index.to_string(), span.start, span.end));
        let value_str = value.to_string();

        let value = if value_str.starts_with('(') && value_str.ends_with(')') {
            let inner = &value_str[1..value_str.len() - 1];
            AssignmentValue::Array(
                self.parse_array_words_from_text(inner, value_start.advanced_by("(")),
            )
        } else if value_str.is_empty() {
            AssignmentValue::Scalar(Word::literal_with_span("", value_span))
        } else if value_str.starts_with('"') && value_str.ends_with('"') {
            let inner = Self::strip_quotes(&value_str);
            let mut word =
                self.parse_word_with_context(inner, value_span, value_start.advanced_by("\""));
            word.quoted = true;
            AssignmentValue::Scalar(word)
        } else if value_str.starts_with('\'') && value_str.ends_with('\'') {
            let inner = Self::strip_quotes(&value_str);
            AssignmentValue::Scalar(Word::quoted_literal_with_span(
                inner.to_string(),
                value_span,
            ))
        } else {
            AssignmentValue::Scalar(self.parse_word_with_context(
                &value_str,
                value_span,
                value_start,
            ))
        };

        Some(Assignment {
            name,
            name_span,
            index,
            value,
            append: is_append,
            span: assignment_span,
        })
    }

    fn parse_decl_name_from_text(word: &str, span: Span) -> Option<DeclName> {
        if let Some(bracket_pos) = word.find('[') {
            let name = &word[..bracket_pos];
            if !Self::is_valid_identifier(name) || !word.ends_with(']') {
                return None;
            }

            let index = &word[bracket_pos + 1..word.len() - 1];
            let name_span = Span::from_positions(span.start, span.start.advanced_by(name));
            let index_start = name_span.end.advanced_by("[");
            let index_span = Span::from_positions(index_start, index_start.advanced_by(index));

            return Some(DeclName {
                name: Name::from(name),
                name_span,
                index: Some(index_span.into()),
                span,
            });
        }

        if !Self::is_valid_identifier(word) {
            return None;
        }

        Some(DeclName {
            name: Name::from(word),
            name_span: span,
            index: None,
            span,
        })
    }

    fn is_valid_identifier(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }

        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }

        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    fn word_source_text(&self, word: &Word) -> String {
        if word.span.start.offset <= word.span.end.offset
            && word.span.end.offset <= self.input.len()
        {
            return self.input[word.span.start.offset..word.span.end.offset].to_string();
        }
        word.to_string()
    }

    fn is_literal_flag_word(word: &Word, raw: &str) -> bool {
        if word.quoted || raw.contains('=') {
            return false;
        }

        let Some(first) = raw.chars().next() else {
            return false;
        };
        if first != '-' && first != '+' {
            return false;
        }

        matches!(
            word.parts.as_slice(),
            [WordPart::Literal(value)] if match value {
                LiteralText::Source => true,
                LiteralText::Owned(value) => value.as_ref() == raw,
            }
        )
    }

    fn classify_decl_operand(&mut self, word: Word) -> DeclOperand {
        let raw = self.word_source_text(&word);

        if Self::is_literal_flag_word(&word, &raw) {
            return DeclOperand::Flag(word);
        }

        if let Some(assignment) = self.parse_assignment_from_text(&raw, word.span) {
            return DeclOperand::Assignment(assignment);
        }

        if let Some(name) = Self::parse_decl_name_from_text(&raw, word.span) {
            return DeclOperand::Name(name);
        }

        DeclOperand::Dynamic(word)
    }

    /// Parse the value side of an assignment (`VAR=value`).
    /// Returns `Some((Assignment, needs_advance))` if the current word is an assignment.
    /// The bool indicates whether the caller must call `self.advance()` afterward.
    fn try_parse_assignment(&mut self, w: &str) -> Option<(Assignment, bool)> {
        let (_, _, value_str, _) = Self::is_assignment(w)?;

        // Empty value — check for arr=(...) syntax with separate tokens
        if value_str.is_empty() {
            let assignment_span = self.current_span;
            let (name, index, _, is_append) = Self::is_assignment(w)?;
            let name_span = Span::from_positions(
                assignment_span.start,
                assignment_span.start.advanced_by(name),
            );
            let index_span = index.map(|index| {
                let start = name_span.end.advanced_by("[");
                Span::from_positions(start, start.advanced_by(index))
            });
            self.advance();
            if matches!(self.current_token, Some(Token::LeftParen)) {
                let open_paren_span = self.current_span;
                self.advance(); // consume '('
                let (elements, close_span) = self.collect_array_elements();
                return Some((
                    Assignment {
                        name: Name::from(name),
                        name_span,
                        index: index.zip(index_span).map(|(index, span)| {
                            self.source_text(index.to_string(), span.start, span.end)
                        }),
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
            let value_start_offset = if let Some(pos) = w.find("+=") {
                pos + 2
            } else {
                w.find('=')? + 1
            };
            let value_span = Span::from_positions(
                assignment_span.start.advanced_by(&w[..value_start_offset]),
                assignment_span.end,
            );
            return Some((
                Assignment {
                    name: Name::from(name),
                    name_span,
                    index: index.zip(index_span).map(|(index, span)| {
                        self.source_text(index.to_string(), span.start, span.end)
                    }),
                    value: AssignmentValue::Scalar(Word::literal_with_span("", value_span)),
                    append: is_append,
                    span: assignment_span,
                },
                false,
            ));
        }

        self.parse_assignment_from_text(w, self.current_span)
            .map(|assignment| (assignment, true))
    }

    /// Parse a compound array argument in arg position (e.g. `declare -a arr=(x y z)`).
    /// Called when the current word ends with `=` and the next token is `(`.
    /// Returns the compound word if successful, or `None` if not a compound assignment.
    fn try_parse_compound_array_arg(&mut self, saved_w: String, saved_span: Span) -> Option<Word> {
        if !matches!(self.current_token, Some(Token::LeftParen)) {
            return None;
        }

        self.advance(); // consume '('
        let mut compound = saved_w;
        let mut closing_span = Span::new();
        loop {
            match &self.current_token {
                Some(Token::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(Token::Word(elem))
                | Some(Token::LiteralWord(elem))
                | Some(Token::QuotedWord(elem)) => {
                    compound.push(' ');
                    compound.push_str(elem);
                    self.advance();
                }
                None => break,
                _ => {
                    self.advance();
                }
            }
        }

        let span = if closing_span == Span::new() {
            saved_span
        } else {
            saved_span.merge(closing_span)
        };

        if saved_span.start.offset <= span.end.offset && span.end.offset <= self.input.len() {
            let source = &self.input[saved_span.start.offset..span.end.offset];
            return Some(self.parse_word_with_context(source, span, saved_span.start));
        }

        Some(self.parse_word_with_context(&compound, span, saved_span.start))
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

        let heredoc = self.lexer.read_heredoc(&delimiter);
        let content_span = heredoc.content_span;
        let content = heredoc.content;

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
            Word::quoted_literal_with_span(content, content_span)
        } else {
            self.parse_word_with_context(&content, content_span, content_span.start)
        };

        let kind = if strip_tabs {
            RedirectKind::HereDocStrip
        } else {
            RedirectKind::HereDoc
        };

        redirects.push(Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                            fd_var_span: None,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
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
                            fd_var_span: None,
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
    fn pop_fd_var(&self, words: &mut Vec<Word>) -> (Option<Name>, Option<Span>) {
        if let Some(last) = words.last()
            && last.parts.len() == 1
            && let WordPart::Literal(ref s) = last.parts[0]
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

    fn parse_simple_command(&mut self) -> Result<Option<SimpleCommand>> {
        self.tick()?;
        self.skip_newlines()?;
        self.check_error_token()?;
        let start_span = self.current_span;

        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirects = Vec::new();

        loop {
            self.check_error_token()?;
            match &self.current_token {
                Some(Token::Word(w)) | Some(Token::LiteralWord(w)) | Some(Token::QuotedWord(w)) => {
                    let is_literal = matches!(&self.current_token, Some(Token::LiteralWord(_)));
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
                        let original_word = self.current_word_to_word();
                        let saved_span = self.current_span;
                        self.advance();
                        if let Some(word) = self.try_parse_compound_array_arg(w.clone(), saved_span)
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
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectAppend) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::RedirectIn) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::HereString) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::HereString,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::HereDoc) | Some(Token::HereDocStrip) => {
                    let strip_tabs = matches!(self.current_token, Some(Token::HereDocStrip));
                    self.parse_heredoc_redirect(strip_tabs, &mut redirects)?;
                    break;
                }
                Some(Token::ProcessSubIn) | Some(Token::ProcessSubOut) => {
                    let word = self.expect_word()?;
                    words.push(word);
                }
                Some(Token::RedirectBoth) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::OutputBoth,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                Some(Token::DupOutput) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(1) },
                        fd_var,
                        fd_var_span,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
                        kind: RedirectKind::DupOutput,
                        span: operator_span,
                        target: Word::literal(dst_fd.to_string()),
                    });
                }
                Some(Token::DupInput) => {
                    let operator_span = self.current_span;
                    let (fd_var, fd_var_span) = self.pop_fd_var(&mut words);
                    self.advance();
                    let target = self.expect_word()?;
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(0) },
                        fd_var,
                        fd_var_span,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
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
                        fd_var_span: None,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target,
                    });
                }
                // { and } as arguments (not in command position) are literal words
                Some(Token::LeftBrace) | Some(Token::RightBrace) if !words.is_empty() => {
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
            Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_)) => {
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
            Some(Token::Word(_)) | Some(Token::LiteralWord(_)) | Some(Token::QuotedWord(_))
        )
    }

    #[allow(dead_code)]
    /// Get the string content if current token is a word
    fn current_word_str(&self) -> Option<&str> {
        match &self.current_token {
            Some(Token::Word(w)) | Some(Token::LiteralWord(w)) | Some(Token::QuotedWord(w)) => {
                Some(w.as_str())
            }
            _ => None,
        }
    }

    /// Parse a word string into a Word with proper parts (variables, literals)
    fn parse_word(&mut self, s: &str) -> Word {
        self.parse_word_with_context(s, Span::new(), Position::new())
    }

    fn parse_word_with_context(&mut self, s: &str, span: Span, base: Position) -> Word {
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

            self.flush_literal_part(
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
                    WordPart::Literal(self.literal_text(ansi, part_start, cursor)),
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
                    let expr_start = cursor;
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
                        WordPart::ArithmeticExpansion(self.source_text(
                            expr.clone(),
                            expr_start,
                            expr_start.advanced_by(&expr),
                        )),
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
                            WordPart::ArrayLength(var_name.into())
                        } else {
                            WordPart::Length(format!("{}[{}]", var_name, index).into())
                        };
                        Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    } else {
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::Length(var_name.into()),
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
                            WordPart::ArrayIndices(var_name.into())
                        } else {
                            WordPart::Variable(format!("!{}[{}]", var_name, index).into())
                        };
                        Self::push_word_part(&mut parts, &mut part_spans, part, part_start, cursor);
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        Self::push_word_part(
                            &mut parts,
                            &mut part_spans,
                            WordPart::IndirectExpansion {
                                name: var_name.into(),
                                operator: None,
                                operand: None,
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
                                    name: var_name.into(),
                                    operator: Some(operator),
                                    operand: Some(operand),
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
                                WordPart::Variable(format!("!{}{}", var_name, suffix).into()),
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
                                name: var_name.into(),
                                operator: Some(operator),
                                operand: Some(operand),
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
                            WordPart::PrefixMatch(
                                format!("{}{}", var_name, &suffix[..suffix.len() - 1]).into(),
                            )
                        } else {
                            WordPart::Variable(format!("!{}{}", var_name, suffix).into())
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
                    let index_start = cursor;
                    let mut index = String::new();
                    let mut index_end = cursor;
                    let mut bracket_depth: i32 = 0;
                    let mut brace_depth: i32 = 0;
                    while let Some(&c) = chars.peek() {
                        if c == ']' && bracket_depth == 0 && brace_depth == 0 {
                            index_end = cursor;
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
                        index_end = cursor;
                    }

                    if index.len() >= 2
                        && ((index.starts_with('"') && index.ends_with('"'))
                            || (index.starts_with('\'') && index.ends_with('\'')))
                    {
                        index = index[1..index.len() - 1].to_string();
                    }
                    let index = self.source_text(index, index_start, index_end);

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
                                let arr_name = format!("{}[{}]", var_name, index.slice(self.input));
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
                                    name: arr_name.into(),
                                    operator,
                                    operand: Some(operand),
                                    colon_variant: true,
                                }
                            } else {
                                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                let offset =
                                    self.read_source_text_while(&mut chars, &mut cursor, |c| {
                                        c != ':' && c != '}'
                                    });
                                let length =
                                    if Self::consume_word_char_if(&mut chars, &mut cursor, ':') {
                                        Some(self.read_source_text_while(
                                            &mut chars,
                                            &mut cursor,
                                            |c| c != '}',
                                        ))
                                    } else {
                                        None
                                    };
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::ArraySlice {
                                    name: var_name.into(),
                                    offset,
                                    length,
                                }
                            }
                        } else if matches!(next_c, '-' | '+' | '=' | '?') {
                            let arr_name = format!("{}[{}]", var_name, index.slice(self.input));
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
                                name: arr_name.into(),
                                operator,
                                operand: Some(operand),
                                colon_variant: false,
                            }
                        } else {
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ArrayAccess {
                                name: var_name.into(),
                                index,
                            }
                        }
                    } else {
                        WordPart::ArrayAccess {
                            name: var_name.into(),
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
                                        name: var_name.into(),
                                        operator,
                                        operand: Some(operand),
                                        colon_variant: true,
                                    }
                                }
                                _ => {
                                    let offset = self.read_source_text_while(
                                        &mut chars,
                                        &mut cursor,
                                        |ch| ch != ':' && ch != '}',
                                    );
                                    let length =
                                        if Self::consume_word_char_if(&mut chars, &mut cursor, ':')
                                        {
                                            Some(self.read_source_text_while(
                                                &mut chars,
                                                &mut cursor,
                                                |ch| ch != '}',
                                            ))
                                        } else {
                                            None
                                        };
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                    WordPart::Substring {
                                        name: var_name.into(),
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
                                name: var_name.into(),
                                operator,
                                operand: Some(operand),
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
                                name: var_name.into(),
                                operator,
                                operand: Some(operand),
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
                                name: var_name.into(),
                                operator,
                                operand: Some(operand),
                                colon_variant: false,
                            }
                        }
                        '/' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let replace_all =
                                Self::consume_word_char_if(&mut chars, &mut cursor, '/');
                            let pattern_start = cursor;
                            let mut pattern = String::new();
                            let mut pattern_end = cursor;
                            while let Some(&ch) = chars.peek() {
                                if ch == '/' || ch == '}' {
                                    pattern_end = cursor;
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
                                        pattern_end = cursor;
                                        continue;
                                    }
                                    pattern.push('\\');
                                    continue;
                                }
                                pattern.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
                                pattern_end = cursor;
                            }
                            let pattern = self.source_text(pattern, pattern_start, pattern_end);
                            let replacement =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '/') {
                                    self.read_source_text_while(&mut chars, &mut cursor, |ch| {
                                        ch != '}'
                                    })
                                } else {
                                    self.empty_source_text(cursor)
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
                                name: var_name.into(),
                                operator,
                                operand: None,
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
                                name: var_name.into(),
                                operator,
                                operand: None,
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
                                name: var_name.into(),
                                operator,
                                operand: None,
                                colon_variant: false,
                            }
                        }
                        '@' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            if chars.peek().is_some() {
                                let operator = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Transformation {
                                    name: var_name.into(),
                                    operator,
                                }
                            } else {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Variable(var_name.into())
                            }
                        }
                        '}' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            WordPart::Variable(var_name.into())
                        }
                        _ => {
                            while let Some(&next) = chars.peek() {
                                let consumed = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                if next == '}' || consumed == '}' {
                                    break;
                                }
                            }
                            WordPart::Variable(var_name.into())
                        }
                    }
                } else {
                    WordPart::Variable(var_name.into())
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
                        WordPart::Variable(name.into()),
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
                            WordPart::Variable(var_name.into()),
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

        self.flush_literal_part(
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
                WordPart::Literal(self.literal_text(String::new(), base, cursor)),
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
    ) -> SourceText {
        let start = *cursor;
        let mut operand = String::new();
        let mut depth = 1;
        while let Some(&c) = chars.peek() {
            if c == '{' {
                depth += 1;
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    let end = *cursor;
                    Self::next_word_char_unwrap(chars, cursor);
                    return self.source_text(operand, start, end);
                }
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            } else {
                operand.push(Self::next_word_char_unwrap(chars, cursor));
            }
        }
        self.source_text(operand, start, *cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_compound(command: &Command) -> (&CompoundCommand, &[Redirect]) {
        let Command::Compound(compound, redirects) = command else {
            panic!("expected compound command");
        };
        (compound, redirects.as_slice())
    }

    #[test]
    fn test_parse_simple_command() {
        let input = "echo hello";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        assert_eq!(script.commands.len(), 1);

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.render(input), "echo");
            assert_eq!(cmd.args.len(), 1);
            assert_eq!(cmd.args[0].render(input), "hello");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_break_as_typed_builtin() {
        let input = "break 2";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Builtin(BuiltinCommand::Break(command)) = &script.commands[0] else {
            panic!("expected break builtin");
        };

        assert_eq!(command.depth.as_ref().unwrap().render(input), "2");
        assert!(command.extra_args.is_empty());
    }

    #[test]
    fn test_parse_continue_preserves_extra_args() {
        let input = "continue 1 extra";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Builtin(BuiltinCommand::Continue(command)) = &script.commands[0] else {
            panic!("expected continue builtin");
        };

        assert_eq!(command.depth.as_ref().unwrap().render(input), "1");
        assert_eq!(command.extra_args.len(), 1);
        assert_eq!(command.extra_args[0].render(input), "extra");
    }

    #[test]
    fn test_parse_return_preserves_assignments_and_redirects() {
        let input = "FOO=bar return 42 > out.txt";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Builtin(BuiltinCommand::Return(command)) = &script.commands[0] else {
            panic!("expected return builtin");
        };

        assert_eq!(command.code.as_ref().unwrap().render(input), "42");
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].name, "FOO");
        assert_eq!(command.redirects.len(), 1);
        assert_eq!(command.redirects[0].target.render(input), "out.txt");
    }

    #[test]
    fn test_parse_exit_as_typed_builtin() {
        let input = "exit 1";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Builtin(BuiltinCommand::Exit(command)) = &script.commands[0] else {
            panic!("expected exit builtin");
        };

        assert_eq!(command.code.as_ref().unwrap().render(input), "1");
        assert!(command.extra_args.is_empty());
    }

    #[test]
    fn test_parse_quoted_flow_control_name_stays_simple_command() {
        let input = "'break' 2";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        assert!(command.name.quoted);
        assert_eq!(command.name.render(input), "break");
        assert_eq!(command.args[0].render(input), "2");
    }

    #[test]
    fn test_parse_multiple_args() {
        let input = "echo hello world";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.render(input), "echo");
            assert_eq!(cmd.args.len(), 2);
            assert_eq!(cmd.args[0].render(input), "hello");
            assert_eq!(cmd.args[1].render(input), "world");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_variable() {
        let parser = Parser::new("echo $HOME");
        let script = parser.parse().unwrap().script;

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
        let input = "echo one\ncat >\necho two\n";
        let recovered = Parser::new(input).parse_recovered();

        assert_eq!(recovered.script.commands.len(), 2);
        assert_eq!(recovered.diagnostics.len(), 1);
        assert_eq!(recovered.diagnostics[0].message, "expected word");
        assert_eq!(recovered.diagnostics[0].span.start.line, 2);

        let Command::Simple(first) = &recovered.script.commands[0] else {
            panic!("expected first command to be simple");
        };
        assert_eq!(first.name.render(input), "echo");
        assert_eq!(first.args[0].render(input), "one");

        let Command::Simple(second) = &recovered.script.commands[1] else {
            panic!("expected second command to be simple");
        };
        assert_eq!(second.name.render(input), "echo");
        assert_eq!(second.args[0].render(input), "two");
    }

    #[test]
    fn test_parse_pipeline() {
        let parser = Parser::new("echo hello | cat");
        let script = parser.parse().unwrap().script;

        assert_eq!(script.commands.len(), 1);
        assert!(matches!(&script.commands[0], Command::Pipeline(_)));

        if let Command::Pipeline(pipeline) = &script.commands[0] {
            assert_eq!(pipeline.commands.len(), 2);
        }
    }

    #[test]
    fn test_parse_redirect_out() {
        let input = "echo hello > /tmp/out";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.redirects.len(), 1);
            assert_eq!(cmd.redirects[0].kind, RedirectKind::Output);
            assert_eq!(cmd.redirects[0].target.render(input), "/tmp/out");
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_redirect_append() {
        let parser = Parser::new("echo hello >> /tmp/out");
        let script = parser.parse().unwrap().script;

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
        let script = parser.parse().unwrap().script;

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
        let script = parser.parse().unwrap().script;

        assert!(matches!(&script.commands[0], Command::List(_)));
    }

    #[test]
    fn test_parse_command_list_or() {
        let parser = Parser::new("false || echo fallback");
        let script = parser.parse().unwrap().script;

        assert!(matches!(&script.commands[0], Command::List(_)));
    }

    #[test]
    fn test_heredoc_pipe() {
        let parser = Parser::new("cat <<EOF | sort\nc\na\nb\nEOF\n");
        let script = parser.parse().unwrap().script;
        assert!(
            matches!(&script.commands[0], Command::Pipeline(_)),
            "heredoc with pipe should parse as Pipeline"
        );
    }

    #[test]
    fn test_heredoc_multiple_on_line() {
        let input = "while cat <<E1 && cat <<E2; do cat <<E3; break; done\n1\nE1\n2\nE2\n3\nE3\n";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;
        assert_eq!(script.commands.len(), 1);
        let (compound, _) = expect_compound(&script.commands[0]);
        if let CompoundCommand::While(w) = compound {
            assert!(
                !w.condition.is_empty(),
                "while condition should be non-empty"
            );
            assert!(!w.body.is_empty(), "while body should be non-empty");
        } else {
            panic!("expected While compound command");
        }
    }

    #[test]
    fn test_heredoc_target_preserves_body_span() {
        let input = "cat <<'EOF'\nhello $name\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.redirects.len(), 1);

        let redirect = &command.redirects[0];
        assert_eq!(redirect.target.span.slice(input), "hello $name\n");
        assert!(redirect.target.quoted);
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
        let input = "echo ${arr[$RANDOM % ${#arr[@]}]}";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;
        assert_eq!(script.commands.len(), 1);
        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.name.render(input), "echo");
            assert_eq!(cmd.args.len(), 1);
            // The arg should contain an ArrayAccess with the full nested index
            let arg = &cmd.args[0];
            let has_array_access = arg.parts.iter().any(|p| {
                matches!(
                    p,
                    WordPart::ArrayAccess { name, index }
                    if name == "arr" && index.slice(input).contains("${#arr[@]}")
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
        let script = Parser::new("foo=bar echo hi > out\n")
            .parse()
            .unwrap()
            .script;

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
        let script = Parser::new(input).parse().unwrap().script;

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
        let script = Parser::new(input).parse().unwrap().script;

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
        let script = Parser::new(input).parse().unwrap().script;

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
    fn test_word_part_spans_track_parenthesized_arithmetic_expansion() {
        let input = "echo $((a <= (1 || 2)))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.part_spans.len(), 1);
        assert_eq!(word.part_spans[0].slice(input), "$((a <= (1 || 2)))");

        let WordPart::ArithmeticExpansion(expression) = &word.parts[0] else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(expression.slice(input), "a <= (1 || 2)");
    }

    #[test]
    fn test_word_part_spans_track_nested_arithmetic_expansion() {
        let input = "echo $(((a) + ((b))))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.part_spans.len(), 1);
        assert_eq!(word.part_spans[0].slice(input), "$(((a) + ((b))))");

        let WordPart::ArithmeticExpansion(expression) = &word.parts[0] else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(expression.slice(input), "(a) + ((b))");
    }

    #[test]
    fn test_parse_arithmetic_command_preserves_exact_spans() {
        let input = "(( 1 +\n 2 <= 3 ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.right_paren_span.slice(input), "))");
        assert_eq!(command.expr_span.unwrap().slice(input), " 1 +\n 2 <= 3 ");
    }

    #[test]
    fn test_parse_arithmetic_command_with_nested_parens_and_double_right_paren() {
        let input = "(( (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.right_paren_span.slice(input), "))");
        assert_eq!(
            command.expr_span.unwrap().slice(input),
            " (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) "
        );
    }

    #[test]
    fn test_parse_arithmetic_command_with_command_substitution() {
        let input = "(($(date -u) > DATE))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.right_paren_span.slice(input), "))");
        assert_eq!(command.expr_span.unwrap().slice(input), "$(date -u) > DATE");
    }

    #[test]
    fn test_parse_arithmetic_command_with_nested_parens_before_outer_close() {
        let input = "(( a <= (1 || 2)))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.right_paren_span.slice(input), "))");
        assert_eq!(command.expr_span.unwrap().slice(input), " a <= (1 || 2)");
    }

    #[test]
    fn test_parse_arithmetic_for_preserves_header_spans() {
        let input = "for (( i = 0 ; i < 10 ; i += 2 )); do echo \"$i\"; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.init_span.unwrap().slice(input), " i = 0 ");
        assert_eq!(command.first_semicolon_span.slice(input), ";");
        assert_eq!(command.condition_span.unwrap().slice(input), " i < 10 ");
        assert_eq!(command.second_semicolon_span.slice(input), ";");
        assert_eq!(command.step_span.unwrap().slice(input), " i += 2 ");
        assert_eq!(command.right_paren_span.slice(input), "))");
    }

    #[test]
    fn test_parse_arithmetic_for_preserves_compact_header_spans() {
        let input = "for ((i=0;i<10;i++)) do echo \"$i\"; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.init_span.unwrap().slice(input), "i=0");
        assert_eq!(command.first_semicolon_span.slice(input), ";");
        assert_eq!(command.condition_span.unwrap().slice(input), "i<10");
        assert_eq!(command.second_semicolon_span.slice(input), ";");
        assert_eq!(command.step_span.unwrap().slice(input), "i++");
        assert_eq!(command.right_paren_span.slice(input), "))");
    }

    #[test]
    fn test_parse_arithmetic_for_allows_all_empty_segments() {
        let input = "for ((;;)); do foo; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert!(command.init_span.is_none());
        assert_eq!(command.first_semicolon_span.slice(input), ";");
        assert!(command.condition_span.is_none());
        assert_eq!(command.second_semicolon_span.slice(input), ";");
        assert!(command.step_span.is_none());
        assert_eq!(command.right_paren_span.slice(input), "))");
    }

    #[test]
    fn test_parse_arithmetic_for_allows_only_init_segment() {
        let input = "for ((i = 0;;)); do foo; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.init_span.unwrap().slice(input), "i = 0");
        assert_eq!(command.first_semicolon_span.slice(input), ";");
        assert!(command.condition_span.is_none());
        assert_eq!(command.second_semicolon_span.slice(input), ";");
        assert!(command.step_span.is_none());
        assert_eq!(command.right_paren_span.slice(input), "))");
    }

    #[test]
    fn test_parse_arithmetic_for_with_nested_parens_before_outer_close() {
        let input = "for (( i = 0 ; i < 10 ; i += ($# - 1))); do echo \"$i\"; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(command.init_span.unwrap().slice(input), " i = 0 ");
        assert_eq!(command.first_semicolon_span.slice(input), ";");
        assert_eq!(command.condition_span.unwrap().slice(input), " i < 10 ");
        assert_eq!(command.second_semicolon_span.slice(input), ";");
        assert_eq!(command.step_span.unwrap().slice(input), " i += ($# - 1)");
        assert_eq!(command.right_paren_span.slice(input), "))");
    }

    #[test]
    fn test_identifier_spans_track_function_loop_assignment_and_fd_var_names() {
        let input = "\
my_fn() { true; }
for item in a; do echo \"$item\"; done
select choice in a; do echo \"$choice\"; done
foo[10]=bar
exec {myfd}>&-
coproc worker { true; }
";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Function(function) = &script.commands[0] else {
            panic!("expected function definition");
        };
        assert_eq!(function.name_span.slice(input), "my_fn");

        let (compound, _) = expect_compound(&script.commands[1]);
        let CompoundCommand::For(command) = compound else {
            panic!("expected for loop");
        };
        assert_eq!(command.variable_span.slice(input), "item");

        let (compound, _) = expect_compound(&script.commands[2]);
        let CompoundCommand::Select(command) = compound else {
            panic!("expected select loop");
        };
        assert_eq!(command.variable_span.slice(input), "choice");

        let Command::Simple(command) = &script.commands[3] else {
            panic!("expected assignment-only simple command");
        };
        assert_eq!(command.assignments[0].name_span.slice(input), "foo");
        assert_eq!(
            command.assignments[0].index.as_ref().unwrap().slice(input),
            "10"
        );

        let Command::Simple(command) = &script.commands[4] else {
            panic!("expected exec simple command");
        };
        assert_eq!(
            command.redirects[0].fd_var_span.unwrap().slice(input),
            "myfd"
        );

        let (compound, _) = expect_compound(&script.commands[5]);
        let CompoundCommand::Coproc(command) = compound else {
            panic!("expected coproc command");
        };
        assert_eq!(command.name_span.unwrap().slice(input), "worker");
    }

    #[test]
    fn test_parse_conditional_builds_structured_logical_ast() {
        let script = Parser::new("[[ ! (foo && bar) ]]\n")
            .parse()
            .unwrap()
            .script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        assert_eq!(unary.op, ConditionalUnaryOp::Not);

        let ConditionalExpr::Parenthesized(paren) = unary.expr.as_ref() else {
            panic!("expected parenthesized conditional");
        };
        let ConditionalExpr::Binary(binary) = paren.expr.as_ref() else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::And);
        assert!(matches!(binary.left.as_ref(), ConditionalExpr::Word(_)));
        assert!(matches!(binary.right.as_ref(), ConditionalExpr::Word(_)));
        assert_eq!(command.left_bracket_span.start.column, 1);
        assert_eq!(command.right_bracket_span.start.column, 19);
    }

    #[test]
    fn test_parse_conditional_pattern_rhs_preserves_structure() {
        let input = "[[ foo == (bar|baz)* ]]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::PatternEq);

        let ConditionalExpr::Pattern(word) = binary.right.as_ref() else {
            panic!("expected pattern rhs");
        };
        assert_eq!(word.render(input), "(bar|baz)*");
    }

    #[test]
    fn test_parse_conditional_regex_rhs_preserves_structure() {
        let input = "[[ foo =~ [ab](c|d) ]]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

        let ConditionalExpr::Regex(word) = binary.right.as_ref() else {
            panic!("expected regex rhs");
        };
        assert_eq!(word.render(input), "[ab](c|d)");
    }

    #[test]
    fn test_parse_conditional_regex_rhs_with_double_left_paren_groups() {
        let input = "[[ x =~ ^\\\"\\-1[[:blank:]]((\\?[luds])+).* ]]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

        let ConditionalExpr::Regex(word) = binary.right.as_ref() else {
            panic!("expected regex rhs");
        };
        assert_eq!(word.render(input), "^\"-1[[:blank:]]((?[luds])+).*");
    }

    #[test]
    fn test_command_substitution_spans_are_absolute() {
        let script = Parser::new("out=$(\n  printf '%s\\n' $x\n)\n")
            .parse()
            .unwrap()
            .script;

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
    fn test_parse_command_substitution_with_open_paren_inside_double_quotes() {
        Parser::new("x=$(echo \"(\")\n").parse().unwrap();
    }

    #[test]
    fn test_process_substitution_spans_are_absolute() {
        let script = Parser::new("cat <(\n  printf '%s\\n' $x\n)\n")
            .parse()
            .unwrap()
            .script;

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

    #[test]
    fn test_parse_declare_clause_classifies_operands_and_prefix_assignments() {
        let input = "FOO=1 declare -a arr=(\"hello world\" two) bar >out\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Decl(command) = &script.commands[0] else {
            panic!("expected declaration clause");
        };

        assert_eq!(command.variant, "declare");
        assert_eq!(command.variant_span.slice(input), "declare");
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].name, "FOO");
        assert_eq!(command.redirects.len(), 1);
        assert_eq!(command.redirects[0].target.span.slice(input), "out");
        assert_eq!(command.operands.len(), 3);

        let DeclOperand::Flag(flag) = &command.operands[0] else {
            panic!("expected flag operand");
        };
        assert_eq!(flag.span.slice(input), "-a");

        let DeclOperand::Assignment(assignment) = &command.operands[1] else {
            panic!("expected assignment operand");
        };
        assert_eq!(assignment.name, "arr");
        let AssignmentValue::Array(elements) = &assignment.value else {
            panic!("expected array assignment");
        };
        assert_eq!(elements.len(), 2);
        assert!(elements[0].quoted);
        assert_eq!(elements[0].span.slice(input), "\"hello world\"");
        assert_eq!(elements[1].span.slice(input), "two");

        let DeclOperand::Name(name) = &command.operands[2] else {
            panic!("expected bare name operand");
        };
        assert_eq!(name.name, "bar");
    }

    #[test]
    fn test_parse_export_uses_dynamic_operand_for_invalid_assignment() {
        let script = Parser::new("export foo-bar=(one two)\n")
            .parse()
            .unwrap()
            .script;

        let Command::Decl(command) = &script.commands[0] else {
            panic!("expected declaration clause");
        };

        assert_eq!(command.variant, "export");
        assert_eq!(command.operands.len(), 1);
        let DeclOperand::Dynamic(word) = &command.operands[0] else {
            panic!("expected dynamic operand");
        };
        assert_eq!(
            word.span.slice("export foo-bar=(one two)\n"),
            "foo-bar=(one two)"
        );
    }

    #[test]
    fn test_parse_typeset_clause_classifies_flags_and_assignments() {
        let input = "typeset -xr VAR=value other\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Decl(command) = &script.commands[0] else {
            panic!("expected declaration clause");
        };

        assert_eq!(command.variant, "typeset");
        assert_eq!(command.variant_span.slice(input), "typeset");
        assert_eq!(command.operands.len(), 3);

        let DeclOperand::Flag(flag) = &command.operands[0] else {
            panic!("expected flag operand");
        };
        assert_eq!(flag.span.slice(input), "-xr");

        let DeclOperand::Assignment(assignment) = &command.operands[1] else {
            panic!("expected assignment operand");
        };
        assert_eq!(assignment.name, "VAR");
        assert!(
            matches!(&assignment.value, AssignmentValue::Scalar(value) if value.span.slice(input) == "value")
        );

        let DeclOperand::Name(name) = &command.operands[2] else {
            panic!("expected bare name operand");
        };
        assert_eq!(name.name, "other");
    }

    #[test]
    fn test_alias_expansion_can_form_a_for_loop_header() {
        let input = "\
shopt -s expand_aliases
alias FOR1='for '
alias FOR2='FOR1 '
alias eye1='i '
alias eye2='eye1 '
alias IN='in '
alias onetwo='1 2 '
FOR2 eye2 IN onetwo 3; do echo $i; done
";
        let script = Parser::new(input).parse().unwrap().script;

        let Some(command) = script.commands.last() else {
            panic!("expected final command to be a for loop");
        };
        let (compound, _) = expect_compound(command);
        let CompoundCommand::For(command) = compound else {
            panic!("expected final command to be a for loop");
        };
        assert_eq!(command.variable, "i");
        assert_eq!(command.words.as_ref().map(Vec::len), Some(3));
    }

    #[test]
    fn test_alias_expansion_can_open_a_brace_group() {
        let input = "\
shopt -s expand_aliases
alias LEFT='{'
LEFT echo one; echo two; }
";
        let script = Parser::new(input).parse().unwrap().script;

        let Some(command) = script.commands.last() else {
            panic!("expected final command to be a brace group");
        };
        let (compound, _) = expect_compound(command);
        let CompoundCommand::BraceGroup(commands) = compound else {
            panic!("expected final command to be a brace group");
        };
        assert!(matches!(commands.as_slice(), [Command::List(_)]));
    }

    #[test]
    fn test_alias_expansion_can_open_a_subshell() {
        let input = "\
shopt -s expand_aliases
alias LEFT='('
LEFT echo one; echo two )
";
        let script = Parser::new(input).parse().unwrap().script;

        let Some(command) = script.commands.last() else {
            panic!("expected final command to be a subshell");
        };
        let (compound, _) = expect_compound(command);
        let CompoundCommand::Subshell(commands) = compound else {
            panic!("expected final command to be a subshell");
        };
        assert!(matches!(commands.as_slice(), [Command::List(_)]));
    }

    // -----------------------------------------------------------------------
    // Comment range tests — verify Comment.range is valid for all comments
    // -----------------------------------------------------------------------

    /// Assert every comment range is within source bounds, on char boundaries,
    /// and starts with `#`.
    fn assert_comment_ranges_valid(source: &str, output: &ParseOutput) {
        for (i, comment) in output.comments.iter().enumerate() {
            let start = usize::from(comment.range.start());
            let end = usize::from(comment.range.end());
            assert!(
                end <= source.len(),
                "comment {i}: end ({end}) exceeds source length ({})",
                source.len()
            );
            assert!(
                source.is_char_boundary(start),
                "comment {i}: start ({start}) not on char boundary"
            );
            assert!(
                source.is_char_boundary(end),
                "comment {i}: end ({end}) not on char boundary"
            );
            let text = &source[start..end];
            assert!(
                text.starts_with('#'),
                "comment {i}: expected '#' at start, got {:?}",
                text.chars().next()
            );
            assert!(
                !text.contains('\n'),
                "comment {i}: spans multiple lines: {text:?}"
            );
        }
    }

    #[test]
    fn test_comment_ranges_simple() {
        let source = "# head\necho hi # inline\n# tail\n";
        let output = Parser::new(source).parse().unwrap();
        assert_eq!(output.comments.len(), 3);
        assert_comment_ranges_valid(source, &output);
    }

    #[test]
    fn test_comment_ranges_with_unicode() {
        let source = "# café résumé\necho ok\n# 你好世界\n";
        let output = Parser::new(source).parse().unwrap();
        assert_eq!(output.comments.len(), 2);
        assert_comment_ranges_valid(source, &output);
    }

    #[test]
    fn test_comment_ranges_heredoc_no_false_comments() {
        // Lines with # inside a heredoc must NOT produce Comment entries
        let source = "cat <<EOF\n# not a comment\nline two\nEOF\n# real\n";
        let output = Parser::new(source).parse().unwrap();
        assert_comment_ranges_valid(source, &output);
        // Only the real comment after EOF should be collected
        let texts: Vec<&str> = output
            .comments
            .iter()
            .map(|c| c.range.slice(source))
            .collect();
        assert!(
            !texts.iter().any(|t| t.contains("not a comment")),
            "heredoc body produced a false comment: {texts:?}"
        );
    }

    #[test]
    fn test_comment_ranges_heredoc_with_unicode() {
        let source = "cat <<EOF\n# 你好\ncafé\nEOF\n# end\n";
        let output = Parser::new(source).parse().unwrap();
        assert_comment_ranges_valid(source, &output);
    }

    #[test]
    fn test_comment_ranges_heredoc_desktop_entry() {
        // Reproduces the pattern from the distrobox corpus file:
        // a heredoc containing lines with ${var} expansions and no actual comments
        let source = r#"cat << EOF > "${HOME}/test.desktop"
[Desktop Entry]
Name=${entry_name}
GenericName=Terminal entering ${entry_name}
Comment=Terminal entering ${entry_name}
Categories=Distrobox;System;Utility
Exec=${distrobox_path}/distrobox enter ${extra_flags} ${container_name}
Icon=${icon}
Terminal=true
Type=Application
EOF
# done
"#;
        let output = Parser::new(source).parse().unwrap();
        assert_comment_ranges_valid(source, &output);
        let texts: Vec<&str> = output
            .comments
            .iter()
            .map(|c| c.range.slice(source))
            .collect();
        // None of the heredoc lines should appear as comments
        for text in &texts {
            assert!(
                !text.contains("Desktop") && !text.contains("entry_name"),
                "heredoc body leaked as comment: {text:?}"
            );
        }
    }
}
