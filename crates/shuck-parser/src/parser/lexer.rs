//! Lexer for bash scripts
//!
//! Tokenizes input into a stream of tokens with source position tracking.

use std::collections::VecDeque;

use memchr::memchr;
use shuck_ast::{Position, Span, Token, TokenKind};

/// A token with its source location span.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub kind: TokenKind,
    pub token: Token,
    pub span: Span,
}

impl SpannedToken {
    pub(crate) fn new(token: Token, span: Span) -> Self {
        Self {
            kind: token.kind(),
            token,
            span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TokenFlags(u8);

impl TokenFlags {
    const COOKED_TEXT: u8 = 1 << 0;
    const SYNTHETIC: u8 = 1 << 1;

    const fn empty() -> Self {
        Self(0)
    }

    const fn cooked_text() -> Self {
        Self(Self::COOKED_TEXT)
    }

    pub(crate) const fn with_synthetic(self) -> Self {
        Self(self.0 | Self::SYNTHETIC)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenText<'a> {
    Borrowed(&'a str),
    Owned(String),
}

impl TokenText<'_> {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(text) => text,
            Self::Owned(text) => text,
        }
    }

    fn into_owned<'a>(self) -> TokenText<'a> {
        match self {
            Self::Borrowed(text) => TokenText::Owned(text.to_string()),
            Self::Owned(text) => TokenText::Owned(text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexedWordSegmentKind {
    Plain,
    Literal,
    DoubleQuoted,
    Composite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexedWordSegment<'a> {
    kind: LexedWordSegmentKind,
    text: TokenText<'a>,
    span: Option<Span>,
}

impl LexedWordSegment<'_> {
    pub(crate) fn as_str(&self) -> &str {
        self.text.as_str()
    }

    pub(crate) const fn kind(&self) -> LexedWordSegmentKind {
        self.kind
    }

    pub(crate) const fn span(&self) -> Option<Span> {
        self.span
    }

    fn into_owned<'a>(self) -> LexedWordSegment<'a> {
        LexedWordSegment {
            kind: self.kind,
            text: self.text.into_owned(),
            span: self.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexedWord<'a> {
    primary_segment: LexedWordSegment<'a>,
    trailing_segments: Vec<LexedWordSegment<'a>>,
    flattened: Option<TokenText<'a>>,
}

impl<'a> LexedWord<'a> {
    fn borrowed(kind: LexedWordSegmentKind, text: &'a str, span: Option<Span>) -> Self {
        Self {
            primary_segment: LexedWordSegment {
                kind,
                text: TokenText::Borrowed(text),
                span,
            },
            trailing_segments: Vec::new(),
            flattened: None,
        }
    }

    fn owned(kind: LexedWordSegmentKind, text: String) -> Self {
        Self {
            primary_segment: LexedWordSegment {
                kind,
                text: TokenText::Owned(text),
                span: None,
            },
            trailing_segments: Vec::new(),
            flattened: None,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        self.flattened
            .as_ref()
            .map_or_else(|| self.primary_segment.as_str(), TokenText::as_str)
    }

    pub(crate) fn single_segment(&self) -> Option<&LexedWordSegment<'a>> {
        self.trailing_segments
            .is_empty()
            .then_some(&self.primary_segment)
    }

    fn into_owned<'b>(self) -> LexedWord<'b> {
        LexedWord {
            primary_segment: self.primary_segment.into_owned(),
            trailing_segments: self
                .trailing_segments
                .into_iter()
                .map(LexedWordSegment::into_owned)
                .collect(),
            flattened: self.flattened.map(TokenText::into_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexerErrorKind {
    CommandSubstitution,
    BacktickSubstitution,
    SingleQuote,
    DoubleQuote,
}

impl LexerErrorKind {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::CommandSubstitution => "unterminated command substitution",
            Self::BacktickSubstitution => "unterminated backtick substitution",
            Self::SingleQuote => "unterminated single quote",
            Self::DoubleQuote => "unterminated double quote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenPayload<'a> {
    None,
    Word(LexedWord<'a>),
    Fd(i32),
    FdPair(i32, i32),
    Error(LexerErrorKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexedToken<'a> {
    pub kind: TokenKind,
    pub span: Span,
    pub flags: TokenFlags,
    payload: TokenPayload<'a>,
}

impl<'a> LexedToken<'a> {
    fn word_segment_kind(kind: TokenKind) -> LexedWordSegmentKind {
        match kind {
            TokenKind::Word => LexedWordSegmentKind::Plain,
            TokenKind::LiteralWord => LexedWordSegmentKind::Literal,
            TokenKind::QuotedWord => LexedWordSegmentKind::DoubleQuoted,
            _ => LexedWordSegmentKind::Composite,
        }
    }

    pub(crate) fn punctuation(kind: TokenKind) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::None,
        }
    }

    fn borrowed_word(kind: TokenKind, text: &'a str, text_span: Option<Span>) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::Word(LexedWord::borrowed(
                Self::word_segment_kind(kind),
                text,
                text_span,
            )),
        }
    }

    fn owned_word(kind: TokenKind, text: String) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::cooked_text(),
            payload: TokenPayload::Word(LexedWord::owned(Self::word_segment_kind(kind), text)),
        }
    }

    fn owned_composite_word(kind: TokenKind, text: String) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::cooked_text(),
            payload: TokenPayload::Word(LexedWord::owned(LexedWordSegmentKind::Composite, text)),
        }
    }

    fn comment() -> Self {
        Self {
            kind: TokenKind::Comment,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::None,
        }
    }

    fn fd(kind: TokenKind, fd: i32) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::Fd(fd),
        }
    }

    fn fd_pair(kind: TokenKind, src_fd: i32, dst_fd: i32) -> Self {
        Self {
            kind,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::FdPair(src_fd, dst_fd),
        }
    }

    fn error(kind: LexerErrorKind) -> Self {
        Self {
            kind: TokenKind::Error,
            span: Span::new(),
            flags: TokenFlags::empty(),
            payload: TokenPayload::Error(kind),
        }
    }

    pub(crate) fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    pub(crate) fn rebased(mut self, base: Position) -> Self {
        self.span = self.span.rebased(base);
        self
    }

    pub(crate) fn with_synthetic_flag(mut self) -> Self {
        self.flags = self.flags.with_synthetic();
        self
    }

    pub(crate) fn into_owned<'b>(self) -> LexedToken<'b> {
        let payload = match self.payload {
            TokenPayload::None => TokenPayload::None,
            TokenPayload::Word(word) => TokenPayload::Word(word.into_owned()),
            TokenPayload::Fd(fd) => TokenPayload::Fd(fd),
            TokenPayload::FdPair(src_fd, dst_fd) => TokenPayload::FdPair(src_fd, dst_fd),
            TokenPayload::Error(kind) => TokenPayload::Error(kind),
        };

        LexedToken {
            kind: self.kind,
            span: self.span,
            flags: self.flags,
            payload,
        }
    }

    pub(crate) fn word_text(&self) -> Option<&str> {
        self.kind
            .is_word_like()
            .then_some(())
            .and_then(|_| match &self.payload {
                TokenPayload::Word(word) => Some(word.as_str()),
                _ => None,
            })
    }

    pub(crate) fn word(&self) -> Option<&LexedWord<'a>> {
        match &self.payload {
            TokenPayload::Word(word) => Some(word),
            _ => None,
        }
    }

    pub(crate) fn fd_value(&self) -> Option<i32> {
        match self.payload {
            TokenPayload::Fd(fd) => Some(fd),
            _ => None,
        }
    }

    pub(crate) fn fd_pair_value(&self) -> Option<(i32, i32)> {
        match self.payload {
            TokenPayload::FdPair(src_fd, dst_fd) => Some((src_fd, dst_fd)),
            _ => None,
        }
    }

    pub(crate) fn error_kind(&self) -> Option<LexerErrorKind> {
        match self.payload {
            TokenPayload::Error(kind) => Some(kind),
            _ => None,
        }
    }
}

/// Result of reading a heredoc body from the source.
#[derive(Debug, Clone, PartialEq)]
pub struct HeredocRead {
    pub content: String,
    pub content_span: Span,
}

/// Maximum nesting depth for command substitution in the lexer.
/// Prevents stack overflow from deeply nested $() patterns.
const DEFAULT_MAX_SUBST_DEPTH: usize = 50;

#[derive(Clone, Debug)]
struct Cursor<'a> {
    rest: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Self {
        Self { rest: source }
    }

    fn first(&self) -> Option<char> {
        self.rest.chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.first()?;
        self.rest = &self.rest[ch.len_utf8()..];
        Some(ch)
    }

    fn eat_while(&mut self, mut predicate: impl FnMut(char) -> bool) -> &'a str {
        let start = self.rest;
        let mut end = 0;

        for ch in start.chars() {
            if !predicate(ch) {
                break;
            }
            end += ch.len_utf8();
        }

        self.rest = &start[end..];
        &start[..end]
    }

    fn rest(&self) -> &'a str {
        self.rest
    }

    fn skip_bytes(&mut self, count: usize) {
        self.rest = &self.rest[count..];
    }

    fn find_byte(&self, byte: u8) -> Option<usize> {
        memchr(byte, self.rest.as_bytes())
    }
}

