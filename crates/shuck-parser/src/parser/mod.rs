//! Parser module for shuck
//!
//! Implements a recursive descent parser for bash scripts.

// Parser uses chars().next().unwrap() after validating character presence.
// This is safe because we check bounds before accessing.
#![allow(clippy::unwrap_used)]

mod arithmetic;
mod lexer;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

pub use lexer::{
    HeredocRead, LexedToken, LexedWord, LexedWordSegment, LexedWordSegmentKind, Lexer,
    LexerErrorKind,
};

use shuck_ast::{
    ArithmeticCommand, ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode,
    ArithmeticForCommand, ArithmeticLvalue, Assignment, AssignmentValue, BreakCommand,
    BuiltinCommand, CaseCommand, CaseItem, CaseTerminator, Command, CommandList, CommandListItem,
    CommandSubstitutionSyntax, Comment, CompoundCommand, ConditionalBinaryExpr,
    ConditionalBinaryOp, ConditionalCommand, ConditionalExpr, ConditionalParenExpr,
    ConditionalUnaryExpr, ConditionalUnaryOp, ContinueCommand, CoprocCommand, DeclClause, DeclName,
    DeclOperand, ExitCommand, ForCommand, FunctionDef, Heredoc, HeredocDelimiter, IfCommand,
    ListOperator, LiteralText, Name, ParameterOp, Pipeline, Position, Redirect, RedirectKind,
    RedirectTarget, ReturnCommand, Script, SelectCommand, SimpleCommand, SourceText, Span,
    TextSize, TimeCommand, TokenKind, UntilCommand, WhileCommand, Word, WordPart, WordPartNode,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellDialect {
    Posix,
    Mksh,
    #[default]
    Bash,
}

impl ShellDialect {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "sh" | "dash" | "ksh" | "posix" => Self::Posix,
            "mksh" => Self::Mksh,
            _ => Self::Bash,
        }
    }
}

/// Parser for bash scripts.
#[derive(Clone)]
pub struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    synthetic_tokens: VecDeque<SyntheticToken>,
    alias_replays: Vec<AliasReplay>,
    current_token: Option<LexedToken<'a>>,
    current_word_cache: Option<Word>,
    current_token_kind: Option<TokenKind>,
    current_keyword: Option<Keyword>,
    /// Span of the current token
    current_span: Span,
    /// Lookahead token for function parsing
    peeked_token: Option<LexedToken<'a>>,
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
    dialect: ShellDialect,
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
    tokens: Arc<[LexedToken<'static>]>,
    expands_next_word: bool,
}

#[derive(Debug, Clone)]
struct AliasReplay {
    tokens: Arc<[LexedToken<'static>]>,
    next_index: usize,
    base: Position,
}

impl AliasReplay {
    fn new(alias: &AliasDefinition, base: Position) -> Self {
        Self {
            tokens: Arc::clone(&alias.tokens),
            next_index: 0,
            base,
        }
    }

    fn next_token<'b>(&mut self) -> Option<LexedToken<'b>> {
        let token = self.tokens.get(self.next_index)?.clone();
        self.next_index += 1;
        Some(token.into_owned().rebased(self.base).with_synthetic_flag())
    }
}

#[derive(Debug, Clone, Copy)]
struct SyntheticToken {
    kind: TokenKind,
    span: Span,
}

impl SyntheticToken {
    const fn punctuation(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    fn materialize<'b>(self) -> LexedToken<'b> {
        LexedToken::punctuation(self.kind).with_span(self.span)
    }
}

#[derive(Debug, Clone, Copy)]
enum FlowControlBuiltinKind {
    Break,
    Continue,
    Return,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TokenSet(u64);

impl TokenSet {
    const fn contains(self, kind: TokenKind) -> bool {
        self.0 & (1u64 << kind as u8) != 0
    }
}

macro_rules! token_set {
    ($($kind:path),+ $(,)?) => {
        TokenSet(0 $(| (1u64 << ($kind as u8)))+)
    };
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keyword {
    If,
    For,
    While,
    Until,
    Case,
    Select,
    Time,
    Coproc,
    Function,
    Then,
    Else,
    Elif,
    Fi,
    Do,
    Done,
    Esac,
    In,
}

impl Keyword {
    const fn as_str(self) -> &'static str {
        match self {
            Self::If => "if",
            Self::For => "for",
            Self::While => "while",
            Self::Until => "until",
            Self::Case => "case",
            Self::Select => "select",
            Self::Time => "time",
            Self::Coproc => "coproc",
            Self::Function => "function",
            Self::Then => "then",
            Self::Else => "else",
            Self::Elif => "elif",
            Self::Fi => "fi",
            Self::Do => "do",
            Self::Done => "done",
            Self::Esac => "esac",
            Self::In => "in",
        }
    }
}

impl std::fmt::Display for Keyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeywordSet(u32);

impl KeywordSet {
    const fn single(keyword: Keyword) -> Self {
        Self(1u32 << keyword as u8)
    }

    const fn contains(self, keyword: Keyword) -> bool {
        self.0 & (1u32 << keyword as u8) != 0
    }
}

macro_rules! keyword_set {
    ($($keyword:ident),+ $(,)?) => {
        KeywordSet(0 $(| (1u32 << (Keyword::$keyword as u8)))+)
    };
}

const PIPE_OPERATOR_TOKENS: TokenSet = token_set![TokenKind::Pipe, TokenKind::PipeBoth];
const REDIRECT_TOKENS: TokenSet = token_set![
    TokenKind::RedirectOut,
    TokenKind::Clobber,
    TokenKind::RedirectAppend,
    TokenKind::RedirectIn,
    TokenKind::RedirectReadWrite,
    TokenKind::HereString,
    TokenKind::HereDoc,
    TokenKind::HereDocStrip,
    TokenKind::RedirectBoth,
    TokenKind::RedirectBothAppend,
    TokenKind::DupOutput,
    TokenKind::RedirectFd,
    TokenKind::RedirectFdAppend,
    TokenKind::DupFd,
    TokenKind::DupInput,
    TokenKind::DupFdIn,
    TokenKind::DupFdClose,
    TokenKind::RedirectFdIn,
    TokenKind::RedirectFdReadWrite,
];
const NON_COMMAND_KEYWORDS: KeywordSet = keyword_set![Then, Else, Elif, Fi, Do, Done, Esac, In];
const IF_BODY_TERMINATORS: KeywordSet = keyword_set![Elif, Else, Fi];

impl<'a> Parser<'a> {
    /// Create a new parser for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_limits_and_dialect(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellDialect::Bash,
        )
    }

    /// Create a new parser for the given input and shell dialect.
    pub fn with_dialect(input: &'a str, dialect: ShellDialect) -> Self {
        Self::with_limits_and_dialect(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            dialect,
        )
    }

    /// Create a new parser with a custom maximum AST depth.
    pub fn with_max_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_limits_and_dialect(
            input,
            max_depth,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellDialect::Bash,
        )
    }

    /// Create a new parser with a custom fuel limit.
    pub fn with_fuel(input: &'a str, max_fuel: usize) -> Self {
        Self::with_limits_and_dialect(input, DEFAULT_MAX_AST_DEPTH, max_fuel, ShellDialect::Bash)
    }

    /// Create a new parser with custom depth and fuel limits.
    ///
    /// `max_depth` is clamped to `HARD_MAX_AST_DEPTH` (500)
    /// to prevent stack overflow from misconfiguration. Even if the caller passes
    /// `max_depth = 1_000_000`, the parser will cap it at 500.
    pub fn with_limits(input: &'a str, max_depth: usize, max_fuel: usize) -> Self {
        Self::with_limits_and_dialect(input, max_depth, max_fuel, ShellDialect::Bash)
    }

