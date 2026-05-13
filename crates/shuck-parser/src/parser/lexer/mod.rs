//! Lexer for bash scripts
//!
//! Tokenizes input into a stream of tokens with source position tracking.

use std::{collections::VecDeque, ops::Range, sync::Arc};

use memchr::{memchr, memchr_iter, memrchr};
use shuck_ast::{Position, Span, TokenKind};
use smallvec::SmallVec;

use super::{ShellDialect, ShellProfile, ZshOptionState, ZshOptionTimeline};

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

    pub(crate) const fn has_cooked_text(self) -> bool {
        self.0 & Self::COOKED_TEXT != 0
    }

    pub(crate) const fn is_synthetic(self) -> bool {
        self.0 & Self::SYNTHETIC != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenText<'a> {
    Borrowed(&'a str),
    Shared {
        source: Arc<str>,
        range: Range<usize>,
    },
    Owned(String),
}

impl TokenText<'_> {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(text) => text,
            Self::Shared { source, range } => &source[range.clone()],
            Self::Owned(text) => text,
        }
    }

    fn into_owned<'a>(self) -> TokenText<'a> {
        match self {
            Self::Borrowed(text) => TokenText::Owned(text.to_string()),
            Self::Shared { source, range } => TokenText::Shared { source, range },
            Self::Owned(text) => TokenText::Owned(text),
        }
    }

    fn into_shared<'a>(self, source: &Arc<str>, span: Option<Span>) -> TokenText<'a> {
        match self {
            Self::Borrowed(text) => span
                .filter(|span| span.end.offset <= source.len())
                .map_or_else(
                    || TokenText::Owned(text.to_string()),
                    |span| TokenText::Shared {
                        source: Arc::clone(source),
                        range: span.start.offset..span.end.offset,
                    },
                ),
            Self::Shared { source, range } => TokenText::Shared { source, range },
            Self::Owned(text) => TokenText::Owned(text),
        }
    }
}

/// Classification of one segment inside a lexed shell word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexedWordSegmentKind {
    /// Unquoted or otherwise plain text.
    Plain,
    /// Text from a single-quoted string.
    SingleQuoted,
    /// Text from a `$'...'` string.
    DollarSingleQuoted,
    /// Text from a double-quoted string.
    DoubleQuoted,
    /// Text from a `$"..."` string.
    DollarDoubleQuoted,
    /// Text composed from multiple lexical forms.
    Composite,
}

/// One segment of a lexed shell word, optionally backed by source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexedWordSegment<'a> {
    kind: LexedWordSegmentKind,
    text: TokenText<'a>,
    span: Option<Span>,
    wrapper_span: Option<Span>,
}

impl<'a> LexedWordSegment<'a> {
    fn borrowed(kind: LexedWordSegmentKind, text: &'a str, span: Option<Span>) -> Self {
        Self {
            kind,
            text: TokenText::Borrowed(text),
            span,
            wrapper_span: span,
        }
    }

    fn borrowed_with_spans(
        kind: LexedWordSegmentKind,
        text: &'a str,
        span: Option<Span>,
        wrapper_span: Option<Span>,
    ) -> Self {
        Self {
            kind,
            text: TokenText::Borrowed(text),
            span,
            wrapper_span,
        }
    }

    fn owned(kind: LexedWordSegmentKind, text: String) -> Self {
        Self {
            kind,
            text: TokenText::Owned(text),
            span: None,
            wrapper_span: None,
        }
    }

    fn owned_with_spans(
        kind: LexedWordSegmentKind,
        text: String,
        span: Option<Span>,
        wrapper_span: Option<Span>,
    ) -> Self {
        Self {
            kind,
            text: TokenText::Owned(text),
            span,
            wrapper_span,
        }
    }

    /// Borrow this segment's cooked text.
    pub(crate) fn as_str(&self) -> &str {
        self.text.as_str()
    }

    pub(crate) const fn text_is_source_backed(&self) -> bool {
        matches!(self.text, TokenText::Borrowed(_) | TokenText::Shared { .. })
    }

    /// Return the lexical classification of this segment.
    pub(crate) const fn kind(&self) -> LexedWordSegmentKind {
        self.kind
    }

