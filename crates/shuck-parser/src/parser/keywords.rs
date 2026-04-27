use shuck_ast::TokenKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenSet(u64);

impl TokenSet {
    pub(super) const fn contains(self, kind: TokenKind) -> bool {
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
pub(super) enum Keyword {
    If,
    For,
    Repeat,
    Foreach,
    While,
    Until,
    Case,
    Select,
    Time,
    Coproc,
    Function,
    Always,
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
            Self::Repeat => "repeat",
            Self::Foreach => "foreach",
            Self::While => "while",
            Self::Until => "until",
            Self::Case => "case",
            Self::Select => "select",
            Self::Time => "time",
            Self::Coproc => "coproc",
            Self::Function => "function",
            Self::Always => "always",
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
pub(super) struct KeywordSet(u32);

impl KeywordSet {
    pub(super) const fn single(keyword: Keyword) -> Self {
        Self(1u32 << keyword as u8)
    }

    pub(super) const fn contains(self, keyword: Keyword) -> bool {
        self.0 & (1u32 << keyword as u8) != 0
    }
}

macro_rules! keyword_set {
    ($($keyword:ident),+ $(,)?) => {
        KeywordSet(0 $(| (1u32 << (Keyword::$keyword as u8)))+)
    };
}

pub(super) const PIPE_OPERATOR_TOKENS: TokenSet = token_set![TokenKind::Pipe, TokenKind::PipeBoth];
pub(super) const REDIRECT_TOKENS: TokenSet = token_set![
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
pub(super) const NON_COMMAND_KEYWORDS: KeywordSet =
    keyword_set![Then, Else, Elif, Fi, Do, Done, Esac, In, Always];
pub(super) const IF_BODY_TERMINATORS: KeywordSet = keyword_set![Elif, Else, Fi];