    /// Create a new parser with custom depth, fuel, and dialect settings.
    pub fn with_limits_and_dialect(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        dialect: ShellDialect,
    ) -> Self {
        let mut lexer = Lexer::with_max_subst_depth(input, max_depth.min(HARD_MAX_AST_DEPTH));
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
            comments,
            aliases: HashMap::new(),
            expand_aliases: false,
            expand_next_word: false,
            dialect,
        }
    }

    pub fn dialect(&self) -> ShellDialect {
        self.dialect
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
            true,
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
            true,
        )
    }

    fn maybe_record_comment(&mut self, token: &LexedToken<'_>) {
        if token.kind == TokenKind::Comment && !token.flags.is_synthetic() {
            self.comments.push(Comment {
                range: token.span.to_range(),
            });
        }
    }

    fn word_from_token(&mut self, token: &LexedToken<'_>, span: Span) -> Option<Word> {
        if let Some(word) = self.simple_word_from_token(token, span) {
            return Some(word);
        }

        self.decode_word_from_token(token, span)
    }

    fn word_text_needs_parse(text: &str) -> bool {
        text.contains(['$', '`', '\x00'])
    }

    fn simple_word_from_token(&self, token: &LexedToken<'_>, span: Span) -> Option<Word> {
        let word = token.word()?;
        let mut parts = Vec::new();

        for segment in word.segments() {
            let text = segment.as_str();
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            match segment.kind() {
                LexedWordSegmentKind::Plain
                | LexedWordSegmentKind::DoubleQuoted
                | LexedWordSegmentKind::DollarDoubleQuoted
                    if Self::word_text_needs_parse(text) =>
                {
                    return None;
                }
                LexedWordSegmentKind::Plain
                | LexedWordSegmentKind::SingleQuoted
                | LexedWordSegmentKind::DollarSingleQuoted
                | LexedWordSegmentKind::DoubleQuoted
                | LexedWordSegmentKind::DollarDoubleQuoted => {}
                LexedWordSegmentKind::Composite => return None,
            }

            let content_span = Self::segment_content_span(segment, span);
            let wrapper_span = Self::segment_wrapper_span(segment, span);
            let part = match segment.kind() {
                LexedWordSegmentKind::Plain => {
                    self.literal_part_from_text(text, content_span, source_backed)
                }
                LexedWordSegmentKind::SingleQuoted => {
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false)
                }
                LexedWordSegmentKind::DollarSingleQuoted => {
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true)
                }
                LexedWordSegmentKind::DoubleQuoted => self.double_quoted_literal_part_from_text(
                    text,
                    content_span,
                    wrapper_span,
                    source_backed,
                    false,
                ),
                LexedWordSegmentKind::DollarDoubleQuoted => self
                    .double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        true,
                    ),
                LexedWordSegmentKind::Composite => unreachable!(),
            };
            parts.push(part);
        }

        Some(Word { parts, span })
    }

    fn segment_content_span(segment: &LexedWordSegment<'_>, fallback: Span) -> Span {
        segment
            .span()
            .or_else(|| segment.wrapper_span())
            .unwrap_or(fallback)
    }

    fn segment_wrapper_span(segment: &LexedWordSegment<'_>, fallback: Span) -> Span {
        segment
            .wrapper_span()
            .or_else(|| segment.span())
            .unwrap_or(fallback)
    }

    fn literal_part_from_text(&self, text: &str, span: Span, source_backed: bool) -> WordPartNode {
        WordPartNode::new(
            WordPart::Literal(if source_backed {
                LiteralText::source()
            } else {
                LiteralText::owned(text.to_string())
            }),
            span,
        )
    }

    fn single_quoted_part_from_text(
        &self,
        text: &str,
        content_span: Span,
        wrapper_span: Span,
        dollar: bool,
    ) -> WordPartNode {
        WordPartNode::new(
            WordPart::SingleQuoted {
                value: self.source_text(text.to_string(), content_span.start, content_span.end),
                dollar,
            },
            wrapper_span,
        )
    }

    fn double_quoted_literal_part_from_text(
        &self,
        text: &str,
        content_span: Span,
        wrapper_span: Span,
        source_backed: bool,
        dollar: bool,
    ) -> WordPartNode {
        WordPartNode::new(
            WordPart::DoubleQuoted {
                parts: vec![self.literal_part_from_text(text, content_span, source_backed)],
                dollar,
            },
            wrapper_span,
        )
    }

    fn decode_word_from_token(&mut self, token: &LexedToken<'_>, span: Span) -> Option<Word> {
        let word = token.word()?;

        if let Some(segment) = word.single_segment() {
            let text = segment.as_str();
            let content_span = Self::segment_content_span(segment, span);
            let wrapper_span = Self::segment_wrapper_span(segment, span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();

            return match segment.kind() {
                LexedWordSegmentKind::SingleQuoted => Some(Word {
                    parts: vec![self.single_quoted_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        false,
                    )],
                    span,
                }),
                LexedWordSegmentKind::DollarSingleQuoted => Some(Word {
                    parts: vec![self.single_quoted_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        true,
                    )],
                    span,
                }),
                LexedWordSegmentKind::Plain if Self::word_text_needs_parse(text) => {
                    Some(self.decode_word_text(text, span, content_span.start, source_backed))
                }
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted
                    if Self::word_text_needs_parse(text) =>
                {
                    let inner = self.decode_word_text(
                        text,
                        content_span,
                        content_span.start,
                        source_backed,
                    );
                    Some(Word {
                        parts: vec![WordPartNode::new(
                            WordPart::DoubleQuoted {
                                parts: inner.parts,
                                dollar: matches!(
                                    segment.kind(),
                                    LexedWordSegmentKind::DollarDoubleQuoted
                                ),
                            },
                            wrapper_span,
                        )],
                        span,
                    })
                }
                LexedWordSegmentKind::Plain => Some(Word {
                    parts: vec![self.literal_part_from_text(text, content_span, source_backed)],
                    span,
                }),
                LexedWordSegmentKind::DoubleQuoted => Some(Word {
                    parts: vec![self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        false,
                    )],
                    span,
                }),
                LexedWordSegmentKind::DollarDoubleQuoted => Some(Word {
                    parts: vec![self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        true,
                    )],
                    span,
                }),
                LexedWordSegmentKind::Composite => None,
            };
        }

        let mut parts = Vec::new();
        let mut cursor = span.start;

        for segment in word.segments() {
            let text = segment.as_str();
            let content_span = if let Some(segment_span) = segment.span() {
                cursor = segment_span.end;
                segment_span
            } else {
                let start = cursor;
                let end = start.advanced_by(text);
                cursor = end;
                Span::from_positions(start, end)
            };
            let wrapper_span = segment.wrapper_span().unwrap_or(content_span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();

            match segment.kind() {
                LexedWordSegmentKind::SingleQuoted => parts.push(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false),
                ),
                LexedWordSegmentKind::DollarSingleQuoted => parts.push(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true),
                ),
                LexedWordSegmentKind::Plain => {
                    if Self::word_text_needs_parse(text) {
                        self.decode_word_parts_into(
                            text,
                            content_span.start,
                            source_backed,
                            &mut parts,
                        );
                    } else {
                        parts.push(self.literal_part_from_text(text, content_span, source_backed));
                    }
                }
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted => {
                    if Self::word_text_needs_parse(text) {
                        let inner = self.decode_word_text(
                            text,
                            content_span,
                            content_span.start,
                            source_backed,
                        );
                        parts.push(WordPartNode::new(
                            WordPart::DoubleQuoted {
                                parts: inner.parts,
                                dollar: matches!(
                                    segment.kind(),
                                    LexedWordSegmentKind::DollarDoubleQuoted
                                ),
                            },
                            wrapper_span,
                        ));
                    } else {
                        parts.push(self.double_quoted_literal_part_from_text(
                            text,
                            content_span,
                            wrapper_span,
                            source_backed,
                            matches!(segment.kind(), LexedWordSegmentKind::DollarDoubleQuoted),
                        ));
                    }
                }
                LexedWordSegmentKind::Composite => return None,
            }
        }

        Some(Word { parts, span })
    }

    fn current_word(&mut self) -> Option<Word> {
        if let Some(word) = self.current_word_cache.as_ref() {
            return Some(word.clone());
        }

        let span = self.current_span;

        if let Some(token) = self.current_token.as_ref()
            && let Some(word) = self.simple_word_from_token(token, span)
        {
            return Some(word);
        }

        let token = self.current_token.take()?;
        let word = self.decode_word_from_token(&token, span);
        self.current_token = Some(token);
        if let Some(word) = word.as_ref() {
            self.current_word_cache = Some(word.clone());
        }
        word
    }

    fn token_source_like_word_text(&self, token: &LexedToken<'a>) -> Option<Cow<'a, str>> {
        token
            .source_slice(self.input)
            .map(Cow::Borrowed)
            .or_else(|| token.word_string().map(Cow::Owned))
    }

    fn current_source_like_word_text(&self) -> Option<Cow<'a, str>> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and(self.current_token.as_ref())
            .and_then(|token| self.token_source_like_word_text(token))
    }

    fn keyword_from_token(token: &LexedToken<'_>) -> Option<Keyword> {
        (token.kind == TokenKind::Word)
            .then(|| token.word_text())
            .flatten()
            .and_then(Self::classify_keyword)
    }

    fn current_conditional_literal_word(&self) -> Option<Word> {
        match self.current_token_kind? {
            TokenKind::LeftBrace | TokenKind::RightBrace => Some(Word::literal_with_span(
                self.input[self.current_span.start.offset..self.current_span.end.offset]
                    .to_string(),
                self.current_span,
            )),
            _ => None,
        }
    }

    fn current_name_token(&self) -> Option<(Name, Span)> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and_then(|_| self.current_word_str())
            .map(|word| (Name::from(word), self.current_span))
    }

    fn current_static_token_text(&self) -> Option<(String, bool)> {
        let token = self.current_token.as_ref()?;
        let text = token.word_string()?;

        match token.kind {
            TokenKind::LiteralWord => Some((text, true)),
            TokenKind::QuotedWord if !Self::word_text_needs_parse(&text) => Some((text, true)),
            TokenKind::Word if !Self::word_text_needs_parse(&text) => Some((text, false)),
            _ => None,
        }
    }

    fn nested_commands_from_source(&mut self, source: &str, base: Position) -> Vec<Command> {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let inner_parser =
            Parser::with_limits_and_dialect(source, remaining_depth, self.fuel, self.dialect);
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

        if self.at(TokenKind::RightParen) {
            let right_paren_span = Span::from_positions(right_paren_start, self.current_span.end);
            self.advance();
            Ok(right_paren_span)
        } else {
            Err(Error::parse(format!(
                "expected ')' after '))' in {context}"
            )))
        }
    }

    fn split_double_semicolon(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by(";");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    fn split_double_left_paren(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by("(");
        (
            Span::from_positions(span.start, middle),
            Span::from_positions(middle, span.end),
        )
    }

    fn split_double_right_paren(span: Span) -> (Span, Span) {
        let middle = span.start.advanced_by(")");
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
                for item in &mut list.rest {
                    item.operator_span = item.operator_span.rebased(base);
                    Self::rebase_command(&mut item.command, base);
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
                    if let Some(expr) = &mut name.index_ast {
                        Self::rebase_arithmetic_expr(expr, base);
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
                if let Some(expr) = &mut command.init_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.first_semicolon_span = command.first_semicolon_span.rebased(base);
                command.condition_span = command.condition_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.condition_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                command.second_semicolon_span = command.second_semicolon_span.rebased(base);
                command.step_span = command.step_span.map(|span| span.rebased(base));
                if let Some(expr) = &mut command.step_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
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
                if let Some(expr) = &mut command.expr_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
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
        Self::rebase_word_parts(&mut word.parts, base);
    }

    fn rebase_word_parts(parts: &mut [WordPartNode], base: Position) {
        for part in parts {
            Self::rebase_word_part(part, base);
        }
    }

    fn rebase_word_part(part: &mut WordPartNode, base: Position) {
        part.span = part.span.rebased(base);
        match &mut part.kind {
            WordPart::SingleQuoted { value, .. } => value.rebased(base),
            WordPart::DoubleQuoted { parts, .. } => Self::rebase_word_parts(parts, base),
            WordPart::ParameterExpansion { operand, .. } => {
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
            }
            WordPart::ArrayAccess {
                index, index_ast, ..
            } => {
                index.rebased(base);
                if let Some(expr) = index_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::Substring {
                offset,
                offset_ast,
                length,
                length_ast,
                ..
            }
            | WordPart::ArraySlice {
                offset,
                offset_ast,
                length,
                length_ast,
                ..
            } => {
                offset.rebased(base);
                if let Some(expr) = offset_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                if let Some(length) = length {
                    length.rebased(base);
                }
                if let Some(expr) = length_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::IndirectExpansion { operand, .. } => {
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                ..
            } => {
                expression.rebased(base);
                if let Some(expr) = expression_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::CommandSubstitution { commands, .. }
            | WordPart::ProcessSubstitution { commands, .. } => {
                Self::rebase_commands(commands, base)
            }
            WordPart::Literal(_)
            | WordPart::Variable(_)
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::PrefixMatch(_)
            | WordPart::Transformation { .. } => {}
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

    fn rebase_arithmetic_expr(expr: &mut ArithmeticExprNode, base: Position) {
        expr.span = expr.span.rebased(base);
        match &mut expr.kind {
            ArithmeticExpr::Number(text) => text.rebased(base),
            ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => Self::rebase_arithmetic_expr(index, base),
            ArithmeticExpr::ShellWord(word) => Self::rebase_word(word, base),
            ArithmeticExpr::Parenthesized { expression } => {
                Self::rebase_arithmetic_expr(expression, base)
            }
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                Self::rebase_arithmetic_expr(expr, base)
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                Self::rebase_arithmetic_expr(left, base);
                Self::rebase_arithmetic_expr(right, base);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::rebase_arithmetic_expr(condition, base);
                Self::rebase_arithmetic_expr(then_expr, base);
                Self::rebase_arithmetic_expr(else_expr, base);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                Self::rebase_arithmetic_lvalue(target, base);
                Self::rebase_arithmetic_expr(value, base);
            }
        }
    }

    fn rebase_arithmetic_lvalue(target: &mut ArithmeticLvalue, base: Position) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => Self::rebase_arithmetic_expr(index, base),
        }
    }

    fn push_word_part(
        parts: &mut Vec<WordPartNode>,
        part: WordPart,
        start: Position,
        end: Position,
    ) {
        parts.push(WordPartNode::new(part, Span::from_positions(start, end)));
    }

    fn flush_literal_part(
        &self,
        parts: &mut Vec<WordPartNode>,
        current: &mut String,
        current_start: Position,
        end: Position,
    ) {
        if !current.is_empty() {
            Self::push_word_part(
                parts,
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

    fn parse_explicit_arithmetic_span(
        &self,
        span: Option<Span>,
        context: &'static str,
    ) -> Result<Option<ArithmeticExprNode>> {
        let Some(span) = span else {
            return Ok(None);
        };
        if span.slice(self.input).trim().is_empty() {
            return Ok(None);
        }
        arithmetic::parse_expression(
            span.slice(self.input),
            span,
            self.max_depth.saturating_sub(self.current_depth),
            self.fuel,
        )
        .map(Some)
        .map_err(|error| match error {
            Error::Parse { message, .. } => self.error(format!("{context}: {message}")),
        })
    }

    fn parse_source_text_as_arithmetic(&self, text: &SourceText) -> Result<ArithmeticExprNode> {
        arithmetic::parse_expression(
            text.slice(self.input),
            text.span(),
            self.max_depth.saturating_sub(self.current_depth),
            self.fuel,
        )
    }

    fn maybe_parse_source_text_as_arithmetic(
        &self,
        text: &SourceText,
    ) -> Option<ArithmeticExprNode> {
        if !text.is_source_backed() {
            return None;
        }
        self.parse_source_text_as_arithmetic(text).ok()
    }

    fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == text
    }

    fn single_literal_word_text<'b>(&'b self, word: &'b Word) -> Option<&'b str> {
        if Self::is_fully_quoted_word(word) || word.parts.len() != 1 {
            return None;
        }
        let WordPart::Literal(text) = &word.parts[0].kind else {
            return None;
        };
        Some(text.as_str(self.input, word.part_span(0)?))
    }

    fn literal_word_text(&self, word: &Word) -> Option<String> {
        let mut text = String::new();
        self.collect_literal_word_text(&word.parts, &mut text)?;
        Some(text)
    }

    fn collect_literal_word_text(&self, parts: &[WordPartNode], out: &mut String) -> Option<()> {
        for part in parts {
            match &part.kind {
                WordPart::Literal(literal) => out.push_str(literal.as_str(self.input, part.span)),
                WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(self.input)),
                WordPart::DoubleQuoted { parts, .. } => {
                    self.collect_literal_word_text(parts, out)?;
                }
                _ => return None,
            }
        }

        Some(())
    }

    fn is_fully_quoted_word(word: &Word) -> bool {
        matches!(
            word.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. },
                ..
            }]
        )
    }

    fn fd_var_from_text(text: &str, span: Span) -> Option<(Name, Span)> {
        if !text.starts_with('{') || !text.ends_with('}') || text.len() <= 2 {
            return None;
        }

        let inner = &text[1..text.len() - 1];
        if !inner.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }

        let start = span.start.advanced_by("{");
        let span = Span::from_positions(start, start.advanced_by(inner));
        Some((Name::from(inner), span))
    }

    fn current_fd_var(&mut self) -> Option<(Name, Span)> {
        if let Some(token) = self.current_token.as_ref()
            && token.kind == TokenKind::Word
            && let Some(word) = token.word()
            && let Some(segment) = word.single_segment()
            && segment.kind() == LexedWordSegmentKind::Plain
            && !Self::word_text_needs_parse(segment.as_str())
            && let Some(fd_var) = Self::fd_var_from_text(
                segment.as_str(),
                segment.span().unwrap_or(self.current_span),
            )
        {
            return Some(fd_var);
        }

        let word = self.current_word()?;
        let text = self.literal_word_text(&word)?;
        Self::fd_var_from_text(&text, word.span)
    }

    fn is_redirect_kind(kind: TokenKind) -> bool {
        REDIRECT_TOKENS.contains(kind)
    }

    fn current_static_heredoc_delimiter(&mut self) -> Option<(Word, String, bool)> {
        let word = self.current_word()?;
        let raw_text = word.span.slice(self.input);
        let quoted_parts = Self::word_has_quoted_parts(&word.parts);

        if let Some((text, token_quoted)) = self.current_static_token_text() {
            let quoted = quoted_parts || token_quoted || raw_text != text;
            return Some((word, text, quoted));
        }

        let text = self.literal_word_text(&word)?;
        let quoted = quoted_parts || raw_text != text;
        Some((word, text, quoted))
    }

    fn word_has_quoted_parts(parts: &[WordPartNode]) -> bool {
        parts.iter().any(|part| {
            matches!(
                &part.kind,
                WordPart::SingleQuoted { .. } | WordPart::DoubleQuoted { .. }
            )
        })
    }

    fn current_fd_value(&self) -> Option<i32> {
        self.current_token.as_ref().and_then(LexedToken::fd_value)
    }

    fn current_fd_pair(&self) -> Option<(i32, i32)> {
        self.current_token
            .as_ref()
            .and_then(LexedToken::fd_pair_value)
    }

    fn set_current_spanned(&mut self, token: LexedToken<'a>) {
        let span = token.span;
        self.current_token_kind = Some(token.kind);
        self.current_keyword = Self::keyword_from_token(&token);
        self.current_token = Some(token);
        self.current_word_cache = None;
        self.current_span = span;
    }

    fn set_current_kind(&mut self, kind: TokenKind, span: Span) {
        self.current_token_kind = Some(kind);
        self.current_keyword = None;
        self.current_token = Some(LexedToken::punctuation(kind).with_span(span));
        self.current_word_cache = None;
        self.current_span = span;
    }

    fn clear_current_token(&mut self) {
        self.current_token = None;
        self.current_word_cache = None;
        self.current_token_kind = None;
        self.current_keyword = None;
    }

    fn next_pending_token(&mut self) -> Option<LexedToken<'a>> {
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

    fn next_spanned_token_with_comments(&mut self) -> Option<LexedToken<'a>> {
        self.next_pending_token()
            .or_else(|| self.lexer.next_lexed_token_with_comments())
    }

    fn compile_alias_definition(&self, value: &str) -> AliasDefinition {
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

    fn maybe_expand_current_alias_chain(&mut self) {
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
                    self.aliases
                        .insert(alias_name.to_string(), self.compile_alias_definition(value));
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
                for item in &list.rest {
                    self.apply_command_effects(&item.command);
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
        mut predicate: F,
        source_backed: bool,
    ) -> SourceText
    where
        F: FnMut(char) -> bool,
    {
        let start = *cursor;
        if source_backed {
            while let Some(&ch) = chars.peek() {
                if !predicate(ch) {
                    break;
                }
                Self::next_word_char_unwrap(chars, cursor);
            }
            SourceText::source(Span::from_positions(start, *cursor))
        } else {
            let text = Self::read_word_while(chars, cursor, predicate);
            self.source_text(text, start, *cursor)
        }
    }

    fn rebase_redirects(redirects: &mut [Redirect], base: Position) {
        for redirect in redirects {
            redirect.span = redirect.span.rebased(base);
            redirect.fd_var_span = redirect.fd_var_span.map(|span| span.rebased(base));
            match &mut redirect.target {
                RedirectTarget::Word(word) => Self::rebase_word(word, base),
                RedirectTarget::Heredoc(heredoc) => {
                    heredoc.delimiter.span = heredoc.delimiter.span.rebased(base);
                    Self::rebase_word(&mut heredoc.delimiter.raw, base);
                    Self::rebase_word(&mut heredoc.body, base);
                }
            }
        }
    }

    fn rebase_assignments(assignments: &mut [Assignment], base: Position) {
        for assignment in assignments {
            assignment.span = assignment.span.rebased(base);
            assignment.name_span = assignment.name_span.rebased(base);
            if let Some(index) = &mut assignment.index {
                index.rebased(base);
            }
            if let Some(expr) = &mut assignment.index_ast {
                Self::rebase_arithmetic_expr(expr, base);
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

    fn ensure_bash_or_mksh(&self, feature: &str) -> Result<()> {
        if matches!(self.dialect, ShellDialect::Posix) {
            Err(self.error(format!("{feature} is not available in POSIX shell mode")))
        } else {
            Ok(())
        }
    }

    fn ensure_bash_only(&self, feature: &str) -> Result<()> {
        if matches!(self.dialect, ShellDialect::Bash) {
            Ok(())
        } else {
            Err(self.error(format!("{feature} is only available in Bash mode")))
        }
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

    fn is_recovery_separator(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Newline
                | TokenKind::Semicolon
                | TokenKind::Background
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Pipe
                | TokenKind::DoubleSemicolon
                | TokenKind::SemiAmp
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
            self.set_current_spanned(peeked);
        } else {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if st.kind == TokenKind::Comment => {
                        self.maybe_record_comment(&st);
                    }
                    Some(st) => {
                        self.set_current_spanned(st);
                        break;
                    }
                    None => {
                        self.clear_current_token();
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
    fn peek_next(&mut self) -> Option<&LexedToken<'a>> {
        if self.peeked_token.is_none() {
            loop {
                match self.next_spanned_token_with_comments() {
                    Some(st) if st.kind == TokenKind::Comment => {
                        self.maybe_record_comment(&st);
                    }
                    other => {
                        self.peeked_token = other;
                        break;
                    }
                }
            }
        }
        self.peeked_token.as_ref()
    }

    fn peek_next_kind(&mut self) -> Option<TokenKind> {
        self.peek_next()?;
        self.peeked_token.as_ref().map(|st| st.kind)
    }

    fn peek_next_is(&mut self, kind: TokenKind) -> bool {
        self.peek_next_kind() == Some(kind)
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current_token_kind == Some(kind)
    }

    fn at_in_set(&self, set: TokenSet) -> bool {
        self.current_token_kind
            .is_some_and(|kind| set.contains(kind))
    }

    fn at_word_like(&self) -> bool {
        self.current_token_kind.is_some_and(TokenKind::is_word_like)
    }

    fn current_word_str(&self) -> Option<&str> {
        self.current_token_kind
            .filter(|kind| kind.is_word_like())
            .and(self.current_token.as_ref())
            .and_then(LexedToken::word_text)
    }

    fn classify_keyword(word: &str) -> Option<Keyword> {
        match word.as_bytes() {
            b"if" => Some(Keyword::If),
            b"for" => Some(Keyword::For),
            b"while" => Some(Keyword::While),
            b"until" => Some(Keyword::Until),
            b"case" => Some(Keyword::Case),
            b"select" => Some(Keyword::Select),
            b"time" => Some(Keyword::Time),
            b"coproc" => Some(Keyword::Coproc),
            b"function" => Some(Keyword::Function),
            b"then" => Some(Keyword::Then),
            b"else" => Some(Keyword::Else),
            b"elif" => Some(Keyword::Elif),
            b"fi" => Some(Keyword::Fi),
            b"do" => Some(Keyword::Do),
            b"done" => Some(Keyword::Done),
            b"esac" => Some(Keyword::Esac),
            b"in" => Some(Keyword::In),
            _ => None,
        }
    }

    fn current_keyword(&self) -> Option<Keyword> {
        self.current_keyword
    }

    fn skip_newlines(&mut self) -> Result<()> {
        while self.at(TokenKind::Newline) {
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

        let mut rest = Vec::with_capacity(1);

        loop {
            let (op, allow_empty_tail) = match self.current_token_kind {
                Some(TokenKind::And) => (ListOperator::And, false),
                Some(TokenKind::Or) => (ListOperator::Or, false),
                Some(TokenKind::Semicolon) => (ListOperator::Semicolon, true),
                Some(TokenKind::Background) => (ListOperator::Background, true),
                _ => break,
            };
            let operator_span = self.current_span;
            self.advance();

            self.skip_newlines()?;
            if allow_empty_tail && self.current_token.is_none() {
                if matches!(op, ListOperator::Background) {
                    rest.push(CommandListItem {
                        operator: ListOperator::Background,
                        operator_span,
                        command: Command::Simple(SimpleCommand {
                            name: Word::literal(""),
                            args: vec![],
                            redirects: vec![],
                            assignments: vec![],
                            span: self.current_span,
                        }),
                    });
                }
                break;
            }

            if let Some(cmd) = self.parse_pipeline()? {
                rest.push(CommandListItem {
                    operator: op,
                    operator_span,
                    command: cmd,
                });
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
        let negated = self.at(TokenKind::Word) && self.current_word_str() == Some("!");
        if negated {
            self.advance();
        }

        let first = match self.parse_command()? {
            Some(cmd) => cmd,
            None => {
                if negated {
                    return Err(self.error("expected command after !"));
                }
                return Ok(None);
            }
        };

        let mut commands = Vec::with_capacity(2);
        commands.push(first);

        while self.at_in_set(PIPE_OPERATOR_TOKENS) {
            let pipe_both = self.at(TokenKind::PipeBoth);
            let operator_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            if pipe_both {
                Self::append_pipe_both_redirect(commands.last_mut().unwrap(), operator_span);
            }

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

    fn append_pipe_both_redirect(command: &mut Command, span: Span) {
        let redirect = Redirect {
            fd: Some(2),
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::DupOutput,
            span,
            target: RedirectTarget::Word(Word::literal("1")),
        };

        match command {
            Command::Simple(simple) => simple.redirects.push(redirect),
            Command::Builtin(BuiltinCommand::Break(command)) => command.redirects.push(redirect),
            Command::Builtin(BuiltinCommand::Continue(command)) => command.redirects.push(redirect),
            Command::Builtin(BuiltinCommand::Return(command)) => command.redirects.push(redirect),
            Command::Builtin(BuiltinCommand::Exit(command)) => command.redirects.push(redirect),
            Command::Decl(decl) => decl.redirects.push(redirect),
            Command::Compound(_, redirects) => redirects.push(redirect),
            Command::Function(function) => {
                Self::append_pipe_both_redirect(function.body.as_mut(), span);
            }
            Command::Pipeline(_) | Command::List(_) => {}
        }
    }

    fn push_redirect_both_append(redirects: &mut Vec<Redirect>, operator_span: Span, target: Word) {
        redirects.push(Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::Append,
            span: Self::redirect_span(operator_span, &target),
            target: RedirectTarget::Word(target),
        });
        redirects.push(Redirect {
            fd: Some(2),
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::DupOutput,
            span: operator_span,
            target: RedirectTarget::Word(Word::literal("1")),
        });
    }

    fn redirect_supports_fd_var(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::RedirectOut
                | TokenKind::Clobber
                | TokenKind::RedirectAppend
                | TokenKind::RedirectIn
                | TokenKind::RedirectReadWrite
                | TokenKind::HereString
                | TokenKind::RedirectBoth
                | TokenKind::DupOutput
                | TokenKind::DupInput
        )
    }

    fn maybe_expect_word(&mut self, strict: bool) -> Result<Option<Word>> {
        if strict {
            self.expect_word().map(Some)
        } else {
            Ok(self.expect_word().ok())
        }
    }

    fn consume_non_heredoc_redirect(
        &mut self,
        redirects: &mut Vec<Redirect>,
        fd_var: Option<Name>,
        fd_var_span: Option<Span>,
        strict: bool,
    ) -> Result<bool> {
        match self.current_token_kind {
            Some(TokenKind::RedirectOut) | Some(TokenKind::Clobber) => {
                let operator_span = self.current_span;
                let kind = if self.at(TokenKind::Clobber) {
                    RedirectKind::Clobber
                } else {
                    RedirectKind::Output
                };
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectAppend) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectIn) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectReadWrite) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::ReadWrite,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::HereString) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::HereString,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectBoth) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: None,
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::OutputBoth,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectBothAppend) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    Self::push_redirect_both_append(redirects, operator_span, target);
                }
                Ok(true)
            }
            Some(TokenKind::DupOutput) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(1) },
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::DupOutput,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFd) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Output,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFdAppend) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Append,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::RedirectFdReadWrite) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::ReadWrite,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::DupFd) => {
                let (src_fd, dst_fd) = self.current_fd_pair().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(src_fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupOutput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal(dst_fd.to_string())),
                });
                Ok(true)
            }
            Some(TokenKind::DupInput) => {
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: if fd_var.is_some() { None } else { Some(0) },
                        fd_var,
                        fd_var_span,
                        kind: RedirectKind::DupInput,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            Some(TokenKind::DupFdIn) => {
                let (src_fd, dst_fd) = self.current_fd_pair().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(src_fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupInput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal(dst_fd.to_string())),
                });
                Ok(true)
            }
            Some(TokenKind::DupFdClose) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                redirects.push(Redirect {
                    fd: Some(fd),
                    fd_var: None,
                    fd_var_span: None,
                    kind: RedirectKind::DupInput,
                    span: operator_span,
                    target: RedirectTarget::Word(Word::literal("-")),
                });
                Ok(true)
            }
            Some(TokenKind::RedirectFdIn) => {
                let fd = self.current_fd_value().unwrap_or_default();
                let operator_span = self.current_span;
                self.advance();
                if let Some(target) = self.maybe_expect_word(strict)? {
                    redirects.push(Redirect {
                        fd: Some(fd),
                        fd_var: None,
                        fd_var_span: None,
                        kind: RedirectKind::Input,
                        span: Self::redirect_span(operator_span, &target),
                        target: RedirectTarget::Word(target),
                    });
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn strip_heredoc_tabs(content: String) -> String {
        let had_trailing_newline = content.ends_with('\n');
        let mut stripped: String = content
            .lines()
            .map(|line: &str| line.trim_start_matches('\t'))
            .collect::<Vec<_>>()
            .join("\n");
        if had_trailing_newline {
            stripped.push('\n');
        }
        stripped
    }

    fn consume_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
        fd_var: Option<Name>,
        fd_var_span: Option<Span>,
        strict: bool,
        collect_trailing_redirects: bool,
    ) -> Result<bool> {
        let operator_span = self.current_span;
        self.advance();
        let Some((raw_delimiter, delimiter_text, quoted)) = self.current_static_heredoc_delimiter()
        else {
            if strict {
                return Err(Error::parse(
                    "expected static heredoc delimiter".to_string(),
                ));
            }
            return Ok(false);
        };

        let delimiter_span = raw_delimiter.span;
        let delimiter = HeredocDelimiter {
            raw: raw_delimiter,
            cooked: delimiter_text.clone(),
            span: delimiter_span,
            quoted,
            expands_body: !quoted,
            strip_tabs,
        };

        let heredoc = self.lexer.read_heredoc(&delimiter_text);
        let content_span = heredoc.content_span;
        let content = if strip_tabs {
            Self::strip_heredoc_tabs(heredoc.content)
        } else {
            heredoc.content
        };

        let body = if quoted {
            Word::quoted_literal_with_span(content, content_span)
        } else {
            self.decode_word_text(&content, content_span, content_span.start, !strip_tabs)
        };

        redirects.push(Redirect {
            fd: None,
            fd_var,
            fd_var_span,
            kind: if strip_tabs {
                RedirectKind::HereDocStrip
            } else {
                RedirectKind::HereDoc
            },
            span: operator_span.merge(delimiter.span),
            target: RedirectTarget::Heredoc(Heredoc { delimiter, body }),
        });

        // Advance so re-injected rest-of-line tokens are picked up.
        self.advance();

        if collect_trailing_redirects {
            self.collect_trailing_redirects(redirects)?;
        }

        Ok(true)
    }

    /// Parse redirections that follow a compound command (>, >>, 2>, etc.)
    fn parse_trailing_redirects(&mut self) -> Vec<Redirect> {
        let mut redirects = Vec::new();
        let mut pending_fd_var = None;
        loop {
            if pending_fd_var.is_none()
                && let Some((fd_var, fd_var_span)) = self.current_fd_var()
                && self.peek_next_kind().is_some_and(Self::is_redirect_kind)
            {
                pending_fd_var = Some((fd_var, fd_var_span));
                self.advance();
                continue;
            }

            match self.current_token_kind {
                Some(TokenKind::HereDoc) | Some(TokenKind::HereDocStrip) => {
                    let strip_tabs = self.at(TokenKind::HereDocStrip);
                    let (fd_var, fd_var_span) = pending_fd_var.take().unzip();
                    if !self
                        .consume_heredoc_redirect(
                            strip_tabs,
                            &mut redirects,
                            fd_var,
                            fd_var_span,
                            false,
                            false,
                        )
                        .unwrap_or(false)
                    {
                        break;
                    }
                    continue;
                }
                Some(kind) => {
                    let (fd_var, fd_var_span) = if Self::redirect_supports_fd_var(kind) {
                        pending_fd_var.take().unzip()
                    } else {
                        let _ = pending_fd_var.take();
                        (None, None)
                    };

                    if self
                        .consume_non_heredoc_redirect(&mut redirects, fd_var, fd_var_span, false)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    break;
                }
                None => break,
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

    fn looks_like_command_style_double_paren(&self) -> bool {
        let mut probe = self.clone();
        if probe.current_token_kind != Some(TokenKind::DoubleLeftParen) {
            return false;
        }

        probe.advance();
        let mut paren_depth = 0_i32;
        let mut previous_top_level_operand = false;

        loop {
            match probe.current_token_kind {
                Some(TokenKind::DoubleLeftParen) => {
                    paren_depth += 2;
                    previous_top_level_operand = false;
                    probe.advance();
                }
                Some(TokenKind::LeftParen) => {
                    paren_depth += 1;
                    previous_top_level_operand = false;
                    probe.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
                    if paren_depth == 0 {
                        return false;
                    }
                    if paren_depth == 1 {
                        return false;
                    }
                    paren_depth -= 2;
                    previous_top_level_operand = false;
                    probe.advance();
                }
                Some(TokenKind::RightParen) => {
                    if paren_depth == 0 {
                        return true;
                    }
                    paren_depth -= 1;
                    previous_top_level_operand = false;
                    probe.advance();
                }
                Some(TokenKind::Newline) | Some(TokenKind::Semicolon) if paren_depth == 0 => {
                    previous_top_level_operand = false;
                    probe.advance();
                }
                Some(_)
                    if paren_depth == 0
                        && probe
                            .current_token
                            .as_ref()
                            .is_some_and(Self::is_operand_like_double_paren_token) =>
                {
                    if previous_top_level_operand {
                        return true;
                    }
                    previous_top_level_operand = true;
                    probe.advance();
                }
                Some(_) => {
                    previous_top_level_operand = false;
                    probe.advance();
                }
                None => return false,
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

        // Check for compound commands and function keyword
        match self.current_keyword() {
            Some(Keyword::If) => return self.parse_compound_with_redirects(|s| s.parse_if()),
            Some(Keyword::For) => return self.parse_compound_with_redirects(|s| s.parse_for()),
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
            // Check for POSIX-style function: name() { body }
            // Exclude obvious assignment-like heads such as `a[(1+2)*3]=9`.
            && !word.contains('=')
            && !word.contains('[')
            && self.peek_next_is(TokenKind::LeftParen)
        {
            return self.parse_function_posix().map(Some);
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

            let mut arithmetic_probe = self.clone();
            if let Ok(compound) = arithmetic_probe.parse_arithmetic_command() {
                let redirects = arithmetic_probe.parse_trailing_redirects();
                *self = arithmetic_probe;
                return Ok(Some(Command::Compound(compound, redirects)));
            }

            self.split_current_double_left_paren();
            return self.parse_compound_with_redirects(|s| s.parse_subshell());
        }

        // Check for subshell
        if self.at(TokenKind::LeftParen) {
            return self.parse_compound_with_redirects(|s| s.parse_subshell());
        }

        // Check for brace group
        if self.at(TokenKind::LeftBrace) {
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
        let condition = self.parse_compound_list(Keyword::Then)?;

        // Expect 'then'
        self.expect_keyword(Keyword::Then)?;
        self.skip_newlines()?;

        // Parse then branch
        let then_branch = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;

        // Bash requires at least one command in then branch
        if then_branch.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty then clause"));
        }

        // Parse elif branches
        let mut elif_branches = Vec::new();
        while self.is_keyword(Keyword::Elif) {
            self.advance(); // consume 'elif'
            self.skip_newlines()?;

            let elif_condition = self.parse_compound_list(Keyword::Then)?;
            self.expect_keyword(Keyword::Then)?;
            self.skip_newlines()?;

            let elif_body = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;

            // Bash requires at least one command in elif branch
            if elif_body.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty elif clause"));
            }

            elif_branches.push((elif_condition, elif_body));
        }

        // Parse else branch
        let else_branch = if self.is_keyword(Keyword::Else) {
            self.advance(); // consume 'else'
            self.skip_newlines()?;
            let branch = self.parse_compound_list(Keyword::Fi)?;

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
        self.expect_keyword(Keyword::Fi)?;

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
        if self.at(TokenKind::DoubleLeftParen) {
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
        let words = if self.is_keyword(Keyword::In) {
            self.advance(); // consume 'in'

            // Parse word list until do/newline/;
            let mut words = Vec::new();
            loop {
                match self.current_token_kind {
                    _ if self.current_keyword() == Some(Keyword::Do) => break,
                    Some(kind) if kind.is_word_like() => {
                        if let Some(word) = self.current_word() {
                            words.push(word);
                        }
                        self.advance();
                    }
                    Some(TokenKind::Newline | TokenKind::Semicolon) => {
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
            if self.at(TokenKind::Semicolon) {
                self.advance();
            }
            None
        };

        self.skip_newlines()?;

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list(Keyword::Done)?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty for loop body"));
        }

        // Expect 'done'
        self.expect_keyword(Keyword::Done)?;

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
        self.ensure_bash_or_mksh("select loops")?;
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
                    if let Some(word) = self.current_word() {
                        words.push(word);
                    }
                    self.advance();
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
        let body = self.parse_compound_list(Keyword::Done)?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty select loop body"));
        }

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
        self.ensure_bash_only("c-style for loops")?;
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
            let body = self.parse_brace_group()?;
            (vec![Command::Compound(body, Vec::new())], self.current_span)
        } else {
            // Expect 'do'
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            // Parse body
            let body = self.parse_compound_list(Keyword::Done)?;

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
            (body, done_span)
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
        let condition = self.parse_compound_list(Keyword::Do)?;

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list(Keyword::Done)?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty while loop body"));
        }

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
        let condition = self.parse_compound_list(Keyword::Do)?;

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body = self.parse_compound_list(Keyword::Done)?;

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty until loop body"));
        }

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

            // Parse patterns (pattern1 | pattern2 | ...)
            // Optional leading (
            if self.at(TokenKind::LeftParen) {
                self.advance();
            }

            let mut patterns = Vec::new();
            while self.at_word_like() {
                if let Some(word) = self.current_word() {
                    patterns.push(word);
                }
                self.advance();

                // Check for | between patterns
                if self.at(TokenKind::Pipe) {
                    self.advance();
                } else {
                    break;
                }
            }

            // Expect )
            if !self.at(TokenKind::RightParen) {
                self.pop_depth();
                return Err(self.error("expected ')' after case pattern"));
            }
            self.advance();
            self.skip_newlines()?;

            // Parse commands until ;; or esac
            let mut commands = Vec::new();
            while !self.is_case_terminator()
                && !self.is_keyword(Keyword::Esac)
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
        self.expect_keyword(Keyword::Esac)?;

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
        let posix_format = if self.at(TokenKind::Word) && self.current_word_str() == Some("-p") {
            self.advance();
            self.skip_newlines()?;
            true
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
        self.ensure_bash_only("coprocess commands")?;
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
        )
    }

    /// Parse case terminator: `;;` (break), `;&` (fallthrough), `;;&` (continue matching)
    fn parse_case_terminator(&mut self) -> CaseTerminator {
        match self.current_token_kind {
            Some(TokenKind::SemiAmp) => {
                self.advance();
                CaseTerminator::FallThrough
            }
            Some(TokenKind::DoubleSemiAmp) => {
                self.advance();
                CaseTerminator::Continue
            }
            Some(TokenKind::DoubleSemicolon) => {
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
            commands.push(self.parse_command_list_required()?);
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
        Ok(CompoundCommand::Subshell(commands))
    }

    /// Parse a brace group
    fn parse_brace_group(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        self.advance(); // consume '{'
        self.skip_newlines()?;

        let mut commands = Vec::new();
        while !matches!(self.current_token_kind, Some(TokenKind::RightBrace) | None) {
            self.skip_newlines()?;
            if self.at(TokenKind::RightBrace) {
                break;
            }
            commands.push(self.parse_command_list_required()?);
        }

        if !self.at(TokenKind::RightBrace) {
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
        self.ensure_bash_or_mksh("[[ ]] conditionals")?;
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
                ConditionalExpr::Word(self.parse_conditional_operand_word()?)
            };

            return Ok(ConditionalExpr::Unary(ConditionalUnaryExpr {
                op,
                op_span,
                expr: Box::new(expr),
            }));
        }

        if self.at(TokenKind::DoubleLeftParen) {
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

        let Some(word) = self
            .current_word()
            .or_else(|| self.current_conditional_literal_word())
        else {
            return Err(self.error("expected conditional operand"));
        };
        self.advance();
        Ok(word)
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
                let gap = self.input[prev_end.offset..self.current_span.start.offset].to_string();
                if !gap.is_empty() {
                    parts.push(WordPartNode::new(
                        WordPart::Literal(self.literal_text(gap, gap_span.start, gap_span.end)),
                        gap_span,
                    ));
                    composite = true;
                }
            }

            match self.current_token_kind {
                Some(TokenKind::Word | TokenKind::LiteralWord | TokenKind::QuotedWord) => {
                    let word = self
                        .current_word()
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
                    previous_end = Some(self.current_span.end);
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    if start.is_none() {
                        start = Some(self.current_span.start);
                    }
                    end = Some(self.current_span.end);
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

        Ok(Word {
            parts,
            span: Span::from_positions(start, end),
        })
    }

    fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        self.ensure_bash_or_mksh("arithmetic commands")?;
        let left_paren_span = self.current_span;
        self.advance(); // consume '(('

        let mut depth = 0_i32;
        let right_paren_span = loop {
            match self.current_token_kind {
                Some(TokenKind::DoubleLeftParen) => {
                    depth += 2;
                    self.advance();
                }
                Some(TokenKind::LeftParen) => {
                    depth += 1;
                    self.advance();
                }
                Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                    depth += 1;
                    self.advance();
                }
                Some(TokenKind::DoubleRightParen) => {
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
                Some(TokenKind::RightParen) => {
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

    fn parse_function_body_command(&mut self) -> Result<Command> {
        let compound = match self.current_token_kind {
            Some(TokenKind::LeftBrace) => self.parse_brace_group()?,
            Some(TokenKind::LeftParen) => self.parse_subshell()?,
            _ => {
                return Err(Error::parse(
                    "expected '{' or '(' for function body".to_string(),
                ));
            }
        };
        let redirects = self.parse_trailing_redirects();
        Ok(Command::Compound(compound, redirects))
    }

    /// Parse function definition with 'function' keyword: function name { body }
    fn parse_function_keyword(&mut self) -> Result<Command> {
        self.ensure_bash_or_mksh("function keyword definitions")?;
        let start_span = self.current_span;
        self.advance(); // consume 'function'
        self.skip_newlines()?;

        // Get function name
        let Some(name_text) = self
            .at(TokenKind::Word)
            .then(|| self.current_source_like_word_text())
            .flatten()
        else {
            return Err(self.error("expected function name"));
        };
        let (name, name_span) = (Name::from(name_text.as_ref()), self.current_span);
        self.advance();
        self.skip_newlines()?;

        // Optional () after name
        if self.at(TokenKind::LeftParen) {
            self.advance(); // consume '('
            if !self.at(TokenKind::RightParen) {
                return Err(Error::parse(
                    "expected ')' in function definition".to_string(),
                ));
            }
            self.advance(); // consume ')'
            self.skip_newlines()?;
        }

        let body = self.parse_function_body_command()?;

        Ok(Command::Function(FunctionDef {
            name,
            name_span,
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse POSIX-style function definition: name() { body }
    fn parse_function_posix(&mut self) -> Result<Command> {
        let start_span = self.current_span;
        // Get function name
        let Some(name_text) = self
            .at(TokenKind::Word)
            .then(|| self.current_source_like_word_text())
            .flatten()
        else {
            return Err(self.error("expected function name"));
        };
        let (name, name_span) = (Name::from(name_text.as_ref()), self.current_span);
        self.advance();

        // Consume ()
        if !self.at(TokenKind::LeftParen) {
            return Err(self.error("expected '(' in function definition"));
        }
        self.advance(); // consume '('

        if !self.at(TokenKind::RightParen) {
            return Err(self.error("expected ')' in function definition"));
        }
        self.advance(); // consume ')'
        self.skip_newlines()?;

        let body = self.parse_function_body_command()?;

        Ok(Command::Function(FunctionDef {
            name,
            name_span,
            body: Box::new(body),
            span: start_span.merge(self.current_span),
        }))
    }

    /// Parse commands until a terminating keyword
    fn parse_compound_list(&mut self, terminator: Keyword) -> Result<Vec<Command>> {
        self.parse_compound_list_until(KeywordSet::single(terminator))
    }

    /// Parse commands until one of the terminating keywords
    fn parse_compound_list_until(&mut self, terminators: KeywordSet) -> Result<Vec<Command>> {
        let mut commands = Vec::with_capacity(4);

        loop {
            self.skip_newlines()?;

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

            let command = self.parse_command_list_required()?;
            self.apply_command_effects(&command);
            commands.push(command);
        }

        Ok(commands)
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

    fn scan_split_indexed_assignment(&self, start: Position) -> Option<(String, Position)> {
        if start.offset >= self.input.len() {
            return None;
        }

        let source = &self.input[start.offset..];
        let mut chars = source.chars().peekable();
        let mut cursor = start;
        let mut text = String::new();

        let first = *chars.peek()?;
        if !first.is_ascii_alphabetic() && first != '_' {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
        text.push_str(&Self::read_word_while(&mut chars, &mut cursor, |c| {
            c.is_ascii_alphanumeric() || c == '_'
        }));

        if chars.peek() != Some(&'[') {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));

        let mut bracket_depth = 1_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(ch) = Self::next_word_char(&mut chars, &mut cursor) {
            text.push(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double => {
                    bracket_depth -= 1;
                    if bracket_depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        if bracket_depth != 0 {
            return None;
        }

        if chars.peek() == Some(&'+') {
            text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));
        }

        if chars.peek() != Some(&'=') {
            return None;
        }
        text.push(Self::next_word_char_unwrap(&mut chars, &mut cursor));

        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(&ch) = chars.peek() {
            if !in_single
                && !in_double
                && paren_depth == 0
                && brace_depth == 0
                && matches!(ch, ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | ')')
            {
                break;
            }

            let ch = Self::next_word_char_unwrap(&mut chars, &mut cursor);
            text.push(ch);

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }
        }

        Some((text, cursor))
    }

    fn try_parse_split_indexed_assignment(&mut self) -> Option<Assignment> {
        if !self.at(TokenKind::Word) {
            return None;
        }
        let word = self.current_source_like_word_text()?;
        if !word.contains('[') || Self::is_assignment(&word).is_some() {
            return None;
        }

        let start = self.current_span.start;
        let (text, end) = self.scan_split_indexed_assignment(start)?;
        let span = Span::from_positions(start, end);
        let assignment = self.parse_assignment_from_text(&text, span)?;

        while self.current_token.is_some() && self.current_span.start.offset < end.offset {
            self.advance();
        }

        Some(assignment)
    }

    /// Parse a simple command with redirections
    /// Collect array elements between `(` and `)` tokens into a `Vec<Word>`.
    fn collect_array_elements(&mut self) -> (Vec<Word>, Span) {
        let mut elements = Vec::new();
        let mut closing_span = Span::new();
        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    if let Some(word) = self.current_word() {
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

        while let Some(token) = lexer.next_lexed_token() {
            if token.kind.is_word_like() {
                let span = token.span.rebased(base);
                if let Some(word) = self.word_from_token(&token, span) {
                    elements.push(word);
                }
            }
        }

        elements
    }

    fn raw_value_is_fully_quoted(raw: &str) -> bool {
        raw.len() >= 2
            && ((raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\'')))
    }

    fn trim_literal_prefix(
        &self,
        literal: LiteralText,
        span: Span,
        start: Position,
    ) -> Option<(LiteralText, Span)> {
        if start.offset <= span.start.offset {
            return Some((literal, span));
        }
        if start.offset >= span.end.offset {
            return None;
        }

        let trimmed_span = Span::from_positions(start, span.end);
        let literal = match literal {
            LiteralText::Source => LiteralText::source(),
            LiteralText::Owned(text) => {
                let split_at = start.offset.saturating_sub(span.start.offset);
                LiteralText::owned(text.get(split_at..)?.to_string())
            }
        };
        Some((literal, trimmed_span))
    }

    fn trim_word_part_prefix(
        &self,
        part: WordPart,
        span: Span,
        start: Position,
    ) -> Option<(WordPart, Span)> {
        if start.offset <= span.start.offset {
            return Some((part, span));
        }
        if start.offset >= span.end.offset {
            return None;
        }

        match part {
            WordPart::Literal(literal) => self
                .trim_literal_prefix(literal, span, start)
                .map(|(literal, span)| (WordPart::Literal(literal), span)),
            _ => None,
        }
    }

    fn split_word_at(&self, word: Word, start: Position, _quoted: bool) -> Word {
        let value_span = Span::from_positions(start, word.span.end);
        let mut parts = Vec::new();

        for part in word.parts {
            if let Some((kind, span)) = self.trim_word_part_prefix(part.kind, part.span, start) {
                parts.push(WordPartNode::new(kind, span));
            }
        }

        Word {
            parts,
            span: value_span,
        }
    }

    fn decode_assignment_value_from_token(
        &mut self,
        token: &LexedToken<'_>,
        value_start: Position,
        _quoted: bool,
    ) -> Option<Word> {
        let lexed = token.word()?;
        let value_span = Span::from_positions(value_start, token.span.end);
        let mut parts = Vec::new();
        let mut cursor = token.span.start;

        for segment in lexed.segments() {
            let text = segment.as_str();
            let mut content_span = if let Some(segment_span) = segment.span() {
                cursor = segment_span.end;
                segment_span
            } else {
                let start = cursor;
                let end = start.advanced_by(text);
                cursor = end;
                Span::from_positions(start, end)
            };
            let wrapper_span = segment.wrapper_span().unwrap_or(content_span);
            if wrapper_span.end.offset <= value_start.offset {
                continue;
            }

            let mut text = text;
            if content_span.start.offset < value_start.offset {
                let split_at = value_start.offset.saturating_sub(content_span.start.offset);
                text = text.get(split_at..)?;
                content_span = Span::from_positions(value_start, content_span.end);
            }

            match segment.kind() {
                LexedWordSegmentKind::SingleQuoted => parts.push(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, false),
                ),
                LexedWordSegmentKind::DollarSingleQuoted => parts.push(
                    self.single_quoted_part_from_text(text, content_span, wrapper_span, true),
                ),
                LexedWordSegmentKind::Plain => {
                    if Self::word_text_needs_parse(text) {
                        self.decode_word_parts_into(
                            text,
                            content_span.start,
                            segment.span().is_some(),
                            &mut parts,
                        );
                    } else {
                        parts.push(self.literal_part_from_text(text, content_span, true));
                    }
                }
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted => {
                    if Self::word_text_needs_parse(text) {
                        let inner = self.decode_word_text(
                            text,
                            content_span,
                            content_span.start,
                            segment.span().is_some(),
                        );
                        parts.push(WordPartNode::new(
                            WordPart::DoubleQuoted {
                                parts: inner.parts,
                                dollar: matches!(
                                    segment.kind(),
                                    LexedWordSegmentKind::DollarDoubleQuoted
                                ),
                            },
                            wrapper_span,
                        ));
                    } else {
                        parts.push(self.double_quoted_literal_part_from_text(
                            text,
                            content_span,
                            wrapper_span,
                            true,
                            matches!(segment.kind(), LexedWordSegmentKind::DollarDoubleQuoted),
                        ));
                    }
                }
                LexedWordSegmentKind::Composite => return None,
            }
        }

        Some(Word {
            parts,
            span: value_span,
        })
    }

    fn parse_assignment_from_current_token(&mut self, raw: &str) -> Option<Assignment> {
        let token = self.current_token.take()?;
        let assignment = (|| {
            let assignment_span = token.span;
            let (name, index, value, is_append) = Self::is_assignment(raw)?;
            let name_span = Span::from_positions(
                assignment_span.start,
                assignment_span.start.advanced_by(name),
            );
            let index_span = index.map(|index| {
                let start = name_span.end.advanced_by("[");
                Span::from_positions(start, start.advanced_by(index))
            });
            let value_start_offset = if let Some(pos) = raw.find("+=") {
                pos + 2
            } else {
                raw.find('=')? + 1
            };
            let value_start = assignment_span
                .start
                .advanced_by(&raw[..value_start_offset]);
            let value_span = Span::from_positions(value_start, assignment_span.end);
            let name = Name::from(name);
            let index = index
                .zip(index_span)
                .map(|(index, span)| self.source_text(index.to_string(), span.start, span.end));
            let index_ast = index
                .as_ref()
                .and_then(|index| self.maybe_parse_source_text_as_arithmetic(index));

            let value = if value.starts_with('(') && value.ends_with(')') {
                let inner = &value[1..value.len() - 1];
                AssignmentValue::Array(
                    self.parse_array_words_from_text(inner, value_start.advanced_by("(")),
                )
            } else if value.is_empty() {
                AssignmentValue::Scalar(Word::literal_with_span("", value_span))
            } else {
                AssignmentValue::Scalar(self.decode_assignment_value_from_token(
                    &token,
                    value_start,
                    Self::raw_value_is_fully_quoted(value),
                )?)
            };

            Some(Assignment {
                name,
                name_span,
                index,
                index_ast,
                value,
                append: is_append,
                span: assignment_span,
            })
        })();
        self.current_token = Some(token);
        assignment
    }

    fn parse_assignment_from_word(&mut self, word: Word, raw: &str) -> Option<Assignment> {
        let assignment_span = word.span;
        let (name, index, value, is_append) = Self::is_assignment(raw)?;
        let name_span = Span::from_positions(
            assignment_span.start,
            assignment_span.start.advanced_by(name),
        );
        let index_span = index.map(|index| {
            let start = name_span.end.advanced_by("[");
            Span::from_positions(start, start.advanced_by(index))
        });
        let value_start_offset = if let Some(pos) = raw.find("+=") {
            pos + 2
        } else {
            raw.find('=')? + 1
        };
        let value_start = assignment_span
            .start
            .advanced_by(&raw[..value_start_offset]);
        let name = Name::from(name);
        let index = index
            .zip(index_span)
            .map(|(index, span)| self.source_text(index.to_string(), span.start, span.end));
        let index_ast = index
            .as_ref()
            .and_then(|index| self.maybe_parse_source_text_as_arithmetic(index));

        let value = if value.starts_with('(') && value.ends_with(')') {
            let inner = &value[1..value.len() - 1];
            AssignmentValue::Array(
                self.parse_array_words_from_text(inner, value_start.advanced_by("(")),
            )
        } else if value.is_empty() {
            let value_span = Span::from_positions(value_start, assignment_span.end);
            AssignmentValue::Scalar(Word::literal_with_span("", value_span))
        } else {
            let scalar =
                self.split_word_at(word, value_start, Self::raw_value_is_fully_quoted(value));
            if scalar.parts.is_empty() {
                return self.parse_assignment_from_text(raw, assignment_span);
            }
            AssignmentValue::Scalar(scalar)
        };

        Some(Assignment {
            name,
            name_span,
            index,
            index_ast,
            value,
            append: is_append,
            span: assignment_span,
        })
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
        let index_ast = index
            .as_ref()
            .and_then(|index| self.maybe_parse_source_text_as_arithmetic(index));
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
            let content_start = value_start.advanced_by("\"");
            let content_span =
                Span::from_positions(content_start, content_start.advanced_by(inner));
            let inner = self.decode_word_text(inner, content_span, content_start, true);
            AssignmentValue::Scalar(Word {
                parts: vec![WordPartNode::new(
                    WordPart::DoubleQuoted {
                        parts: inner.parts,
                        dollar: false,
                    },
                    value_span,
                )],
                span: value_span,
            })
        } else if value_str.starts_with('\'') && value_str.ends_with('\'') {
            let inner = Self::strip_quotes(&value_str);
            let content_start = value_start.advanced_by("'");
            let content_span =
                Span::from_positions(content_start, content_start.advanced_by(inner));
            AssignmentValue::Scalar(Word {
                parts: vec![self.single_quoted_part_from_text(
                    inner,
                    content_span,
                    value_span,
                    false,
                )],
                span: value_span,
            })
        } else {
            AssignmentValue::Scalar(self.decode_word_text(
                &value_str,
                value_span,
                value_start,
                false,
            ))
        };

        Some(Assignment {
            name,
            name_span,
            index,
            index_ast,
            value,
            append: is_append,
            span: assignment_span,
        })
    }

    fn parse_decl_name_from_text(&self, word: &str, span: Span) -> Option<DeclName> {
        if let Some(bracket_pos) = word.find('[') {
            let name = &word[..bracket_pos];
            if !Self::is_valid_identifier(name) || !word.ends_with(']') {
                return None;
            }

            let index = &word[bracket_pos + 1..word.len() - 1];
            let name_span = Span::from_positions(span.start, span.start.advanced_by(name));
            let index_start = name_span.end.advanced_by("[");
            let index_span = Span::from_positions(index_start, index_start.advanced_by(index));

            let index = SourceText::source(index_span);
            return Some(DeclName {
                name: Name::from(name),
                name_span,
                index_ast: self.maybe_parse_source_text_as_arithmetic(&index),
                index: Some(index),
                span,
            });
        }

        if !Self::is_valid_identifier(word) {
            return None;
        }

        Some(DeclName {
            name: Name::from(word),
            name_span: span,
            index_ast: None,
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
        let rendered = word.render(self.input);
        if word.span.start.offset <= word.span.end.offset
            && word.span.end.offset <= self.input.len()
        {
            let source = &self.input[word.span.start.offset..word.span.end.offset];
            if rendered == source {
                return source.to_string();
            }
        }
        rendered
    }

    fn is_literal_flag_word(word: &Word, raw: &str) -> bool {
        if Self::is_fully_quoted_word(word) || raw.contains('=') {
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
            [part] if match &part.kind {
                WordPart::Literal(value) => match value {
                    LiteralText::Source => true,
                    LiteralText::Owned(value) => value.as_ref() == raw,
                },
                _ => false,
            }
        )
    }

    fn classify_decl_operand(&mut self, word: Word) -> DeclOperand {
        let raw = self.word_source_text(&word);

        if Self::is_literal_flag_word(&word, &raw) {
            return DeclOperand::Flag(word);
        }

        if let Some(assignment) = self.parse_assignment_from_word(word.clone(), &raw) {
            return DeclOperand::Assignment(assignment);
        }

        if let Some(name) = self.parse_decl_name_from_text(&raw, word.span) {
            return DeclOperand::Name(name);
        }

        DeclOperand::Dynamic(word)
    }

    /// Parse the value side of an assignment (`VAR=value`).
    /// Returns `Some((Assignment, needs_advance))` if the current word is an assignment.
    /// The bool indicates whether the caller must call `self.advance()` afterward.
    fn try_parse_assignment(&mut self, raw: &str) -> Option<(Assignment, bool)> {
        let (_, _, value_str, _) = Self::is_assignment(raw)?;

        // Empty value — check for arr=(...) syntax with separate tokens
        if value_str.is_empty() {
            let assignment_span = self.current_span;
            let (name, index, _, is_append) = Self::is_assignment(raw)?;
            let name_span = Span::from_positions(
                assignment_span.start,
                assignment_span.start.advanced_by(name),
            );
            let index_span = index.map(|index| {
                let start = name_span.end.advanced_by("[");
                Span::from_positions(start, start.advanced_by(index))
            });
            self.advance();
            if self.at(TokenKind::LeftParen) {
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
                        index_ast: index
                            .zip(index_span)
                            .map(|(index, span)| {
                                self.source_text(index.to_string(), span.start, span.end)
                            })
                            .as_ref()
                            .and_then(|index| self.maybe_parse_source_text_as_arithmetic(index)),
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
            let value_start_offset = if let Some(pos) = raw.find("+=") {
                pos + 2
            } else {
                raw.find('=')? + 1
            };
            let value_span = Span::from_positions(
                assignment_span
                    .start
                    .advanced_by(&raw[..value_start_offset]),
                assignment_span.end,
            );
            return Some((
                Assignment {
                    name: Name::from(name),
                    name_span,
                    index: index.zip(index_span).map(|(index, span)| {
                        self.source_text(index.to_string(), span.start, span.end)
                    }),
                    index_ast: index
                        .zip(index_span)
                        .map(|(index, span)| {
                            self.source_text(index.to_string(), span.start, span.end)
                        })
                        .as_ref()
                        .and_then(|index| self.maybe_parse_source_text_as_arithmetic(index)),
                    value: AssignmentValue::Scalar(Word::literal_with_span("", value_span)),
                    append: is_append,
                    span: assignment_span,
                },
                false,
            ));
        }

        self.parse_assignment_from_current_token(raw)
            .or_else(|| {
                self.current_word()
                    .and_then(|word| self.parse_assignment_from_word(word, raw))
            })
            .map(|assignment| (assignment, true))
    }

    /// Parse a compound array argument in arg position (e.g. `declare -a arr=(x y z)`).
    /// Called when the current word ends with `=` and the next token is `(`.
    /// Returns the compound word if successful, or `None` if not a compound assignment.
    fn try_parse_compound_array_arg(&mut self, saved_w: String, saved_span: Span) -> Option<Word> {
        if !self.at(TokenKind::LeftParen) {
            return None;
        }

        self.advance(); // consume '('
        let mut compound = saved_w;
        let mut closing_span = Span::new();
        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    let elem = self.current_source_like_word_text().unwrap();
                    compound.push(' ');
                    compound.push_str(&elem);
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
            return Some(self.decode_word_text(source, span, saved_span.start, true));
        }

        Some(self.decode_word_text(&compound, span, saved_span.start, false))
    }

    /// Parse a heredoc redirect (`<<` or `<<-`) and any trailing redirects on the same line.
    fn parse_heredoc_redirect(
        &mut self,
        strip_tabs: bool,
        redirects: &mut Vec<Redirect>,
    ) -> Result<()> {
        self.consume_heredoc_redirect(strip_tabs, redirects, None, None, true, true)?;
        Ok(())
    }

    /// Consume redirect tokens that follow a heredoc on the same line.
    fn collect_trailing_redirects(&mut self, redirects: &mut Vec<Redirect>) -> Result<()> {
        while self.consume_non_heredoc_redirect(redirects, None, None, false)? {}
        Ok(())
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
            match self.current_token_kind {
                Some(kind) if kind.is_word_like() => {
                    let is_literal = kind == TokenKind::LiteralWord;
                    let word_text = self.current_source_like_word_text().unwrap();

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
                        && let Some((assignment, needs_advance)) =
                            self.try_parse_assignment(word_text.as_ref())
                    {
                        if needs_advance {
                            self.advance();
                        }
                        assignments.push(assignment);
                        continue;
                    }

                    if words.is_empty()
                        && !is_literal
                        && let Some(assignment) = self.try_parse_split_indexed_assignment()
                    {
                        assignments.push(assignment);
                        continue;
                    }

                    // Handle compound array assignment in arg position:
                    // declare -a arr=(x y z) → arr=(x y z) as single arg
                    if word_text.ends_with('=') && !words.is_empty() {
                        let original_word = self.current_word();
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

                    if let Some(word) = self.current_word() {
                        words.push(word);
                    }
                    self.advance();
                }
                Some(kind) if Self::is_redirect_kind(kind) => {
                    if matches!(kind, TokenKind::HereDoc | TokenKind::HereDocStrip) {
                        self.parse_heredoc_redirect(
                            kind == TokenKind::HereDocStrip,
                            &mut redirects,
                        )?;
                        continue;
                    }

                    let (fd_var, fd_var_span) = if Self::redirect_supports_fd_var(kind) {
                        self.pop_fd_var(&mut words)
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
                // { and } as arguments (not in command position) are literal words
                Some(TokenKind::LeftBrace) | Some(TokenKind::RightBrace) if !words.is_empty() => {
                    let sym = if self.at(TokenKind::LeftBrace) {
                        "{"
                    } else {
                        "}"
                    };
                    words.push(Word::literal_with_span(sym, self.current_span));
                    self.advance();
                }
                Some(TokenKind::Newline)
                | Some(TokenKind::Semicolon)
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

    /// Expect a word token and return it as a Word
    fn expect_word(&mut self) -> Result<Word> {
        match self.current_token_kind {
            Some(kind) if kind.is_word_like() => {
                let word = self
                    .current_word()
                    .ok_or_else(|| self.error("expected word"))?;
                self.advance();
                Ok(word)
            }
            Some(TokenKind::ProcessSubIn) | Some(TokenKind::ProcessSubOut) => {
                // Process substitution <(cmd) or >(cmd)
                let is_input = self.at(TokenKind::ProcessSubIn);
                let process_span = self.current_span;
                self.advance();

                // Walk tokens until the matching closing paren, then reparse the original
                // source slice so nested command spans remain absolute.
                let mut depth = 1;
                let close_span = loop {
                    match self.current_token_kind {
                        Some(TokenKind::LeftParen) => {
                            depth += 1;
                            self.advance();
                        }
                        Some(TokenKind::RightParen) => {
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
                    parts: vec![WordPartNode::new(
                        WordPart::ProcessSubstitution { commands, is_input },
                        process_span.merge(close_span),
                    )],
                    span: process_span.merge(close_span),
                })
            }
            _ => Err(self.error("expected word")),
        }
    }

    fn decode_word_parts_into(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        parts: &mut Vec<WordPartNode>,
    ) {
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

            if ch == '`' {
                self.flush_literal_part(parts, &mut current, current_start, part_start);

                let inner_start = cursor;
                let commands = if source_backed {
                    let mut inner_end = inner_start;
                    let mut escaped = false;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if escaped {
                            escaped = false;
                            inner_end = cursor;
                            continue;
                        }

                        match c {
                            '\\' => {
                                escaped = true;
                                inner_end = cursor;
                            }
                            '`' => break,
                            _ => inner_end = cursor,
                        }
                    }
                    self.nested_commands_from_current_input(inner_start, inner_end)
                } else {
                    let mut cmd_str = String::new();
                    let mut escaped = false;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        if escaped {
                            escaped = false;
                            cmd_str.push(c);
                            continue;
                        }

                        match c {
                            '\\' => {
                                escaped = true;
                                cmd_str.push(c);
                            }
                            '`' => break,
                            _ => cmd_str.push(c),
                        }
                    }
                    self.nested_commands_from_source(&cmd_str, inner_start)
                };

                Self::push_word_part(
                    parts,
                    WordPart::CommandSubstitution {
                        commands,
                        syntax: CommandSubstitutionSyntax::Backtick,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if ch != '$' {
                if current.is_empty() {
                    current_start = part_start;
                }
                current.push(ch);
                continue;
            }

            self.flush_literal_part(parts, &mut current, current_start, part_start);

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
                    parts,
                    WordPart::SingleQuoted {
                        value: self.source_text(ansi, part_start, cursor),
                        dollar: true,
                    },
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
                    let expression = if source_backed {
                        let mut depth = 2;
                        let mut expr_end = expr_start;
                        while chars.peek().is_some() {
                            let c = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match c {
                                '(' => {
                                    depth += 1;
                                    expr_end = cursor;
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 1 {
                                        continue;
                                    }
                                    if depth == 0 {
                                        break;
                                    }
                                    expr_end = cursor;
                                }
                                _ => expr_end = cursor,
                            }
                        }
                        SourceText::source(Span::from_positions(expr_start, expr_end))
                    } else {
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
                        let expr_end = expr_start.advanced_by(&expr);
                        self.source_text(expr, expr_start, expr_end)
                    };
                    Self::push_word_part(
                        parts,
                        WordPart::ArithmeticExpansion {
                            expression_ast: self.parse_source_text_as_arithmetic(&expression).ok(),
                            expression,
                            syntax: ArithmeticExpansionSyntax::DollarParenParen,
                        },
                        part_start,
                        cursor,
                    );
                } else {
                    let inner_start = cursor;
                    let commands = if source_backed {
                        let mut depth = 1;
                        let mut inner_end = inner_start;
                        while chars.peek().is_some() {
                            let c = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            match c {
                                '(' => {
                                    depth += 1;
                                    inner_end = cursor;
                                }
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                    inner_end = cursor;
                                }
                                _ => inner_end = cursor,
                            }
                        }
                        self.nested_commands_from_current_input(inner_start, inner_end)
                    } else {
                        let mut cmd_str = String::new();
                        let mut depth = 1;
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
                        self.nested_commands_from_source(&cmd_str, inner_start)
                    };
                    Self::push_word_part(
                        parts,
                        WordPart::CommandSubstitution {
                            commands,
                            syntax: CommandSubstitutionSyntax::DollarParen,
                        },
                        part_start,
                        cursor,
                    );
                }
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'[') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let expr_start = cursor;
                let expression = if source_backed {
                    let mut bracket_depth = 1_i32;
                    let mut expr_end = expr_start;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        match c {
                            '[' => {
                                bracket_depth += 1;
                                expr_end = cursor;
                            }
                            ']' => {
                                bracket_depth -= 1;
                                if bracket_depth == 0 {
                                    break;
                                }
                                expr_end = cursor;
                            }
                            _ => expr_end = cursor,
                        }
                    }
                    SourceText::source(Span::from_positions(expr_start, expr_end))
                } else {
                    let mut expr = String::new();
                    let mut bracket_depth = 1_i32;
                    while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                        match c {
                            '[' => {
                                bracket_depth += 1;
                                expr.push(c);
                            }
                            ']' => {
                                bracket_depth -= 1;
                                if bracket_depth == 0 {
                                    break;
                                }
                                expr.push(c);
                            }
                            _ => expr.push(c),
                        }
                    }
                    let expr_end = expr_start.advanced_by(&expr);
                    self.source_text(expr, expr_start, expr_end)
                };
                Self::push_word_part(
                    parts,
                    WordPart::ArithmeticExpansion {
                        expression_ast: self.parse_source_text_as_arithmetic(&expression).ok(),
                        expression,
                        syntax: ArithmeticExpansionSyntax::LegacyBracket,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if chars.peek() == Some(&'{') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);

                if Self::consume_word_char_if(&mut chars, &mut cursor, '#') {
                    let var_name =
                        Self::read_word_while(&mut chars, &mut cursor, |c| c != '}' && c != '[');
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let index = self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = if matches!(index.slice(self.input), "@" | "*") {
                            WordPart::ArrayLength(var_name.into())
                        } else {
                            WordPart::Length(
                                format!("{}[{}]", var_name, index.slice(self.input)).into(),
                            )
                        };
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else {
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        Self::push_word_part(
                            parts,
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
                        let index = self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = if matches!(index.slice(self.input), "@" | "*") {
                            WordPart::ArrayIndices(var_name.into())
                        } else {
                            WordPart::Variable(
                                format!("!{}[{}]", var_name, index.slice(self.input)).into(),
                            )
                        };
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        Self::push_word_part(
                            parts,
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
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            Self::push_word_part(
                                parts,
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
                                parts,
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
                        let operand =
                            self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                        let operator = match op_char {
                            '-' => ParameterOp::UseDefault,
                            '=' => ParameterOp::AssignDefault,
                            '+' => ParameterOp::UseReplacement,
                            '?' => ParameterOp::Error,
                            _ => unreachable!(),
                        };
                        Self::push_word_part(
                            parts,
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
                        Self::push_word_part(parts, part, part_start, cursor);
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
                    let index = self.read_array_index(&mut chars, &mut cursor, source_backed);

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
                                let operand =
                                    self.read_brace_operand(&mut chars, &mut cursor, source_backed);
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
                                let offset = self.read_source_text_while(
                                    &mut chars,
                                    &mut cursor,
                                    |c| c != ':' && c != '}',
                                    source_backed,
                                );
                                let length =
                                    if Self::consume_word_char_if(&mut chars, &mut cursor, ':') {
                                        Some(self.read_source_text_while(
                                            &mut chars,
                                            &mut cursor,
                                            |c| c != '}',
                                            source_backed,
                                        ))
                                    } else {
                                        None
                                    };
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                let offset_ast =
                                    self.maybe_parse_source_text_as_arithmetic(&offset);
                                let length_ast = length.as_ref().and_then(|length| {
                                    self.maybe_parse_source_text_as_arithmetic(length)
                                });
                                WordPart::ArraySlice {
                                    name: var_name.into(),
                                    offset,
                                    offset_ast,
                                    length,
                                    length_ast,
                                }
                            }
                        } else if matches!(next_c, '-' | '+' | '=' | '?') {
                            let arr_name = format!("{}[{}]", var_name, index.slice(self.input));
                            let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
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
                            let index_ast = self.maybe_parse_source_text_as_arithmetic(&index);
                            WordPart::ArrayAccess {
                                name: var_name.into(),
                                index,
                                index_ast,
                            }
                        }
                    } else {
                        let index_ast = self.maybe_parse_source_text_as_arithmetic(&index);
                        WordPart::ArrayAccess {
                            name: var_name.into(),
                            index,
                            index_ast,
                        }
                    };

                    Self::push_word_part(parts, part, part_start, cursor);
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
                                    let operand = self.read_brace_operand(
                                        &mut chars,
                                        &mut cursor,
                                        source_backed,
                                    );
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
                                        source_backed,
                                    );
                                    let length =
                                        if Self::consume_word_char_if(&mut chars, &mut cursor, ':')
                                        {
                                            Some(self.read_source_text_while(
                                                &mut chars,
                                                &mut cursor,
                                                |ch| ch != '}',
                                                source_backed,
                                            ))
                                        } else {
                                            None
                                        };
                                    Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                    let offset_ast =
                                        self.maybe_parse_source_text_as_arithmetic(&offset);
                                    let length_ast = length.as_ref().and_then(|length| {
                                        self.maybe_parse_source_text_as_arithmetic(length)
                                    });
                                    WordPart::Substring {
                                        name: var_name.into(),
                                        offset,
                                        offset_ast,
                                        length,
                                        length_ast,
                                    }
                                }
                            }
                        }
                        '-' | '=' | '+' | '?' => {
                            let op_char = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
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
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
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
                            let operand =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
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
                            let pattern = self.read_replacement_pattern(
                                &mut chars,
                                &mut cursor,
                                source_backed,
                            );
                            let replacement =
                                if Self::consume_word_char_if(&mut chars, &mut cursor, '/') {
                                    self.read_source_text_while(
                                        &mut chars,
                                        &mut cursor,
                                        |ch| ch != '}',
                                        source_backed,
                                    )
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

                Self::push_word_part(parts, part, part_start, cursor);
                current_start = cursor;
                continue;
            }

            if let Some(&c) = chars.peek() {
                if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-') || c.is_ascii_digit() {
                    let name = Self::next_word_char_unwrap(&mut chars, &mut cursor).to_string();
                    Self::push_word_part(
                        parts,
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
                            parts,
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

        self.flush_literal_part(parts, &mut current, current_start, cursor);

        if parts.is_empty() {
            Self::push_word_part(
                parts,
                WordPart::Literal(self.literal_text(String::new(), base, cursor)),
                base,
                cursor,
            );
        }
    }

    fn read_array_index(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> SourceText {
        let start = *cursor;
        let mut text = (!source_backed).then(String::new);
        let mut end = *cursor;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;

        while let Some(&c) = chars.peek() {
            if c == ']' && bracket_depth == 0 && brace_depth == 0 {
                end = *cursor;
                Self::next_word_char_unwrap(chars, cursor);
                break;
            }

            match c {
                '[' => bracket_depth += 1,
                ']' => bracket_depth -= 1,
                '$' => {
                    let dollar = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(text) = text.as_mut() {
                        text.push(dollar);
                    }
                    end = *cursor;
                    if chars.peek() == Some(&'{') {
                        brace_depth += 1;
                        let brace = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(text) = text.as_mut() {
                            text.push(brace);
                        }
                        end = *cursor;
                    }
                    continue;
                }
                '{' => brace_depth += 1,
                '}' if brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }

            let ch = Self::next_word_char_unwrap(chars, cursor);
            if let Some(text) = text.as_mut() {
                text.push(ch);
            }
            end = *cursor;
        }

        if source_backed {
            let span = Span::from_positions(start, end);
            let raw = span.slice(self.input);
            if raw.len() >= 2
                && ((raw.starts_with('"') && raw.ends_with('"'))
                    || (raw.starts_with('\'') && raw.ends_with('\'')))
            {
                SourceText::cooked(span, raw[1..raw.len() - 1].to_string())
            } else {
                SourceText::source(span)
            }
        } else {
            let mut text = text.unwrap_or_default();
            if text.len() >= 2
                && ((text.starts_with('"') && text.ends_with('"'))
                    || (text.starts_with('\'') && text.ends_with('\'')))
            {
                text = text[1..text.len() - 1].to_string();
            }
            self.source_text(text, start, end)
        }
    }

    fn read_replacement_pattern(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> SourceText {
        let start = *cursor;

        if source_backed {
            let mut end = *cursor;
            let mut has_escaped_slash = false;

            while let Some(&ch) = chars.peek() {
                if ch == '/' || ch == '}' {
                    end = *cursor;
                    break;
                }

                if ch == '\\' {
                    Self::next_word_char_unwrap(chars, cursor);
                    if let Some(&next) = chars.peek()
                        && next == '/'
                    {
                        has_escaped_slash = true;
                        Self::next_word_char_unwrap(chars, cursor);
                    }
                    end = *cursor;
                    continue;
                }

                Self::next_word_char_unwrap(chars, cursor);
                end = *cursor;
            }

            let span = Span::from_positions(start, end);
            if has_escaped_slash {
                SourceText::cooked(span, span.slice(self.input).replace("\\/", "/"))
            } else {
                SourceText::source(span)
            }
        } else {
            let mut pattern = String::new();
            let mut end = *cursor;
            while let Some(&ch) = chars.peek() {
                if ch == '/' || ch == '}' {
                    end = *cursor;
                    break;
                }
                if ch == '\\' {
                    Self::next_word_char_unwrap(chars, cursor);
                    if let Some(&next) = chars.peek()
                        && next == '/'
                    {
                        pattern.push(Self::next_word_char_unwrap(chars, cursor));
                        end = *cursor;
                        continue;
                    }
                    pattern.push('\\');
                    end = *cursor;
                    continue;
                }
                pattern.push(Self::next_word_char_unwrap(chars, cursor));
                end = *cursor;
            }
            self.source_text(pattern, start, end)
        }
    }

    fn decode_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        let mut parts = Vec::new();
        self.decode_word_parts_into(s, base, source_backed, &mut parts);
        Word { parts, span }
    }

    fn parse_word_with_context(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text(s, span, base, source_backed)
    }

    /// Read operand for brace expansion (everything until closing brace)
    fn read_brace_operand(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
        cursor: &mut Position,
        source_backed: bool,
    ) -> SourceText {
        let start = *cursor;
        let mut depth = 1;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut operand = (!source_backed).then(String::new);

        while let Some(&c) = chars.peek() {
            if escaped {
                let ch = Self::next_word_char_unwrap(chars, cursor);
                if let Some(operand) = operand.as_mut() {
                    operand.push(ch);
                }
                escaped = false;
                continue;
            }

            match c {
                '\\' if !in_single => {
                    escaped = true;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '\'' if !in_double => {
                    in_single = !in_single;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '"' if !in_single => {
                    in_double = !in_double;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                '$' if !in_single => {
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                    if chars.peek() == Some(&'{') {
                        depth += 1;
                        let brace = Self::next_word_char_unwrap(chars, cursor);
                        if let Some(operand) = operand.as_mut() {
                            operand.push(brace);
                        }
                    }
                }
                '}' if !in_single && !in_double => {
                    if depth == 1 {
                        let end = *cursor;
                        Self::next_word_char_unwrap(chars, cursor);
                        return if source_backed {
                            SourceText::source(Span::from_positions(start, end))
                        } else {
                            self.source_text(operand.unwrap_or_default(), start, end)
                        };
                    }
                    depth -= 1;
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
                _ => {
                    let ch = Self::next_word_char_unwrap(chars, cursor);
                    if let Some(operand) = operand.as_mut() {
                        operand.push(ch);
                    }
                }
            }
        }
        if source_backed {
            SourceText::source(Span::from_positions(start, *cursor))
        } else {
            self.source_text(operand.unwrap_or_default(), start, *cursor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_ast::{
        ArithmeticAssignOp, ArithmeticBinaryOp, ArithmeticPostfixOp, ArithmeticUnaryOp,
    };

    fn is_fully_quoted(word: &Word) -> bool {
        Parser::is_fully_quoted_word(word)
    }

    fn top_level_part_slices<'a>(word: &'a Word, input: &'a str) -> Vec<&'a str> {
        word.parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect()
    }

    fn redirect_word_target(redirect: &Redirect) -> &Word {
        redirect
            .word_target()
            .expect("expected non-heredoc redirect target")
    }

    fn redirect_heredoc(redirect: &Redirect) -> &Heredoc {
        redirect.heredoc().expect("expected heredoc redirect")
    }

    fn expect_compound(command: &Command) -> (&CompoundCommand, &[Redirect]) {
        let Command::Compound(compound, redirects) = command else {
            panic!("expected compound command");
        };
        (compound, redirects.as_slice())
    }

    fn expect_variable(expr: &ArithmeticExprNode, expected: &str) {
        let ArithmeticExpr::Variable(name) = &expr.kind else {
            panic!("expected arithmetic variable, got {:?}", expr.kind);
        };
        assert_eq!(name, expected);
    }

    fn expect_number(expr: &ArithmeticExprNode, input: &str, expected: &str) {
        let ArithmeticExpr::Number(number) = &expr.kind else {
            panic!("expected arithmetic number, got {:?}", expr.kind);
        };
        assert_eq!(number.slice(input), expected);
    }

    fn expect_shell_word(expr: &ArithmeticExprNode, input: &str, expected: &str) {
        let ArithmeticExpr::ShellWord(word) = &expr.kind else {
            panic!("expected arithmetic shell word, got {:?}", expr.kind);
        };
        assert_eq!(word.render(input), expected);
    }

    #[test]
    fn test_current_word_cache_tracks_token_changes() {
        let input = "\"$foo\" bar\n";
        let mut parser = Parser::new(input);

        let first = parser.current_word().unwrap();
        assert_eq!(first.render(input), "$foo");
        assert!(is_fully_quoted(&first));
        let [quoted_part] = parser.current_word_cache.as_ref().unwrap().parts.as_slice() else {
            panic!("expected one quoted part");
        };
        let WordPart::DoubleQuoted { parts, .. } = &quoted_part.kind else {
            panic!("expected double-quoted word");
        };
        assert!(matches!(
            parts.as_slice(),
            [part] if matches!(&part.kind, WordPart::Variable(_))
        ));

        let repeated = parser.current_word().unwrap();
        assert_eq!(repeated.span, first.span);

        parser.advance();
        assert!(parser.current_word_cache.is_none());

        let next = parser.current_word().unwrap();
        assert_eq!(next.render(input), "bar");
        assert!(parser.current_word_cache.is_none());
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
        assert_eq!(
            redirect_word_target(&command.redirects[0]).render(input),
            "out.txt"
        );
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

        assert!(is_fully_quoted(&command.name));
        assert_eq!(command.name.render(input), "break");
        assert_eq!(command.args[0].render(input), "2");
    }

    #[test]
    fn test_parse_mixed_literal_word_consumes_segmented_token_directly() {
        let input = "printf foo\"bar\"'baz'";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        let arg = &command.args[0];
        assert!(!is_fully_quoted(arg));
        assert_eq!(arg.render(input), "foobarbaz");
        assert_eq!(arg.parts.len(), 3);
        assert_eq!(arg.part_span(0).unwrap().slice(input), "foo");
        assert_eq!(arg.part_span(1).unwrap().slice(input), "\"bar\"");
        assert_eq!(arg.part_span(2).unwrap().slice(input), "'baz'");
        let WordPart::DoubleQuoted { parts, .. } = &arg.parts[1].kind else {
            panic!("expected double-quoted middle part");
        };
        assert_eq!(parts[0].span.slice(input), "bar");
        let WordPart::SingleQuoted { value, .. } = &arg.parts[2].kind else {
            panic!("expected single-quoted suffix part");
        };
        assert_eq!(value.slice(input), "baz");
    }

    #[test]
    fn test_parse_single_quoted_prefix_word_consumes_segmented_token_directly() {
        let input = "printf 'foo'bar";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        let arg = &command.args[0];
        assert!(!is_fully_quoted(arg));
        assert_eq!(arg.render(input), "foobar");
        assert_eq!(arg.parts.len(), 2);
        assert_eq!(arg.part_span(0).unwrap().slice(input), "'foo'");
        assert_eq!(arg.part_span(1).unwrap().slice(input), "bar");
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
            assert!(matches!(&cmd.args[0].parts[0].kind, WordPart::Variable(v) if v == "HOME"));
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
    fn test_parse_pipe_both_pipeline() {
        let input = "echo hello |& cat";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Pipeline(pipeline) = &script.commands[0] else {
            panic!("expected pipeline");
        };
        assert_eq!(pipeline.commands.len(), 2);

        let Command::Simple(first) = &pipeline.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(first.redirects.len(), 1);
        assert_eq!(first.redirects[0].fd, Some(2));
        assert_eq!(first.redirects[0].kind, RedirectKind::DupOutput);
        assert_eq!(redirect_word_target(&first.redirects[0]).render(input), "1");
    }

    #[test]
    fn test_parse_redirect_out() {
        let input = "echo hello > /tmp/out";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().script;

        if let Command::Simple(cmd) = &script.commands[0] {
            assert_eq!(cmd.redirects.len(), 1);
            assert_eq!(cmd.redirects[0].kind, RedirectKind::Output);
            assert_eq!(
                redirect_word_target(&cmd.redirects[0]).render(input),
                "/tmp/out"
            );
        } else {
            panic!("expected simple command");
        }
    }

    #[test]
    fn test_parse_redirect_both_append() {
        let input = "echo hello &>> /tmp/out";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(cmd) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(cmd.redirects.len(), 2);
        assert_eq!(cmd.redirects[0].kind, RedirectKind::Append);
        assert_eq!(
            redirect_word_target(&cmd.redirects[0]).render(input),
            "/tmp/out"
        );
        assert_eq!(cmd.redirects[1].fd, Some(2));
        assert_eq!(cmd.redirects[1].kind, RedirectKind::DupOutput);
        assert_eq!(redirect_word_target(&cmd.redirects[1]).render(input), "1");
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
    fn test_parse_redirect_read_write() {
        let input = "exec 8<> /tmp/rw";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(cmd) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(cmd.redirects.len(), 1);
        assert_eq!(cmd.redirects[0].fd, Some(8));
        assert_eq!(cmd.redirects[0].kind, RedirectKind::ReadWrite);
        assert_eq!(
            redirect_word_target(&cmd.redirects[0]).render(input),
            "/tmp/rw"
        );
    }

    #[test]
    fn test_parse_named_fd_redirect_read_write() {
        let input = "exec {rw}<> /tmp/rw";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(cmd) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(cmd.redirects.len(), 1);
        assert_eq!(cmd.redirects[0].fd_var.as_deref(), Some("rw"));
        assert_eq!(cmd.redirects[0].kind, RedirectKind::ReadWrite);
        assert_eq!(
            redirect_word_target(&cmd.redirects[0]).render(input),
            "/tmp/rw"
        );
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
    fn test_parse_command_list_preserves_operator_spans() {
        let input = "true && false || echo fallback";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::List(list) = &script.commands[0] else {
            panic!("expected command list");
        };

        assert_eq!(list.rest.len(), 2);
        assert_eq!(list.rest[0].operator_span.slice(input), "&&");
        assert_eq!(list.rest[1].operator_span.slice(input), "||");
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
    fn test_prefix_heredoc_before_command_in_pipeline_parses() {
        let input = "<<EOF tac | tr '\\n' 'X'\none\ntwo\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Pipeline(pipeline) = &script.commands[0] else {
            panic!("expected pipeline");
        };
        assert_eq!(pipeline.commands.len(), 2);
        let Command::Simple(command) = &pipeline.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.name.render(input), "tac");
        assert_eq!(command.redirects.len(), 1);
        assert_eq!(command.redirects[0].kind, RedirectKind::HereDoc);
    }

    #[test]
    fn test_redirect_only_command_parses() {
        let input = ">myfile\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert!(command.name.render(input).is_empty());
        assert_eq!(command.redirects.len(), 1);
        assert_eq!(command.redirects[0].kind, RedirectKind::Output);
        assert_eq!(
            redirect_word_target(&command.redirects[0]).render(input),
            "myfile"
        );
    }

    #[test]
    fn test_function_definition_absorbs_trailing_heredoc_redirect() {
        let input = "f() { cat; } <<EOF\nhello\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Function(function) = &script.commands[0] else {
            panic!("expected function definition");
        };
        let Command::Compound(_, redirects) = function.body.as_ref() else {
            panic!("expected compound function body");
        };
        assert_eq!(redirects.len(), 1);
        assert_eq!(redirects[0].kind, RedirectKind::HereDoc);
    }

    #[test]
    fn test_function_body_command_with_heredoc_parses() {
        let input = "f() {\n  read head << EOF\nref: refs/heads/dev/andy\nEOF\n}\nf\n";
        let script = Parser::new(input).parse().unwrap().script;

        assert_eq!(script.commands.len(), 2);

        let Command::Function(function) = &script.commands[0] else {
            panic!("expected function definition");
        };
        let Command::Compound(CompoundCommand::BraceGroup(body), redirects) =
            function.body.as_ref()
        else {
            panic!("expected brace-group function body");
        };
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_posix_function_allows_subshell_body() {
        let input = "inc_subshell() ( j=$((j+5)); )\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Function(function) = &script.commands[0] else {
            panic!("expected function definition");
        };
        let Command::Compound(CompoundCommand::Subshell(body), redirects) = function.body.as_ref()
        else {
            panic!("expected subshell function body");
        };
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_function_keyword_allows_subshell_body() {
        let input = "function inc_subshell() ( j=$((j+5)); )\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Function(function) = &script.commands[0] else {
            panic!("expected function definition");
        };
        let Command::Compound(CompoundCommand::Subshell(body), redirects) = function.body.as_ref()
        else {
            panic!("expected subshell function body");
        };
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_dynamic_heredoc_delimiter_is_rejected() {
        let parser = Parser::new("cat <<\"$@\"\nbody\n$@\n");
        assert!(
            parser.parse().is_err(),
            "dynamic heredoc delimiter should fail"
        );
    }

    #[test]
    fn test_non_static_heredoc_delimiter_forms_are_rejected() {
        let cases = [
            ("short parameter", "cat <<$bar\n"),
            ("brace parameter", "cat <<${bar}\n"),
            ("command substitution", "cat <<$(bar)\n"),
            ("backquoted command substitution", "cat <<`bar`\n"),
            ("arithmetic expansion", "cat <<$((1 + 2))\n"),
            ("special parameter", "cat <<$-\n"),
            ("quoted parameter expansion", "cat <<\"$bar\"\n"),
        ];

        for (name, input) in cases {
            let error = Parser::new(input).parse().unwrap_err();
            let Error::Parse { message, .. } = error;
            assert_eq!(
                message, "expected static heredoc delimiter",
                "{name} should fail via the static-delimiter check"
            );
        }
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
    fn test_heredoc_multiple_lines_preserve_while_do_boundary() {
        let input =
            "while cat <<E1 && cat <<E2\n1\nE1\n2\nE2\ndo\n  cat <<E3\n3\nE3\n  break\ndone\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        assert!(redirects.is_empty());
        let CompoundCommand::While(command) = compound else {
            panic!("expected while command");
        };
        assert_eq!(command.condition.len(), 1);
        assert_eq!(command.body.len(), 2);
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
        let heredoc = redirect_heredoc(redirect);
        assert_eq!(heredoc.body.span.slice(input), "hello $name\n");
        assert!(is_fully_quoted(&heredoc.body));
    }

    #[test]
    fn test_heredoc_delimiter_metadata_tracks_flags_and_spans() {
        let input = "cat <<EOF\nhello\nEOF\ncat <<'EOF'\nhello\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(unquoted) = &script.commands[0] else {
            panic!("expected first simple command");
        };
        let unquoted_redirect = &unquoted.redirects[0];
        let unquoted_heredoc = redirect_heredoc(unquoted_redirect);
        assert_eq!(unquoted_redirect.span.slice(input), "<<EOF");
        assert_eq!(unquoted_heredoc.delimiter.span.slice(input), "EOF");
        assert_eq!(unquoted_heredoc.delimiter.raw.span.slice(input), "EOF");
        assert_eq!(unquoted_heredoc.delimiter.cooked, "EOF");
        assert!(!unquoted_heredoc.delimiter.quoted);
        assert!(unquoted_heredoc.delimiter.expands_body);
        assert!(!unquoted_heredoc.delimiter.strip_tabs);

        let Command::Simple(quoted) = &script.commands[1] else {
            panic!("expected second simple command");
        };
        let quoted_redirect = &quoted.redirects[0];
        let quoted_heredoc = redirect_heredoc(quoted_redirect);
        assert_eq!(quoted_redirect.span.slice(input), "<<'EOF'");
        assert_eq!(quoted_heredoc.delimiter.span.slice(input), "'EOF'");
        assert_eq!(quoted_heredoc.delimiter.raw.span.slice(input), "'EOF'");
        assert_eq!(quoted_heredoc.delimiter.cooked, "EOF");
        assert!(quoted_heredoc.delimiter.quoted);
        assert!(!quoted_heredoc.delimiter.expands_body);
        assert!(!quoted_heredoc.delimiter.strip_tabs);
    }

    #[test]
    fn test_heredoc_delimiter_preserves_mixed_quoted_raw_and_cooked_value() {
        let input = "cat <<'EOF'\"2\"\nbody\nEOF2\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let redirect = &command.redirects[0];
        let heredoc = redirect_heredoc(redirect);

        assert_eq!(redirect.span.slice(input), "<<'EOF'\"2\"");
        assert_eq!(heredoc.delimiter.raw.span.slice(input), "'EOF'\"2\"");
        assert_eq!(heredoc.delimiter.cooked, "EOF2");
        assert!(heredoc.delimiter.quoted);
        assert!(!heredoc.delimiter.expands_body);
    }

    #[test]
    fn test_backslash_escaped_heredoc_delimiter_is_treated_as_quoted_static_text() {
        let input = "cat <<\\EOF\nhello $name\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let redirect = &command.redirects[0];
        let heredoc = redirect_heredoc(redirect);

        assert_eq!(redirect.span.slice(input), "<<\\EOF");
        assert_eq!(heredoc.delimiter.span.slice(input), "\\EOF");
        assert_eq!(heredoc.delimiter.raw.span.slice(input), "\\EOF");
        assert_eq!(heredoc.delimiter.cooked, "EOF");
        assert!(heredoc.delimiter.quoted);
        assert!(!heredoc.delimiter.expands_body);
        assert!(!heredoc.delimiter.strip_tabs);
        assert!(is_fully_quoted(&heredoc.body));
        assert_eq!(heredoc.body.render(input), "hello $name\n");
    }

    #[test]
    fn test_heredoc_strip_tabs_sets_delimiter_metadata() {
        let input = "cat <<-EOF\n\t$NAME\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let redirect = &command.redirects[0];
        let heredoc = redirect_heredoc(redirect);

        assert_eq!(redirect.span.slice(input), "<<-EOF");
        assert!(heredoc.delimiter.strip_tabs);
        assert!(heredoc.delimiter.expands_body);
        assert_eq!(heredoc.delimiter.cooked, "EOF");
    }

    #[test]
    fn test_heredoc_targets_preserve_quoted_and_unquoted_decode_behavior() {
        let input = "cat <<EOF\nhello $name\nEOF\ncat <<'EOF'\nhello $name\nEOF\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(unquoted) = &script.commands[0] else {
            panic!("expected first simple command");
        };
        let unquoted_target = &redirect_heredoc(&unquoted.redirects[0]).body;
        assert!(!is_fully_quoted(unquoted_target));
        assert_eq!(unquoted_target.render(input), "hello $name\n");
        let unquoted_slices = top_level_part_slices(unquoted_target, input);
        assert_eq!(unquoted_slices, vec!["hello ", "$name", "\n"]);
        assert!(matches!(
            unquoted_target.parts[1].kind,
            WordPart::Variable(_)
        ));

        let Command::Simple(quoted) = &script.commands[1] else {
            panic!("expected second simple command");
        };
        let quoted_target = &redirect_heredoc(&quoted.redirects[0]).body;
        assert!(is_fully_quoted(quoted_target));
        assert_eq!(quoted_target.render(input), "hello $name\n");
        assert!(matches!(
            quoted_target.parts.as_slice(),
            [part] if matches!(&part.kind, WordPart::SingleQuoted { .. })
        ));
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
                    &p.kind,
                    WordPart::ArrayAccess { name, index, .. }
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
    fn test_indexed_assignment_with_spaces_in_subscript_parses() {
        let input = "a[1 + 2]=3\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].name, "a");
        assert_eq!(
            command.assignments[0].index.as_ref().unwrap().slice(input),
            "1 + 2"
        );
        assert!(command.name.render(input).is_empty());
    }

    #[test]
    fn test_parenthesized_indexed_assignment_is_not_function_definition() {
        let input = "a[(1+2)*3]=9\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].name, "a");
        assert_eq!(
            command.assignments[0].index.as_ref().unwrap().slice(input),
            "(1+2)*3"
        );
        assert!(command.name.render(input).is_empty());
    }

    #[test]
    fn test_assignment_index_ast_tracks_arithmetic_subscripts() {
        let input = "a[i + 1]=x\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let assignment = &command.assignments[0];
        let expr = assignment
            .index_ast
            .as_ref()
            .expect("expected arithmetic subscript AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive subscript");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_variable(left, "i");
        expect_number(right, input, "1");
    }

    #[test]
    fn test_decl_name_and_array_access_attach_arithmetic_index_asts() {
        let input = "declare foo[1+2]\necho ${arr[i+1]}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Decl(command) = &script.commands[0] else {
            panic!("expected declaration command");
        };
        let DeclOperand::Name(name) = &command.operands[0] else {
            panic!("expected declaration name operand");
        };
        let expr = name
            .index_ast
            .as_ref()
            .expect("expected declaration index AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive expression in declaration index");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_number(left, input, "1");
        expect_number(right, input, "2");

        let Command::Simple(command) = &script.commands[1] else {
            panic!("expected simple command");
        };
        let WordPart::ArrayAccess {
            index, index_ast, ..
        } = &command.args[0].parts[0].kind
        else {
            panic!("expected array access word part");
        };
        assert_eq!(index.slice(input), "i+1");
        let expr = index_ast.as_ref().expect("expected array access index AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive array index");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_variable(left, "i");
        expect_number(right, input, "1");
    }

    #[test]
    fn test_substring_and_array_slice_attach_arithmetic_companion_asts() {
        let input = "echo ${s:i+1:len*2} ${arr[@]:i:j}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        let WordPart::Substring {
            offset_ast,
            length_ast,
            ..
        } = &command.args[0].parts[0].kind
        else {
            panic!("expected substring expansion");
        };
        let offset_ast = offset_ast.as_ref().expect("expected substring offset AST");
        let ArithmeticExpr::Binary { left, op, right } = &offset_ast.kind else {
            panic!("expected additive substring offset");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_variable(left, "i");
        expect_number(right, input, "1");
        let length_ast = length_ast.as_ref().expect("expected substring length AST");
        let ArithmeticExpr::Binary {
            left: len_left,
            op: len_op,
            right: len_right,
        } = &length_ast.kind
        else {
            panic!("expected multiplicative substring length");
        };
        assert_eq!(*len_op, ArithmeticBinaryOp::Multiply);
        expect_variable(len_left, "len");
        expect_number(len_right, input, "2");

        let WordPart::ArraySlice {
            offset_ast,
            length_ast,
            ..
        } = &command.args[1].parts[0].kind
        else {
            panic!("expected array slice expansion");
        };
        expect_variable(
            offset_ast
                .as_ref()
                .expect("expected array slice offset AST"),
            "i",
        );
        expect_variable(
            length_ast
                .as_ref()
                .expect("expected array slice length AST"),
            "j",
        );
    }

    #[test]
    fn test_non_arithmetic_subscripts_leave_companion_ast_empty() {
        let input = "echo ${arr[@]} ${arr[*]} ${map[\"key\"]}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };

        let WordPart::ArrayAccess { index_ast, .. } = &command.args[0].parts[0].kind else {
            panic!("expected first array access");
        };
        assert!(index_ast.is_none());

        let WordPart::ArrayAccess { index_ast, .. } = &command.args[1].parts[0].kind else {
            panic!("expected second array access");
        };
        assert!(index_ast.is_none());

        let WordPart::ArrayAccess { index_ast, .. } = &command.args[2].parts[0].kind else {
            panic!("expected quoted-key array access");
        };
        assert!(index_ast.is_none());
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
        assert_eq!(
            redirect_word_target(&command.redirects[0])
                .span
                .start
                .column,
            19
        );
    }

    #[test]
    fn test_word_part_spans_track_mixed_expansions() {
        let input = "echo pre${name:-fallback}$(printf hi)$((1+2))post\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let slices = top_level_part_slices(word, input);

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

        assert_eq!(
            top_level_part_slices(word, input),
            vec!["\"x$HOME$(pwd)y\""]
        );
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
        assert_eq!(slices, vec!["x", "$HOME", "$(pwd)", "y"]);
    }

    #[test]
    fn test_mixed_segment_word_preserves_expansion_boundaries() {
        let input = "echo foo\"$bar\"baz\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let slices = top_level_part_slices(word, input);

        assert_eq!(slices, vec!["foo", "\"$bar\"", "baz"]);
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[1].kind else {
            panic!("expected quoted middle segment");
        };
        assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
    }

    #[test]
    fn test_assignment_value_preserves_mixed_quoted_boundaries() {
        let input = "foo=\"$bar\"baz echo\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };

        let slices = top_level_part_slices(word, input);

        assert!(!is_fully_quoted(word));
        assert_eq!(slices, vec!["\"$bar\"", "baz"]);
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected quoted prefix");
        };
        assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
    }

    #[test]
    fn test_assignment_value_stays_quoted_when_entire_value_is_quoted() {
        let input = "foo=\"$bar\"\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };

        let slices = top_level_part_slices(word, input);

        assert!(is_fully_quoted(word));
        assert_eq!(slices, vec!["\"$bar\""]);
        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected fully quoted value");
        };
        assert!(matches!(parts.as_slice(), [part] if matches!(&part.kind, WordPart::Variable(_))));
    }

    #[test]
    fn test_backtick_command_substitution_inside_double_quotes_preserves_syntax_form() {
        let input = "echo \"pre `printf hi` post\"\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert!(is_fully_quoted(word));
        assert_eq!(
            top_level_part_slices(word, input),
            vec!["\"pre `printf hi` post\""]
        );

        let WordPart::DoubleQuoted { parts, dollar } = &word.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        assert!(!dollar);

        let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
        assert_eq!(slices, vec!["pre ", "`printf hi`", " post"]);

        let WordPart::CommandSubstitution { commands, syntax } = &parts[1].kind else {
            panic!("expected command substitution");
        };
        assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);

        let Command::Simple(inner) = &commands[0] else {
            panic!("expected simple command in substitution");
        };
        assert_eq!(inner.name.render(input), "printf");
        assert_eq!(inner.args[0].render(input), "hi");
    }

    #[test]
    fn test_dollar_quoted_words_preserve_quote_variants() {
        let input = "printf $'line\\n' $\"prefix $HOME\"\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 2);

        let ansi = &command.args[0];
        assert!(is_fully_quoted(ansi));
        assert_eq!(top_level_part_slices(ansi, input), vec!["$'line\\n'"]);
        let WordPart::SingleQuoted { value, dollar } = &ansi.parts[0].kind else {
            panic!("expected single-quoted word");
        };
        assert!(*dollar);
        assert_eq!(value.slice(input), "line\n");

        let translated = &command.args[1];
        assert!(is_fully_quoted(translated));
        assert_eq!(
            top_level_part_slices(translated, input),
            vec!["$\"prefix $HOME\""]
        );
        let WordPart::DoubleQuoted { parts, dollar } = &translated.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        assert!(*dollar);
        let slices: Vec<&str> = parts.iter().map(|part| part.span.slice(input)).collect();
        assert_eq!(slices, vec!["prefix ", "$HOME"]);
        assert!(matches!(parts[1].kind, WordPart::Variable(ref name) if name == "HOME"));
    }

    #[test]
    fn test_word_part_spans_track_nested_array_expansions() {
        let input = "echo ${arr[$RANDOM % ${#arr[@]}]}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.parts.len(), 1);
        assert_eq!(
            word.part_span(0).unwrap().slice(input),
            "${arr[$RANDOM % ${#arr[@]}]}"
        );

        let WordPart::ArrayAccess { index, .. } = &word.parts[0].kind else {
            panic!("expected array access");
        };
        assert!(index.is_source_backed());
        assert_eq!(index.slice(input), "$RANDOM % ${#arr[@]}");
    }

    #[test]
    fn test_word_part_spans_track_parenthesized_arithmetic_expansion() {
        let input = "echo $((a <= (1 || 2)))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.parts.len(), 1);
        assert_eq!(
            word.part_span(0).unwrap().slice(input),
            "$((a <= (1 || 2)))"
        );

        let WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } = &word.parts[0].kind
        else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
        assert!(expression.is_source_backed());
        assert_eq!(expression.slice(input), "a <= (1 || 2)");
        let expr = expression_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
        expect_variable(left, "a");
        let ArithmeticExpr::Parenthesized { expression } = &right.kind else {
            panic!("expected parenthesized right operand");
        };
        let ArithmeticExpr::Binary {
            left: inner_left,
            op: inner_op,
            right: inner_right,
        } = &expression.kind
        else {
            panic!("expected logical-or inside parentheses");
        };
        assert_eq!(*inner_op, ArithmeticBinaryOp::LogicalOr);
        expect_number(inner_left, input, "1");
        expect_number(inner_right, input, "2");
    }

    #[test]
    fn test_word_part_spans_track_nested_arithmetic_expansion() {
        let input = "echo $(((a) + ((b))))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.parts.len(), 1);
        assert_eq!(word.part_span(0).unwrap().slice(input), "$(((a) + ((b))))");

        let WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } = &word.parts[0].kind
        else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
        assert!(expression.is_source_backed());
        assert_eq!(expression.slice(input), "(a) + ((b))");
        let expr = expression_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        assert!(matches!(left.kind, ArithmeticExpr::Parenthesized { .. }));
        assert!(matches!(right.kind, ArithmeticExpr::Parenthesized { .. }));
    }

    #[test]
    fn test_arithmetic_expansion_inside_double_quotes_preserves_legacy_and_modern_syntax() {
        let input = "echo \"$((1 + 2))\" \"$[3 + 4]\"\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 2);

        let modern = &command.args[0];
        assert!(is_fully_quoted(modern));
        let WordPart::DoubleQuoted { parts, dollar } = &modern.parts[0].kind else {
            panic!("expected double-quoted modern arithmetic");
        };
        assert!(!dollar);
        assert_eq!(parts[0].span.slice(input), "$((1 + 2))");
        let WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } = &parts[0].kind
        else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(*syntax, ArithmeticExpansionSyntax::DollarParenParen);
        assert!(expression.is_source_backed());
        assert_eq!(expression.slice(input), "1 + 2");
        let expr = expression_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_number(left, input, "1");
        expect_number(right, input, "2");

        let legacy = &command.args[1];
        assert!(is_fully_quoted(legacy));
        let WordPart::DoubleQuoted { parts, dollar } = &legacy.parts[0].kind else {
            panic!("expected double-quoted legacy arithmetic");
        };
        assert!(!dollar);
        assert_eq!(parts[0].span.slice(input), "$[3 + 4]");
        let WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            syntax,
            ..
        } = &parts[0].kind
        else {
            panic!("expected arithmetic expansion");
        };
        assert_eq!(*syntax, ArithmeticExpansionSyntax::LegacyBracket);
        assert!(expression.is_source_backed());
        assert_eq!(expression.slice(input), "3 + 4");
        let expr = expression_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_number(left, input, "3");
        expect_number(right, input, "4");
    }

    #[test]
    fn test_parameter_expansion_operand_stays_source_backed() {
        let input = "echo ${var:-$(pwd)}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let WordPart::ParameterExpansion { operand, .. } = &word.parts[0].kind else {
            panic!("expected parameter expansion");
        };
        let operand = operand.as_ref().expect("expected operand");
        assert!(operand.is_source_backed());
        assert_eq!(operand.slice(input), "$(pwd)");
    }

    #[test]
    fn test_parameter_expansion_trim_operand_accepts_literal_left_brace_after_multiline_quote() {
        let input = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Function(function) = &script.commands[1] else {
            panic!("expected function definition");
        };
        let Command::Compound(CompoundCommand::BraceGroup(body), redirects) =
            function.body.as_ref()
        else {
            panic!("expected brace-group function body");
        };
        assert!(redirects.is_empty());
        let Command::Compound(CompoundCommand::If(if_command), redirects) = &body[0] else {
            panic!("expected if command");
        };
        assert!(redirects.is_empty());
        let Command::Simple(command) = &if_command.then_branch[0] else {
            panic!("expected simple command in then branch");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        let WordPart::ParameterExpansion { operand, .. } = &word.parts[0].kind else {
            panic!("expected parameter expansion");
        };

        let operand = operand.as_ref().expect("expected operand");
        assert!(operand.slice(input).contains("$_sub_domain"));
    }

    #[test]
    fn test_parameter_expansion_trim_operand_tracks_nested_parameter_expansions() {
        let input = "echo ${var#${prefix:-fallback}}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let WordPart::ParameterExpansion { operand, .. } = &word.parts[0].kind else {
            panic!("expected parameter expansion");
        };

        let operand = operand.as_ref().expect("expected operand");
        assert!(operand.is_source_backed());
        assert_eq!(operand.slice(input), "${prefix:-fallback}");
    }

    #[test]
    fn test_parameter_replacement_pattern_stays_source_backed() {
        let input = "echo ${var/foo/bar}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let WordPart::ParameterExpansion { operator, .. } = &word.parts[0].kind else {
            panic!("expected parameter expansion");
        };
        let ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } = operator
        else {
            panic!("expected replace-first operator");
        };

        assert!(pattern.is_source_backed());
        assert!(replacement.is_source_backed());
        assert_eq!(pattern.slice(input), "foo");
        assert_eq!(replacement.slice(input), "bar");
    }

    #[test]
    fn test_parameter_replacement_pattern_cooks_escaped_slash() {
        let input = r#"echo ${var/foo\/bar/baz}"#;
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let WordPart::ParameterExpansion { operator, .. } = &word.parts[0].kind else {
            panic!("expected parameter expansion");
        };
        let ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } = operator
        else {
            panic!("expected replace-first operator");
        };

        assert!(!pattern.is_source_backed());
        assert!(replacement.is_source_backed());
        assert_eq!(pattern.slice(input), "foo/bar");
        assert_eq!(replacement.slice(input), "baz");
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
        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
        let ArithmeticExpr::Binary {
            left: add_left,
            op: add_op,
            right: add_right,
        } = &left.kind
        else {
            panic!("expected additive left operand");
        };
        assert_eq!(*add_op, ArithmeticBinaryOp::Add);
        expect_number(add_left, input, "1");
        expect_number(add_right, input, "2");
        expect_number(right, input, "3");
    }

    #[test]
    fn test_parse_empty_arithmetic_command_keeps_span_without_typed_ast() {
        let input = "((   ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.expr_span.unwrap().slice(input), "   ");
        assert!(command.expr_ast.is_none());
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
        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::GreaterThan);
        expect_shell_word(left, input, "$(date -u)");
        expect_variable(right, "DATE");
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
    fn test_parse_arithmetic_command_with_nested_double_parens_and_grouping() {
        let input = "(( x = ((1 + 2) * (3 - 4)) ))\n";
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
            " x = ((1 + 2) * (3 - 4)) "
        );

        let ArithmeticExpr::Assignment { target, op, value } = &command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST")
            .kind
        else {
            panic!("expected arithmetic assignment");
        };
        assert_eq!(*op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable assignment target");
        };
        assert_eq!(name, "x");
        assert!(matches!(value.kind, ArithmeticExpr::Parenthesized { .. }));
    }

    #[test]
    fn test_parse_arithmetic_command_respects_precedence_and_associativity() {
        let input = "(( a + b * c ** d ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary {
            left,
            op: add_op,
            right,
        } = &expr.kind
        else {
            panic!("expected additive expression");
        };
        assert_eq!(*add_op, ArithmeticBinaryOp::Add);
        expect_variable(left, "a");

        let ArithmeticExpr::Binary {
            left: mul_left,
            op: mul_op,
            right: mul_right,
        } = &right.kind
        else {
            panic!("expected multiplicative expression");
        };
        assert_eq!(*mul_op, ArithmeticBinaryOp::Multiply);
        expect_variable(mul_left, "b");

        let ArithmeticExpr::Binary {
            left: pow_left,
            op: pow_op,
            right: pow_right,
        } = &mul_right.kind
        else {
            panic!("expected power expression");
        };
        assert_eq!(*pow_op, ArithmeticBinaryOp::Power);
        expect_variable(pow_left, "c");
        expect_variable(pow_right, "d");
    }

    #[test]
    fn test_parse_arithmetic_command_distinguishes_assignment_from_comparison() {
        let input = "(( a = b == c ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Assignment { target, op, value } = &expr.kind else {
            panic!("expected arithmetic assignment");
        };
        assert_eq!(*op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable assignment target");
        };
        assert_eq!(name, "a");

        let ArithmeticExpr::Binary {
            left,
            op: cmp_op,
            right,
        } = &value.kind
        else {
            panic!("expected comparison on assignment right-hand side");
        };
        assert_eq!(*cmp_op, ArithmeticBinaryOp::Equal);
        expect_variable(left, "b");
        expect_variable(right, "c");
    }

    #[test]
    fn test_parse_arithmetic_command_parses_updates_ternary_and_comma() {
        let input = "(( ++i ? j-- : (k = 1), m ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary {
            left,
            op: comma_op,
            right,
        } = &expr.kind
        else {
            panic!("expected comma expression");
        };
        assert_eq!(*comma_op, ArithmeticBinaryOp::Comma);
        expect_variable(right, "m");

        let ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } = &left.kind
        else {
            panic!("expected conditional expression");
        };

        let ArithmeticExpr::Unary { op: unary_op, expr } = &condition.kind else {
            panic!("expected prefix update condition");
        };
        assert_eq!(*unary_op, ArithmeticUnaryOp::PreIncrement);
        expect_variable(expr, "i");

        let ArithmeticExpr::Postfix {
            expr,
            op: postfix_op,
        } = &then_expr.kind
        else {
            panic!("expected postfix update in then branch");
        };
        assert_eq!(*postfix_op, ArithmeticPostfixOp::Decrement);
        expect_variable(expr, "j");

        let ArithmeticExpr::Parenthesized { expression } = &else_expr.kind else {
            panic!("expected parenthesized else branch");
        };
        let ArithmeticExpr::Assignment { target, op, value } = &expression.kind else {
            panic!("expected assignment inside else branch");
        };
        assert_eq!(*op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable else target");
        };
        assert_eq!(name, "k");
        expect_number(value, input, "1");
    }

    #[test]
    fn test_parse_arithmetic_command_accepts_command_substitutions_and_quoted_words() {
        let input = "(( \"$(date -u)\" + '3' ))\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        let expr = command
            .expr_ast
            .as_ref()
            .expect("expected typed arithmetic AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected binary arithmetic expression");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        let ArithmeticExpr::ShellWord(left_word) = &left.kind else {
            panic!("expected quoted shell word on left");
        };
        assert_eq!(left_word.span.slice(input), "\"$(date -u)\"");
        let ArithmeticExpr::ShellWord(right_word) = &right.kind else {
            panic!("expected quoted shell word on right");
        };
        assert_eq!(right_word.span.slice(input), "'3'");
    }

    #[test]
    fn test_double_left_paren_command_closed_with_spaced_right_parens_parses_as_subshells() {
        let input = "(( echo 1\necho 2\n(( x ))\n: $(( x ))\necho 3\n) )\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::Subshell(commands) = compound else {
            panic!("expected outer subshell");
        };
        assert!(redirects.is_empty());
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            Command::Compound(CompoundCommand::Subshell(_), _)
        ));
    }

    #[test]
    fn test_double_left_paren_test_clause_parses_as_command() {
        let input =
            "if ! ((test x\\\"$i\\\" = x-g) || (test x\\\"$i\\\" = x-O2)); then\n  echo bye\nfi\n";
        Parser::new(input).parse().unwrap();
    }

    #[test]
    fn test_double_left_paren_pipeline_parses_as_command() {
        let input = "((cat </dev/zero; echo $? >&7) | true) 7>&1\n";
        Parser::new(input).parse().unwrap();
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
        let ArithmeticExpr::Assignment {
            target,
            op: init_op,
            value: init_value,
        } = &command
            .init_ast
            .as_ref()
            .expect("expected init arithmetic AST")
            .kind
        else {
            panic!("expected assignment init expression");
        };
        assert_eq!(*init_op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable init target");
        };
        assert_eq!(name, "i");
        expect_number(init_value, input, "0");

        let ArithmeticExpr::Binary {
            left: condition_left,
            op: condition_op,
            right: condition_right,
        } = &command
            .condition_ast
            .as_ref()
            .expect("expected condition arithmetic AST")
            .kind
        else {
            panic!("expected binary condition expression");
        };
        assert_eq!(*condition_op, ArithmeticBinaryOp::LessThan);
        expect_variable(condition_left, "i");
        expect_number(condition_right, input, "10");

        let ArithmeticExpr::Assignment {
            target,
            op: step_op,
            value: step_value,
        } = &command
            .step_ast
            .as_ref()
            .expect("expected step arithmetic AST")
            .kind
        else {
            panic!("expected assignment step expression");
        };
        assert_eq!(*step_op, ArithmeticAssignOp::AddAssign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable step target");
        };
        assert_eq!(name, "i");
        expect_number(step_value, input, "2");
    }

    #[test]
    fn test_parse_arithmetic_for_with_nested_double_parens_in_segments() {
        let input = "for (( x = ((1 + 2) * (3 - 4)); y < ((5 + 6) * (7 - 8)); z = ((9 + 10) * (11 - 12)) )); do :; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.left_paren_span.slice(input), "((");
        assert_eq!(
            command.init_span.unwrap().slice(input),
            " x = ((1 + 2) * (3 - 4))"
        );
        assert_eq!(
            command.condition_span.unwrap().slice(input),
            " y < ((5 + 6) * (7 - 8))"
        );
        assert_eq!(
            command.step_span.unwrap().slice(input),
            " z = ((9 + 10) * (11 - 12)) "
        );

        let ArithmeticExpr::Assignment { target, op, value } = &command
            .init_ast
            .as_ref()
            .expect("expected init arithmetic AST")
            .kind
        else {
            panic!("expected assignment init expression");
        };
        assert_eq!(*op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable init target");
        };
        assert_eq!(name, "x");
        assert!(matches!(value.kind, ArithmeticExpr::Parenthesized { .. }));

        let ArithmeticExpr::Binary {
            left: condition_left,
            op: condition_op,
            right: condition_right,
        } = &command
            .condition_ast
            .as_ref()
            .expect("expected condition arithmetic AST")
            .kind
        else {
            panic!("expected binary condition expression");
        };
        assert_eq!(*condition_op, ArithmeticBinaryOp::LessThan);
        expect_variable(condition_left, "y");
        assert!(matches!(
            condition_right.kind,
            ArithmeticExpr::Parenthesized { .. }
        ));

        let ArithmeticExpr::Assignment {
            target,
            op: step_op,
            value: step_value,
        } = &command
            .step_ast
            .as_ref()
            .expect("expected step arithmetic AST")
            .kind
        else {
            panic!("expected assignment step expression");
        };
        assert_eq!(*step_op, ArithmeticAssignOp::Assign);
        let ArithmeticLvalue::Variable(name) = target else {
            panic!("expected variable step target");
        };
        assert_eq!(name, "z");
        assert!(matches!(
            step_value.kind,
            ArithmeticExpr::Parenthesized { .. }
        ));
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
        assert!(command.init_ast.is_none());
        assert!(command.condition_ast.is_none());
        assert!(command.step_ast.is_none());
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
    fn test_parse_arithmetic_for_treats_less_than_left_paren_as_arithmetic() {
        let input = "for (( n=0; n<(3-(1)); n++ )) ; do echo $n; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.condition_span.unwrap().slice(input), " n<(3-(1))");
    }

    #[test]
    fn test_parse_arithmetic_for_treats_spaced_less_than_left_paren_as_arithmetic() {
        let input = "for (( n=0; n<(3- (1)); n++ )) ; do echo $n; done\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.condition_span.unwrap().slice(input), " n<(3- (1))");
    }

    #[test]
    fn test_parse_arithmetic_for_accepts_brace_group_body() {
        let input = "for ((a=1; a <= 3; a++)) {\n  echo $a\n}\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, redirects) = expect_compound(&script.commands[0]);
        let CompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.body.len(), 1);

        let Command::Compound(CompoundCommand::BraceGroup(body), body_redirects) = &command.body[0]
        else {
            panic!("expected brace-group loop body");
        };
        assert!(body_redirects.is_empty());
        assert_eq!(body.len(), 1);
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
    fn test_for_loop_words_consume_segmented_tokens_directly() {
        let input = "for item in foo\"bar\" 'baz'qux; do echo \"$item\"; done";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::For(command) = compound else {
            panic!("expected for loop");
        };

        let words = command.words.as_ref().expect("expected explicit for words");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].render(input), "foobar");
        assert_eq!(words[0].parts.len(), 2);
        assert_eq!(words[0].part_span(0).unwrap().slice(input), "foo");
        assert_eq!(words[0].part_span(1).unwrap().slice(input), "\"bar\"");

        assert_eq!(words[1].render(input), "bazqux");
        assert!(!is_fully_quoted(&words[1]));
        assert_eq!(words[1].parts.len(), 2);
        assert_eq!(words[1].part_span(0).unwrap().slice(input), "'baz'");
        assert_eq!(words[1].part_span(1).unwrap().slice(input), "qux");
    }

    #[test]
    fn test_case_patterns_consume_segmented_tokens_directly() {
        let input = "case $x in foo\"bar\"|'baz'qux) echo hi ;; esac";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Case(command) = compound else {
            panic!("expected case command");
        };

        let patterns = &command.cases[0].patterns;
        assert_eq!(patterns.len(), 2);

        assert_eq!(patterns[0].render(input), "foobar");
        assert_eq!(patterns[0].parts.len(), 2);
        assert_eq!(patterns[0].part_span(0).unwrap().slice(input), "foo");
        assert_eq!(patterns[0].part_span(1).unwrap().slice(input), "\"bar\"");

        assert_eq!(patterns[1].render(input), "bazqux");
        assert!(!is_fully_quoted(&patterns[1]));
        assert_eq!(patterns[1].parts.len(), 2);
        assert_eq!(patterns[1].part_span(0).unwrap().slice(input), "'baz'");
        assert_eq!(patterns[1].part_span(1).unwrap().slice(input), "qux");
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
    fn test_parse_conditional_accepts_nested_grouping_with_double_parens() {
        let input = "[[ ! -e \"$cache\" && (( -e \"$prefix/n\" && ! -w \"$prefix/n\" ) || ( ! -e \"$prefix/n\" && ! -w \"$prefix\" )) ]]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::And);

        let ConditionalExpr::Parenthesized(paren) = binary.right.as_ref() else {
            panic!("expected parenthesized conditional term");
        };
        assert_eq!(
            paren.span().slice(input),
            "(( -e \"$prefix/n\" && ! -w \"$prefix/n\" ) || ( ! -e \"$prefix/n\" && ! -w \"$prefix\" ))"
        );

        let ConditionalExpr::Binary(inner) = paren.expr.as_ref() else {
            panic!("expected grouped binary conditional");
        };
        assert_eq!(inner.op, ConditionalBinaryOp::Or);
        assert!(matches!(
            inner.left.as_ref(),
            ConditionalExpr::Parenthesized(_)
        ));
        assert!(matches!(
            inner.right.as_ref(),
            ConditionalExpr::Parenthesized(_)
        ));
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
    fn test_parse_conditional_regex_allows_left_brace_operand() {
        let input = "[[ { =~ \"{\" ]]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let (compound, _) = expect_compound(&script.commands[0]);
        let CompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::RegexMatch);

        let ConditionalExpr::Word(left) = binary.left.as_ref() else {
            panic!("expected literal left operand");
        };
        assert_eq!(left.span.slice(input), "{");

        let ConditionalExpr::Regex(right) = binary.right.as_ref() else {
            panic!("expected regex rhs");
        };
        assert_eq!(right.render(input), "{");
    }

    #[test]
    fn test_parse_conditional_regex_rejects_unquoted_right_brace_operand() {
        let input = "[[ { =~ { ]]\n";
        assert!(Parser::new(input).parse().is_err());
    }

    #[test]
    fn test_parse_glob_word_with_embedded_quote_stays_single_arg() {
        let input = "echo [hello\"]\"\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[hello\"]\"");
    }

    #[test]
    fn test_parse_glob_word_with_command_sub_in_bracket_expression_stays_single_arg() {
        let input = "echo [$(echo abc)]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[$(echo abc)]");
    }

    #[test]
    fn test_parse_glob_word_with_extglob_chars_stays_single_arg() {
        let input = "echo [+()]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[+()]");
    }

    #[test]
    fn test_parse_glob_word_with_trailing_literal_right_paren_stays_single_arg() {
        let input = "echo [+(])\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[+(])");
    }

    #[test]
    fn test_parse_glob_of_unescaped_double_left_bracket_stays_word() {
        let input = "echo [[z] []z]\n";
        let script = Parser::new(input).parse().unwrap().script;

        let Command::Simple(command) = &script.commands[0] else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0].span.slice(input), "[[z]");
        assert_eq!(command.args[1].span.slice(input), "[]z]");
    }

    #[test]
    fn test_parse_parameter_expansion_operands_allow_quoted_and_escaped_right_brace() {
        let input = r###"echo "${var#\}}"
echo "${var#'}'}"
echo "${var#"}"}"
echo "${var-\}}"
echo "${var-'}'}"
echo "${var-"}"}"
"###;

        let script = Parser::new(input).parse().unwrap().script;
        assert_eq!(script.commands.len(), 6);
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
        let WordPart::CommandSubstitution { commands, syntax } = &word.parts[0].kind else {
            panic!("expected command substitution");
        };
        assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);
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
    fn test_parse_command_substitution_with_case_pattern_right_paren() {
        let input = "echo $(foo=a; case $foo in [0-9]) echo number;; [a-z]) echo letter ;; esac)\n";
        Parser::new(input).parse().unwrap();
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
        let WordPart::ProcessSubstitution { commands, is_input } = &command.args[0].parts[0].kind
        else {
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
        assert_eq!(
            redirect_word_target(&command.redirects[0])
                .span
                .slice(input),
            "out"
        );
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
        assert!(is_fully_quoted(&elements[0]));
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

    #[test]
    fn test_alias_expansion_with_trailing_space_expands_next_word() {
        let input = "\
shopt -s expand_aliases
alias greet='echo '
alias subject='hello'
greet subject
";
        let script = Parser::new(input).parse().unwrap().script;

        let Some(Command::Simple(command)) = script.commands.last() else {
            panic!("expected final command to be a simple command");
        };

        assert_eq!(command.name.render(input), "echo");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].render(input), "hello");

        let WordPart::Literal(name_text) = &command.name.parts[0].kind else {
            panic!("expected alias-expanded command name to stay literal");
        };
        let WordPart::Literal(arg_text) = &command.args[0].parts[0].kind else {
            panic!("expected alias-expanded arg to stay literal");
        };

        assert!(!name_text.is_source_backed());
        assert!(!arg_text.is_source_backed());
    }

    #[test]
    fn test_alias_expansion_recursive_guard_stops_self_reference() {
        let input = "\
shopt -s expand_aliases
alias loop='loop echo'
loop
";
        let script = Parser::new(input).parse().unwrap().script;

        let Some(Command::Simple(command)) = script.commands.last() else {
            panic!("expected final command to be a simple command");
        };

        assert_eq!(command.name.render(input), "loop");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].render(input), "echo");
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

    #[test]
    fn test_posix_dialect_rejects_double_bracket_conditionals() {
        let error = Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Posix)
            .parse()
            .unwrap_err();

        assert!(matches!(
            error,
            Error::Parse { message, .. } if message.contains("[[ ]] conditionals")
        ));
    }

    #[test]
    fn test_bash_and_mksh_dialects_accept_double_bracket_conditionals() {
        Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Bash)
            .parse()
            .unwrap();
        Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Mksh)
            .parse()
            .unwrap();
    }

    #[test]
    fn test_non_bash_dialects_reject_c_style_for_loops() {
        let error = Parser::with_dialect(
            "for ((i=0; i<2; i++)); do echo hi; done\n",
            ShellDialect::Mksh,
        )
        .parse()
        .unwrap_err();

        assert!(matches!(
            error,
            Error::Parse { message, .. } if message.contains("c-style for loops")
        ));
    }
}