    /// Return the span of the inner text, if it is tracked.
    pub(crate) const fn span(&self) -> Option<Span> {
        self.span
    }

    /// Return the span including surrounding quoting syntax when available.
    pub(crate) fn wrapper_span(&self) -> Option<Span> {
        self.wrapper_span.or(self.span)
    }

    fn rebased(mut self, base: Position) -> Self {
        self.span = self.span.map(|span| span.rebased(base));
        self.wrapper_span = self.wrapper_span.map(|span| span.rebased(base));
        self
    }

    fn into_owned<'b>(self) -> LexedWordSegment<'b> {
        LexedWordSegment {
            kind: self.kind,
            text: self.text.into_owned(),
            span: self.span,
            wrapper_span: self.wrapper_span,
        }
    }

    fn into_shared<'b>(self, source: &Arc<str>) -> LexedWordSegment<'b> {
        LexedWordSegment {
            kind: self.kind,
            text: self.text.into_shared(source, self.span),
            span: self.span,
            wrapper_span: self.wrapper_span,
        }
    }
}

/// Source-backed representation of a shell word produced by the lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LexedWord<'a> {
    primary_segment: LexedWordSegment<'a>,
    trailing_segments: Vec<LexedWordSegment<'a>>,
}

impl<'a> LexedWord<'a> {
    fn from_segment(primary_segment: LexedWordSegment<'a>) -> Self {
        Self {
            primary_segment,
            trailing_segments: Vec::new(),
        }
    }

    fn borrowed(kind: LexedWordSegmentKind, text: &'a str, span: Option<Span>) -> Self {
        Self::from_segment(LexedWordSegment::borrowed(kind, text, span))
    }

    fn owned(kind: LexedWordSegmentKind, text: String) -> Self {
        Self::from_segment(LexedWordSegment::owned(kind, text))
    }

    fn push_segment(&mut self, segment: LexedWordSegment<'a>) {
        self.trailing_segments.push(segment);
    }

    /// Iterate over the segments that make up this word.
    pub(crate) fn segments(&self) -> impl Iterator<Item = &LexedWordSegment<'a>> {
        std::iter::once(&self.primary_segment).chain(self.trailing_segments.iter())
    }

    /// Return the word text when it is represented by a single segment.
    pub(crate) fn text(&self) -> Option<&str> {
        self.single_segment().map(LexedWordSegment::as_str)
    }

    /// Join all segments into an owned string.
    pub(crate) fn joined_text(&self) -> String {
        let mut text = String::new();
        for segment in self.segments() {
            text.push_str(segment.as_str());
        }
        text
    }

    /// Return the only segment when this word is not segmented.
    pub(crate) fn single_segment(&self) -> Option<&LexedWordSegment<'a>> {
        self.trailing_segments
            .is_empty()
            .then_some(&self.primary_segment)
    }

    fn has_cooked_text(&self) -> bool {
        self.segments()
            .any(|segment| matches!(segment.text, TokenText::Owned(_)))
    }

    fn rebased(mut self, base: Position) -> Self {
        self.primary_segment = self.primary_segment.rebased(base);
        self.trailing_segments = self
            .trailing_segments
            .into_iter()
            .map(|segment| segment.rebased(base))
            .collect();
        self
    }

    fn into_owned<'b>(self) -> LexedWord<'b> {
        LexedWord {
            primary_segment: self.primary_segment.into_owned(),
            trailing_segments: self
                .trailing_segments
                .into_iter()
                .map(LexedWordSegment::into_owned)
                .collect(),
        }
    }

    fn into_shared<'b>(self, source: &Arc<str>) -> LexedWord<'b> {
        LexedWord {
            primary_segment: self.primary_segment.into_shared(source),
            trailing_segments: self
                .trailing_segments
                .into_iter()
                .map(|segment| segment.into_shared(source))
                .collect(),
        }
    }
}

/// Kinds of lexer error payloads attached to `TokenKind::Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexerErrorKind {
    /// Unterminated `$()` command substitution.
    CommandSubstitution,
    /// Unterminated backtick command substitution.
    BacktickSubstitution,
    /// Unterminated single-quoted string.
    SingleQuote,
    /// Unterminated double-quoted string.
    DoubleQuote,
}

