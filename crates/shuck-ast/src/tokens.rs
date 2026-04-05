//! Token types for the lexer
//!
//! Many token types are defined for future implementation phases.

#![allow(dead_code)]

/// Token types produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A word (command name, argument, etc.) - may contain variable expansions
    Word(String),

    /// A literal word (single-quoted) - no variable expansion
    LiteralWord(String),

    /// A double-quoted word - may contain variable expansions inside,
    /// but is marked as quoted (affects heredoc delimiter semantics)
    QuotedWord(String),

    /// A comment body without the leading `#` and without the trailing newline
    Comment(String),

    /// Newline character
    Newline,

    /// Semicolon (;)
    Semicolon,

    /// Double semicolon (;;) — case break
    DoubleSemicolon,

    /// Case fallthrough (;&)
    SemiAmp,

    /// Case continue-matching (;;&)
    DoubleSemiAmp,

    /// Pipe (|)
    Pipe,

    /// Pipe stdout and stderr (|&)
    PipeBoth,

    /// And (&&)
    And,

    /// Or (||)
    Or,

    /// Background (&)
    Background,

    /// Redirect output (>)
    RedirectOut,

    /// Redirect output append (>>)
    RedirectAppend,

    /// Redirect input (<)
    RedirectIn,

    /// Redirect input and output (<>)
    RedirectReadWrite,

    /// Here document (<<)
    HereDoc,

    /// Here document with tab stripping (<<-)
    HereDocStrip,

    /// Here string (<<<)
    HereString,

    /// Left parenthesis (()
    LeftParen,

    /// Right parenthesis ())
    RightParen,

    /// Double left parenthesis ((()
    DoubleLeftParen,

    /// Double right parenthesis ()))
    DoubleRightParen,

    /// Left brace ({)
    LeftBrace,

    /// Right brace (})
    RightBrace,

    /// Double left bracket ([[)
    DoubleLeftBracket,

    /// Double right bracket (]])
    DoubleRightBracket,

    /// Assignment (=)
    Assignment,

    /// Process substitution input <(cmd)
    ProcessSubIn,

    /// Process substitution output >(cmd)
    ProcessSubOut,

    /// Redirect both stdout and stderr (&>)
    RedirectBoth,

    /// Redirect both stdout and stderr with append (&>>)
    RedirectBothAppend,

    /// Clobber redirect (>|) - force overwrite even with noclobber
    Clobber,

    /// Duplicate output file descriptor (>&)
    DupOutput,

    /// Duplicate input file descriptor (<&)
    DupInput,

    /// Redirect with file descriptor (e.g., 2>)
    RedirectFd(i32),

    /// Redirect and append with file descriptor (e.g., 2>>)
    RedirectFdAppend(i32),

    /// Duplicate fd to another (e.g., 2>&1)
    DupFd(i32, i32),

    /// Duplicate input fd to another (e.g., 4<&0)
    DupFdIn(i32, i32),

    /// Close fd (e.g., 4<&- or 4>&-)
    DupFdClose(i32),

    /// Redirect input with file descriptor (e.g., 4<)
    RedirectFdIn(i32),

    /// Redirect input and output with file descriptor (e.g., 4<>)
    RedirectFdReadWrite(i32),

    /// Lexer error (e.g., unterminated string)
    Error(String),
}

/// Cheap token classification for parser dispatch.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Word,
    LiteralWord,
    QuotedWord,
    Comment,
    Newline,
    Semicolon,
    DoubleSemicolon,
    SemiAmp,
    DoubleSemiAmp,
    Pipe,
    PipeBoth,
    And,
    Or,
    Background,
    RedirectOut,
    RedirectAppend,
    RedirectIn,
    RedirectReadWrite,
    HereDoc,
    HereDocStrip,
    HereString,
    LeftParen,
    RightParen,
    DoubleLeftParen,
    DoubleRightParen,
    LeftBrace,
    RightBrace,
    DoubleLeftBracket,
    DoubleRightBracket,
    Assignment,
    ProcessSubIn,
    ProcessSubOut,
    RedirectBoth,
    RedirectBothAppend,
    Clobber,
    DupOutput,
    DupInput,
    RedirectFd,
    RedirectFdAppend,
    DupFd,
    DupFdIn,
    DupFdClose,
    RedirectFdIn,
    RedirectFdReadWrite,
    Error,
}

