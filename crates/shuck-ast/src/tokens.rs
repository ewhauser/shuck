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