impl LexerErrorKind {
    /// Human-readable message for this lexer error kind.
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

/// Token produced by the shell lexer.
///
/// Public consumers can inspect the token kind and source span. Word payloads,
/// descriptor payloads, and lexer recovery details are currently parser-internal
/// so the lexer can evolve without expanding the public API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedToken<'a> {
    /// Token kind used by the parser.
    pub kind: TokenKind,
    /// Source span covered by the token.
    pub span: Span,
    pub(crate) flags: TokenFlags,
    payload: TokenPayload<'a>,
}

impl<'a> LexedToken<'a> {
    fn word_segment_kind(kind: TokenKind) -> LexedWordSegmentKind {
        match kind {
            TokenKind::Word => LexedWordSegmentKind::Plain,
            TokenKind::LiteralWord => LexedWordSegmentKind::SingleQuoted,
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

    fn with_word_payload(kind: TokenKind, word: LexedWord<'a>) -> Self {
        let flags = if word.has_cooked_text() {
            TokenFlags::cooked_text()
        } else {
            TokenFlags::empty()
        };

        Self {
            kind,
            span: Span::new(),
            flags,
            payload: TokenPayload::Word(word),
        }
    }

    fn borrowed_word(kind: TokenKind, text: &'a str, text_span: Option<Span>) -> Self {
        Self::with_word_payload(
            kind,
            LexedWord::borrowed(Self::word_segment_kind(kind), text, text_span),
        )
    }

    fn owned_word(kind: TokenKind, text: String) -> Self {
        Self::with_word_payload(kind, LexedWord::owned(Self::word_segment_kind(kind), text))
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
        self.payload = match self.payload {
            TokenPayload::Word(word) => TokenPayload::Word(word.rebased(base)),
            payload => payload,
        };
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

    pub(crate) fn into_shared<'b>(self, source: &Arc<str>) -> LexedToken<'b> {
        let payload = match self.payload {
            TokenPayload::None => TokenPayload::None,
            TokenPayload::Word(word) => TokenPayload::Word(word.into_shared(source)),
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

    /// Borrow the token text when it is a single-segment word token.
    pub(crate) fn word_text(&self) -> Option<&str> {
        self.kind
            .is_word_like()
            .then_some(())
            .and_then(|_| match &self.payload {
                TokenPayload::Word(word) => word.text(),
                _ => None,
            })
    }

    /// Return an owned string containing the token's word text.
    pub(crate) fn word_string(&self) -> Option<String> {
        self.kind
            .is_word_like()
            .then_some(())
            .and_then(|_| match &self.payload {
                TokenPayload::Word(word) => Some(word.joined_text()),
                _ => None,
            })
    }

    /// Borrow the structured word payload for word-like tokens.
    pub(crate) fn word(&self) -> Option<&LexedWord<'a>> {
        match &self.payload {
            TokenPayload::Word(word) => Some(word),
            _ => None,
        }
    }

    /// Borrow the original source slice when the token is source-backed and uncooked.
    pub(crate) fn source_slice<'b>(&self, source: &'b str) -> Option<&'b str> {
        if !self.kind.is_word_like() || self.flags.has_cooked_text() || self.flags.is_synthetic() {
            return None;
        }

        (self.span.start.offset <= self.span.end.offset && self.span.end.offset <= source.len())
            .then(|| &source[self.span.start.offset..self.span.end.offset])
    }

    /// Return the file-descriptor payload for redirection tokens that carry one.
    pub(crate) fn fd_value(&self) -> Option<i32> {
        match self.payload {
            TokenPayload::Fd(fd) => Some(fd),
            _ => None,
        }
    }

    /// Return the `(source_fd, target_fd)` payload for descriptor-pair redirections.
    pub(crate) fn fd_pair_value(&self) -> Option<(i32, i32)> {
        match self.payload {
            TokenPayload::FdPair(src_fd, dst_fd) => Some((src_fd, dst_fd)),
            _ => None,
        }
    }

    /// Return the lexer error payload when this token represents `TokenKind::Error`.
    pub(crate) fn error_kind(&self) -> Option<LexerErrorKind> {
        match self.payload {
            TokenPayload::Error(kind) => Some(kind),
            _ => None,
        }
    }
}

/// Result of reading a heredoc body from the source.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeredocRead {
    /// Decoded heredoc content.
    pub content: String,
    /// Source span covering the heredoc body content.
    pub content_span: Span,
}

/// Maximum nesting depth for command substitution in the lexer.
/// Prevents stack overflow from deeply nested $() patterns.
const DEFAULT_MAX_SUBST_DEPTH: usize = 50;
const MAX_PARAMETER_EXPANSION_SCAN_DEPTH: usize = 4;

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