impl TokenKind {
    pub const fn is_word_like(self) -> bool {
        matches!(self, Self::Word | Self::LiteralWord | Self::QuotedWord)
    }
}

impl Token {
    pub const fn kind(&self) -> TokenKind {
        match self {
            Self::Word(_) => TokenKind::Word,
            Self::LiteralWord(_) => TokenKind::LiteralWord,
            Self::QuotedWord(_) => TokenKind::QuotedWord,
            Self::Comment(_) => TokenKind::Comment,
            Self::Newline => TokenKind::Newline,
            Self::Semicolon => TokenKind::Semicolon,
            Self::DoubleSemicolon => TokenKind::DoubleSemicolon,
            Self::SemiAmp => TokenKind::SemiAmp,
            Self::DoubleSemiAmp => TokenKind::DoubleSemiAmp,
            Self::Pipe => TokenKind::Pipe,
            Self::PipeBoth => TokenKind::PipeBoth,
            Self::And => TokenKind::And,
            Self::Or => TokenKind::Or,
            Self::Background => TokenKind::Background,
            Self::RedirectOut => TokenKind::RedirectOut,
            Self::RedirectAppend => TokenKind::RedirectAppend,
            Self::RedirectIn => TokenKind::RedirectIn,
            Self::RedirectReadWrite => TokenKind::RedirectReadWrite,
            Self::HereDoc => TokenKind::HereDoc,
            Self::HereDocStrip => TokenKind::HereDocStrip,
            Self::HereString => TokenKind::HereString,
            Self::LeftParen => TokenKind::LeftParen,
            Self::RightParen => TokenKind::RightParen,
            Self::DoubleLeftParen => TokenKind::DoubleLeftParen,
            Self::DoubleRightParen => TokenKind::DoubleRightParen,
            Self::LeftBrace => TokenKind::LeftBrace,
            Self::RightBrace => TokenKind::RightBrace,
            Self::DoubleLeftBracket => TokenKind::DoubleLeftBracket,
            Self::DoubleRightBracket => TokenKind::DoubleRightBracket,
            Self::Assignment => TokenKind::Assignment,
            Self::ProcessSubIn => TokenKind::ProcessSubIn,
            Self::ProcessSubOut => TokenKind::ProcessSubOut,
            Self::RedirectBoth => TokenKind::RedirectBoth,
            Self::RedirectBothAppend => TokenKind::RedirectBothAppend,
            Self::Clobber => TokenKind::Clobber,
            Self::DupOutput => TokenKind::DupOutput,
            Self::DupInput => TokenKind::DupInput,
            Self::RedirectFd(_) => TokenKind::RedirectFd,
            Self::RedirectFdAppend(_) => TokenKind::RedirectFdAppend,
            Self::DupFd(_, _) => TokenKind::DupFd,
            Self::DupFdIn(_, _) => TokenKind::DupFdIn,
            Self::DupFdClose(_) => TokenKind::DupFdClose,
            Self::RedirectFdIn(_) => TokenKind::RedirectFdIn,
            Self::RedirectFdReadWrite(_) => TokenKind::RedirectFdReadWrite,
            Self::Error(_) => TokenKind::Error,
        }
    }

    pub fn word_text(&self) -> Option<&str> {
        match self {
            Self::Word(text) | Self::LiteralWord(text) | Self::QuotedWord(text) => Some(text),
            _ => None,
        }
    }

    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error(message) => Some(message),
            _ => None,
        }
    }
}
