//! Lexer for bash scripts
//!
//! Tokenizes input into a stream of tokens with source position tracking.

use std::{collections::VecDeque, ops::Range, sync::Arc};

use memchr::{memchr, memchr_iter, memrchr};
use shuck_ast::{Position, Span, TokenKind};

use super::{ShellProfile, ZshOptionState, ZshOptionTimeline};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexedWordSegmentKind {
    Plain,
    SingleQuoted,
    DollarSingleQuoted,
    DoubleQuoted,
    DollarDoubleQuoted,
    Composite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedWordSegment<'a> {
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

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    pub const fn kind(&self) -> LexedWordSegmentKind {
        self.kind
    }

    pub const fn span(&self) -> Option<Span> {
        self.span
    }

    pub fn wrapper_span(&self) -> Option<Span> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedWord<'a> {
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

    pub fn segments(&self) -> impl Iterator<Item = &LexedWordSegment<'a>> {
        std::iter::once(&self.primary_segment).chain(self.trailing_segments.iter())
    }

    pub fn text(&self) -> Option<&str> {
        self.single_segment().map(LexedWordSegment::as_str)
    }

    pub fn joined_text(&self) -> String {
        let mut text = String::new();
        for segment in self.segments() {
            text.push_str(segment.as_str());
        }
        text
    }

    pub fn single_segment(&self) -> Option<&LexedWordSegment<'a>> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexerErrorKind {
    CommandSubstitution,
    BacktickSubstitution,
    SingleQuote,
    DoubleQuote,
}

impl LexerErrorKind {
    pub const fn message(self) -> &'static str {
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
pub struct LexedToken<'a> {
    pub kind: TokenKind,
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

    pub fn word_text(&self) -> Option<&str> {
        self.kind
            .is_word_like()
            .then_some(())
            .and_then(|_| match &self.payload {
                TokenPayload::Word(word) => word.text(),
                _ => None,
            })
    }

    pub fn word_string(&self) -> Option<String> {
        self.kind
            .is_word_like()
            .then_some(())
            .and_then(|_| match &self.payload {
                TokenPayload::Word(word) => Some(word.joined_text()),
                _ => None,
            })
    }