/// Lexer for bash scripts.
#[derive(Clone)]
pub struct Lexer<'a> {
    #[allow(dead_code)] // Stored for error reporting in future
    input: &'a str,
    /// Current position in the input
    position: Position,
    cursor: Cursor<'a>,
    /// Buffer for re-injected characters (e.g., rest-of-line after heredoc delimiter).
    /// Consumed before `cursor`.
    reinject_buf: VecDeque<char>,
    /// Cursor position to restore once a heredoc replay buffer is exhausted.
    reinject_resume_position: Option<Position>,
    /// Maximum allowed nesting depth for command substitution
    max_subst_depth: usize,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_max_subst_depth(input, DEFAULT_MAX_SUBST_DEPTH)
    }

    /// Create a new lexer with a custom max substitution nesting depth.
    /// Limits recursion in read_command_subst_into().
    pub fn with_max_subst_depth(input: &'a str, max_depth: usize) -> Self {
        Self {
            input,
            position: Position::new(),
            cursor: Cursor::new(input),
            reinject_buf: VecDeque::new(),
            reinject_resume_position: None,
            max_subst_depth: max_depth,
        }
    }

    /// Get the current position in the input.
    pub fn position(&self) -> Position {
        self.position
    }

    fn sync_position_to_cursor(&mut self) {
        if self.reinject_buf.is_empty()
            && let Some(position) = self.reinject_resume_position.take()
        {
            self.position = position;
        }
    }

    /// Get the next token from the input (without span info).
    pub fn next_token(&mut self) -> Option<Token> {
        self.next_lexed_token()
            .map(|token| self.materialize_legacy_token(&token))
    }

    /// Get the next token from the input, preserving line comments.
    pub fn next_token_with_comments(&mut self) -> Option<Token> {
        self.next_lexed_token_with_comments()
            .map(|token| self.materialize_legacy_token(&token))
    }

    fn peek_char(&mut self) -> Option<char> {
        self.sync_position_to_cursor();
        if let Some(&ch) = self.reinject_buf.front() {
            Some(ch)
        } else {
            self.cursor.first()
        }
    }

    fn advance(&mut self) -> Option<char> {
        self.sync_position_to_cursor();
        let ch = if !self.reinject_buf.is_empty() {
            self.reinject_buf.pop_front()
        } else {
            self.cursor.bump()
        };
        if let Some(c) = ch {
            self.position.advance(c);
        }
        ch
    }

    fn lookahead_chars(&self) -> impl Iterator<Item = char> + '_ {
        self.reinject_buf
            .iter()
            .copied()
            .chain(self.cursor.rest().chars())
    }

    fn peek_nth_char(&self, n: usize) -> Option<char> {
        self.lookahead_chars().nth(n)
    }

    fn advance_position_without_newline(&mut self, text: &str) {
        debug_assert!(!text.contains('\n'));

        self.position.offset += text.len();
        self.position.column += if text.is_ascii() {
            text.len()
        } else {
            text.chars().count()
        };
    }

    fn consume_source_bytes_without_newline(&mut self, byte_len: usize) {
        debug_assert!(self.reinject_buf.is_empty());
        self.sync_position_to_cursor();
        let text = &self.cursor.rest()[..byte_len];
        self.advance_position_without_newline(text);
        self.cursor.skip_bytes(byte_len);
    }

    fn materialize_legacy_token(&self, token: &LexedToken<'_>) -> Token {
        match token.kind {
            TokenKind::Word => Token::Word(token.word_text().unwrap_or_default().to_string()),
            TokenKind::LiteralWord => {
                Token::LiteralWord(token.word_text().unwrap_or_default().to_string())
            }
            TokenKind::QuotedWord => {
                Token::QuotedWord(token.word_text().unwrap_or_default().to_string())
            }
            TokenKind::Comment => {
                let start = token.span.start.offset.saturating_add(1);
                Token::Comment(self.input[start..token.span.end.offset].to_string())
            }
            TokenKind::Newline => Token::Newline,
            TokenKind::Semicolon => Token::Semicolon,
            TokenKind::DoubleSemicolon => Token::DoubleSemicolon,
            TokenKind::SemiAmp => Token::SemiAmp,
            TokenKind::DoubleSemiAmp => Token::DoubleSemiAmp,
            TokenKind::Pipe => Token::Pipe,
            TokenKind::PipeBoth => Token::PipeBoth,
            TokenKind::And => Token::And,
            TokenKind::Or => Token::Or,
            TokenKind::Background => Token::Background,
            TokenKind::RedirectOut => Token::RedirectOut,
            TokenKind::RedirectAppend => Token::RedirectAppend,
            TokenKind::RedirectIn => Token::RedirectIn,
            TokenKind::RedirectReadWrite => Token::RedirectReadWrite,
            TokenKind::HereDoc => Token::HereDoc,
            TokenKind::HereDocStrip => Token::HereDocStrip,
            TokenKind::HereString => Token::HereString,
            TokenKind::LeftParen => Token::LeftParen,
            TokenKind::RightParen => Token::RightParen,
            TokenKind::DoubleLeftParen => Token::DoubleLeftParen,
            TokenKind::DoubleRightParen => Token::DoubleRightParen,
            TokenKind::LeftBrace => Token::LeftBrace,
            TokenKind::RightBrace => Token::RightBrace,
            TokenKind::DoubleLeftBracket => Token::DoubleLeftBracket,
            TokenKind::DoubleRightBracket => Token::DoubleRightBracket,
            TokenKind::Assignment => Token::Assignment,
            TokenKind::ProcessSubIn => Token::ProcessSubIn,
            TokenKind::ProcessSubOut => Token::ProcessSubOut,
            TokenKind::RedirectBoth => Token::RedirectBoth,
            TokenKind::RedirectBothAppend => Token::RedirectBothAppend,
            TokenKind::Clobber => Token::Clobber,
            TokenKind::DupOutput => Token::DupOutput,
            TokenKind::DupInput => Token::DupInput,
            TokenKind::RedirectFd => Token::RedirectFd(token.fd_value().unwrap_or_default()),
            TokenKind::RedirectFdAppend => {
                Token::RedirectFdAppend(token.fd_value().unwrap_or_default())
            }
            TokenKind::DupFd => {
                let (src_fd, dst_fd) = token.fd_pair_value().unwrap_or_default();
                Token::DupFd(src_fd, dst_fd)
            }
            TokenKind::DupFdIn => {
                let (src_fd, dst_fd) = token.fd_pair_value().unwrap_or_default();
                Token::DupFdIn(src_fd, dst_fd)
            }
            TokenKind::DupFdClose => Token::DupFdClose(token.fd_value().unwrap_or_default()),
            TokenKind::RedirectFdIn => Token::RedirectFdIn(token.fd_value().unwrap_or_default()),
            TokenKind::RedirectFdReadWrite => {
                Token::RedirectFdReadWrite(token.fd_value().unwrap_or_default())
            }
            TokenKind::Error => Token::Error(
                token
                    .error_kind()
                    .map(LexerErrorKind::message)
                    .unwrap_or("unknown lexer error")
                    .to_string(),
            ),
        }
    }

    /// Get the next token with its source span.
    pub fn next_spanned_token(&mut self) -> Option<SpannedToken> {
        self.next_lexed_token().map(|token| {
            let span = token.span;
            SpannedToken::new(self.materialize_legacy_token(&token), span)
        })
    }

    /// Get the next token with its source span, preserving line comments.
    pub fn next_spanned_token_with_comments(&mut self) -> Option<SpannedToken> {
        self.next_lexed_token_with_comments().map(|token| {
            let span = token.span;
            SpannedToken::new(self.materialize_legacy_token(&token), span)
        })
    }

    pub(crate) fn next_lexed_token(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.position;
        let token = self.next_lexed_token_inner(false)?;
        let end = self.position;
        Some(token.with_span(Span::from_positions(start, end)))
    }

    pub(crate) fn next_lexed_token_with_comments(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.position;
        let token = self.next_lexed_token_inner(true)?;
        let end = self.position;
        Some(token.with_span(Span::from_positions(start, end)))
    }

    /// Internal: get next token without recording position (called after whitespace skip)
    fn next_lexed_token_inner(&mut self, preserve_comments: bool) -> Option<LexedToken<'a>> {
        let ch = self.peek_char()?;

        match ch {
            '\n' => {
                self.advance();
                Some(LexedToken::punctuation(TokenKind::Newline))
            }
            ';' => {
                self.advance();
                if self.peek_char() == Some(';') {
                    self.advance();
                    if self.peek_char() == Some('&') {
                        self.advance();
                        Some(LexedToken::punctuation(TokenKind::DoubleSemiAmp)) // ;;&
                    } else {
                        Some(LexedToken::punctuation(TokenKind::DoubleSemicolon)) // ;;
                    }
                } else if self.peek_char() == Some('&') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::SemiAmp)) // ;&
                } else {
                    Some(LexedToken::punctuation(TokenKind::Semicolon))
                }
            }
            '|' => {
                self.advance();
                if self.peek_char() == Some('|') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::Or))
                } else if self.peek_char() == Some('&') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::PipeBoth))
                } else {
                    Some(LexedToken::punctuation(TokenKind::Pipe))
                }
            }
            '&' => {
                self.advance();
                if self.peek_char() == Some('&') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::And))
                } else if self.peek_char() == Some('>') {
                    self.advance();
                    if self.peek_char() == Some('>') {
                        self.advance();
                        Some(LexedToken::punctuation(TokenKind::RedirectBothAppend))
                    } else {
                        Some(LexedToken::punctuation(TokenKind::RedirectBoth))
                    }
                } else {
                    Some(LexedToken::punctuation(TokenKind::Background))
                }
            }
            '>' => {
                self.advance();
                if self.peek_char() == Some('>') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::RedirectAppend))
                } else if self.peek_char() == Some('|') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::Clobber))
                } else if self.peek_char() == Some('(') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::ProcessSubOut))
                } else if self.peek_char() == Some('&') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DupOutput))
                } else {
                    Some(LexedToken::punctuation(TokenKind::RedirectOut))
                }
            }
            '<' => {
                self.advance();
                if self.peek_char() == Some('<') {
                    self.advance();
                    if self.peek_char() == Some('<') {
                        self.advance();
                        Some(LexedToken::punctuation(TokenKind::HereString))
                    } else if self.peek_char() == Some('-') {
                        self.advance();
                        Some(LexedToken::punctuation(TokenKind::HereDocStrip))
                    } else {
                        Some(LexedToken::punctuation(TokenKind::HereDoc))
                    }
                } else if self.peek_char() == Some('>') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::RedirectReadWrite))
                } else if self.peek_char() == Some('(') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::ProcessSubIn))
                } else if self.peek_char() == Some('&') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DupInput))
                } else {
                    Some(LexedToken::punctuation(TokenKind::RedirectIn))
                }
            }
            '(' => {
                self.advance();
                if self.peek_char() == Some('(') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DoubleLeftParen))
                } else {
                    Some(LexedToken::punctuation(TokenKind::LeftParen))
                }
            }
            ')' => {
                self.advance();
                if self.peek_char() == Some(')') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DoubleRightParen))
                } else {
                    Some(LexedToken::punctuation(TokenKind::RightParen))
                }
            }
            '{' => {
                // Look ahead to see if this is a brace expansion like {a,b,c} or {1..5}
                // vs a brace group like { cmd; }
                // Note: { must be followed by space/newline to be a brace group
                if self.looks_like_brace_expansion() {
                    self.read_brace_expansion_word()
                } else if self.is_brace_group_start() {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::LeftBrace))
                } else {
                    // {single} without comma/dot-dot is kept as literal word
                    self.read_brace_literal_word()
                }
            }
            '}' => {
                self.advance();
                Some(LexedToken::punctuation(TokenKind::RightBrace))
            }
            '[' => {
                self.advance();
                if self.peek_char() == Some('[')
                    && matches!(
                        self.peek_nth_char(1),
                        Some(' ') | Some('\t') | Some('\n') | None
                    )
                {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DoubleLeftBracket))
                } else {
                    // `[` can start the test command when followed by whitespace, or it can be
                    // ordinary word text such as a glob bracket expression.
                    //
                    // Read the whole token with the normal word scanner so forms like `[[z]`,
                    // `[hello"]"`, and `[+(])` stay attached to one word instead of producing
                    // structural tokens mid-word.
                    match self.peek_char() {
                        Some(' ') | Some('\t') | Some('\n') | None => {
                            Some(LexedToken::borrowed_word(TokenKind::Word, "[", None))
                        }
                        _ => self.read_word_starting_with("["),
                    }
                }
            }
            ']' => {
                self.advance();
                if self.peek_char() == Some(']') {
                    self.advance();
                    Some(LexedToken::punctuation(TokenKind::DoubleRightBracket))
                } else {
                    Some(LexedToken::borrowed_word(TokenKind::Word, "]", None))
                }
            }
            '\'' => self.read_single_quoted_string(),
            '"' => self.read_double_quoted_string(),
            '#' => {
                if preserve_comments {
                    self.read_comment();
                    Some(LexedToken::comment())
                } else {
                    self.skip_comment();
                    self.next_lexed_token_inner(false)
                }
            }
            // Handle file descriptor redirects like 2> or 2>&1
            '0'..='9' => self.read_word_or_fd_redirect(),
            _ => self.read_word(),
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch == ' ' || ch == '\t' {
                self.advance();
            } else if ch == '\\' {
                // Check for backslash-newline (line continuation) between tokens
                if self.reinject_buf.is_empty() && self.cursor.rest().starts_with("\\\n") {
                    self.consume_source_bytes_without_newline(1);
                    self.advance(); // consume newline
                } else if self.peek_nth_char(1) == Some('\n') {
                    self.advance(); // consume backslash
                    self.advance(); // consume newline
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        if self.reinject_buf.is_empty() {
            let end = self
                .cursor
                .find_byte(b'\n')
                .unwrap_or(self.cursor.rest().len());
            self.consume_source_bytes_without_newline(end);
            return;
        }

        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    fn read_comment(&mut self) {
        debug_assert_eq!(self.peek_char(), Some('#'));

        if self.reinject_buf.is_empty() {
            let rest = self.cursor.rest();
            let end = self.cursor.find_byte(b'\n').unwrap_or(rest.len());
            self.consume_source_bytes_without_newline(end);
            return;
        }

        self.advance(); // consume '#'

        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    /// Check if this is a file descriptor redirect (e.g., 2>, 2>>, 2>&1)
    /// or just a regular word starting with a digit
    fn read_word_or_fd_redirect(&mut self) -> Option<LexedToken<'a>> {
        if let Some(first_digit) = self.peek_char().filter(|ch| ch.is_ascii_digit()) {
            let fd: i32 = first_digit.to_digit(10).unwrap() as i32;

            match (self.peek_nth_char(1), self.peek_nth_char(2)) {
                (Some('>'), Some('>')) => {
                    self.advance(); // consume digit
                    self.advance(); // consume >
                    self.advance(); // consume >
                    return Some(LexedToken::fd(TokenKind::RedirectFdAppend, fd));
                }
                (Some('>'), Some('&')) => {
                    self.advance(); // consume digit
                    self.advance(); // consume >
                    self.advance(); // consume &

                    let mut target_str = String::new();
                    while let Some(c) = self.peek_char() {
                        if c.is_ascii_digit() {
                            target_str.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }

                    if target_str.is_empty() {
                        return Some(LexedToken::fd(TokenKind::RedirectFd, fd));
                    }

                    let target_fd: i32 = target_str.parse().unwrap_or(1);
                    return Some(LexedToken::fd_pair(TokenKind::DupFd, fd, target_fd));
                }
                (Some('>'), _) => {
                    self.advance(); // consume digit
                    self.advance(); // consume >
                    return Some(LexedToken::fd(TokenKind::RedirectFd, fd));
                }
                (Some('<'), Some('&')) => {
                    self.advance(); // consume digit
                    self.advance(); // consume <
                    self.advance(); // consume &

                    let mut target_str = String::new();
                    while let Some(c) = self.peek_char() {
                        if c.is_ascii_digit() || c == '-' {
                            target_str.push(c);
                            self.advance();
                            if c == '-' {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    if target_str == "-" {
                        return Some(LexedToken::fd(TokenKind::DupFdClose, fd));
                    }
                    let target_fd: i32 = target_str.parse().unwrap_or(0);
                    return Some(LexedToken::fd_pair(TokenKind::DupFdIn, fd, target_fd));
                }
                (Some('<'), Some('>')) => {
                    self.advance(); // consume digit
                    self.advance(); // consume <
                    self.advance(); // consume >
                    return Some(LexedToken::fd(TokenKind::RedirectFdReadWrite, fd));
                }
                (Some('<'), Some('<')) => {}
                (Some('<'), _) => {
                    self.advance(); // consume digit
                    self.advance(); // consume <
                    return Some(LexedToken::fd(TokenKind::RedirectFdIn, fd));
                }
                _ => {}
            }
        }

        // Not a fd redirect pattern, read as regular word
        self.read_word()
    }

    fn read_word_starting_with(&mut self, prefix: &str) -> Option<LexedToken<'a>> {
        let mut word = prefix.to_string();
        // Use the same logic as read_word but with pre-seeded content
        while let Some(ch) = self.peek_char() {
            if ch == '"' || ch == '\'' {
                // Word already has content (the prefix) — concatenate the quoted segment
                let quote_char = ch;
                self.advance();
                while let Some(c) = self.peek_char() {
                    if c == quote_char {
                        self.advance();
                        break;
                    }
                    if c == '\\' && quote_char == '"' {
                        self.advance();
                        if let Some(next) = self.peek_char() {
                            match next {
                                '\n' => {
                                    self.advance();
                                }
                                '$' => {
                                    // Use NUL sentinel so parse_word() treats this
                                    // as a literal '$' rather than a variable expansion.
                                    word.push('\x00');
                                    word.push('$');
                                    self.advance();
                                }
                                '"' | '\\' | '`' => {
                                    word.push(next);
                                    self.advance();
                                }
                                _ => {
                                    word.push('\\');
                                    word.push(next);
                                    self.advance();
                                }
                            }
                            continue;
                        }
                    }
                    word.push(c);
                    self.advance();
                }
                continue;
            } else if ch == '$' {
                word.push(ch);
                self.advance();
                // Read variable/expansion following $
                if let Some(nc) = self.peek_char() {
                    if nc == '{' || nc == '(' {
                        word.push(nc);
                        self.advance();
                        let (open, close) = if nc == '{' { ('{', '}') } else { ('(', ')') };
                        let mut depth = 1;
                        while let Some(bc) = self.peek_char() {
                            word.push(bc);
                            self.advance();
                            if bc == open {
                                depth += 1;
                            } else if bc == close {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                    } else if nc.is_ascii_alphanumeric()
                        || nc == '_'
                        || matches!(nc, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                    {
                        word.push(nc);
                        self.advance();
                        if nc.is_ascii_alphabetic() || nc == '_' {
                            while let Some(vc) = self.peek_char() {
                                if vc.is_ascii_alphanumeric() || vc == '_' {
                                    word.push(vc);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                continue;
            } else if ch == '(' && word.ends_with(['@', '?', '*', '+', '!']) {
                // Extglob: @(...), ?(...), *(...), +(...), !(...)
                // Consume through matching ) including nested parens.
                word.push(ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    word.push(c);
                    self.advance();
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                word.push(esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            } else if Self::is_plain_word_char(ch) || ch == ']' {
                if self.reinject_buf.is_empty() {
                    let chunk = self
                        .cursor
                        .eat_while(|c| Self::is_plain_word_char(c) || c == ']');
                    word.push_str(chunk);
                    self.advance_position_without_newline(chunk);
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }
        Some(LexedToken::owned_composite_word(TokenKind::Word, word))
    }

    fn read_word(&mut self) -> Option<LexedToken<'a>> {
        let start = self.position;

        if self.reinject_buf.is_empty() {
            let chunk = self.cursor.eat_while(Self::is_plain_word_char);
            if !chunk.is_empty() {
                self.advance_position_without_newline(chunk);

                let continues = matches!(
                    self.peek_char(),
                    Some(next)
                        if Self::is_word_char(next)
                            || matches!(next, '\'' | '"')
                            || next == '{'
                            || (next == '('
                                && (chunk.ends_with('=') || chunk.ends_with(['@', '?', '*', '+', '!'])))
                );

                if !continues {
                    return Some(LexedToken::borrowed_word(
                        TokenKind::Word,
                        &self.input[start.offset..self.position.offset],
                        Some(Span::from_positions(start, self.position)),
                    ));
                }

                return self.read_complex_word(chunk.to_string());
            }
        }

        self.read_complex_word(String::new())
    }

    fn read_complex_word(&mut self, mut word: String) -> Option<LexedToken<'a>> {
        while let Some(ch) = self.peek_char() {
            // Handle quoted strings within words (e.g., a="Hello" or VAR="value")
            // This handles the case where a word like `a=` is followed by a quoted string
            if ch == '"' || ch == '\'' {
                if word.is_empty() {
                    // Start of a new token — let the main tokenizer handle quotes
                    break;
                }
                // Word already has content — concatenate the quoted segment
                // This handles: VAR="val", date +"%Y", echo foo"bar"
                let quote_char = ch;
                self.advance(); // consume opening quote
                while let Some(c) = self.peek_char() {
                    if c == quote_char {
                        self.advance(); // consume closing quote
                        break;
                    }
                    if c == '\\' && quote_char == '"' {
                        self.advance();
                        if let Some(next) = self.peek_char() {
                            match next {
                                '\n' => {
                                    // \<newline> is line continuation: discard both
                                    self.advance();
                                }
                                '$' => {
                                    // Use NUL sentinel so parse_word() treats this
                                    // as a literal '$' rather than a variable expansion.
                                    word.push('\x00');
                                    word.push('$');
                                    self.advance();
                                }
                                '"' | '\\' | '`' => {
                                    word.push(next);
                                    self.advance();
                                }
                                _ => {
                                    word.push('\\');
                                    word.push(next);
                                    self.advance();
                                }
                            }
                            continue;
                        }
                    }
                    // Handle $(...) inside double-quoted word segments
                    // and ${...} parameter expansion inside double-quoted word
                    // segments so nested quotes don't terminate the outer string.
                    if c == '$' && quote_char == '"' {
                        word.push(c);
                        self.advance();
                        if self.peek_char() == Some('(') {
                            word.push('(');
                            self.advance();
                            if !self.read_command_subst_into(&mut word) {
                                return Some(LexedToken::error(
                                    LexerErrorKind::CommandSubstitution,
                                ));
                            }
                            continue;
                        } else if self.peek_char() == Some('{') {
                            word.push('{');
                            self.advance();
                            self.read_param_expansion_into(&mut word);
                            continue;
                        }
                        continue;
                    }
                    word.push(c);
                    self.advance();
                }
                continue;
            } else if ch == '$' {
                // Handle variable references and command substitution
                self.advance();

                // $'...' — ANSI-C quoting: resolve escapes at parse time
                if self.peek_char() == Some('\'') {
                    self.advance(); // consume opening '
                    word.push_str(&self.read_dollar_single_quoted_content());
                    continue;
                }

                // $"..." — locale translation synonym, treated like "..."
                if self.peek_char() == Some('"') {
                    self.advance(); // consume opening "
                    while let Some(c) = self.peek_char() {
                        if c == '"' {
                            self.advance();
                            break;
                        }
                        if c == '\\' {
                            self.advance();
                            if let Some(next) = self.peek_char() {
                                match next {
                                    '\n' => {
                                        self.advance();
                                    }
                                    '$' => {
                                        word.push('\x00');
                                        word.push('$');
                                        self.advance();
                                    }
                                    '"' | '\\' | '`' => {
                                        word.push(next);
                                        self.advance();
                                    }
                                    _ => {
                                        word.push('\\');
                                        word.push(next);
                                        self.advance();
                                    }
                                }
                                continue;
                            }
                        }
                        if c == '$' {
                            word.push(c);
                            self.advance();
                            if let Some(nc) = self.peek_char() {
                                if nc == '{' {
                                    word.push(nc);
                                    self.advance();
                                    while let Some(bc) = self.peek_char() {
                                        word.push(bc);
                                        self.advance();
                                        if bc == '}' {
                                            break;
                                        }
                                    }
                                } else if nc == '(' {
                                    word.push(nc);
                                    self.advance();
                                    let mut depth = 1;
                                    while let Some(pc) = self.peek_char() {
                                        word.push(pc);
                                        self.advance();
                                        if pc == '(' {
                                            depth += 1;
                                        } else if pc == ')' {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                    }
                                } else if nc.is_ascii_alphanumeric()
                                    || nc == '_'
                                    || matches!(nc, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                                {
                                    word.push(nc);
                                    self.advance();
                                    if nc.is_ascii_alphabetic() || nc == '_' {
                                        while let Some(vc) = self.peek_char() {
                                            if vc.is_ascii_alphanumeric() || vc == '_' {
                                                word.push(vc);
                                                self.advance();
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        word.push(c);
                        self.advance();
                    }
                    continue;
                }

                word.push(ch); // push the '$'

                // Check for $( - command substitution or arithmetic
                if self.peek_char() == Some('(') {
                    word.push('(');
                    self.advance();

                    // Check for $(( - arithmetic expansion
                    if self.peek_char() == Some('(') {
                        word.push('(');
                        self.advance();
                        // Read until ))
                        let mut depth = 2;
                        while let Some(c) = self.peek_char() {
                            word.push(c);
                            self.advance();
                            if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                    } else {
                        if !self.read_command_subst_into(&mut word) {
                            return Some(LexedToken::error(LexerErrorKind::CommandSubstitution));
                        }
                    }
                } else if self.peek_char() == Some('{') {
                    // ${VAR} format — track nested braces so ${a[${#b[@]}]}
                    // doesn't stop at the inner }.
                    word.push('{');
                    self.advance();
                    self.read_param_expansion_into(&mut word);
                } else {
                    // Check for special single-character variables ($?, $#, $@, $*, $!, $$, $-, $0-$9)
                    if let Some(c) = self.peek_char() {
                        if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                            || c.is_ascii_digit()
                        {
                            word.push(c);
                            self.advance();
                        } else {
                            // Read variable name (alphanumeric + _)
                            while let Some(c) = self.peek_char() {
                                if c.is_ascii_alphanumeric() || c == '_' {
                                    word.push(c);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            } else if ch == '{' {
                // Brace expansion pattern - include entire {...} in word
                word.push(ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    word.push(c);
                    self.advance();
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else if ch == '`' {
                // Backtick command substitution: convert `cmd` to $(cmd)
                self.advance(); // consume opening `
                word.push_str("$(");
                let mut closed = false;
                while let Some(c) = self.peek_char() {
                    if c == '`' {
                        self.advance(); // consume closing `
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        // In backticks, backslash only escapes $, `, \, newline
                        self.advance();
                        if let Some(next) = self.peek_char() {
                            if matches!(next, '$' | '`' | '\\' | '\n') {
                                word.push(next);
                                self.advance();
                            } else {
                                word.push('\\');
                                word.push(next);
                                self.advance();
                            }
                        }
                    } else {
                        word.push(c);
                        self.advance();
                    }
                }
                if !closed {
                    return Some(LexedToken::error(LexerErrorKind::BacktickSubstitution));
                }
                word.push(')');
            } else if ch == '\\' {
                self.advance();
                if let Some(next) = self.peek_char() {
                    if next == '\n' {
                        // Line continuation: skip backslash + newline
                        self.advance();
                    } else {
                        // Escaped character: backslash quotes the next char
                        // (quote removal — only the literal char survives)
                        word.push(next);
                        self.advance();
                    }
                } else {
                    word.push('\\');
                }
            } else if ch == '(' && word.ends_with('=') && self.looks_like_assoc_assign() {
                // Associative compound assignment: var=([k]="v" ...) — keep entire
                // (...) as part of word so declare -A m=([k]="v") stays one token.
                word.push(ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    word.push(c);
                    self.advance();
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '"' => {
                            while let Some(qc) = self.peek_char() {
                                word.push(qc);
                                self.advance();
                                if qc == '"' {
                                    break;
                                }
                                if qc == '\\'
                                    && let Some(esc) = self.peek_char()
                                {
                                    word.push(esc);
                                    self.advance();
                                }
                            }
                        }
                        '\'' => {
                            while let Some(qc) = self.peek_char() {
                                word.push(qc);
                                self.advance();
                                if qc == '\'' {
                                    break;
                                }
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                word.push(esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if ch == '(' && word.ends_with(['@', '?', '*', '+', '!']) {
                // Extglob: @(...), ?(...), *(...), +(...), !(...)
                // Consume through matching ) including nested parens
                word.push(ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    word.push(c);
                    self.advance();
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                word.push(esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if Self::is_plain_word_char(ch) {
                if self.reinject_buf.is_empty() {
                    let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                    word.push_str(chunk);
                    self.advance_position_without_newline(chunk);
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        if word.is_empty() {
            None
        } else {
            Some(LexedToken::owned_composite_word(TokenKind::Word, word))
        }
    }

    fn read_single_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.advance(); // consume opening '
        let content_start = self.position;
        let can_borrow = self.reinject_buf.is_empty();
        let mut content_end = content_start;
        let mut content = String::new();
        let mut closed = false;

        while let Some(ch) = self.peek_char() {
            if ch == '\'' {
                content_end = self.position;
                self.advance(); // consume closing '
                closed = true;
                break;
            }
            if !can_borrow {
                content.push(ch);
            }
            self.advance();
        }

        if !closed {
            return Some(LexedToken::error(LexerErrorKind::SingleQuote));
        }

        if can_borrow
            && !matches!(
                self.peek_char(),
                Some(ch) if Self::is_word_char(ch) || matches!(ch, '\'' | '"' | '$')
            )
        {
            return Some(LexedToken::borrowed_word(
                TokenKind::LiteralWord,
                &self.input[content_start.offset..content_end.offset],
                Some(Span::from_positions(content_start, content_end)),
            ));
        }

        if can_borrow {
            content.push_str(&self.input[content_start.offset..content_end.offset]);
        }

        // If next char is another quote or word char, concatenate (e.g., 'EOF'"2" -> EOF2).
        // Any quoting makes the whole token literal.
        self.read_continuation_into(&mut content);

        // Single-quoted strings are literal - no variable expansion
        Some(LexedToken::owned_composite_word(
            TokenKind::LiteralWord,
            content,
        ))
    }

    /// After a closing quote, read any adjacent quoted or unquoted word chars
    /// into `content`.  Handles concatenation like `'foo'"bar"baz` -> `foobarbaz`.
    fn read_continuation_into(&mut self, content: &mut String) {
        loop {
            match self.peek_char() {
                Some('\'') => {
                    self.advance(); // opening '
                    while let Some(ch) = self.peek_char() {
                        if ch == '\'' {
                            self.advance(); // closing '
                            break;
                        }
                        content.push(ch);
                        self.advance();
                    }
                }
                Some('"') => {
                    self.advance(); // opening "
                    while let Some(ch) = self.peek_char() {
                        if ch == '"' {
                            self.advance(); // closing "
                            break;
                        }
                        if ch == '\\' {
                            self.advance();
                            if let Some(next) = self.peek_char() {
                                match next {
                                    '$' => {
                                        content.push('\x00');
                                        content.push('$');
                                        self.advance();
                                    }
                                    '"' | '\\' | '`' => {
                                        content.push(next);
                                        self.advance();
                                    }
                                    _ => {
                                        content.push('\\');
                                        content.push(next);
                                        self.advance();
                                    }
                                }
                                continue;
                            }
                        }
                        content.push(ch);
                        self.advance();
                    }
                }
                Some('$') => {
                    // Check for $'...' ANSI-C quoting in continuation
                    if self.peek_nth_char(1) == Some('\'') {
                        self.advance(); // consume $
                        self.advance(); // consume opening '
                        content.push_str(&self.read_dollar_single_quoted_content());
                    } else {
                        content.push('$');
                        self.advance();
                    }
                }
                Some(ch) if Self::is_plain_word_char(ch) => {
                    if self.reinject_buf.is_empty() {
                        let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                        content.push_str(chunk);
                        self.advance_position_without_newline(chunk);
                    } else {
                        content.push(ch);
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    /// Read ANSI-C quoted content ($'...').
    /// Opening $' already consumed. Returns the resolved string.
    fn read_dollar_single_quoted_content(&mut self) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek_char() {
            if ch == '\'' {
                self.advance();
                break;
            }
            if ch == '\\' {
                self.advance();
                if let Some(esc) = self.peek_char() {
                    self.advance();
                    match esc {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'a' => out.push('\x07'),
                        'b' => out.push('\x08'),
                        'f' => out.push('\x0C'),
                        'v' => out.push('\x0B'),
                        'e' | 'E' => out.push('\x1B'),
                        '\\' => out.push('\\'),
                        '\'' => out.push('\''),
                        '"' => out.push('"'),
                        '?' => out.push('?'),
                        'x' => {
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u8::from_str_radix(&hex, 16) {
                                out.push(val as char);
                            }
                        }
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u32::from_str_radix(&hex, 16)
                                && let Some(c) = char::from_u32(val)
                            {
                                out.push(c);
                            }
                        }
                        'U' => {
                            let mut hex = String::new();
                            for _ in 0..8 {
                                if let Some(h) = self.peek_char() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u32::from_str_radix(&hex, 16)
                                && let Some(c) = char::from_u32(val)
                            {
                                out.push(c);
                            }
                        }
                        '0'..='7' => {
                            let mut oct = String::new();
                            oct.push(esc);
                            for _ in 0..2 {
                                if let Some(o) = self.peek_char() {
                                    if o.is_ascii_digit() && o < '8' {
                                        oct.push(o);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(val) = u8::from_str_radix(&oct, 8) {
                                out.push(val as char);
                            }
                        }
                        _ => {
                            out.push('\\');
                            out.push(esc);
                        }
                    }
                } else {
                    out.push('\\');
                }
                continue;
            }
            out.push(ch);
            self.advance();
        }
        out
    }

    fn read_double_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.advance(); // consume opening "
        let content_start = self.position;
        let mut content_end = content_start;
        let mut simple = self.reinject_buf.is_empty();
        let mut borrowable = self.reinject_buf.is_empty();
        let mut content = String::new();
        let mut closed = false;

        while let Some(ch) = self.peek_char() {
            if simple {
                match ch {
                    '"' => {
                        content_end = self.position;
                        self.advance(); // consume closing "
                        closed = true;
                        break;
                    }
                    '\\' | '$' | '`' => {
                        simple = false;
                        if matches!(ch, '\\' | '`') {
                            borrowable = false;
                        }
                        content.push_str(&self.input[content_start.offset..self.position.offset]);
                    }
                    _ => {
                        self.advance();
                    }
                }
                if simple {
                    continue;
                }
            }

            match ch {
                '"' => {
                    if borrowable {
                        content_end = self.position;
                    }
                    self.advance(); // consume closing "
                    closed = true;
                    break;
                }
                '\\' => {
                    borrowable = false;
                    self.advance();
                    if let Some(next) = self.peek_char() {
                        // Handle escape sequences
                        match next {
                            '\n' => {
                                // \<newline> is line continuation: discard both
                                self.advance();
                            }
                            '$' => {
                                // Use NUL sentinel so parse_word() treats this
                                // as a literal '$' rather than a variable expansion.
                                content.push('\x00');
                                content.push('$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                content.push(next);
                                self.advance();
                            }
                            _ => {
                                content.push('\\');
                                content.push(next);
                                self.advance();
                            }
                        }
                    }
                }
                '$' => {
                    content.push('$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        // $(...) command substitution — track paren depth
                        content.push('(');
                        self.advance();
                        self.read_command_subst_into(&mut content);
                    } else if self.peek_char() == Some('{') {
                        // ${...} parameter expansion — track brace depth so
                        // inner quotes (e.g. ${arr["key"]}) don't end the string
                        content.push('{');
                        self.advance();
                        self.read_param_expansion_into(&mut content);
                    }
                }
                '`' => {
                    borrowable = false;
                    // Backtick command substitution inside double quotes
                    self.advance(); // consume opening `
                    content.push_str("$(");
                    while let Some(c) = self.peek_char() {
                        if c == '`' {
                            self.advance();
                            break;
                        }
                        if c == '\\' {
                            self.advance();
                            if let Some(next) = self.peek_char() {
                                if matches!(next, '$' | '`' | '\\' | '"') {
                                    content.push(next);
                                    self.advance();
                                } else {
                                    content.push('\\');
                                    content.push(next);
                                    self.advance();
                                }
                            }
                        } else {
                            content.push(c);
                            self.advance();
                        }
                    }
                    content.push(')');
                }
                _ => {
                    content.push(ch);
                    self.advance();
                }
            }
        }

        if !closed {
            return Some(LexedToken::error(LexerErrorKind::DoubleQuote));
        }

        // Check for continuation after closing quote: "foo"bar or "foo"/* etc.
        // If there's adjacent unquoted content (word chars, globs, more quotes),
        // concatenate and return as Word (not QuotedWord) so glob expansion works
        // on the unquoted portion.
        if simple
            && !matches!(
                self.peek_char(),
                Some(ch) if Self::is_word_char(ch) || matches!(ch, '\'' | '"' | '$')
            )
        {
            return Some(LexedToken::borrowed_word(
                TokenKind::QuotedWord,
                &self.input[content_start.offset..content_end.offset],
                Some(Span::from_positions(content_start, content_end)),
            ));
        }

        if simple {
            content.push_str(&self.input[content_start.offset..content_end.offset]);
        }

        if let Some(ch) = self.peek_char()
            && (Self::is_word_char(ch) || ch == '\'' || ch == '"' || ch == '$')
        {
            self.read_continuation_into(&mut content);
            return Some(LexedToken::owned_composite_word(TokenKind::Word, content));
        }

        if borrowable {
            return Some(LexedToken::borrowed_word(
                TokenKind::QuotedWord,
                &self.input[content_start.offset..content_end.offset],
                Some(Span::from_positions(content_start, content_end)),
            ));
        }

        Some(LexedToken::owned_composite_word(
            TokenKind::QuotedWord,
            content,
        ))
    }

    /// Read command substitution content after `$(`, handling nested parens and quotes.
    /// Appends chars to `content` and adds the closing `)`.
    /// `subst_depth` tracks nesting to prevent stack overflow.
    fn read_command_subst_into(&mut self, content: &mut String) -> bool {
        self.read_command_subst_into_depth(content, 0)
    }

    fn flush_command_subst_keyword(
        current_word: &mut String,
        pending_case_headers: &mut usize,
        case_clause_depth: &mut usize,
    ) {
        if current_word.is_empty() {
            return;
        }

        match current_word.as_str() {
            "case" => *pending_case_headers += 1,
            "in" if *pending_case_headers > 0 => {
                *pending_case_headers -= 1;
                *case_clause_depth += 1;
            }
            "esac" if *case_clause_depth > 0 => *case_clause_depth -= 1,
            _ => {}
        }

        current_word.clear();
    }

    fn read_command_subst_into_depth(&mut self, content: &mut String, subst_depth: usize) -> bool {
        if subst_depth >= self.max_subst_depth {
            // Depth limit exceeded — consume until matching ')' and emit error token
            let mut depth = 1;
            while let Some(c) = self.peek_char() {
                self.advance();
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            content.push(')');
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            return false;
        }

        let mut depth = 1;
        let mut pending_case_headers = 0usize;
        let mut case_clause_depth = 0usize;
        let mut current_word = String::new();
        while let Some(c) = self.peek_char() {
            match c {
                '(' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    depth += 1;
                    content.push(c);
                    self.advance();
                }
                ')' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    if depth == 1 && case_clause_depth > 0 {
                        content.push(')');
                        self.advance();
                        continue;
                    }
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        content.push(')');
                        return true;
                    }
                    content.push(c);
                }
                '"' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    // Nested double-quoted string inside $()
                    content.push('"');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        match qc {
                            '"' => {
                                content.push('"');
                                self.advance();
                                break;
                            }
                            '\\' => {
                                content.push('\\');
                                self.advance();
                                if let Some(esc) = self.peek_char() {
                                    content.push(esc);
                                    self.advance();
                                }
                            }
                            '$' => {
                                content.push('$');
                                self.advance();
                                if self.peek_char() == Some('(') {
                                    content.push('(');
                                    self.advance();
                                    if !self.read_command_subst_into_depth(content, subst_depth + 1)
                                    {
                                        return false;
                                    }
                                }
                            }
                            _ => {
                                content.push(qc);
                                self.advance();
                            }
                        }
                    }
                }
                '\'' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    // Single-quoted string inside $()
                    content.push('\'');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        content.push(qc);
                        self.advance();
                        if qc == '\'' {
                            break;
                        }
                    }
                }
                '\\' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    content.push('\\');
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        content.push(esc);
                        self.advance();
                    }
                }
                _ => {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        current_word.push(c);
                    } else {
                        Self::flush_command_subst_keyword(
                            &mut current_word,
                            &mut pending_case_headers,
                            &mut case_clause_depth,
                        );
                    }
                    content.push(c);
                    self.advance();
                }
            }
        }

        false
    }

    /// Read parameter expansion content after `${`, handling nested braces and quotes.
    /// In bash, quotes inside `${...}` (e.g. `${arr["key"]}`) don't terminate the
    /// outer double-quoted string. Appends chars including closing `}` to `content`.
    fn read_param_expansion_into(&mut self, content: &mut String) {
        let mut depth = 1;
        let mut in_single = false;
        let mut in_double = false;
        while let Some(c) = self.peek_char() {
            if in_single {
                match c {
                    '\'' => {
                        content.push(c);
                        self.advance();
                        in_single = false;
                    }
                    '\\' => {
                        self.advance();
                        if let Some(esc) = self.peek_char() {
                            match esc {
                                '$' => {
                                    content.push('\x00');
                                    content.push('$');
                                    self.advance();
                                }
                                '"' | '\\' | '`' => {
                                    content.push(esc);
                                    self.advance();
                                }
                                '}' => {
                                    content.push('\\');
                                    content.push('}');
                                    self.advance();
                                }
                                _ => {
                                    content.push('\\');
                                    content.push(esc);
                                    self.advance();
                                }
                            }
                        } else {
                            content.push('\\');
                        }
                    }
                    _ => {
                        content.push(c);
                        self.advance();
                    }
                }
                continue;
            }

            match c {
                '{' if !in_single && !in_double => {
                    depth += 1;
                    content.push(c);
                    self.advance();
                }
                '}' if !in_single && !in_double => {
                    depth -= 1;
                    self.advance();
                    content.push('}');
                    if depth == 0 {
                        break;
                    }
                }
                '"' => {
                    // Quotes inside ${...} are part of the expansion, not string delimiters
                    content.push('"');
                    self.advance();
                    in_double = !in_double;
                }
                '\'' => {
                    content.push('\'');
                    self.advance();
                    if !in_double {
                        in_single = true;
                    }
                }
                '\\' => {
                    // Inside ${...} within double quotes, same escape rules apply:
                    // \", \\, \$, \` produce the escaped char; others keep backslash
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        match esc {
                            '$' => {
                                content.push('\x00');
                                content.push('$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                content.push(esc);
                                self.advance();
                            }
                            '}' => {
                                // \} should be a literal } without closing the expansion
                                content.push('\\');
                                content.push('}');
                                self.advance();
                            }
                            _ => {
                                content.push('\\');
                                content.push(esc);
                                self.advance();
                            }
                        }
                    } else {
                        content.push('\\');
                    }
                }
                '$' => {
                    content.push('$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        content.push('(');
                        self.advance();
                        self.read_command_subst_into(content);
                    } else if self.peek_char() == Some('{') {
                        content.push('{');
                        self.advance();
                        self.read_param_expansion_into(content);
                    }
                }
                _ => {
                    content.push(c);
                    self.advance();
                }
            }
        }
    }

    /// Check if the content starting with { looks like a brace expansion
    /// Brace expansion: {a,b,c} or {1..5} (contains , or ..)
    /// Brace group: { cmd; } (contains spaces, semicolons, newlines)
    /// Caps lookahead to prevent O(n^2) scanning when input
    /// contains many unmatched `{` characters (issue #997).
    fn looks_like_brace_expansion(&self) -> bool {
        const MAX_LOOKAHEAD: usize = 10_000;

        let mut chars = self.lookahead_chars();

        // Skip the opening {
        if chars.next() != Some('{') {
            return false;
        }

        let mut depth = 1;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut prev_char = None;
        let mut scanned = 0usize;

        for ch in chars {
            scanned += 1;
            if scanned > MAX_LOOKAHEAD {
                return false;
            }
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching }, check if we have brace expansion markers
                        return has_comma || has_dot_dot;
                    }
                }
                ',' if depth == 1 => has_comma = true,
                '.' if prev_char == Some('.') && depth == 1 => has_dot_dot = true,
                // Brace groups have whitespace/newlines/semicolons at depth 1
                ' ' | '\t' | '\n' | ';' if depth == 1 => return false,
                _ => {}
            }
            prev_char = Some(ch);
        }

        false
    }

    /// Check if { is followed by whitespace (brace group start)
    fn is_brace_group_start(&self) -> bool {
        let mut chars = self.lookahead_chars();
        // Skip the opening {
        if chars.next() != Some('{') {
            return false;
        }
        // If next char is whitespace or newline, it's a brace group
        matches!(chars.next(), Some(' ') | Some('\t') | Some('\n') | None)
    }

    /// Read a {literal} pattern without comma/dot-dot as a word
    fn read_brace_literal_word(&mut self) -> Option<LexedToken<'a>> {
        let mut word = String::new();

        // Read the opening {
        if let Some('{') = self.peek_char() {
            word.push('{');
            self.advance();
        } else {
            return None;
        }

        // Read until matching }
        let mut depth = 1;
        while let Some(ch) = self.peek_char() {
            word.push(ch);
            self.advance();
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        // Continue reading any suffix
        while let Some(ch) = self.peek_char() {
            if Self::is_word_char(ch) {
                if self.reinject_buf.is_empty() {
                    let chunk = self.cursor.eat_while(Self::is_word_char);
                    word.push_str(chunk);
                    self.advance_position_without_newline(chunk);
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        Some(LexedToken::owned_word(TokenKind::Word, word))
    }

    /// Read a brace expansion pattern as a word
    fn read_brace_expansion_word(&mut self) -> Option<LexedToken<'a>> {
        let mut word = String::new();

        // Read the opening {
        if let Some('{') = self.peek_char() {
            word.push('{');
            self.advance();
        } else {
            return None;
        }

        // Read until matching }
        let mut depth = 1;
        while let Some(ch) = self.peek_char() {
            word.push(ch);
            self.advance();
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        // Continue reading any suffix after the brace pattern
        while let Some(ch) = self.peek_char() {
            if Self::is_word_char(ch) || ch == '{' {
                if ch == '{' {
                    // Another brace pattern - include it
                    word.push(ch);
                    self.advance();
                    let mut inner_depth = 1;
                    while let Some(c) = self.peek_char() {
                        word.push(c);
                        self.advance();
                        match c {
                            '{' => inner_depth += 1,
                            '}' => {
                                inner_depth -= 1;
                                if inner_depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    word.push(ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        Some(LexedToken::owned_word(TokenKind::Word, word))
    }

    /// Peek ahead (without consuming) to see if `=(` starts an associative
    /// compound assignment like `([key]=val ...)`.  Returns true when the
    /// first non-whitespace char after `(` is `[`.
    fn looks_like_assoc_assign(&self) -> bool {
        let mut chars = self.lookahead_chars();
        // Skip the `(` we haven't consumed yet
        if chars.next() != Some('(') {
            return false;
        }
        // Skip optional whitespace
        for ch in chars {
            match ch {
                ' ' | '\t' => continue,
                '[' => return true,
                _ => return false,
            }
        }
        false
    }

    fn is_word_char(ch: char) -> bool {
        !matches!(
            ch,
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | '(' | ')' | '{' | '}' | '\'' | '"'
        )
    }

    fn is_plain_word_char(ch: char) -> bool {
        Self::is_word_char(ch) && !matches!(ch, '$' | '{' | '`' | '\\')
    }

    /// Read here document content until the delimiter line is found
    pub fn read_heredoc(&mut self, delimiter: &str) -> HeredocRead {
        let mut content = String::new();
        let mut current_line = String::new();

        // Save rest of current line (after the delimiter token on the command line).
        // For `cat <<EOF | sort`, this captures ` | sort` so the parser can
        // tokenize the pipe and subsequent command after the heredoc body.
        //
        // Quoted strings may span multiple lines (e.g., `cat <<EOF; echo "two\nthree"`),
        // so we track quoting state and continue across newlines until quotes close.
        let mut rest_of_line = String::new();
        let rest_of_line_start = self.position;
        let mut in_double_quote = false;
        let mut in_single_quote = false;
        while let Some(ch) = self.peek_char() {
            self.advance();
            if ch == '\n' && !in_double_quote && !in_single_quote {
                break;
            }
            if ch == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
            } else if ch == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
            } else if ch == '\\' && in_double_quote {
                // Escaped char inside double quotes — skip the next char too
                rest_of_line.push(ch);
                if let Some(next) = self.peek_char() {
                    rest_of_line.push(next);
                    self.advance();
                }
                continue;
            }
            rest_of_line.push(ch);
        }

        // If we just drained a heredoc replay buffer (for example when multiple
        // heredocs share one command line), resume tracking from the true cursor
        // position before we measure the body span.
        self.sync_position_to_cursor();
        let content_start = self.position;
        let mut current_line_start = self.position;
        let content_end;

        // Read lines until we find the delimiter
        loop {
            if self.reinject_buf.is_empty() {
                let rest = self.cursor.rest();
                if rest.is_empty() {
                    content_end = self.position;
                    break;
                }

                let line_len = self.cursor.find_byte(b'\n').unwrap_or(rest.len());
                let line = &rest[..line_len];
                let has_newline = line_len < rest.len();

                if line.trim() == delimiter {
                    content_end = current_line_start;
                    self.consume_source_bytes_without_newline(line_len);
                    if has_newline {
                        self.advance();
                    }
                    break;
                }

                content.push_str(line);
                self.consume_source_bytes_without_newline(line_len);

                if has_newline {
                    self.advance();
                    content.push('\n');
                    current_line_start = self.position;
                    continue;
                }

                content_end = self.position;
                break;
            }

            match self.peek_char() {
                Some('\n') => {
                    self.advance();
                    // Check if current line matches delimiter
                    if current_line.trim() == delimiter {
                        content_end = current_line_start;
                        break;
                    }
                    content.push_str(&current_line);
                    content.push('\n');
                    current_line.clear();
                    current_line_start = self.position;
                }
                Some(ch) => {
                    current_line.push(ch);
                    self.advance();
                }
                None => {
                    // End of input - check last line
                    if current_line.trim() == delimiter {
                        content_end = current_line_start;
                        break;
                    }
                    if !current_line.is_empty() {
                        content.push_str(&current_line);
                    }
                    content_end = self.position;
                    break;
                }
            }
        }

        // Re-inject the command-line tail so subsequent same-line tokens (pipes,
        // redirects, command words, additional heredocs) stay visible to the
        // parser. Always replay a terminating newline so parsing stops before
        // tokens that originally lived on later source lines, like `}` or `do`.
        let post_heredoc_position = self.position;
        self.position = rest_of_line_start;
        for ch in rest_of_line.chars() {
            self.reinject_buf.push_back(ch);
        }
        self.reinject_buf.push_back('\n');
        self.reinject_resume_position = Some(post_heredoc_position);

        HeredocRead {
            content,
            content_span: Span::from_positions(content_start, content_end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_non_newline_tokens_stay_on_one_line(input: &str) {
        let mut lexer = Lexer::new(input);

        while let Some(token) = lexer.next_spanned_token() {
            if matches!(token.token, Token::Newline | Token::Comment(_)) {
                continue;
            }

            assert_eq!(
                token.span.start.line, token.span.end.line,
                "token should stay on one line: {:?}",
                token
            );
        }
    }

    #[test]
    fn test_simple_words() {
        let mut lexer = Lexer::new("echo hello world");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("hello".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("world".to_string())));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_single_quoted_string() {
        let mut lexer = Lexer::new("echo 'hello world'");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        // Single-quoted strings return LiteralWord (no variable expansion)
        assert_eq!(
            lexer.next_token(),
            Some(Token::LiteralWord("hello world".to_string()))
        );
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_double_quoted_string() {
        let mut lexer = Lexer::new("echo \"hello world\"");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(
            lexer.next_token(),
            Some(Token::QuotedWord("hello world".to_string()))
        );
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_double_quoted_expansion_token_keeps_source_backing() {
        let source = r#""$bar""#;
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::QuotedWord);
        assert_eq!(token.word_text(), Some("$bar"));

        let word = token.word().unwrap();
        let segment = word.single_segment().unwrap();
        assert_eq!(segment.kind(), LexedWordSegmentKind::DoubleQuoted);
        assert_eq!(segment.span().unwrap().slice(source), "$bar");
    }

    #[test]
    fn test_parameter_expansion_replacing_double_quote_stays_on_one_line() {
        let source = r#"out_line="${out_line//'"'/'\"'}"
"#;
        let mut lexer = Lexer::new(source);

        assert_eq!(
            lexer.next_token(),
            Some(Token::Word(r#"out_line=${out_line//'"'/'"'}"#.to_string()))
        );
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_parameter_expansion_replacing_double_quote_does_not_swallow_following_commands() {
        let source = r#"out_line="${out_line//'"'/'\"'}"
echo "Error: Missing python3!"
cat << 'EOF' > "${pywrapper}"
import os
EOF
"#;
        let mut lexer = Lexer::new(source);

        assert_eq!(
            lexer.next_token(),
            Some(Token::Word(r#"out_line=${out_line//'"'/'"'}"#.to_string()))
        );
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(
            lexer.next_token(),
            Some(Token::QuotedWord("Error: Missing python3!".to_string()))
        );
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(
            lexer.next_token(),
            Some(Token::LiteralWord("EOF".to_string()))
        );
        assert_eq!(lexer.next_token(), Some(Token::RedirectOut));
        assert_eq!(
            lexer.next_token(),
            Some(Token::QuotedWord("${pywrapper}".to_string()))
        );
    }

    #[test]
    fn test_operators() {
        let mut lexer = Lexer::new("a |& b | c && d || e; f &");

        assert_eq!(lexer.next_token(), Some(Token::Word("a".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::PipeBoth));
        assert_eq!(lexer.next_token(), Some(Token::Word("b".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Pipe));
        assert_eq!(lexer.next_token(), Some(Token::Word("c".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::And));
        assert_eq!(lexer.next_token(), Some(Token::Word("d".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Or));
        assert_eq!(lexer.next_token(), Some(Token::Word("e".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Semicolon));
        assert_eq!(lexer.next_token(), Some(Token::Word("f".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Background));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_double_left_bracket_requires_separator() {
        let mut lexer = Lexer::new("[[ foo ]]\n[[z]\n");

        assert_eq!(lexer.next_token(), Some(Token::DoubleLeftBracket));
        assert_eq!(lexer.next_token(), Some(Token::Word("foo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::DoubleRightBracket));
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("[[z]".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_redirects() {
        let mut lexer = Lexer::new("a > b >> c < d << e <<< f &>> g <> h");

        assert_eq!(lexer.next_token(), Some(Token::Word("a".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::RedirectOut));
        assert_eq!(lexer.next_token(), Some(Token::Word("b".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::RedirectAppend));
        assert_eq!(lexer.next_token(), Some(Token::Word("c".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::RedirectIn));
        assert_eq!(lexer.next_token(), Some(Token::Word("d".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("e".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereString));
        assert_eq!(lexer.next_token(), Some(Token::Word("f".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::RedirectBothAppend));
        assert_eq!(lexer.next_token(), Some(Token::Word("g".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::RedirectReadWrite));
        assert_eq!(lexer.next_token(), Some(Token::Word("h".to_string())));
    }

    #[test]
    fn test_comment() {
        let mut lexer = Lexer::new("echo hello # this is a comment\necho world");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("hello".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("world".to_string())));
    }

    #[test]
    fn test_comment_token_with_span() {
        let mut lexer = Lexer::new("# lead\necho hi # tail");

        let comment = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(comment.token, Token::Comment(" lead".to_string()));
        assert_eq!(comment.span.start.line, 1);
        assert_eq!(comment.span.start.column, 1);
        assert_eq!(comment.span.end.line, 1);
        assert_eq!(comment.span.end.column, 7);

        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("hi".to_string())));

        let inline = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(inline.token, Token::Comment(" tail".to_string()));
        assert_eq!(inline.span.start.line, 2);
        assert_eq!(inline.span.start.column, 9);
    }

    #[test]
    fn test_comment_token_preserves_hash_boundaries() {
        let mut lexer = Lexer::new("echo foo#bar ${x#y} '# nope' \"# nope\" # yep");

        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("echo".to_string()))
        );
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("foo#bar".to_string()))
        );
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("${x#y}".to_string()))
        );
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::LiteralWord("# nope".to_string()))
        );
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::QuotedWord("# nope".to_string()))
        );
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Comment(" yep".to_string()))
        );
        assert_eq!(lexer.next_token_with_comments(), None);
    }

    #[test]
    fn test_variable_words() {
        let mut lexer = Lexer::new("echo $HOME $USER");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("$HOME".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("$USER".to_string())));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_pipeline_tokens() {
        let mut lexer = Lexer::new("echo hello | cat");

        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("hello".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Pipe));
        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_read_heredoc() {
        // Simulate state after reading "cat <<EOF" - positioned at newline before content
        let mut lexer = Lexer::new("\nhello\nworld\nEOF");
        let content = lexer.read_heredoc("EOF");
        assert_eq!(content.content, "hello\nworld\n");
    }

    #[test]
    fn test_read_heredoc_single_line() {
        let mut lexer = Lexer::new("\ntest\nEOF");
        let content = lexer.read_heredoc("EOF");
        assert_eq!(content.content, "test\n");
    }

    #[test]
    fn test_read_heredoc_full_scenario() {
        // Full scenario: "cat <<EOF\nhello\nworld\nEOF"
        let mut lexer = Lexer::new("cat <<EOF\nhello\nworld\nEOF");

        // Parser would read these tokens
        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("EOF".to_string())));

        // Now read heredoc content
        let content = lexer.read_heredoc("EOF");
        assert_eq!(content.content, "hello\nworld\n");
    }

    #[test]
    fn test_read_heredoc_with_redirect() {
        // Rest-of-line (> file.txt) is re-injected into the lexer buffer
        let mut lexer = Lexer::new("cat <<EOF > file.txt\nhello\nEOF");
        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("EOF".to_string())));
        let content = lexer.read_heredoc("EOF");
        assert_eq!(content.content, "hello\n");
        // The redirect tokens are now available from the lexer
        assert_eq!(lexer.next_token(), Some(Token::RedirectOut));
        assert_eq!(
            lexer.next_token(),
            Some(Token::Word("file.txt".to_string()))
        );
    }

    #[test]
    fn test_read_heredoc_with_redirect_preserves_following_spans() {
        let source = "cat <<EOF > file.txt\nhello\nEOF\n# done\n";
        let mut lexer = Lexer::new(source);

        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("EOF".to_string())));

        let heredoc = lexer.read_heredoc("EOF");
        assert_eq!(heredoc.content, "hello\n");

        let redirect = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(redirect.token, Token::RedirectOut);
        assert_eq!(redirect.span.slice(source), ">");

        let target = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(target.token, Token::Word("file.txt".to_string()));
        assert_eq!(target.span.slice(source), "file.txt");

        let newline = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(newline.token, Token::Newline);
        assert_eq!(newline.span.slice(source), "\n");

        let comment = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(comment.token, Token::Comment(" done".to_string()));
        assert_eq!(comment.span.slice(source), "# done");
    }

    #[test]
    fn test_comment_with_unicode() {
        // Comment containing multi-byte UTF-8 characters
        let source = "# café résumé\necho ok";
        let mut lexer = Lexer::new(source);

        let comment = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(comment.token, Token::Comment(" café résumé".to_string()));
        // Span should cover exactly the comment bytes (including #)
        let start = comment.span.start.offset;
        let end = comment.span.end.offset;
        assert_eq!(start, 0);
        assert_eq!(&source[start..end], "# café résumé");
        assert!(source.is_char_boundary(start));
        assert!(source.is_char_boundary(end));

        assert_eq!(lexer.next_token_with_comments(), Some(Token::Newline));
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("echo".to_string()))
        );
    }

    #[test]
    fn test_comment_with_cjk_characters() {
        // CJK characters are 3-byte UTF-8; offsets must land on char boundaries
        let source = "# 你好世界\necho ok";
        let mut lexer = Lexer::new(source);

        let comment = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(comment.token, Token::Comment(" 你好世界".to_string()));
        let start = comment.span.start.offset;
        let end = comment.span.end.offset;
        assert_eq!(&source[start..end], "# 你好世界");
        assert!(source.is_char_boundary(start));
        assert!(source.is_char_boundary(end));
    }

    #[test]
    fn test_heredoc_with_comments_inside() {
        // Comments inside heredoc body should NOT appear as comment tokens
        let source = "cat <<EOF\n# not a comment\nreal line\nEOF\n# real comment\n";
        let mut lexer = Lexer::new(source);

        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("cat".to_string()))
        );
        assert_eq!(lexer.next_token_with_comments(), Some(Token::HereDoc));
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("EOF".to_string()))
        );

        let heredoc = lexer.read_heredoc("EOF");
        assert_eq!(heredoc.content, "# not a comment\nreal line\n");

        // After heredoc, replayed line termination should appear before
        // tokens from following source lines.
        assert_eq!(lexer.next_token_with_comments(), Some(Token::Newline));
        let comment = lexer.next_spanned_token_with_comments().unwrap();
        assert_eq!(comment.token, Token::Comment(" real comment".to_string()));
    }

    #[test]
    fn test_heredoc_with_hash_in_variable() {
        // ${var#pattern} inside heredoc should not produce comment tokens
        let source = "cat <<EOF\nval=${x#prefix}\nEOF\n";
        let mut lexer = Lexer::new(source);

        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("cat".to_string()))
        );
        assert_eq!(lexer.next_token_with_comments(), Some(Token::HereDoc));
        assert_eq!(
            lexer.next_token_with_comments(),
            Some(Token::Word("EOF".to_string()))
        );

        let heredoc = lexer.read_heredoc("EOF");
        assert_eq!(heredoc.content, "val=${x#prefix}\n");
    }

    #[test]
    fn test_heredoc_span_does_not_leak() {
        // Heredoc content span must be within source bounds and must not
        // overlap with content before or after.
        let source = "cat <<EOF\nhello\nworld\nEOF\necho after";
        let mut lexer = Lexer::new(source);

        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("EOF".to_string())));

        let heredoc = lexer.read_heredoc("EOF");
        let start = heredoc.content_span.start.offset;
        let end = heredoc.content_span.end.offset;
        assert!(
            end <= source.len(),
            "heredoc span end ({end}) exceeds source length ({})",
            source.len()
        );
        assert_eq!(&source[start..end], "hello\nworld\n");

        // Tokens after heredoc should still parse correctly
        assert_eq!(lexer.next_token(), Some(Token::Newline));
        assert_eq!(lexer.next_token(), Some(Token::Word("echo".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::Word("after".to_string())));
    }

    #[test]
    fn test_heredoc_with_unicode_content() {
        // Heredoc containing multi-byte characters; spans must be on char boundaries
        let source = "cat <<EOF\n# 你好\ncafé\nEOF\n";
        let mut lexer = Lexer::new(source);

        assert_eq!(lexer.next_token(), Some(Token::Word("cat".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::HereDoc));
        assert_eq!(lexer.next_token(), Some(Token::Word("EOF".to_string())));

        let heredoc = lexer.read_heredoc("EOF");
        assert_eq!(heredoc.content, "# 你好\ncafé\n");
        let start = heredoc.content_span.start.offset;
        let end = heredoc.content_span.end.offset;
        assert!(
            source.is_char_boundary(start),
            "heredoc span start ({start}) not on char boundary"
        );
        assert!(
            source.is_char_boundary(end),
            "heredoc span end ({end}) not on char boundary"
        );
        assert_eq!(&source[start..end], "# 你好\ncafé\n");
    }

    #[test]
    fn test_assoc_compound_assignment() {
        // declare -A m=([foo]="bar" [baz]="qux") should keep the compound
        // assignment as a single Word token
        let mut lexer = Lexer::new(r#"m=([foo]="bar" [baz]="qux")"#);
        assert_eq!(
            lexer.next_token(),
            Some(Token::Word(r#"m=([foo]="bar" [baz]="qux")"#.to_string()))
        );
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_indexed_array_not_collapsed() {
        // arr=("hello world") should NOT be collapsed — parser handles
        // quoted elements token-by-token via the LeftParen path
        let mut lexer = Lexer::new(r#"arr=("hello world")"#);
        assert_eq!(lexer.next_token(), Some(Token::Word("arr=".to_string())));
        assert_eq!(lexer.next_token(), Some(Token::LeftParen));
    }

    /// Regression test for fuzz crash: single digit at EOF should not panic
    /// (crash-13c5f6f887a11b2296d67f9857975d63b205ac4b)
    #[test]
    fn test_digit_at_eof_no_panic() {
        // A lone digit with no following redirect operator must not panic
        let mut lexer = Lexer::new("2");
        let token = lexer.next_token();
        assert!(token.is_some());
    }

    /// Issue #599: Nested ${...} inside unquoted ${...} must be a single token.
    #[test]
    fn test_nested_brace_expansion_single_token() {
        // ${arr[${#arr[@]} - 1]} should be ONE word token, not split at inner }
        let mut lexer = Lexer::new("${arr[${#arr[@]} - 1]}");
        let token = lexer.next_token();
        assert_eq!(
            token,
            Some(Token::Word("${arr[${#arr[@]} - 1]}".to_string()))
        );
        // No more tokens — everything was consumed
        assert_eq!(lexer.next_token(), None);
    }

    /// Simple ${var} still works after brace depth change.
    #[test]
    fn test_simple_brace_expansion_unchanged() {
        let mut lexer = Lexer::new("${foo}");
        assert_eq!(lexer.next_token(), Some(Token::Word("${foo}".to_string())));
        assert_eq!(lexer.next_token(), None);
    }

    #[test]
    fn test_nvm_fixture_lexes_without_stalling() {
        let input = include_str!("../../../shuck-benchmark/resources/files/nvm.sh");
        let mut lexer = Lexer::new(input);
        let mut tokens = 0usize;

        while lexer.next_token().is_some() {
            tokens += 1;
            assert!(
                tokens < 100_000,
                "lexer should continue making progress on the nvm fixture"
            );
        }

        assert!(tokens > 0, "nvm fixture should produce at least one token");
    }

    #[test]
    fn test_case_arm_with_quoted_space_substitution_stays_line_local() {
        let input = concat!(
            "case \"${_input_type:-}\" in\n",
            "  html) _hashtag_pattern=\"<a\\ href=\\\"${_hashtag_replacement_url//' '/%20}\\\">\\#\\\\2<\\/a>\" ;;\n",
            "  org)  _hashtag_pattern=\"[[${_hashtag_replacement_url//' '/%20}][\\#\\\\2]]\" ;;\n",
            "esac\n",
        );

        assert_non_newline_tokens_stay_on_one_line(input);

        let mut lexer = Lexer::new(input);
        let tokens = std::iter::from_fn(|| lexer.next_token()).collect::<Vec<_>>();
        assert!(tokens.contains(&Token::DoubleSemicolon));
        assert!(tokens.contains(&Token::Word("esac".to_string())));
    }

    #[test]
    fn test_inline_if_with_array_append_stays_line_local() {
        let input = concat!(
            "if [[ -n $arr ]]; then pyout+=(\"${output}\")\n",
            "elif [[ -n $var ]]; then pyout+=\"${output}${ln:+\\n}\"; fi\n",
        );

        assert_non_newline_tokens_stay_on_one_line(input);
    }
}