    fn second(&self) -> Option<char> {
        let mut chars = self.rest.chars();
        chars.next()?;
        chars.next()
    }

    fn third(&self) -> Option<char> {
        let mut chars = self.rest.chars();
        chars.next()?;
        chars.next()?;
        chars.next()
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

#[derive(Clone, Debug)]
struct PositionMap<'a> {
    source: &'a str,
    line_starts: Arc<[usize]>,
    cached: Position,
}

#[cfg(feature = "benchmarking")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LexerBenchmarkCounters {
    pub(crate) current_position_calls: u64,
}

impl<'a> PositionMap<'a> {
    fn new(source: &'a str) -> Self {
        let mut line_starts =
            Vec::with_capacity(source.bytes().filter(|byte| *byte == b'\n').count() + 1);
        line_starts.push(0);
        line_starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        );

        Self {
            source,
            line_starts: line_starts.into(),
            cached: Position::new(),
        }
    }

    fn position(&mut self, offset: usize) -> Position {
        if offset == self.cached.offset {
            return self.cached;
        }

        let position = if offset > self.cached.offset && offset <= self.source.len() {
            Self::advance_from(self.cached, &self.source[self.cached.offset..offset])
        } else {
            self.position_uncached(offset)
        };
        self.cached = position;
        position
    }

    fn position_uncached(&self, offset: usize) -> Position {
        let offset = offset.min(self.source.len());
        let line_index = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index];
        let line_text = &self.source[line_start..offset];
        let column = if line_text.is_ascii() {
            line_text.len() + 1
        } else {
            line_text.chars().count() + 1
        };

        Position {
            line: line_index + 1,
            column,
            offset,
        }
    }

    fn advance_from(mut position: Position, text: &str) -> Position {
        position.offset += text.len();
        let newline_count = memchr_iter(b'\n', text.as_bytes()).count();
        if newline_count == 0 {
            position.column += if text.is_ascii() {
                text.len()
            } else {
                text.chars().count()
            };
            return position;
        }

        position.line += newline_count;
        let tail_start = memrchr(b'\n', text.as_bytes())
            .map(|index| index + 1)
            .unwrap_or_default();
        let tail = &text[tail_start..];
        position.column = if tail.is_ascii() {
            tail.len() + 1
        } else {
            tail.chars().count() + 1
        };
        position
    }
}

/// Source-backed lexer for shell scripts.
///
/// The public lexer surface is intended for lower-level tooling and
/// benchmarks. It tokenizes using the default bash profile; use the parser
/// constructors when dialect or zsh option state matters.
#[derive(Clone)]
pub struct Lexer<'a> {
    input: &'a str,
    /// Current byte offset in the input/reinjected stream.
    offset: usize,
    cursor: Cursor<'a>,
    position_map: PositionMap<'a>,
    /// Buffer for re-injected characters (e.g., rest-of-line after heredoc delimiter).
    /// Consumed before `cursor`.
    reinject_buf: VecDeque<char>,
    /// Cursor byte offset to restore once a heredoc replay buffer is exhausted.
    reinject_resume_offset: Option<usize>,
    /// Maximum allowed nesting depth for command substitution
    max_subst_depth: usize,
    initial_zsh_options: Option<ZshOptionState>,
    zsh_timeline: Option<Arc<ZshOptionTimeline>>,
    zsh_timeline_index: usize,
    #[cfg(feature = "benchmarking")]
    benchmark_counters: Option<LexerBenchmarkCounters>,
}

mod cursor;
mod heredoc;
mod quotes;
mod substitutions;
mod tokens;
mod word;

pub(super) use heredoc::heredoc_line_matches_delimiter;
pub(super) use substitutions::{
    line_has_unclosed_double_paren, scan_command_substitution_body_len,
    scan_command_substitution_body_len_inner,
};
#[cfg(test)]
mod tests;