    pub fn word(&self) -> Option<&LexedWord<'a>> {
        match &self.payload {
            TokenPayload::Word(word) => Some(word),
            _ => None,
        }
    }

    pub fn source_slice<'b>(&self, source: &'b str) -> Option<&'b str> {
        if !self.kind.is_word_like() || self.flags.has_cooked_text() || self.flags.is_synthetic() {
            return None;
        }

        (self.span.start.offset <= self.span.end.offset && self.span.end.offset <= source.len())
            .then(|| &source[self.span.start.offset..self.span.end.offset])
    }

    pub fn fd_value(&self) -> Option<i32> {
        match self.payload {
            TokenPayload::Fd(fd) => Some(fd),
            _ => None,
        }
    }

    pub fn fd_pair_value(&self) -> Option<(i32, i32)> {
        match self.payload {
            TokenPayload::FdPair(src_fd, dst_fd) => Some((src_fd, dst_fd)),
            _ => None,
        }
    }

    pub fn error_kind(&self) -> Option<LexerErrorKind> {
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
    line_starts: Vec<usize>,
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
            line_starts,
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

/// Lexer for bash scripts.
#[derive(Clone)]
pub struct Lexer<'a> {
    #[allow(dead_code)] // Stored for error reporting in future
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

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given input.
    pub fn new(input: &'a str) -> Self {
        Self::with_max_subst_depth_and_profile(
            input,
            DEFAULT_MAX_SUBST_DEPTH,
            &ShellProfile::native(super::ShellDialect::Bash),
            None,
        )
    }

    /// Create a new lexer with a custom max substitution nesting depth.
    /// Limits recursion in read_command_subst_into().
    pub fn with_max_subst_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_max_subst_depth_and_profile(
            input,
            max_depth,
            &ShellProfile::native(super::ShellDialect::Bash),
            None,
        )
    }

    pub fn with_profile(input: &'a str, shell_profile: &ShellProfile) -> Self {
        let zsh_timeline = (shell_profile.dialect == super::ShellDialect::Zsh)
            .then(|| ZshOptionTimeline::build(input, shell_profile))
            .flatten()
            .map(Arc::new);
        Self::with_max_subst_depth_and_profile(
            input,
            DEFAULT_MAX_SUBST_DEPTH,
            shell_profile,
            zsh_timeline,
        )
    }

    pub(crate) fn with_max_subst_depth_and_profile(
        input: &'a str,
        max_depth: usize,
        shell_profile: &ShellProfile,
        zsh_timeline: Option<Arc<ZshOptionTimeline>>,
    ) -> Self {
        Self {
            input,
            offset: 0,
            cursor: Cursor::new(input),
            position_map: PositionMap::new(input),
            reinject_buf: VecDeque::new(),
            reinject_resume_offset: None,
            max_subst_depth: max_depth,
            initial_zsh_options: shell_profile.zsh_options().cloned(),
            zsh_timeline,
            zsh_timeline_index: 0,
            #[cfg(feature = "benchmarking")]
            benchmark_counters: None,
        }
    }

    /// Get the current position in the input.
    pub fn position(&self) -> Position {
        self.position_map.position_uncached(self.offset)
    }

    fn current_position(&mut self) -> Position {
        #[cfg(feature = "benchmarking")]
        self.maybe_record_current_position_call();
        self.position_map.position(self.offset)
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn enable_benchmark_counters(&mut self) {
        self.benchmark_counters = Some(LexerBenchmarkCounters::default());
    }

    #[cfg(feature = "benchmarking")]
    pub(crate) fn benchmark_counters(&self) -> LexerBenchmarkCounters {
        self.benchmark_counters.unwrap_or_default()
    }

    #[cfg(feature = "benchmarking")]
    fn maybe_record_current_position_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.current_position_calls += 1;
        }
    }

    fn sync_offset_to_cursor(&mut self) {
        if self.reinject_buf.is_empty()
            && let Some(offset) = self.reinject_resume_offset.take()
        {
            self.offset = offset;
        }
    }

    /// Get the next token kind from the input without decoding or materializing
    /// any payload text.
    pub fn next_token_kind(&mut self) -> Option<TokenKind> {
        self.next_lexed_token().map(|token| token.kind)
    }

    fn peek_char(&mut self) -> Option<char> {
        self.sync_offset_to_cursor();
        if let Some(&ch) = self.reinject_buf.front() {
            Some(ch)
        } else {
            self.cursor.first()
        }
    }

    fn advance(&mut self) -> Option<char> {
        self.sync_offset_to_cursor();
        let ch = if !self.reinject_buf.is_empty() {
            self.reinject_buf.pop_front()
        } else {
            self.cursor.bump()
        };
        if let Some(c) = ch {
            self.offset += c.len_utf8();
        }
        ch
    }

    fn lookahead_chars(&self) -> impl Iterator<Item = char> + '_ {
        self.reinject_buf
            .iter()
            .copied()
            .chain(self.cursor.rest().chars())
    }

    fn second_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.second(),
            1 => self.cursor.first(),
            _ => self.reinject_buf.get(1).copied(),
        }
    }

    fn third_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.third(),
            1 => self.cursor.second(),
            2 => self.cursor.first(),
            _ => self.reinject_buf.get(2).copied(),
        }
    }

    fn fourth_char(&self) -> Option<char> {
        match self.reinject_buf.len() {
            0 => self.cursor.rest().chars().nth(3),
            1 => self.cursor.third(),
            2 => self.cursor.second(),
            3 => self.cursor.first(),
            _ => self.reinject_buf.get(3).copied(),
        }
    }

    fn consume_source_bytes(&mut self, byte_len: usize) {
        debug_assert!(self.reinject_buf.is_empty());
        self.sync_offset_to_cursor();
        self.offset += byte_len;
        self.cursor.skip_bytes(byte_len);
    }

    fn advance_scanned_source_bytes(&mut self, byte_len: usize) {
        debug_assert!(self.reinject_buf.is_empty());
        self.offset += byte_len;
    }

    fn consume_ascii_chars(&mut self, count: usize) {
        if self.reinject_buf.is_empty() {
            self.consume_source_bytes(count);
            return;
        }

        for _ in 0..count {
            self.advance();
        }
    }

    fn source_horizontal_whitespace_len(&self) -> usize {
        self.cursor
            .rest()
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count()
    }

    fn source_ascii_plain_word_len(&self) -> usize {
        self.cursor
            .rest()
            .as_bytes()
            .iter()
            .take_while(|byte| Self::is_ascii_plain_word_byte(**byte))
            .count()
    }

    fn find_double_quote_special(source: &str) -> Option<usize> {
        source
            .as_bytes()
            .iter()
            .position(|byte| matches!(*byte, b'"' | b'\\' | b'$' | b'`'))
    }

    fn ensure_capture_from_source(
        &self,
        capture: &mut Option<String>,
        start: Position,
        end: Position,
    ) {
        if capture.is_none() {
            *capture = Some(self.input[start.offset..end.offset].to_string());
        }
    }

    fn push_capture_char(capture: &mut Option<String>, ch: char) {
        if let Some(text) = capture.as_mut() {
            text.push(ch);
        }
    }

    fn push_capture_str(capture: &mut Option<String>, text: &str) {
        if let Some(current) = capture.as_mut() {
            current.push_str(text);
        }
    }

    fn current_zsh_options(&mut self) -> Option<&ZshOptionState> {
        if let Some(timeline) = self.zsh_timeline.as_ref() {
            while self.zsh_timeline_index < timeline.entries.len()
                && timeline.entries[self.zsh_timeline_index].offset <= self.offset
            {
                self.zsh_timeline_index += 1;
            }
            return if self.zsh_timeline_index == 0 {
                self.initial_zsh_options.as_ref()
            } else {
                Some(&timeline.entries[self.zsh_timeline_index - 1].state)
            };
        }

        self.initial_zsh_options.as_ref()
    }

    fn comments_enabled(&mut self) -> bool {
        !self
            .current_zsh_options()
            .is_some_and(|options| options.interactive_comments.is_definitely_off())
    }

    fn rc_quotes_enabled(&mut self) -> bool {
        self.current_zsh_options()
            .is_some_and(|options| options.rc_quotes.is_definitely_on())
    }

    fn ignore_braces_enabled(&mut self) -> bool {
        self.current_zsh_options()
            .is_some_and(|options| options.ignore_braces.is_definitely_on())
    }

    fn ignore_close_braces_enabled(&mut self) -> bool {
        self.current_zsh_options().is_some_and(|options| {
            options.ignore_braces.is_definitely_on()
                || options.ignore_close_braces.is_definitely_on()
        })
    }

    fn should_treat_hash_as_word_char(&mut self) -> bool {
        if !self.comments_enabled() {
            return true;
        }
        self.reinject_buf.is_empty()
            && (self
                .input
                .get(..self.offset)
                .and_then(|prefix| prefix.chars().next_back())
                .is_some_and(|prev| {
                    !prev.is_whitespace() && !matches!(prev, ';' | '|' | '&' | '<' | '>')
                })
                || self.is_inside_unclosed_double_paren_on_line())
    }

    fn current_word_text<'b>(&'b self, start: Position, capture: &'b Option<String>) -> &'b str {
        capture
            .as_deref()
            .unwrap_or(&self.input[start.offset..self.offset])
    }

    /// Get the next source-backed token from the input, skipping line comments.
    pub fn next_lexed_token(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.current_position();
        let token = self.next_lexed_token_inner(false)?;
        let end = self.current_position();
        Some(token.with_span(Span::from_positions(start, end)))
    }

    /// Get the next source-backed token from the input, preserving line comments.
    pub fn next_lexed_token_with_comments(&mut self) -> Option<LexedToken<'a>> {
        self.skip_whitespace();
        let start = self.current_position();
        let token = self.next_lexed_token_inner(true)?;
        let end = self.current_position();
        Some(token.with_span(Span::from_positions(start, end)))
    }

    /// Internal: get next token without recording position (called after whitespace skip)
    fn next_lexed_token_inner(&mut self, preserve_comments: bool) -> Option<LexedToken<'a>> {
        let ch = self.peek_char()?;

        match ch {
            '\n' => {
                self.consume_ascii_chars(1);
                Some(LexedToken::punctuation(TokenKind::Newline))
            }
            ';' => {
                if self.second_char() == Some(';') {
                    if self.third_char() == Some('&') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::DoubleSemiAmp)) // ;;&
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::DoubleSemicolon)) // ;;
                    }
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::SemiPipe)) // ;|
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::SemiAmp)) // ;&
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Semicolon))
                }
            }
            '|' => {
                if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::Or))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::PipeBoth))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Pipe))
                }
            }
            '&' => {
                if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::And))
                } else if self.second_char() == Some('>') {
                    if self.third_char() == Some('>') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::RedirectBothAppend))
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::RedirectBoth))
                    }
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::BackgroundPipe))
                } else if self.second_char() == Some('!') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::BackgroundBang))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::Background))
                }
            }
            '>' => {
                if self.second_char() == Some('>') {
                    if self.third_char() == Some('|') {
                        self.consume_ascii_chars(3);
                    } else {
                        self.consume_ascii_chars(2);
                    }
                    Some(LexedToken::punctuation(TokenKind::RedirectAppend))
                } else if self.second_char() == Some('|') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::Clobber))
                } else if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::ProcessSubOut))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DupOutput))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RedirectOut))
                }
            }
            '<' => {
                if self.second_char() == Some('<') {
                    if self.third_char() == Some('<') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::HereString))
                    } else if self.third_char() == Some('-') {
                        self.consume_ascii_chars(3);
                        Some(LexedToken::punctuation(TokenKind::HereDocStrip))
                    } else {
                        self.consume_ascii_chars(2);
                        Some(LexedToken::punctuation(TokenKind::HereDoc))
                    }
                } else if self.second_char() == Some('>') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::RedirectReadWrite))
                } else if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::ProcessSubIn))
                } else if self.second_char() == Some('&') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DupInput))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RedirectIn))
                }
            }
            '(' => {
                if self.second_char() == Some('(') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleLeftParen))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::LeftParen))
                }
            }
            ')' => {
                if self.second_char() == Some(')') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleRightParen))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::punctuation(TokenKind::RightParen))
                }
            }
            '{' => {
                if self.ignore_braces_enabled() {
                    let start = self.current_position();
                    self.consume_ascii_chars(1);
                    match self.peek_char() {
                        Some(' ') | Some('\t') | Some('\n') | None => {
                            Some(LexedToken::borrowed_word(TokenKind::Word, "{", None))
                        }
                        _ => self.read_word_starting_with("{", start),
                    }
                } else if self.looks_like_brace_expansion() {
                    // Look ahead to see if this is a brace expansion like {a,b,c} or {1..5}
                    // vs a brace group like { cmd; }
                    // Note: { must be followed by space/newline to be a brace group
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
                self.consume_ascii_chars(1);
                if self.ignore_close_braces_enabled() {
                    Some(LexedToken::borrowed_word(TokenKind::Word, "}", None))
                } else {
                    Some(LexedToken::punctuation(TokenKind::RightBrace))
                }
            }
            '[' => {
                let start = self.current_position();
                self.consume_ascii_chars(1);
                if self.peek_char() == Some('[')
                    && matches!(
                        self.second_char(),
                        Some(' ') | Some('\t') | Some('\n') | None
                    )
                {
                    self.consume_ascii_chars(1);
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
                        _ => self.read_word_starting_with("[", start),
                    }
                }
            }
            ']' => {
                if self.second_char() == Some(']') {
                    self.consume_ascii_chars(2);
                    Some(LexedToken::punctuation(TokenKind::DoubleRightBracket))
                } else {
                    self.consume_ascii_chars(1);
                    Some(LexedToken::borrowed_word(TokenKind::Word, "]", None))
                }
            }
            '\'' => self.read_single_quoted_string(),
            '"' => self.read_double_quoted_string(),
            '#' => {
                if self.should_treat_hash_as_word_char() {
                    let start = self.current_position();
                    return self.read_word_starting_with("#", start);
                }
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
            if self.reinject_buf.is_empty() {
                let whitespace_len = self.source_horizontal_whitespace_len();
                if whitespace_len > 0 {
                    self.consume_source_bytes(whitespace_len);
                    continue;
                }

                if self.cursor.rest().starts_with("\\\n") {
                    self.consume_source_bytes(2);
                    continue;
                }
            }

            if ch == ' ' || ch == '\t' {
                self.consume_ascii_chars(1);
            } else if ch == '\\' {
                // Check for backslash-newline (line continuation) between tokens
                if self.second_char() == Some('\n') {
                    self.consume_ascii_chars(2);
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
            self.consume_source_bytes(end);
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
            self.consume_source_bytes(end);
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

    fn is_inside_unclosed_double_paren_on_line(&self) -> bool {
        if !self.reinject_buf.is_empty() || self.offset > self.input.len() {
            return false;
        }

        let line_start = self.input[..self.offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let prefix = &self.input[line_start..self.offset];
        let mut chars = prefix.chars().peekable();
        let mut depth = 0usize;

        while let Some(ch) = chars.next() {
            if ch == '(' && chars.peek() == Some(&'(') {
                chars.next();
                depth += 1;
                continue;
            }
            if ch == ')' && chars.peek() == Some(&')') {
                chars.next();
                depth = depth.saturating_sub(1);
            }
        }

        depth > 0
    }

    /// Check if this is a file descriptor redirect (e.g., 2>, 2>>, 2>&1)
    /// or just a regular word starting with a digit
    fn read_word_or_fd_redirect(&mut self) -> Option<LexedToken<'a>> {
        if let Some(first_digit) = self.peek_char().filter(|ch| ch.is_ascii_digit()) {
            let fd: i32 = first_digit.to_digit(10).unwrap() as i32;

            match (self.second_char(), self.third_char()) {
                (Some('>'), Some('>')) => {
                    if self.fourth_char() == Some('|') {
                        self.consume_ascii_chars(4);
                    } else {
                        self.consume_ascii_chars(3);
                    }
                    return Some(LexedToken::fd(TokenKind::RedirectFdAppend, fd));
                }
                (Some('>'), Some('&')) => {
                    self.consume_ascii_chars(3);

                    let mut target_str = String::with_capacity(4);
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
                    self.consume_ascii_chars(2);
                    return Some(LexedToken::fd(TokenKind::RedirectFd, fd));
                }
                (Some('<'), Some('&')) => {
                    self.consume_ascii_chars(3);

                    let mut target_str = String::with_capacity(4);
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
                    self.consume_ascii_chars(3);
                    return Some(LexedToken::fd(TokenKind::RedirectFdReadWrite, fd));
                }
                (Some('<'), Some('<')) => {}
                (Some('<'), _) => {
                    self.consume_ascii_chars(2);
                    return Some(LexedToken::fd(TokenKind::RedirectFdIn, fd));
                }
                _ => {}
            }
        }

        // Not a fd redirect pattern, read as regular word
        self.read_word()
    }

    fn read_word_starting_with(
        &mut self,
        _prefix: &str,
        start: Position,
    ) -> Option<LexedToken<'a>> {
        let segment = match self.read_unquoted_segment(start) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        if segment.as_str().is_empty() {
            return None;
        }
        let mut lexed_word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut lexed_word) {
            return Some(LexedToken::error(kind));
        }
        Some(LexedToken::with_word_payload(TokenKind::Word, lexed_word))
    }

    fn read_word(&mut self) -> Option<LexedToken<'a>> {
        let start = self.current_position();

        if self.reinject_buf.is_empty() {
            let ascii_len = self.source_ascii_plain_word_len();
            let chunk = if ascii_len > 0
                && self
                    .cursor
                    .rest()
                    .as_bytes()
                    .get(ascii_len)
                    .is_none_or(|byte| byte.is_ascii())
            {
                self.consume_source_bytes(ascii_len);
                &self.input[start.offset..self.offset]
            } else {
                let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                self.advance_scanned_source_bytes(chunk.len());
                chunk
            };
            if !chunk.is_empty() {
                let continues = matches!(
                    self.peek_char(),
                    Some(next)
                        if Self::is_word_char(next)
                            || matches!(next, '\'' | '"')
                            || next == '{'
                            || (next == '('
                                && (chunk.ends_with('=')
                                    || Self::word_can_take_parenthesized_suffix(chunk)))
                );

                if !continues {
                    let end = self.current_position();
                    return Some(LexedToken::borrowed_word(
                        TokenKind::Word,
                        &self.input[start.offset..self.offset],
                        Some(Span::from_positions(start, end)),
                    ));
                }

                if self.peek_char() == Some('(')
                    && (chunk.ends_with('=') || Self::word_can_take_parenthesized_suffix(chunk))
                {
                    return self.read_complex_word(start);
                }

                let end = self.current_position();
                return self.finish_segmented_word(LexedWord::borrowed(
                    LexedWordSegmentKind::Plain,
                    &self.input[start.offset..self.offset],
                    Some(Span::from_positions(start, end)),
                ));
            }
        }

        self.read_complex_word(start)
    }

    fn finish_segmented_word(&mut self, mut lexed_word: LexedWord<'a>) -> Option<LexedToken<'a>> {
        if let Err(kind) = self.append_segmented_continuation(&mut lexed_word) {
            return Some(LexedToken::error(kind));
        }

        Some(LexedToken::with_word_payload(TokenKind::Word, lexed_word))
    }

    fn read_complex_word(&mut self, start: Position) -> Option<LexedToken<'a>> {
        if self.peek_char() == Some('$') {
            match self.second_char() {
                Some('\'') => return self.read_dollar_single_quoted_string(),
                Some('"') => return self.read_dollar_double_quoted_string(),
                _ => {}
            }
        }

        let segment = match self.read_unquoted_segment(start) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };

        if segment.as_str().is_empty() {
            return None;
        }

        self.finish_segmented_word(LexedWord::from_segment(segment))
    }

    fn read_unquoted_segment(
        &mut self,
        start: Position,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        let mut word = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));
        while let Some(ch) = self.peek_char() {
            if ch == '"' || ch == '\'' {
                break;
            } else if ch == '$' {
                if matches!(self.second_char(), Some('\'') | Some('"'))
                    && (self.current_position().offset > start.offset
                        || word.as_ref().is_some_and(|word| !word.is_empty()))
                {
                    break;
                }

                // Handle variable references and command substitution
                self.advance();

                Self::push_capture_char(&mut word, ch); // push the '$'

                // Check for $( - command substitution or arithmetic
                if self.peek_char() == Some('(') {
                    Self::push_capture_char(&mut word, '(');
                    self.advance();

                    // Check for $(( - arithmetic expansion
                    if self.peek_char() == Some('(') {
                        Self::push_capture_char(&mut word, '(');
                        self.advance();
                        // Read until ))
                        let mut depth = 2;
                        while let Some(c) = self.peek_char() {
                            Self::push_capture_char(&mut word, c);
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
                            return Err(LexerErrorKind::CommandSubstitution);
                        }
                    }
                } else if self.peek_char() == Some('{') {
                    // ${VAR} format — track nested braces so ${a[${#b[@]}]}
                    // doesn't stop at the inner }.
                    Self::push_capture_char(&mut word, '{');
                    self.advance();
                    let _ = self.read_param_expansion_into(&mut word, start);
                } else {
                    // Check for special single-character variables ($?, $#, $@, $*, $!, $$, $-, $0-$9)
                    if let Some(c) = self.peek_char() {
                        if matches!(c, '?' | '#' | '@' | '*' | '!' | '$' | '-')
                            || c.is_ascii_digit()
                        {
                            Self::push_capture_char(&mut word, c);
                            self.advance();
                        } else {
                            // Read variable name (alphanumeric + _)
                            while let Some(c) = self.peek_char() {
                                if c.is_ascii_alphanumeric() || c == '_' {
                                    Self::push_capture_char(&mut word, c);
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
                Self::push_capture_char(&mut word, ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
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
                // Preserve legacy backticks verbatim so the parser can keep the
                // original syntax form.
                let capture_end = self.current_position();
                self.ensure_capture_from_source(&mut word, start, capture_end);
                Self::push_capture_char(&mut word, ch);
                self.advance(); // consume opening `
                let mut closed = false;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
                    self.advance();
                    if c == '`' {
                        closed = true;
                        break;
                    }
                    if c == '\\'
                        && let Some(next) = self.peek_char()
                    {
                        Self::push_capture_char(&mut word, next);
                        self.advance();
                    }
                }
                if !closed {
                    return Err(LexerErrorKind::BacktickSubstitution);
                }
            } else if ch == '\\' {
                let capture_end = self.current_position();
                self.ensure_capture_from_source(&mut word, start, capture_end);
                self.advance();
                if let Some(next) = self.peek_char() {
                    if next == '\n' {
                        // Line continuation: skip backslash + newline
                        self.advance();
                    } else {
                        // Escaped character: backslash quotes the next char
                        // (quote removal — only the literal char survives)
                        Self::push_capture_char(&mut word, next);
                        self.advance();
                        if next == '{'
                            && self.current_word_text(start, &word) == "{"
                            && self.escaped_brace_sequence_looks_like_brace_expansion()
                        {
                            let mut depth = 1;
                            while let Some(c) = self.peek_char() {
                                Self::push_capture_char(&mut word, c);
                                self.advance();
                                match c {
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
                        }
                    }
                } else {
                    Self::push_capture_char(&mut word, '\\');
                }
            } else if ch == '('
                && self.current_word_text(start, &word).ends_with('=')
                && self.looks_like_assoc_assign()
            {
                // Associative compound assignment: var=([k]="v" ...) — keep entire
                // (...) as part of word so declare -A m=([k]="v") stays one token.
                Self::push_capture_char(&mut word, ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
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
                                Self::push_capture_char(&mut word, qc);
                                self.advance();
                                if qc == '"' {
                                    break;
                                }
                                if qc == '\\'
                                    && let Some(esc) = self.peek_char()
                                {
                                    Self::push_capture_char(&mut word, esc);
                                    self.advance();
                                }
                            }
                        }
                        '\'' => {
                            while let Some(qc) = self.peek_char() {
                                Self::push_capture_char(&mut word, qc);
                                self.advance();
                                if qc == '\'' {
                                    break;
                                }
                            }
                        }
                        '\\' => {
                            if let Some(esc) = self.peek_char() {
                                Self::push_capture_char(&mut word, esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if ch == '('
                && self
                    .current_word_text(start, &word)
                    .ends_with(['@', '?', '*', '+', '!'])
            {
                // Extglob: @(...), ?(...), *(...), +(...), !(...)
                // Consume through matching ) including nested parens
                Self::push_capture_char(&mut word, ch);
                self.advance();
                let mut depth = 1;
                while let Some(c) = self.peek_char() {
                    Self::push_capture_char(&mut word, c);
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
                                Self::push_capture_char(&mut word, esc);
                                self.advance();
                            }
                        }
                        _ => {}
                    }
                }
            } else if Self::is_plain_word_char(ch) {
                if self.reinject_buf.is_empty() {
                    let ascii_len = self.source_ascii_plain_word_len();
                    let chunk = if ascii_len > 0
                        && self
                            .cursor
                            .rest()
                            .as_bytes()
                            .get(ascii_len)
                            .is_none_or(|byte| byte.is_ascii())
                    {
                        self.consume_source_bytes(ascii_len);
                        &self.input[self.offset - ascii_len..self.offset]
                    } else {
                        let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                        self.advance_scanned_source_bytes(chunk.len());
                        chunk
                    };
                    Self::push_capture_str(&mut word, chunk);
                } else {
                    Self::push_capture_char(&mut word, ch);
                    self.advance();
                }
            } else {
                break;
            }
        }

        if let Some(word) = word {
            Ok(LexedWordSegment::owned(LexedWordSegmentKind::Plain, word))
        } else {
            let end = self.current_position();
            Ok(LexedWordSegment::borrowed(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..self.offset],
                Some(Span::from_positions(start, end)),
            ))
        }
    }

    fn read_single_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        let segment = match self.read_single_quoted_segment() {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        Some(LexedToken::with_word_payload(TokenKind::LiteralWord, word))
    }

    fn read_single_quoted_segment(&mut self) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        debug_assert_eq!(self.peek_char(), Some('\''));

        let wrapper_start = self.current_position();
        self.consume_ascii_chars(1); // consume opening '
        let content_start = self.current_position();
        let can_borrow = self.reinject_buf.is_empty() && !self.rc_quotes_enabled();
        let mut content_end = content_start;
        let mut content = String::with_capacity(16);
        let mut closed = false;

        if can_borrow {
            let rest = self.cursor.rest();
            if let Some(quote_index) = memchr(b'\'', rest.as_bytes()) {
                self.consume_source_bytes(quote_index);
                content_end = self.current_position();
                self.consume_ascii_chars(1); // consume closing '
                closed = true;
            } else {
                self.consume_source_bytes(rest.len());
            }
        }

        while let Some(ch) = self.peek_char() {
            if closed {
                break;
            }
            if ch == '\'' {
                if self.rc_quotes_enabled() && self.second_char() == Some('\'') {
                    if !can_borrow {
                        content.push('\'');
                    }
                    self.advance();
                    self.advance();
                    continue;
                }
                content_end = self.current_position();
                self.consume_ascii_chars(1); // consume closing '
                closed = true;
                break;
            }
            if !can_borrow {
                content.push(ch);
            }
            self.advance();
        }

        if !closed {
            return Err(LexerErrorKind::SingleQuote);
        }

        let wrapper_span = Some(Span::from_positions(wrapper_start, self.current_position()));
        let content_span = Some(Span::from_positions(content_start, content_end));

        if can_borrow {
            Ok(LexedWordSegment::borrowed_with_spans(
                LexedWordSegmentKind::SingleQuoted,
                &self.input[content_start.offset..content_end.offset],
                content_span,
                wrapper_span,
            ))
        } else {
            Ok(LexedWordSegment::owned_with_spans(
                LexedWordSegmentKind::SingleQuoted,
                content,
                content_span,
                wrapper_span,
            ))
        }
    }

    fn read_dollar_single_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        let segment = match self.read_dollar_single_quoted_segment() {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        let kind = if word.single_segment().is_some() {
            TokenKind::LiteralWord
        } else {
            TokenKind::Word
        };

        Some(LexedToken::with_word_payload(kind, word))
    }

    fn read_dollar_single_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        debug_assert_eq!(self.peek_char(), Some('$'));
        debug_assert_eq!(self.second_char(), Some('\''));

        let wrapper_start = self.current_position();
        self.consume_ascii_chars(2); // consume $'
        let content_start = self.current_position();
        let mut out = String::with_capacity(16);

        while let Some(ch) = self.peek_char() {
            if ch == '\'' {
                let content_end = self.current_position();
                self.advance();
                let wrapper_span =
                    Some(Span::from_positions(wrapper_start, self.current_position()));
                let content_span = Some(Span::from_positions(content_start, content_end));
                return Ok(LexedWordSegment::owned_with_spans(
                    LexedWordSegmentKind::DollarSingleQuoted,
                    out,
                    content_span,
                    wrapper_span,
                ));
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
                        'c' => {
                            if let Some(control) = self.peek_char() {
                                self.advance();
                                out.push(((control as u32 & 0x1F) as u8) as char);
                            } else {
                                out.push('\\');
                                out.push('c');
                            }
                        }
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

        Err(LexerErrorKind::SingleQuote)
    }

    fn read_plain_continuation_segment(&mut self) -> Option<LexedWordSegment<'a>> {
        let start = self.current_position();

        if self.reinject_buf.is_empty() {
            let ascii_len = self.source_ascii_plain_word_len();
            let chunk = if ascii_len > 0
                && self
                    .cursor
                    .rest()
                    .as_bytes()
                    .get(ascii_len)
                    .is_none_or(|byte| byte.is_ascii())
            {
                self.consume_source_bytes(ascii_len);
                &self.input[start.offset..self.offset]
            } else {
                let chunk = self.cursor.eat_while(Self::is_plain_word_char);
                self.advance_scanned_source_bytes(chunk.len());
                chunk
            };
            if chunk.is_empty() {
                return None;
            }

            let end = self.current_position();
            return Some(LexedWordSegment::borrowed(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..self.offset],
                Some(Span::from_positions(start, end)),
            ));
        }

        let ch = self.peek_char()?;
        if !Self::is_plain_word_char(ch) {
            return None;
        }

        let mut text = String::with_capacity(16);
        while let Some(ch) = self.peek_char() {
            if !Self::is_plain_word_char(ch) {
                break;
            }
            text.push(ch);
            self.advance();
        }

        Some(LexedWordSegment::owned(LexedWordSegmentKind::Plain, text))
    }

    /// After a closing quote, read any adjacent quoted or unquoted word chars
    /// into `word`. Handles concatenation like `'foo'"bar"baz`.
    fn append_segmented_continuation(
        &mut self,
        word: &mut LexedWord<'a>,
    ) -> Result<(), LexerErrorKind> {
        loop {
            match self.peek_char() {
                Some('\'') => {
                    word.push_segment(self.read_single_quoted_segment()?);
                }
                Some('"') => {
                    word.push_segment(self.read_double_quoted_segment()?);
                }
                Some('$') if self.second_char() == Some('\'') => {
                    word.push_segment(self.read_dollar_single_quoted_segment()?);
                }
                Some('$') if self.second_char() == Some('"') => {
                    word.push_segment(self.read_dollar_double_quoted_segment()?);
                }
                Some('(') if Self::lexed_word_can_take_parenthesized_suffix(word) => {
                    let segment = self
                        .read_parenthesized_word_suffix_segment()
                        .expect("peeked '(' should produce a suffix segment");
                    word.push_segment(segment);
                }
                _ => {
                    if let Some(segment) = self.read_plain_continuation_segment() {
                        word.push_segment(segment);
                        continue;
                    }

                    let start = self.current_position();
                    let plain = self.read_unquoted_segment(start)?;
                    if plain.as_str().is_empty() {
                        break;
                    }
                    word.push_segment(plain);
                }
            }
        }

        Ok(())
    }

    fn read_parenthesized_word_suffix_segment(&mut self) -> Option<LexedWordSegment<'a>> {
        debug_assert_eq!(self.peek_char(), Some('('));

        let start = self.current_position();
        let mut depth = 0usize;
        let mut escaped = false;
        let mut text = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));

        while let Some(ch) = self.peek_char() {
            if let Some(text) = text.as_mut() {
                text.push(ch);
            }
            self.advance();

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        let end = self.current_position();
        let span = Some(Span::from_positions(start, end));
        if let Some(text) = text {
            Some(LexedWordSegment::owned_with_spans(
                LexedWordSegmentKind::Plain,
                text,
                span,
                span,
            ))
        } else {
            Some(LexedWordSegment::borrowed_with_spans(
                LexedWordSegmentKind::Plain,
                &self.input[start.offset..end.offset],
                span,
                span,
            ))
        }
    }

    fn read_double_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.read_double_quoted_word(false)
    }

    fn read_dollar_double_quoted_string(&mut self) -> Option<LexedToken<'a>> {
        self.read_double_quoted_word(true)
    }

    fn read_double_quoted_word(&mut self, dollar: bool) -> Option<LexedToken<'a>> {
        let segment = match self.read_double_quoted_segment_with_dollar(dollar) {
            Ok(segment) => segment,
            Err(kind) => return Some(LexedToken::error(kind)),
        };
        let mut word = LexedWord::from_segment(segment);
        if let Err(kind) = self.append_segmented_continuation(&mut word) {
            return Some(LexedToken::error(kind));
        }

        let kind = if word.single_segment().is_some() {
            TokenKind::QuotedWord
        } else {
            TokenKind::Word
        };

        Some(LexedToken::with_word_payload(kind, word))
    }

    fn read_double_quoted_segment(&mut self) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        self.read_double_quoted_segment_with_dollar(false)
    }

    fn read_dollar_double_quoted_segment(
        &mut self,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        self.read_double_quoted_segment_with_dollar(true)
    }

    fn read_double_quoted_segment_with_dollar(
        &mut self,
        dollar: bool,
    ) -> Result<LexedWordSegment<'a>, LexerErrorKind> {
        if dollar {
            debug_assert_eq!(self.peek_char(), Some('$'));
            debug_assert_eq!(self.second_char(), Some('"'));
        } else {
            debug_assert_eq!(self.peek_char(), Some('"'));
        }

        let wrapper_start = self.current_position();
        if dollar {
            self.consume_ascii_chars(2); // consume $"
        } else {
            self.consume_ascii_chars(1); // consume opening "
        }
        let content_start = self.current_position();
        let mut content_end = content_start;
        let mut simple = self.reinject_buf.is_empty();
        let mut borrowable = self.reinject_buf.is_empty();
        let mut content = (!self.reinject_buf.is_empty()).then(|| String::with_capacity(16));
        let mut closed = false;

        while let Some(ch) = self.peek_char() {
            if simple {
                if self.reinject_buf.is_empty() {
                    let rest = self.cursor.rest();
                    match Self::find_double_quote_special(rest) {
                        Some(index) if index > 0 => {
                            self.consume_source_bytes(index);
                            continue;
                        }
                        None => {
                            self.consume_source_bytes(rest.len());
                            return Err(LexerErrorKind::DoubleQuote);
                        }
                        _ => {}
                    }
                }

                match ch {
                    '"' => {
                        content_end = self.current_position();
                        self.consume_ascii_chars(1); // consume closing "
                        closed = true;
                        break;
                    }
                    '\\' | '$' | '`' => {
                        simple = false;
                        if ch == '`' {
                            borrowable = false;
                            let capture_end = self.current_position();
                            self.ensure_capture_from_source(
                                &mut content,
                                content_start,
                                capture_end,
                            );
                        }
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
                        content_end = self.current_position();
                    }
                    self.consume_ascii_chars(1); // consume closing "
                    closed = true;
                    break;
                }
                '\\' => {
                    let escape_start = self.current_position();
                    self.advance();
                    if let Some(next) = self.peek_char() {
                        match next {
                            '\n' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                self.advance();
                            }
                            '$' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                Self::push_capture_char(&mut content, '\x00');
                                Self::push_capture_char(&mut content, '$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    &mut content,
                                    content_start,
                                    escape_start,
                                );
                                if next == '`' {
                                    Self::push_capture_char(&mut content, '\x00');
                                }
                                Self::push_capture_char(&mut content, next);
                                self.advance();
                                content_end = self.current_position();
                            }
                            _ => {
                                Self::push_capture_char(&mut content, '\\');
                                Self::push_capture_char(&mut content, next);
                                self.advance();
                                content_end = self.current_position();
                            }
                        }
                    }
                }
                '$' => {
                    Self::push_capture_char(&mut content, '$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        Self::push_capture_char(&mut content, '(');
                        self.advance();
                        self.read_command_subst_into(&mut content);
                    } else if self.peek_char() == Some('{') {
                        Self::push_capture_char(&mut content, '{');
                        self.advance();
                        borrowable &= self.read_param_expansion_into(&mut content, content_start);
                    }
                    content_end = self.current_position();
                }
                '`' => {
                    borrowable = false;
                    let capture_end = self.current_position();
                    self.ensure_capture_from_source(&mut content, content_start, capture_end);
                    Self::push_capture_char(&mut content, '`');
                    self.advance(); // consume opening `
                    while let Some(c) = self.peek_char() {
                        Self::push_capture_char(&mut content, c);
                        self.advance();
                        if c == '`' {
                            break;
                        }
                        if c == '\\'
                            && let Some(next) = self.peek_char()
                        {
                            Self::push_capture_char(&mut content, next);
                            self.advance();
                        }
                    }
                    content_end = self.current_position();
                }
                _ => {
                    Self::push_capture_char(&mut content, ch);
                    self.advance();
                    content_end = self.current_position();
                }
            }
        }

        if !closed {
            return Err(LexerErrorKind::DoubleQuote);
        }

        let wrapper_span = Some(Span::from_positions(wrapper_start, self.current_position()));
        let content_span = Some(Span::from_positions(content_start, content_end));

        if borrowable {
            Ok(LexedWordSegment::borrowed_with_spans(
                if dollar {
                    LexedWordSegmentKind::DollarDoubleQuoted
                } else {
                    LexedWordSegmentKind::DoubleQuoted
                },
                &self.input[content_start.offset..content_end.offset],
                content_span,
                wrapper_span,
            ))
        } else {
            Ok(LexedWordSegment::owned_with_spans(
                if dollar {
                    LexedWordSegmentKind::DollarDoubleQuoted
                } else {
                    LexedWordSegmentKind::DoubleQuoted
                },
                content.unwrap_or_default(),
                content_span,
                wrapper_span,
            ))
        }
    }

    /// Read command substitution content after `$(`, handling nested parens and quotes.
    /// Appends chars to `content` and adds the closing `)`.
    /// `subst_depth` tracks nesting to prevent stack overflow.
    fn read_command_subst_into(&mut self, content: &mut Option<String>) -> bool {
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

    fn read_command_subst_into_depth(
        &mut self,
        content: &mut Option<String>,
        subst_depth: usize,
    ) -> bool {
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
                            Self::push_capture_char(content, ')');
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
        let mut current_word = String::with_capacity(16);
        while let Some(c) = self.peek_char() {
            match c {
                '#' if !self.should_treat_hash_as_word_char() => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    Self::push_capture_char(content, '#');
                    self.advance();
                    while let Some(comment_ch) = self.peek_char() {
                        Self::push_capture_char(content, comment_ch);
                        self.advance();
                        if comment_ch == '\n' {
                            break;
                        }
                    }
                }
                '(' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    depth += 1;
                    Self::push_capture_char(content, c);
                    self.advance();
                }
                ')' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    if depth == 1 && case_clause_depth > 0 {
                        Self::push_capture_char(content, ')');
                        self.advance();
                        continue;
                    }
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        Self::push_capture_char(content, ')');
                        return true;
                    }
                    Self::push_capture_char(content, c);
                }
                '"' => {
                    Self::flush_command_subst_keyword(
                        &mut current_word,
                        &mut pending_case_headers,
                        &mut case_clause_depth,
                    );
                    // Nested double-quoted string inside $()
                    Self::push_capture_char(content, '"');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        match qc {
                            '"' => {
                                Self::push_capture_char(content, '"');
                                self.advance();
                                break;
                            }
                            '\\' => {
                                Self::push_capture_char(content, '\\');
                                self.advance();
                                if let Some(esc) = self.peek_char() {
                                    Self::push_capture_char(content, esc);
                                    self.advance();
                                }
                            }
                            '$' => {
                                Self::push_capture_char(content, '$');
                                self.advance();
                                if self.peek_char() == Some('(') {
                                    Self::push_capture_char(content, '(');
                                    self.advance();
                                    if !self.read_command_subst_into_depth(content, subst_depth + 1)
                                    {
                                        return false;
                                    }
                                }
                            }
                            _ => {
                                Self::push_capture_char(content, qc);
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
                    Self::push_capture_char(content, '\'');
                    self.advance();
                    while let Some(qc) = self.peek_char() {
                        Self::push_capture_char(content, qc);
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
                    Self::push_capture_char(content, '\\');
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        Self::push_capture_char(content, esc);
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
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }

        false
    }

    /// Read parameter expansion content after `${`, handling nested braces and quotes.
    /// In bash, quotes inside `${...}` (e.g. `${arr["key"]}`) don't terminate the
    /// outer double-quoted string. Appends chars including closing `}` to `content`.
    fn read_param_expansion_into(
        &mut self,
        content: &mut Option<String>,
        segment_start: Position,
    ) -> bool {
        let mut borrowable = true;
        let mut depth = 1;
        let mut literal_brace_depth = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        while let Some(c) = self.peek_char() {
            if in_single {
                match c {
                    '\\' => {
                        let escape_start = self.current_position();
                        if self.second_char() == Some('"') {
                            self.advance();
                            borrowable = false;
                            self.ensure_capture_from_source(content, segment_start, escape_start);
                            Self::push_capture_char(content, '"');
                            self.advance();
                        } else {
                            Self::push_capture_char(content, '\\');
                            self.advance();
                        }
                    }
                    '\'' => {
                        Self::push_capture_char(content, c);
                        self.advance();
                        in_single = false;
                    }
                    _ => {
                        Self::push_capture_char(content, c);
                        self.advance();
                    }
                }
                continue;
            }

            match c {
                '}' if !in_single && !in_double => {
                    self.advance();
                    Self::push_capture_char(content, '}');
                    if literal_brace_depth > 0
                        && self.has_later_top_level_param_expansion_closer(depth)
                    {
                        literal_brace_depth -= 1;
                        continue;
                    }
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '{' if !in_single && !in_double => {
                    literal_brace_depth += 1;
                    Self::push_capture_char(content, '{');
                    self.advance();
                }
                '"' => {
                    // Quotes inside ${...} are part of the expansion, not string delimiters
                    Self::push_capture_char(content, '"');
                    self.advance();
                    in_double = !in_double;
                }
                '\'' => {
                    Self::push_capture_char(content, '\'');
                    self.advance();
                    if !in_double {
                        in_single = true;
                    }
                }
                '\\' => {
                    // Inside ${...} within double quotes, same escape rules apply:
                    // \", \\, \$, \` produce the escaped char; others keep backslash
                    let escape_start = self.current_position();
                    self.advance();
                    if let Some(esc) = self.peek_char() {
                        match esc {
                            '$' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    content,
                                    segment_start,
                                    escape_start,
                                );
                                Self::push_capture_char(content, '\x00');
                                Self::push_capture_char(content, '$');
                                self.advance();
                            }
                            '"' | '\\' | '`' => {
                                borrowable = false;
                                self.ensure_capture_from_source(
                                    content,
                                    segment_start,
                                    escape_start,
                                );
                                Self::push_capture_char(content, esc);
                                self.advance();
                            }
                            '}' => {
                                // \} should be a literal } without closing the expansion
                                Self::push_capture_char(content, '\\');
                                Self::push_capture_char(content, '}');
                                self.advance();
                                literal_brace_depth = literal_brace_depth.saturating_sub(1);
                            }
                            _ => {
                                Self::push_capture_char(content, '\\');
                                Self::push_capture_char(content, esc);
                                self.advance();
                            }
                        }
                    } else {
                        Self::push_capture_char(content, '\\');
                    }
                }
                '$' => {
                    Self::push_capture_char(content, '$');
                    self.advance();
                    if self.peek_char() == Some('(') {
                        Self::push_capture_char(content, '(');
                        self.advance();
                        self.read_command_subst_into(content);
                    } else if self.peek_char() == Some('{') {
                        Self::push_capture_char(content, '{');
                        self.advance();
                        borrowable &= self.read_param_expansion_into(content, segment_start);
                    }
                }
                _ => {
                    Self::push_capture_char(content, c);
                    self.advance();
                }
            }
        }
        borrowable
    }

    fn has_later_top_level_param_expansion_closer(&self, target_depth: usize) -> bool {
        let mut chars = self.lookahead_chars().peekable();
        let mut depth = target_depth;
        let mut in_single = false;
        let mut in_double = false;

        while let Some(ch) = chars.next() {
            if in_single {
                match ch {
                    '\'' => in_single = false,
                    '\\' => {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if in_double {
                match ch {
                    '"' => in_double = false,
                    '\\' => {
                        chars.next();
                    }
                    '$' if chars.peek() == Some(&'{') => {
                        chars.next();
                        depth += 1;
                    }
                    _ => {}
                }
                continue;
            }

            match ch {
                '\n' if depth == target_depth => return false,
                '\'' => in_single = true,
                '"' => in_double = true,
                '\\' => {
                    chars.next();
                }
                '$' if chars.peek() == Some(&'{') => {
                    chars.next();
                    depth += 1;
                }
                '}' => {
                    if depth == target_depth {
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }

        false
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

    /// Check whether the text after an escaped `{` looks like a brace-expansion
    /// surface that should stay attached to the current word, e.g. `\{a,b}`.
    fn escaped_brace_sequence_looks_like_brace_expansion(&self) -> bool {
        const MAX_LOOKAHEAD: usize = 10_000;

        let mut chars = self.lookahead_chars();
        let mut depth = 1;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut prev_char = None;
        let mut scanned = 0usize;

        for ch in chars.by_ref() {
            scanned += 1;
            if scanned > MAX_LOOKAHEAD {
                return false;
            }
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return has_comma || has_dot_dot;
                    }
                }
                ',' if depth == 1 => has_comma = true,
                '.' if prev_char == Some('.') && depth == 1 => has_dot_dot = true,
                ' ' | '\t' | '\n' | ';' if depth == 1 => return false,
                _ => {}
            }
            prev_char = Some(ch);
        }

        false
    }

    /// Read a {literal} pattern without comma/dot-dot as a word
    fn read_brace_literal_word(&mut self) -> Option<LexedToken<'a>> {
        let mut word = String::with_capacity(16);

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
                    self.advance_scanned_source_bytes(chunk.len());
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
        let mut word = String::with_capacity(16);

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
            if Self::is_word_char(ch) || matches!(ch, '{' | '}') {
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

    fn word_can_take_parenthesized_suffix(text: &str) -> bool {
        text.ends_with(['@', '?', '*', '+', '!']) || Self::looks_like_zsh_glob_qualifier_base(text)
    }

    fn lexed_word_can_take_parenthesized_suffix(word: &LexedWord<'_>) -> bool {
        word.segments().any(|segment| {
            matches!(
                segment.kind(),
                LexedWordSegmentKind::SingleQuoted
                    | LexedWordSegmentKind::DollarSingleQuoted
                    | LexedWordSegmentKind::DoubleQuoted
                    | LexedWordSegmentKind::DollarDoubleQuoted
            )
        }) || Self::word_can_take_parenthesized_suffix(&word.joined_text())
    }

    fn looks_like_zsh_glob_qualifier_base(text: &str) -> bool {
        text.contains(['*', '?'])
            || text.ends_with('}') && text.contains("${")
            || text.ends_with(']')
                && text
                    .rfind('[')
                    .is_some_and(|open_bracket| !text[..open_bracket].ends_with('$'))
    }

    fn is_word_char(ch: char) -> bool {
        !matches!(
            ch,
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '>' | '<' | '(' | ')' | '{' | '}' | '\'' | '"'
        )
    }

    const fn is_ascii_word_byte(byte: u8) -> bool {
        !matches!(
            byte,
            b' ' | b'\t'
                | b'\n'
                | b';'
                | b'|'
                | b'&'
                | b'>'
                | b'<'
                | b'('
                | b')'
                | b'{'
                | b'}'
                | b'\''
                | b'"'
        )
    }

    const fn is_ascii_plain_word_byte(byte: u8) -> bool {
        Self::is_ascii_word_byte(byte) && !matches!(byte, b'$' | b'{' | b'`' | b'\\')
    }

    fn is_plain_word_char(ch: char) -> bool {
        Self::is_word_char(ch) && !matches!(ch, '$' | '{' | '`' | '\\')
    }

    /// Read here document content until the delimiter line is found
    pub fn read_heredoc(&mut self, delimiter: &str, strip_tabs: bool) -> HeredocRead {
        let mut content = String::with_capacity(64);
        let mut current_line = String::with_capacity(64);

        // Save rest of current line (after the delimiter token on the command line).
        // For `cat <<EOF | sort`, this captures ` | sort` so the parser can
        // tokenize the pipe and subsequent command after the heredoc body.
        //
        // Quoted strings may span multiple lines (e.g., `cat <<EOF; echo "two\nthree"`),
        // so we track quoting state and continue across newlines until quotes close.
        let mut rest_of_line = String::with_capacity(32);
        let rest_of_line_start = self.current_position();
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
        self.sync_offset_to_cursor();
        let content_start = self.current_position();
        let mut current_line_start = content_start;
        let content_end;

        // Read lines until we find the delimiter
        loop {
            if self.reinject_buf.is_empty() {
                // When the body reading drains a reinject buffer (from a
                // previous heredoc on the same command line), the virtual
                // offset drifts away from the cursor. Snap it back before
                // any source-based work so spans and `post_heredoc_offset`
                // stay within bounds.
                self.sync_offset_to_cursor();
                let rest = self.cursor.rest();
                if rest.is_empty() {
                    content_end = self.current_position();
                    break;
                }

                let line_len = self.cursor.find_byte(b'\n').unwrap_or(rest.len());
                let line = &rest[..line_len];
                let has_newline = line_len < rest.len();

                if heredoc_line_matches_delimiter(line, delimiter, strip_tabs) {
                    content_end = current_line_start;
                    self.consume_source_bytes(line_len);
                    if has_newline {
                        self.consume_ascii_chars(1);
                    }
                    break;
                }

                content.push_str(line);
                self.consume_source_bytes(line_len);

                if has_newline {
                    self.consume_ascii_chars(1);
                    content.push('\n');
                    current_line_start = self.current_position();
                    continue;
                }

                content_end = self.current_position();
                break;
            }

            match self.peek_char() {
                Some('\n') => {
                    self.advance();
                    // Check if current line matches delimiter
                    if heredoc_line_matches_delimiter(&current_line, delimiter, strip_tabs) {
                        content_end = current_line_start;
                        break;
                    }
                    content.push_str(&current_line);
                    content.push('\n');
                    current_line.clear();
                    current_line_start = self.current_position();
                }
                Some(ch) => {
                    current_line.push(ch);
                    self.advance();
                }
                None => {
                    // End of input - check last line
                    if heredoc_line_matches_delimiter(&current_line, delimiter, strip_tabs) {
                        content_end = current_line_start;
                        break;
                    }
                    if !current_line.is_empty() {
                        content.push_str(&current_line);
                    }
                    content_end = self.current_position();
                    break;
                }
            }
        }

        // Re-inject the command-line tail so subsequent same-line tokens (pipes,
        // redirects, command words, additional heredocs) stay visible to the
        // parser. Always replay a terminating newline so parsing stops before
        // tokens that originally lived on later source lines, like `}` or `do`.
        let post_heredoc_offset = self.offset;
        self.offset = rest_of_line_start.offset;
        for ch in rest_of_line.chars() {
            self.reinject_buf.push_back(ch);
        }
        self.reinject_buf.push_back('\n');
        self.reinject_resume_offset = Some(post_heredoc_offset);

        HeredocRead {
            content,
            content_span: Span::from_positions(content_start, content_end),
        }
    }
}

fn heredoc_line_matches_delimiter(line: &str, delimiter: &str, strip_tabs: bool) -> bool {
    if strip_tabs {
        line.trim_start_matches('\t') == delimiter
    } else {
        line == delimiter
    }
}

pub(super) fn scan_command_substitution_body_len(input: &str) -> Option<usize> {
    let mut lexer = Lexer::new(input);
    let mut content = Some(String::new());
    lexer
        .read_command_subst_into(&mut content)
        .then_some(lexer.offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_text(token: &LexedToken<'_>, source: &str) -> Option<String> {
        match token.kind {
            kind if kind.is_word_like() => token.word_string(),
            TokenKind::Comment => token
                .span
                .slice(source)
                .strip_prefix('#')
                .map(str::to_string),
            TokenKind::Error => token
                .error_kind()
                .map(LexerErrorKind::message)
                .map(str::to_string),
            _ => None,
        }
    }

    fn assert_next_token(
        lexer: &mut Lexer<'_>,
        expected_kind: TokenKind,
        expected_text: Option<&str>,
    ) {
        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, expected_kind);
        assert_eq!(token_text(&token, lexer.input).as_deref(), expected_text);
    }

    fn assert_next_token_with_comments(
        lexer: &mut Lexer<'_>,
        expected_kind: TokenKind,
        expected_text: Option<&str>,
    ) {
        let token = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(token.kind, expected_kind);
        assert_eq!(token_text(&token, lexer.input).as_deref(), expected_text);
    }

    fn assert_non_newline_tokens_stay_on_one_line(input: &str) {
        let mut lexer = Lexer::new(input);

        while let Some(token) = lexer.next_lexed_token() {
            if token.kind == TokenKind::Newline {
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

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("world"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_single_quoted_string() {
        let mut lexer = Lexer::new("echo 'hello world'");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        // Single-quoted strings return LiteralWord (no variable expansion)
        assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("hello world"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_double_quoted_string() {
        let mut lexer = Lexer::new("echo \"hello world\"");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("hello world"));
        assert!(lexer.next_lexed_token().is_none());
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
    fn test_mixed_word_keeps_segment_kinds() {
        let source = r#"foo"bar"'baz'"#;
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);

        let word = token.word().unwrap();
        let segments: Vec<_> = word
            .segments()
            .map(|segment| (segment.kind(), segment.as_str().to_string()))
            .collect();

        assert_eq!(
            segments,
            vec![
                (LexedWordSegmentKind::Plain, "foo".to_string()),
                (LexedWordSegmentKind::DoubleQuoted, "bar".to_string()),
                (LexedWordSegmentKind::SingleQuoted, "baz".to_string()),
            ]
        );
        assert_eq!(word.joined_text(), "foobarbaz");
        assert_eq!(
            word.segments()
                .next()
                .and_then(LexedWordSegment::span)
                .unwrap()
                .slice(source),
            "foo"
        );
    }

    #[test]
    fn test_single_quoted_prefix_keeps_plain_continuation_segment() {
        let source = "'foo'bar";
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::LiteralWord);

        let word = token.word().unwrap();
        let segments: Vec<_> = word
            .segments()
            .map(|segment| (segment.kind(), segment.as_str().to_string()))
            .collect();

        assert_eq!(
            segments,
            vec![
                (LexedWordSegmentKind::SingleQuoted, "foo".to_string()),
                (LexedWordSegmentKind::Plain, "bar".to_string()),
            ]
        );
        assert_eq!(word.joined_text(), "foobar");
        assert_eq!(
            word.segments()
                .nth(1)
                .and_then(LexedWordSegment::span)
                .unwrap()
                .slice(source),
            "bar"
        );
    }

    #[test]
    fn test_unquoted_command_substitution_word_keeps_source_backing() {
        let source = "$(printf hi)";
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);

        let word = token.word().unwrap();
        let segment = word.single_segment().unwrap();
        assert_eq!(segment.kind(), LexedWordSegmentKind::Plain);
        assert_eq!(segment.as_str(), source);
        assert_eq!(segment.span().unwrap().slice(source), source);
    }

    #[test]
    fn test_unquoted_nested_param_expansion_word_keeps_source_backing() {
        let source = "${arr[$RANDOM % ${#arr[@]}]}";
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);

        let word = token.word().unwrap();
        let segment = word.single_segment().unwrap();
        assert_eq!(segment.kind(), LexedWordSegmentKind::Plain);
        assert_eq!(segment.as_str(), source);
        assert_eq!(segment.span().unwrap().slice(source), source);
    }

    #[test]
    fn test_quoted_prefix_with_command_substitution_continuation_keeps_source_backing() {
        let source = "\"foo\"$(printf hi)";
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);

        let word = token.word().unwrap();
        let continuation = word.segments().nth(1).unwrap();
        assert_eq!(continuation.kind(), LexedWordSegmentKind::Plain);
        assert_eq!(continuation.as_str(), "$(printf hi)");
        assert_eq!(continuation.span().unwrap().slice(source), "$(printf hi)");
    }

    #[test]
    fn test_double_quoted_nested_param_expansion_keeps_source_backing() {
        let source = r#""${arr[$RANDOM % ${#arr[@]}]}""#;
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::QuotedWord);

        let word = token.word().unwrap();
        let segment = word.single_segment().unwrap();
        assert_eq!(segment.kind(), LexedWordSegmentKind::DoubleQuoted);
        assert_eq!(segment.as_str(), "${arr[$RANDOM % ${#arr[@]}]}");
        assert_eq!(
            segment.span().unwrap().slice(source),
            "${arr[$RANDOM % ${#arr[@]}]}"
        );
    }

    #[test]
    fn test_ansi_c_control_escape_can_consume_quote() {
        let mut lexer = Lexer::new("echo $'\\c''");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("\x07"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_parameter_expansion_replacing_double_quote_stays_on_one_line() {
        let source = r#"out_line="${out_line//'"'/'\"'}"
"#;
        let mut lexer = Lexer::new(source);

        assert_next_token(
            &mut lexer,
            TokenKind::Word,
            Some(r#"out_line=${out_line//'"'/'"'}"#),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert!(lexer.next_lexed_token().is_none());
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

        assert_next_token(
            &mut lexer,
            TokenKind::Word,
            Some(r#"out_line=${out_line//'"'/'"'}"#),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(
            &mut lexer,
            TokenKind::QuotedWord,
            Some("Error: Missing python3!"),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("EOF"));
        assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
        assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("${pywrapper}"));
    }

    #[test]
    fn test_trim_pattern_with_literal_left_brace_does_not_swallow_following_tokens() {
        let source = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
        let mut lexer = Lexer::new(source);

        assert_next_token(
            &mut lexer,
            TokenKind::Word,
            Some("dns_servercow_info=ServerCow.de\nSite: ServerCow.de\n"),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);
        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert_next_token(&mut lexer, TokenKind::LeftBrace, None);
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("if"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("true"));
        assert_next_token(&mut lexer, TokenKind::Semicolon, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("then"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(
            &mut lexer,
            TokenKind::Word,
            Some(
                "txtvalue_old=${response#*{\"name\":\"\"$_sub_domain\"\",\"ttl\":20,\"type\":\"TXT\",\"content\":\"}",
            ),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("fi"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::RightBrace, None);
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_operators() {
        let mut lexer = Lexer::new("a |& b | c && d || e; f &");

        assert_next_token(&mut lexer, TokenKind::Word, Some("a"));
        assert_next_token(&mut lexer, TokenKind::PipeBoth, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("b"));
        assert_next_token(&mut lexer, TokenKind::Pipe, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("c"));
        assert_next_token(&mut lexer, TokenKind::And, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("d"));
        assert_next_token(&mut lexer, TokenKind::Or, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("e"));
        assert_next_token(&mut lexer, TokenKind::Semicolon, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
        assert_next_token(&mut lexer, TokenKind::Background, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_double_left_bracket_requires_separator() {
        let mut lexer = Lexer::new("[[ foo ]]\n[[z]\n");

        assert_next_token(&mut lexer, TokenKind::DoubleLeftBracket, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("foo"));
        assert_next_token(&mut lexer, TokenKind::DoubleRightBracket, None);
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("[[z]"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_redirects() {
        let mut lexer = Lexer::new("a > b >> c >>| d 2>>| e < f << g <<< h &>> i <> j");

        assert_next_token(&mut lexer, TokenKind::Word, Some("a"));
        assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("b"));
        assert_next_token(&mut lexer, TokenKind::RedirectAppend, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("c"));
        assert_next_token(&mut lexer, TokenKind::RedirectAppend, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("d"));
        assert_next_token(&mut lexer, TokenKind::RedirectFdAppend, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("e"));
        assert_next_token(&mut lexer, TokenKind::RedirectIn, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("f"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("g"));
        assert_next_token(&mut lexer, TokenKind::HereString, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("h"));
        assert_next_token(&mut lexer, TokenKind::RedirectBothAppend, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("i"));
        assert_next_token(&mut lexer, TokenKind::RedirectReadWrite, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("j"));
    }

    #[test]
    fn test_comment() {
        let mut lexer = Lexer::new("echo hello # this is a comment\necho world");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("world"));
    }

    #[test]
    fn test_comment_token_with_span() {
        let mut lexer = Lexer::new("# lead\necho hi # tail");

        let comment = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(comment.kind, TokenKind::Comment);
        assert_eq!(token_text(&comment, lexer.input).as_deref(), Some(" lead"));
        assert_eq!(comment.span.start.line, 1);
        assert_eq!(comment.span.start.column, 1);
        assert_eq!(comment.span.end.line, 1);
        assert_eq!(comment.span.end.column, 7);

        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("hi"));

        let inline = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(inline.kind, TokenKind::Comment);
        assert_eq!(token_text(&inline, lexer.input).as_deref(), Some(" tail"));
        assert_eq!(inline.span.start.line, 2);
        assert_eq!(inline.span.start.column, 9);
    }

    #[test]
    fn test_comment_token_preserves_hash_boundaries() {
        let mut lexer = Lexer::new("echo foo#bar ${x#y} '# nope' \"# nope\" # yep");

        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("foo#bar"));
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("${x#y}"));
        assert_next_token_with_comments(&mut lexer, TokenKind::LiteralWord, Some("# nope"));
        assert_next_token_with_comments(&mut lexer, TokenKind::QuotedWord, Some("# nope"));
        assert_next_token_with_comments(&mut lexer, TokenKind::Comment, Some(" yep"));
        assert!(lexer.next_lexed_token_with_comments().is_none());
    }

    #[test]
    fn test_zsh_inline_glob_control_after_left_paren_is_not_comment() {
        let mut lexer = Lexer::new("if [[ \"$buf\" == (#b)(*)(${~pat})* ]]; then\n");

        let mut saw_comment = false;
        while let Some(token) = lexer.next_lexed_token_with_comments() {
            if token.kind == TokenKind::Comment {
                saw_comment = true;
                break;
            }
        }

        assert!(
            !saw_comment,
            "zsh inline glob controls inside [[ ]] should not lex as comments"
        );
    }

    #[test]
    fn test_zsh_arithmetic_char_literal_inside_double_parens_is_not_comment() {
        let mut lexer = Lexer::new("(( #c < 256 / $1 * $1 )) && break\n");

        let mut saw_comment = false;
        while let Some(token) = lexer.next_lexed_token_with_comments() {
            if token.kind == TokenKind::Comment {
                saw_comment = true;
                break;
            }
        }

        assert!(
            !saw_comment,
            "zsh arithmetic char literals inside (( )) should not lex as comments"
        );
    }

    #[test]
    fn test_double_quoted_parameter_replacement_with_embedded_quotes_stays_single_word() {
        let mut lexer = Lexer::new(
            "builtin printf '\\e]133;C;cmdline_url=%s\\a' \"${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}\"\n",
        );

        assert_next_token(&mut lexer, TokenKind::Word, Some("builtin"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("printf"));
        assert_next_token(
            &mut lexer,
            TokenKind::LiteralWord,
            Some("\\e]133;C;cmdline_url=%s\\a"),
        );
        assert_next_token(
            &mut lexer,
            TokenKind::QuotedWord,
            Some("${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}"),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
    }

    #[test]
    fn test_anonymous_function_body_with_nested_replacement_word_keeps_closing_brace_token() {
        let mut lexer = Lexer::new(
            "() {\n  builtin printf '\\e]133;C;cmdline_url=%s\\a' \"${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}\"\n} \"$1\"\n",
        );

        assert_next_token(&mut lexer, TokenKind::LeftParen, None);
        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert_next_token(&mut lexer, TokenKind::LeftBrace, None);
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("builtin"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("printf"));
        assert_next_token(
            &mut lexer,
            TokenKind::LiteralWord,
            Some("\\e]133;C;cmdline_url=%s\\a"),
        );
        assert_next_token(
            &mut lexer,
            TokenKind::QuotedWord,
            Some("${1//(#m)[^a-zA-Z0-9\"\\/:_.-!'()~\"]/%${(l:2::0:)$(([##16]#MATCH))}}"),
        );
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::RightBrace, None);
        assert_next_token(&mut lexer, TokenKind::QuotedWord, Some("$1"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
    }

    #[test]
    fn test_variable_words() {
        let mut lexer = Lexer::new("echo $HOME $USER");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("$HOME"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("$USER"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_pipeline_tokens() {
        let mut lexer = Lexer::new("echo hello | cat");

        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("hello"));
        assert_next_token(&mut lexer, TokenKind::Pipe, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_read_heredoc() {
        // Simulate state after reading "cat <<EOF" - positioned at newline before content
        let mut lexer = Lexer::new("\nhello\nworld\nEOF");
        let content = lexer.read_heredoc("EOF", false);
        assert_eq!(content.content, "hello\nworld\n");
    }

    #[test]
    fn test_read_heredoc_single_line() {
        let mut lexer = Lexer::new("\ntest\nEOF");
        let content = lexer.read_heredoc("EOF", false);
        assert_eq!(content.content, "test\n");
    }

    #[test]
    fn test_read_heredoc_full_scenario() {
        // Full scenario: "cat <<EOF\nhello\nworld\nEOF"
        let mut lexer = Lexer::new("cat <<EOF\nhello\nworld\nEOF");

        // Parser would read these tokens
        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

        // Now read heredoc content
        let content = lexer.read_heredoc("EOF", false);
        assert_eq!(content.content, "hello\nworld\n");
    }

    #[test]
    fn test_read_heredoc_with_redirect() {
        // Rest-of-line (> file.txt) is re-injected into the lexer buffer
        let mut lexer = Lexer::new("cat <<EOF > file.txt\nhello\nEOF");
        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));
        let content = lexer.read_heredoc("EOF", false);
        assert_eq!(content.content, "hello\n");
        // The redirect tokens are now available from the lexer
        assert_next_token(&mut lexer, TokenKind::RedirectOut, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("file.txt"));
    }

    #[test]
    fn test_read_heredoc_with_redirect_preserves_following_spans() {
        let source = "cat <<EOF > file.txt\nhello\nEOF\n# done\n";
        let mut lexer = Lexer::new(source);

        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

        let heredoc = lexer.read_heredoc("EOF", false);
        assert_eq!(heredoc.content, "hello\n");

        let redirect = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(redirect.kind, TokenKind::RedirectOut);
        assert_eq!(redirect.span.slice(source), ">");

        let target = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(target.kind, TokenKind::Word);
        assert_eq!(
            token_text(&target, lexer.input).as_deref(),
            Some("file.txt")
        );
        assert_eq!(target.span.slice(source), "file.txt");

        let newline = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(newline.kind, TokenKind::Newline);
        assert_eq!(newline.span.slice(source), "\n");

        let comment = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(comment.kind, TokenKind::Comment);
        assert_eq!(token_text(&comment, lexer.input).as_deref(), Some(" done"));
        assert_eq!(comment.span.slice(source), "# done");
    }

    #[test]
    fn test_comment_with_unicode() {
        // Comment containing multi-byte UTF-8 characters
        let source = "# café résumé\necho ok";
        let mut lexer = Lexer::new(source);

        let comment = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(comment.kind, TokenKind::Comment);
        assert_eq!(
            token_text(&comment, lexer.input).as_deref(),
            Some(" café résumé")
        );
        // Span should cover exactly the comment bytes (including #)
        let start = comment.span.start.offset;
        let end = comment.span.end.offset;
        assert_eq!(start, 0);
        assert_eq!(&source[start..end], "# café résumé");
        assert!(source.is_char_boundary(start));
        assert!(source.is_char_boundary(end));

        assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("echo"));
    }

    #[test]
    fn test_comment_with_cjk_characters() {
        // CJK characters are 3-byte UTF-8; offsets must land on char boundaries
        let source = "# 你好世界\necho ok";
        let mut lexer = Lexer::new(source);

        let comment = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(comment.kind, TokenKind::Comment);
        assert_eq!(
            token_text(&comment, lexer.input).as_deref(),
            Some(" 你好世界")
        );
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

        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token_with_comments(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("EOF"));

        let heredoc = lexer.read_heredoc("EOF", false);
        assert_eq!(heredoc.content, "# not a comment\nreal line\n");

        // After heredoc, replayed line termination should appear before
        // tokens from following source lines.
        assert_next_token_with_comments(&mut lexer, TokenKind::Newline, None);
        let comment = lexer.next_lexed_token_with_comments().unwrap();
        assert_eq!(comment.kind, TokenKind::Comment);
        assert_eq!(
            token_text(&comment, lexer.input).as_deref(),
            Some(" real comment")
        );
    }

    #[test]
    fn test_heredoc_with_hash_in_variable() {
        // ${var#pattern} inside heredoc should not produce comment tokens
        let source = "cat <<EOF\nval=${x#prefix}\nEOF\n";
        let mut lexer = Lexer::new(source);

        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token_with_comments(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("EOF"));

        let heredoc = lexer.read_heredoc("EOF", false);
        assert_eq!(heredoc.content, "val=${x#prefix}\n");
    }

    #[test]
    fn test_heredoc_span_does_not_leak() {
        // Heredoc content span must be within source bounds and must not
        // overlap with content before or after.
        let source = "cat <<EOF\nhello\nworld\nEOF\necho after";
        let mut lexer = Lexer::new(source);

        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

        let heredoc = lexer.read_heredoc("EOF", false);
        let start = heredoc.content_span.start.offset;
        let end = heredoc.content_span.end.offset;
        assert!(
            end <= source.len(),
            "heredoc span end ({end}) exceeds source length ({})",
            source.len()
        );
        assert_eq!(&source[start..end], "hello\nworld\n");

        // Tokens after heredoc should still parse correctly
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("after"));
    }

    #[test]
    fn test_heredoc_with_unicode_content() {
        // Heredoc containing multi-byte characters; spans must be on char boundaries
        let source = "cat <<EOF\n# 你好\ncafé\nEOF\n";
        let mut lexer = Lexer::new(source);

        assert_next_token(&mut lexer, TokenKind::Word, Some("cat"));
        assert_next_token(&mut lexer, TokenKind::HereDoc, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("EOF"));

        let heredoc = lexer.read_heredoc("EOF", false);
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
        assert_next_token(
            &mut lexer,
            TokenKind::Word,
            Some(r#"m=([foo]="bar" [baz]="qux")"#),
        );
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_indexed_array_not_collapsed() {
        // arr=("hello world") should NOT be collapsed — parser handles
        // quoted elements token-by-token via the LeftParen path
        let mut lexer = Lexer::new(r#"arr=("hello world")"#);
        assert_next_token(&mut lexer, TokenKind::Word, Some("arr="));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);
    }

    #[test]
    fn test_array_element_with_quoted_prefix_zsh_glob_qualifier_stays_one_word() {
        let source = r#"plugins=( "$plugin_dir"/*(:t) )"#;
        let mut lexer = Lexer::new(source);

        assert_next_token(&mut lexer, TokenKind::Word, Some("plugins="));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);
        assert_eq!(token.span.slice(source), r#""$plugin_dir"/*(:t)"#);

        let word = token.word().unwrap();
        let segments: Vec<_> = word
            .segments()
            .map(|segment| (segment.kind(), segment.as_str().to_string()))
            .collect();
        assert_eq!(
            segments,
            vec![
                (
                    LexedWordSegmentKind::DoubleQuoted,
                    "$plugin_dir".to_string()
                ),
                (LexedWordSegmentKind::Plain, "/*".to_string()),
                (LexedWordSegmentKind::Plain, "(:t)".to_string()),
            ]
        );

        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_array_element_with_quoted_variable_zsh_qualifier_stays_one_word() {
        let source = r#"__GREP_ALIAS_CACHES=( "$__GREP_CACHE_FILE"(Nm-1) )"#;
        let mut lexer = Lexer::new(source);

        assert_next_token(&mut lexer, TokenKind::Word, Some("__GREP_ALIAS_CACHES="));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);
        assert_eq!(token.span.slice(source), r#""$__GREP_CACHE_FILE"(Nm-1)"#);

        let word = token.word().unwrap();
        let segments: Vec<_> = word
            .segments()
            .map(|segment| (segment.kind(), segment.as_str().to_string()))
            .collect();
        assert_eq!(
            segments,
            vec![
                (
                    LexedWordSegmentKind::DoubleQuoted,
                    "$__GREP_CACHE_FILE".to_string()
                ),
                (LexedWordSegmentKind::Plain, "(Nm-1)".to_string()),
            ]
        );

        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_parameter_expansion_with_zsh_qualifier_stays_single_word() {
        let source = r#"$dir/${~pats}(N)"#;
        let mut lexer = Lexer::new(source);

        let token = lexer.next_lexed_token().unwrap();
        assert_eq!(token.kind, TokenKind::Word);
        assert_eq!(token.span.slice(source), source);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_dollar_word_does_not_absorb_function_parens() {
        let mut lexer = Lexer::new(r#"foo$x()"#);

        assert_next_token(&mut lexer, TokenKind::Word, Some("foo$x"));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);
        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_command_substitution_word_does_not_absorb_function_parens() {
        let mut lexer = Lexer::new(r#"foo-$(echo hi)()"#);

        assert_next_token(&mut lexer, TokenKind::Word, Some("foo-$(echo hi)"));
        assert_next_token(&mut lexer, TokenKind::LeftParen, None);
        assert_next_token(&mut lexer, TokenKind::RightParen, None);
        assert!(lexer.next_lexed_token().is_none());
    }

    /// Regression test for fuzz crash: single digit at EOF should not panic
    /// (crash-13c5f6f887a11b2296d67f9857975d63b205ac4b)
    #[test]
    fn test_digit_at_eof_no_panic() {
        // A lone digit with no following redirect operator must not panic
        let mut lexer = Lexer::new("2");
        let token = lexer.next_lexed_token();
        assert!(token.is_some());
    }

    /// Issue #599: Nested ${...} inside unquoted ${...} must be a single token.
    #[test]
    fn test_nested_brace_expansion_single_token() {
        // ${arr[${#arr[@]} - 1]} should be ONE word token, not split at inner }
        let mut lexer = Lexer::new("${arr[${#arr[@]} - 1]}");
        assert_next_token(&mut lexer, TokenKind::Word, Some("${arr[${#arr[@]} - 1]}"));
        // No more tokens — everything was consumed
        assert!(lexer.next_lexed_token().is_none());
    }

    /// Simple ${var} still works after brace depth change.
    #[test]
    fn test_simple_brace_expansion_unchanged() {
        let mut lexer = Lexer::new("${foo}");
        assert_next_token(&mut lexer, TokenKind::Word, Some("${foo}"));
        assert!(lexer.next_lexed_token().is_none());
    }

    #[test]
    fn test_nvm_fixture_lexes_without_stalling() {
        let input = include_str!("../../../shuck-benchmark/resources/files/nvm.sh");
        let mut lexer = Lexer::new(input);
        let mut tokens = 0usize;

        while lexer.next_lexed_token().is_some() {
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
        let tokens = std::iter::from_fn(|| lexer.next_lexed_token())
            .map(|token| (token.kind, token_text(&token, input)))
            .collect::<Vec<_>>();
        assert!(tokens.contains(&(TokenKind::DoubleSemicolon, None)));
        assert!(tokens.contains(&(TokenKind::Word, Some("esac".to_string()))));
    }

    #[test]
    fn test_case_arm_with_zsh_semipipe_terminator_lexes_as_single_token() {
        let input = concat!(
            "case $2 in\n",
            "  cygwin*) bin='cygwin32/bin' ;|\n",
            "esac\n",
        );

        let mut lexer = Lexer::new(input);
        let tokens = std::iter::from_fn(|| lexer.next_lexed_token())
            .map(|token| (token.kind, token_text(&token, input)))
            .collect::<Vec<_>>();

        assert!(tokens.contains(&(TokenKind::SemiPipe, None)));
        assert!(!tokens.contains(&(TokenKind::Semicolon, None)));
        assert!(!tokens.contains(&(TokenKind::Pipe, None)));
    }

    #[test]
    fn test_inline_if_with_array_append_stays_line_local() {
        let input = concat!(
            "if [[ -n $arr ]]; then pyout+=(\"${output}\")\n",
            "elif [[ -n $var ]]; then pyout+=\"${output}${ln:+\\n}\"; fi\n",
        );

        assert_non_newline_tokens_stay_on_one_line(input);
    }

    #[test]
    fn test_zsh_midfile_unsetopt_interactive_comments_keeps_hash_as_word() {
        let source = "unsetopt interactive_comments\n#literal\n";
        let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
        let mut lexer = Lexer::with_profile(source, &profile);

        assert_next_token(&mut lexer, TokenKind::Word, Some("unsetopt"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("interactive_comments"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token_with_comments(&mut lexer, TokenKind::Word, Some("#literal"));
    }

    #[test]
    fn test_zsh_midfile_setopt_rc_quotes_merges_adjacent_single_quotes() {
        let source = "setopt rc_quotes\nprint 'a''b'\n";
        let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
        let mut lexer = Lexer::with_profile(source, &profile);

        assert_next_token(&mut lexer, TokenKind::Word, Some("setopt"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("rc_quotes"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("print"));
        assert_next_token(&mut lexer, TokenKind::LiteralWord, Some("a'b"));
    }

    #[test]
    fn test_zsh_midfile_setopt_ignore_braces_lexes_braces_as_words() {
        let source = "setopt ignore_braces\n{ echo }\n";
        let profile = ShellProfile::native(crate::parser::ShellDialect::Zsh);
        let mut lexer = Lexer::with_profile(source, &profile);

        assert_next_token(&mut lexer, TokenKind::Word, Some("setopt"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("ignore_braces"));
        assert_next_token(&mut lexer, TokenKind::Newline, None);
        assert_next_token(&mut lexer, TokenKind::Word, Some("{"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("echo"));
        assert_next_token(&mut lexer, TokenKind::Word, Some("}"));
    }

    #[test]
    fn test_heredoc_in_arithmetic_fuzz_crash() {
        // Regression test: the fuzzer found that heredoc re-injection inside
        // arithmetic context can push self.offset past self.input.len(),
        // causing a panic in read_unquoted_segment's borrowed-slice path.
        let data: &[u8] = &[
            35, 33, 111, 98, 105, 110, 41, 41, 10, 40, 40, 32, 36, 111, 98, 105, 110, 41,
            41, 10, 40, 40, 32, 36, 53, 32, 43, 32, 49, 32, 6, 0, 0, 0, 0, 0, 0, 0, 41,
            60, 60, 69, 41, 4, 33, 61, 26, 40, 40, 32, 110, 119, 119, 49, 32, 119, 119,
            109, 119, 119, 119, 119, 119, 119, 122, 39, 122, 122, 122, 122, 122, 122, 122,
            122, 122, 122, 122, 122, 0, 0, 0, 0, 0, 41, 60, 60, 69, 41, 4, 33, 61, 26,
            40, 40, 32, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119, 110, 119, 119, 49,
            32, 119, 119, 109, 119, 119, 119, 0, 14, 119, 122, 39, 122, 122, 122, 122,
            122, 122, 122, 47, 33, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 40,
            122, 122, 122, 122, 39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122,
            122, 122, 122, 0, 53, 32, 43, 32, 49, 32, 41, 41, 10, 40, 40, 32, 36, 53, 32,
            43, 32, 49, 32, 6, 0, 0, 0, 0, 0, 0, 0, 41, 60, 60, 69, 41, 4, 33, 61, 26,
            40, 40, 32, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119, 119, 119, 119,
            119, 122, 39, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 0,
            0, 0, 0, 0, 41, 60, 60, 69, 41, 4, 33, 61, 26, 40, 40, 32, 110, 119, 119, 48,
            32, 119, 119, 109, 119, 119, 110, 119, 119, 49, 32, 119, 119, 109, 119, 119,
            119, 0, 14, 119, 122, 39, 122, 122, 122, 122, 122, 122, 122, 47, 33, 122, 122,
            122, 122, 122, 122, 122, 122, 122, 122, 40, 122, 122, 122, 122, 39, 122, 122,
            122, 122, 122, 122, 122, 88, 88, 88, 88, 122, 122, 40, 122, 122, 122, 122, 39,
            122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122, 0, 53,
            32, 43, 32, 49, 32, 53, 41, 10, 40, 40, 32, 36, 53, 32, 43, 32, 49, 32, 6, 0,
            0, 0, 0, 0, 0, 0, 41, 60, 60, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 0,
            0, 0,
        ];
        let input = std::str::from_utf8(data).unwrap();
        let script = format!("echo $(({input}))\n");
        // Must not panic.
        let _ = crate::parser::Parser::new(&script).parse();
    }
}
