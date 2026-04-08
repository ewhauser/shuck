//! Parser module for shuck
//!
//! Implements a recursive descent parser for bash scripts.

// Parser uses chars().next().unwrap() after validating character presence.
// This is safe because we check bounds before accessing.
#![allow(clippy::unwrap_used)]

mod arithmetic;
mod commands;
mod heredocs;
mod lexer;
mod redirects;
mod words;

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
    AlwaysCommand, AnonymousFunctionCommand, AnonymousFunctionSurface, ArithmeticCommand,
    ArithmeticExpansionSyntax, ArithmeticExpr, ArithmeticExprNode, ArithmeticForCommand,
    ArithmeticLvalue, ArrayElem, ArrayExpr, ArrayKind, Assignment, AssignmentValue,
    BackgroundOperator, BinaryCommand, BinaryOp, BourneParameterExpansion, BraceExpansionKind,
    BraceQuoteContext, BraceSyntax, BraceSyntaxKind, BreakCommand as AstBreakCommand,
    BuiltinCommand as AstBuiltinCommand, CaseCommand, CaseItem, CaseTerminator,
    Command as AstCommand, CommandSubstitutionSyntax, Comment, CompoundCommand,
    ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp,
    ContinueCommand as AstContinueCommand, CoprocCommand, DeclClause as AstDeclClause, DeclOperand,
    ExitCommand as AstExitCommand, File, ForCommand, ForSyntax, ForTarget, ForeachCommand,
    ForeachSyntax, FunctionDef, FunctionHeader, FunctionHeaderEntry, Heredoc, HeredocDelimiter,
    IfCommand, IfSyntax,
    LiteralText, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternGroupKind, PatternPart, PatternPartNode, Position, PrefixMatchKind, Redirect,
    RedirectKind, RedirectTarget, RepeatCommand, RepeatSyntax, ReturnCommand as AstReturnCommand,
    SelectCommand, SimpleCommand as AstSimpleCommand, SourceText, Span, Stmt, StmtSeq,
    StmtTerminator, Subscript, SubscriptInterpretation, SubscriptKind, SubscriptSelector, TextSize,
    TimeCommand, TokenKind, UntilCommand, VarRef, WhileCommand, Word, WordPart, WordPartNode,
    ZshDefaultingOp, ZshExpansionOperation, ZshExpansionTarget, ZshGlobQualifier,
    ZshGlobQualifierGroup, ZshGlobQualifierKind, ZshGlobSegment, ZshInlineGlobControl, ZshModifier,
    ZshParameterExpansion, ZshPatternOp, ZshQualifiedGlob, ZshReplacementOp, ZshTrimOp,
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
    pub file: File,
}

#[derive(Debug, Clone)]
struct SimpleCommand {
    name: Word,
    args: Vec<Word>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
struct BreakCommand {
    depth: Option<Word>,
    extra_args: Vec<Word>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ContinueCommand {
    depth: Option<Word>,
    extra_args: Vec<Word>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ReturnCommand {
    code: Option<Word>,
    extra_args: Vec<Word>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ExitCommand {
    code: Option<Word>,
    extra_args: Vec<Word>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
enum BuiltinCommand {
    Break(BreakCommand),
    Continue(ContinueCommand),
    Return(ReturnCommand),
    Exit(ExitCommand),
}

#[derive(Debug, Clone)]
struct DeclClause {
    variant: Name,
    variant_span: Span,
    operands: Vec<DeclOperand>,
    redirects: Vec<Redirect>,
    assignments: Vec<Assignment>,
    span: Span,
}

#[derive(Debug, Clone)]
struct Pipeline {
    negated: bool,
    commands: Vec<Command>,
    operators: Vec<(BinaryOp, Span)>,
    span: Span,
}

#[derive(Debug, Clone)]
struct CommandList {
    first: Box<Command>,
    rest: Vec<CommandListItem>,
}

#[derive(Debug, Clone)]
struct CommandListItem {
    operator: ListOperator,
    operator_span: Span,
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListOperator {
    And,
    Or,
    Semicolon,
    Background(BackgroundOperator),
}

#[derive(Debug, Clone)]
enum Command {
    Simple(SimpleCommand),
    Builtin(BuiltinCommand),
    Decl(DeclClause),
    Pipeline(Pipeline),
    List(CommandList),
    Compound(Box<CompoundCommand>, Vec<Redirect>),
    Function(FunctionDef),
    AnonymousFunction(AnonymousFunctionCommand, Vec<Redirect>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellDialect {
    Posix,
    Mksh,
    #[default]
    Bash,
    Zsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DialectFeatures {
    double_bracket: bool,
    arithmetic_command: bool,
    arithmetic_for: bool,
    function_keyword: bool,
    select_loop: bool,
    coproc_keyword: bool,
    zsh_repeat_loop: bool,
    zsh_foreach_loop: bool,
    zsh_parameter_modifiers: bool,
    zsh_brace_if: bool,
    zsh_always: bool,
    zsh_background_operators: bool,
    zsh_glob_qualifiers: bool,
}

impl ShellDialect {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "sh" | "dash" | "ksh" | "posix" => Self::Posix,
            "mksh" => Self::Mksh,
            "zsh" => Self::Zsh,
            _ => Self::Bash,
        }
    }

    const fn features(self) -> DialectFeatures {
        match self {
            Self::Posix => DialectFeatures {
                double_bracket: false,
                arithmetic_command: false,
                arithmetic_for: false,
                function_keyword: false,
                select_loop: false,
                coproc_keyword: false,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Mksh => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: false,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: false,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Bash => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: true,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: true,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Zsh => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: true,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: true,
                zsh_repeat_loop: true,
                zsh_foreach_loop: true,
                zsh_parameter_modifiers: true,
                zsh_brace_if: true,
                zsh_always: true,
                zsh_background_operators: true,
                zsh_glob_qualifiers: true,
            },
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
    /// Nesting depth of active brace-delimited statement sequences.
    brace_group_depth: usize,
    /// Active brace-body parsing contexts, used to distinguish compact zsh
    /// closers from literal `}` arguments.
    brace_body_stack: Vec<BraceBodyContext>,
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
    pub file: File,
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
enum BraceBodyContext {
    Ordinary,
    Function,
    IfClause,
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
const NON_COMMAND_KEYWORDS: KeywordSet =
    keyword_set![Then, Else, Elif, Fi, Do, Done, Esac, In, Always];
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
            brace_group_depth: 0,
            brace_body_stack: Vec::new(),
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

    /// Parse a fragment against the original source span so part offsets stay
    /// aligned with the surrounding script.
    pub fn parse_word_fragment(source: &str, text: &str, span: Span) -> Word {
        let mut parser = Parser::new(source);
        let source_backed = span.end.offset <= source.len() && span.slice(source) == text;
        parser.parse_word_with_context(text, span, span.start, source_backed)
    }

    fn maybe_record_comment(&mut self, token: &LexedToken<'_>) {
        if token.kind == TokenKind::Comment && !token.flags.is_synthetic() {
            self.comments.push(Comment {
                range: token.span.to_range(),
            });
        }
    }

    fn word_text_needs_parse(text: &str) -> bool {
        text.contains(['$', '`', '\x00'])
    }

    fn word_with_parts(&self, parts: Vec<WordPartNode>, span: Span) -> Word {
        let brace_syntax = self.brace_syntax_from_parts(&parts);
        Word {
            parts,
            span,
            brace_syntax,
        }
    }

    fn brace_syntax_from_parts(&self, parts: &[WordPartNode]) -> Vec<BraceSyntax> {
        let mut brace_syntax = Vec::new();
        self.collect_brace_syntax_from_parts(parts, BraceQuoteContext::Unquoted, &mut brace_syntax);
        brace_syntax
    }

    fn collect_brace_syntax_from_parts(
        &self,
        parts: &[WordPartNode],
        quote_context: BraceQuoteContext,
        out: &mut Vec<BraceSyntax>,
    ) {
        for part in parts {
            match &part.kind {
                WordPart::Literal(text) => Self::scan_brace_syntax_text(
                    text.as_str(self.input, part.span),
                    part.span.start,
                    quote_context,
                    out,
                ),
                WordPart::ZshQualifiedGlob(glob) => {
                    self.collect_brace_syntax_from_zsh_qualified_glob(glob, quote_context, out)
                }
                WordPart::SingleQuoted { value, .. } => Self::scan_brace_syntax_text(
                    value.slice(self.input),
                    value.span().start,
                    BraceQuoteContext::SingleQuoted,
                    out,
                ),
                WordPart::DoubleQuoted { parts, .. } => self.collect_brace_syntax_from_parts(
                    parts,
                    BraceQuoteContext::DoubleQuoted,
                    out,
                ),
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ArithmeticExpansion { .. }
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::Transformation { .. } => {}
            }
        }
    }

    fn collect_brace_syntax_from_pattern(
        &self,
        pattern: &Pattern,
        quote_context: BraceQuoteContext,
        out: &mut Vec<BraceSyntax>,
    ) {
        for (part, span) in pattern.parts_with_spans() {
            match part {
                PatternPart::Literal(text) => Self::scan_brace_syntax_text(
                    text.as_str(self.input, span),
                    span.start,
                    quote_context,
                    out,
                ),
                PatternPart::CharClass(text) => Self::scan_brace_syntax_text(
                    text.slice(self.input),
                    text.span().start,
                    quote_context,
                    out,
                ),
                PatternPart::Group { patterns, .. } => {
                    for pattern in patterns {
                        self.collect_brace_syntax_from_pattern(pattern, quote_context, out);
                    }
                }
                PatternPart::Word(word) => {
                    self.collect_brace_syntax_from_parts(&word.parts, quote_context, out)
                }
                PatternPart::AnyString | PatternPart::AnyChar => {}
            }
        }
    }

    fn collect_brace_syntax_from_zsh_qualified_glob(
        &self,
        glob: &ZshQualifiedGlob,
        quote_context: BraceQuoteContext,
        out: &mut Vec<BraceSyntax>,
    ) {
        for segment in &glob.segments {
            if let ZshGlobSegment::Pattern(pattern) = segment {
                self.collect_brace_syntax_from_pattern(pattern, quote_context, out);
            }
        }
    }

    fn scan_brace_syntax_text(
        text: &str,
        base: Position,
        quote_context: BraceQuoteContext,
        out: &mut Vec<BraceSyntax>,
    ) {
        let mut index = 0;

        while index < text.len() {
            let rest = &text[index..];
            let Some(ch) = rest.chars().next() else {
                break;
            };

            if matches!(quote_context, BraceQuoteContext::Unquoted) && ch == '\\' {
                index += ch.len_utf8();
                if let Some(next) = text[index..].chars().next() {
                    index += next.len_utf8();
                }
                continue;
            }

            if ch != '{' {
                index += ch.len_utf8();
                continue;
            }

            if let Some(len) = Self::template_placeholder_len(text, index, quote_context) {
                out.push(BraceSyntax {
                    kind: BraceSyntaxKind::TemplatePlaceholder,
                    span: Span::from_positions(
                        Self::text_position(base, text, index),
                        Self::text_position(base, text, index + len),
                    ),
                    quote_context,
                });
                index += len;
                continue;
            }

            if let Some((len, kind)) = Self::brace_construct_len(text, index, quote_context) {
                out.push(BraceSyntax {
                    kind,
                    span: Span::from_positions(
                        Self::text_position(base, text, index),
                        Self::text_position(base, text, index + len),
                    ),
                    quote_context,
                });
                index += len;
                continue;
            }

            index += ch.len_utf8();
        }
    }

    fn text_position(base: Position, text: &str, offset: usize) -> Position {
        base.advanced_by(&text[..offset])
    }

    fn template_placeholder_len(
        text: &str,
        start: usize,
        quote_context: BraceQuoteContext,
    ) -> Option<usize> {
        text.get(start..).filter(|rest| rest.starts_with("{{"))?;

        let mut index = start + "{{".len();
        let mut depth = 1usize;

        while index < text.len() {
            if matches!(quote_context, BraceQuoteContext::Unquoted)
                && text[index..].starts_with('\\')
            {
                index += 1;
                if let Some(next) = text[index..].chars().next() {
                    index += next.len_utf8();
                }
                continue;
            }

            if text[index..].starts_with("{{") {
                depth += 1;
                index += "{{".len();
                continue;
            }

            if text[index..].starts_with("}}") {
                depth -= 1;
                index += "}}".len();
                if depth == 0 {
                    return Some(index - start);
                }
                continue;
            }

            index += text[index..].chars().next()?.len_utf8();
        }

        None
    }

    fn brace_construct_len(
        text: &str,
        start: usize,
        quote_context: BraceQuoteContext,
    ) -> Option<(usize, BraceSyntaxKind)> {
        text.get(start..).filter(|rest| rest.starts_with('{'))?;

        let mut index = start + '{'.len_utf8();
        let mut depth = 1usize;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut prev_char = None;

        while index < text.len() {
            if matches!(quote_context, BraceQuoteContext::Unquoted)
                && text[index..].starts_with('\\')
            {
                index += 1;
                if let Some(next) = text[index..].chars().next() {
                    index += next.len_utf8();
                }
                prev_char = None;
                continue;
            }

            let ch = text[index..].chars().next()?;
            index += ch.len_utf8();

            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let kind = if has_comma {
                            BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
                        } else if has_dot_dot {
                            BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
                        } else {
                            BraceSyntaxKind::Literal
                        };
                        return Some((index - start, kind));
                    }
                }
                ',' if depth == 1 => has_comma = true,
                '.' if depth == 1 && prev_char == Some('.') => has_dot_dot = true,
                _ => {}
            }

            prev_char = Some(ch);
        }

        None
    }

    fn maybe_parse_zsh_qualified_glob_word(
        &mut self,
        text: &str,
        span: Span,
        source_backed: bool,
    ) -> Option<Word> {
        if !self.dialect.features().zsh_glob_qualifiers
            || text.is_empty()
            || text.contains(['\x00', '\\', '\'', '"', '$', '`'])
            || text.chars().any(char::is_whitespace)
        {
            return None;
        }

        let (segments, qualifiers) =
            self.parse_zsh_qualified_glob_segments(text, span, source_backed)?;
        if !segments.iter().any(|segment| {
            matches!(
                segment,
                ZshGlobSegment::Pattern(pattern) if Self::pattern_has_glob_syntax(pattern)
            )
        }) {
            return None;
        }

        Some(self.word_with_parts(
            vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span,
                    segments,
                    qualifiers,
                }),
                span,
            )],
            span,
        ))
    }

    fn parse_zsh_qualified_glob_segments(
        &mut self,
        text: &str,
        span: Span,
        source_backed: bool,
    ) -> Option<(Vec<ZshGlobSegment>, Option<ZshGlobQualifierGroup>)> {
        let mut segments = Vec::new();
        let mut qualifiers = None;
        let mut pattern_start = 0usize;
        let mut index = 0usize;

        while index < text.len() {
            if text[index..].starts_with("(#") {
                if let Some((len, control)) =
                    self.parse_zsh_inline_glob_control(text, span.start, index)
                {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        pattern_start,
                        index,
                        source_backed,
                    );
                    segments.push(ZshGlobSegment::InlineControl(control));
                    index += len;
                    pattern_start = index;
                    continue;
                }

                let suffix_start = Self::text_position(span.start, text, index);
                if let Some(group) = self.parse_zsh_terminal_glob_qualifier_group(
                    &text[index..],
                    suffix_start,
                    source_backed,
                ) {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        pattern_start,
                        index,
                        source_backed,
                    );
                    qualifiers = Some(group);
                    index = text.len();
                    pattern_start = index;
                    break;
                }

                return None;
            }

            if text[index..].starts_with('(') {
                let suffix_start = Self::text_position(span.start, text, index);
                if let Some(group) = self.parse_zsh_terminal_glob_qualifier_group(
                    &text[index..],
                    suffix_start,
                    source_backed,
                ) && matches!(group.kind, ZshGlobQualifierKind::Classic)
                {
                    self.push_zsh_pattern_segment(
                        &mut segments,
                        text,
                        span.start,
                        pattern_start,
                        index,
                        source_backed,
                    );
                    qualifiers = Some(group);
                    index = text.len();
                    pattern_start = index;
                    break;
                }
            }

            index += text[index..].chars().next()?.len_utf8();
        }

        self.push_zsh_pattern_segment(
            &mut segments,
            text,
            span.start,
            pattern_start,
            text.len(),
            source_backed,
        );

        segments
            .iter()
            .any(|segment| matches!(segment, ZshGlobSegment::Pattern(_)))
            .then_some((segments, qualifiers))
    }

    fn push_zsh_pattern_segment(
        &mut self,
        segments: &mut Vec<ZshGlobSegment>,
        text: &str,
        base: Position,
        start: usize,
        end: usize,
        source_backed: bool,
    ) {
        if start >= end {
            return;
        }

        let start_position = Self::text_position(base, text, start);
        let end_position = Self::text_position(base, text, end);
        let span = Span::from_positions(start_position, end_position);
        let pattern_word =
            self.decode_word_text(&text[start..end], span, span.start, source_backed);
        segments.push(ZshGlobSegment::Pattern(
            self.pattern_from_word(&pattern_word),
        ));
    }

    fn parse_zsh_inline_glob_control(
        &self,
        text: &str,
        base: Position,
        start: usize,
    ) -> Option<(usize, ZshInlineGlobControl)> {
        let (len, control) = if text[start..].starts_with("(#i)") {
            (
                "(#i)".len(),
                ZshInlineGlobControl::CaseInsensitive {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#i)".len()),
                    ),
                },
            )
        } else if text[start..].starts_with("(#b)") {
            (
                "(#b)".len(),
                ZshInlineGlobControl::Backreferences {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#b)".len()),
                    ),
                },
            )
        } else {
            return None;
        };

        Some((len, control))
    }

    fn parse_zsh_terminal_glob_qualifier_group(
        &self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> Option<ZshGlobQualifierGroup> {
        let (kind, prefix_len, inner) = if let Some(inner) = text
            .strip_prefix("(#q")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            (ZshGlobQualifierKind::HashQ, "(#q".len(), inner)
        } else {
            let inner = text.strip_prefix('(')?.strip_suffix(')')?;
            (ZshGlobQualifierKind::Classic, "(".len(), inner)
        };

        let fragments = self.parse_zsh_glob_qualifier_fragments(
            inner,
            Self::text_position(base, text, prefix_len),
            source_backed,
        )?;

        Some(ZshGlobQualifierGroup {
            span: Span::from_positions(base, Self::text_position(base, text, text.len())),
            kind,
            fragments,
        })
    }

    fn parse_zsh_glob_qualifier_fragments(
        &self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> Option<Vec<ZshGlobQualifier>> {
        let mut fragments = Vec::new();
        let mut index = 0;
        let mut saw_non_letter_fragment = false;

        while index < text.len() {
            let start = index;
            let ch = text[index..].chars().next()?;

            match ch {
                '^' => {
                    index += ch.len_utf8();
                    fragments.push(ZshGlobQualifier::Negation {
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                    saw_non_letter_fragment = true;
                }
                '.' | '/' | '-' | 'A'..='Z' => {
                    index += ch.len_utf8();
                    fragments.push(ZshGlobQualifier::Flag {
                        name: ch,
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                    saw_non_letter_fragment = true;
                }
                '[' => {
                    index += ch.len_utf8();
                    let number_start = index;
                    while matches!(text[index..].chars().next(), Some('0'..='9')) {
                        index += 1;
                    }
                    if number_start == index {
                        return None;
                    }

                    let start_text = self.zsh_glob_qualifier_source_text(
                        text,
                        base,
                        number_start,
                        index,
                        source_backed,
                    );
                    let end_text = if text[index..].starts_with(',') {
                        index += 1;
                        let range_start = index;
                        while matches!(text[index..].chars().next(), Some('0'..='9')) {
                            index += 1;
                        }
                        if range_start == index {
                            return None;
                        }
                        Some(self.zsh_glob_qualifier_source_text(
                            text,
                            base,
                            range_start,
                            index,
                            source_backed,
                        ))
                    } else {
                        None
                    };

                    if !text[index..].starts_with(']') {
                        return None;
                    }
                    index += "]".len();
                    fragments.push(ZshGlobQualifier::NumericArgument {
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                        start: start_text,
                        end: end_text,
                    });
                    saw_non_letter_fragment = true;
                }
                'a'..='z' => {
                    index += ch.len_utf8();
                    while matches!(text[index..].chars().next(), Some('a'..='z' | 'A'..='Z')) {
                        index += 1;
                    }
                    if index - start <= 1 {
                        return None;
                    }
                    fragments.push(ZshGlobQualifier::LetterSequence {
                        text: self.zsh_glob_qualifier_source_text(
                            text,
                            base,
                            start,
                            index,
                            source_backed,
                        ),
                        span: Span::from_positions(
                            Self::text_position(base, text, start),
                            Self::text_position(base, text, index),
                        ),
                    });
                }
                _ => return None,
            }
        }

        (!fragments.is_empty() && saw_non_letter_fragment).then_some(fragments)
    }

    fn zsh_glob_qualifier_source_text(
        &self,
        text: &str,
        base: Position,
        start: usize,
        end: usize,
        source_backed: bool,
    ) -> SourceText {
        let span = Span::from_positions(
            Self::text_position(base, text, start),
            Self::text_position(base, text, end),
        );
        if source_backed {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text[start..end].to_string())
        }
    }

    fn pattern_has_glob_syntax(pattern: &Pattern) -> bool {
        pattern.parts.iter().any(|part| match &part.kind {
            PatternPart::AnyString | PatternPart::AnyChar | PatternPart::CharClass(_) => true,
            PatternPart::Group { .. } => true,
            PatternPart::Word(word) => Self::pattern_has_glob_word(word),
            PatternPart::Literal(_) => false,
        })
    }

    fn pattern_has_glob_word(word: &Word) -> bool {
        word.parts
            .iter()
            .any(|part| !matches!(part.kind, WordPart::Literal(_)))
    }

    fn simple_word_from_token(&mut self, token: &LexedToken<'_>, span: Span) -> Option<Word> {
        let word = token.word()?;
        let source_backed = !token.flags.is_synthetic();

        if self.dialect.features().zsh_glob_qualifiers
            && let Some(segment) = word.single_segment()
            && segment.kind() == LexedWordSegmentKind::Plain
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(
                segment.as_str(),
                span,
                segment.span().is_some() && source_backed,
            )
        {
            return Some(word);
        }
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

        Some(self.word_with_parts(parts, span))
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
                LexedWordSegmentKind::SingleQuoted => Some(self.word_with_parts(
                    vec![self.single_quoted_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        false,
                    )],
                    span,
                )),
                LexedWordSegmentKind::DollarSingleQuoted => Some(self.word_with_parts(
                    vec![self.single_quoted_part_from_text(text, content_span, wrapper_span, true)],
                    span,
                )),
                LexedWordSegmentKind::Plain if Self::word_text_needs_parse(text) => {
                    Some(self.decode_word_text_preserving_quotes_if_needed(
                        text,
                        span,
                        content_span.start,
                        source_backed,
                    ))
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
                    Some(self.word_with_parts(
                        vec![WordPartNode::new(
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
                    ))
                }
                LexedWordSegmentKind::Plain => Some(self.word_with_parts(
                    vec![self.literal_part_from_text(text, content_span, source_backed)],
                    span,
                )),
                LexedWordSegmentKind::DoubleQuoted => Some(self.word_with_parts(
                    vec![self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        false,
                    )],
                    span,
                )),
                LexedWordSegmentKind::DollarDoubleQuoted => Some(self.word_with_parts(
                    vec![self.double_quoted_literal_part_from_text(
                        text,
                        content_span,
                        wrapper_span,
                        source_backed,
                        true,
                    )],
                    span,
                )),
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
                        parts.extend(
                            self.decode_word_text_preserving_quotes_if_needed(
                                text,
                                content_span,
                                content_span.start,
                                source_backed,
                            )
                            .parts,
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

        Some(self.word_with_parts(parts, span))
    }

    fn current_word(&mut self) -> Option<Word> {
        if let Some(word) = self.current_word_cache.as_ref() {
            return Some(word.clone());
        }

        if let Some(word) = self.current_zsh_glob_word_from_source() {
            self.current_word_cache = Some(word.clone());
            return Some(word);
        }

        let span = self.current_span;
        if let Some(token) = self.current_token.clone()
            && let Some(word) = self.simple_word_from_token(&token, span)
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

    fn current_zsh_glob_word_from_source(&mut self) -> Option<Word> {
        if !matches!(self.current_token_kind, Some(TokenKind::LeftParen))
            && !self.current_token_kind.is_some_and(TokenKind::is_word_like)
        {
            return None;
        }

        let start = self.current_span.start;
        let (text, end) = self.scan_source_word(start)?;
        if !text.contains("(#") {
            return None;
        }
        let span = Span::from_positions(start, end);
        if self.dialect.features().zsh_glob_qualifiers
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(&text, span, true)
        {
            return Some(word);
        }

        Some(self.parse_word_with_context(&text, span, start, true))
    }

    fn scan_source_word(&self, start: Position) -> Option<(String, Position)> {
        if start.offset >= self.input.len() {
            return None;
        }

        let source = &self.input[start.offset..];
        let mut chars = source.chars().peekable();
        let mut cursor = start;
        let mut text = String::new();
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;

        while let Some(&ch) = chars.peek() {
            if !in_single
                && !in_double
                && !in_backtick
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
                '`' if !in_single => in_backtick = !in_backtick,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                _ => {}
            }
        }

        (!text.is_empty()).then_some((text, cursor))
    }

    fn advance_past_word(&mut self, word: &Word) {
        let stop_after_synthetic = self
            .current_token
            .as_ref()
            .is_some_and(|token| token.flags.is_synthetic());
        while self.current_token.is_some() && self.current_span.start.offset < word.span.end.offset
        {
            self.advance();
            if stop_after_synthetic
                && self
                    .current_token
                    .as_ref()
                    .is_none_or(|token| !token.flags.is_synthetic())
            {
                break;
            }
        }
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

    fn nested_stmt_seq_from_source(&mut self, source: &str, base: Position) -> StmtSeq {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let inner_parser =
            Parser::with_limits_and_dialect(source, remaining_depth, self.fuel, self.dialect);
        match inner_parser.parse() {
            Ok(mut output) => {
                Self::rebase_file(&mut output.file, base);
                output.file.body
            }
            Err(_) => StmtSeq {
                leading_comments: Vec::new(),
                stmts: Vec::new(),
                trailing_comments: Vec::new(),
                span: Span::from_positions(base, base),
            },
        }
    }

    fn nested_stmt_seq_from_current_input(&mut self, start: Position, end: Position) -> StmtSeq {
        if start.offset > end.offset || end.offset > self.input.len() {
            return StmtSeq {
                leading_comments: Vec::new(),
                stmts: Vec::new(),
                trailing_comments: Vec::new(),
                span: Span::from_positions(start, start),
            };
        }
        let source = &self.input[start.offset..end.offset];
        self.nested_stmt_seq_from_source(source, start)
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

    fn rebase_file(file: &mut File, base: Position) {
        file.span = file.span.rebased(base);
        Self::rebase_stmt_seq(&mut file.body, base);
    }

    fn rebase_comments(comments: &mut [Comment], base: Position) {
        let base_offset = TextSize::new(base.offset as u32);
        for comment in comments {
            comment.range = comment.range.offset_by(base_offset);
        }
    }

    fn rebase_stmt_seq(sequence: &mut StmtSeq, base: Position) {
        sequence.span = sequence.span.rebased(base);
        Self::rebase_comments(&mut sequence.leading_comments, base);
        for stmt in &mut sequence.stmts {
            Self::rebase_stmt(stmt, base);
        }
        Self::rebase_comments(&mut sequence.trailing_comments, base);
    }

    fn rebase_stmt(stmt: &mut Stmt, base: Position) {
        stmt.span = stmt.span.rebased(base);
        Self::rebase_comments(&mut stmt.leading_comments, base);
        stmt.terminator_span = stmt.terminator_span.map(|span| span.rebased(base));
        if let Some(comment) = &mut stmt.inline_comment {
            let base_offset = TextSize::new(base.offset as u32);
            comment.range = comment.range.offset_by(base_offset);
        }
        Self::rebase_redirects(&mut stmt.redirects, base);
        Self::rebase_ast_command(&mut stmt.command, base);
    }

    fn rebase_ast_command(command: &mut AstCommand, base: Position) {
        match command {
            AstCommand::Simple(simple) => {
                simple.span = simple.span.rebased(base);
                Self::rebase_word(&mut simple.name, base);
                Self::rebase_words(&mut simple.args, base);
                Self::rebase_assignments(&mut simple.assignments, base);
            }
            AstCommand::Builtin(builtin) => match builtin {
                AstBuiltinCommand::Break(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(depth) = &mut command.depth {
                        Self::rebase_word(depth, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Continue(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(depth) = &mut command.depth {
                        Self::rebase_word(depth, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Return(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(code) = &mut command.code {
                        Self::rebase_word(code, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
                AstBuiltinCommand::Exit(command) => {
                    command.span = command.span.rebased(base);
                    if let Some(code) = &mut command.code {
                        Self::rebase_word(code, base);
                    }
                    Self::rebase_words(&mut command.extra_args, base);
                    Self::rebase_assignments(&mut command.assignments, base);
                }
            },
            AstCommand::Decl(decl) => {
                decl.span = decl.span.rebased(base);
                decl.variant_span = decl.variant_span.rebased(base);
                Self::rebase_assignments(&mut decl.assignments, base);
                for operand in &mut decl.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            Self::rebase_word(word, base);
                        }
                        DeclOperand::Name(name) => Self::rebase_var_ref(name, base),
                        DeclOperand::Assignment(assignment) => {
                            Self::rebase_assignments(std::slice::from_mut(assignment), base);
                        }
                    }
                }
            }
            AstCommand::Binary(binary) => {
                binary.span = binary.span.rebased(base);
                binary.op_span = binary.op_span.rebased(base);
                Self::rebase_stmt(binary.left.as_mut(), base);
                Self::rebase_stmt(binary.right.as_mut(), base);
            }
            AstCommand::Compound(compound) => Self::rebase_compound(compound, base),
            AstCommand::Function(function) => {
                function.span = function.span.rebased(base);
                if let Some(span) = &mut function.header.function_keyword_span {
                    *span = span.rebased(base);
                }
                if let Some(span) = &mut function.header.trailing_parens_span {
                    *span = span.rebased(base);
                }
                for entry in &mut function.header.entries {
                    Self::rebase_word(&mut entry.word, base);
                }
                Self::rebase_stmt(function.body.as_mut(), base);
            }
            AstCommand::AnonymousFunction(function) => {
                function.span = function.span.rebased(base);
                function.surface = match function.surface {
                    AnonymousFunctionSurface::FunctionKeyword {
                        function_keyword_span,
                    } => AnonymousFunctionSurface::FunctionKeyword {
                        function_keyword_span: function_keyword_span.rebased(base),
                    },
                    AnonymousFunctionSurface::Parens { parens_span } => {
                        AnonymousFunctionSurface::Parens {
                            parens_span: parens_span.rebased(base),
                        }
                    }
                };
                Self::rebase_stmt(function.body.as_mut(), base);
                Self::rebase_words(&mut function.args, base);
            }
        }
    }

    fn rebase_subscript(subscript: &mut Subscript, base: Position) {
        subscript.text.rebased(base);
        if let Some(raw) = &mut subscript.raw {
            raw.rebased(base);
        }
        if let Some(expr) = &mut subscript.arithmetic_ast {
            Self::rebase_arithmetic_expr(expr, base);
        }
    }

    fn rebase_var_ref(reference: &mut VarRef, base: Position) {
        reference.span = reference.span.rebased(base);
        reference.name_span = reference.name_span.rebased(base);
        if let Some(subscript) = &mut reference.subscript {
            Self::rebase_subscript(subscript, base);
        }
    }

    fn rebase_array_expr(array: &mut ArrayExpr, base: Position) {
        array.span = array.span.rebased(base);
        for element in &mut array.elements {
            match element {
                ArrayElem::Sequential(word) => Self::rebase_word(word, base),
                ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                    Self::rebase_subscript(key, base);
                    Self::rebase_word(value, base);
                }
            }
        }
    }

    fn rebase_compound(compound: &mut CompoundCommand, base: Position) {
        match compound {
            CompoundCommand::If(command) => {
                command.span = command.span.rebased(base);
                command.syntax = match command.syntax {
                    IfSyntax::ThenFi { then_span, fi_span } => IfSyntax::ThenFi {
                        then_span: then_span.rebased(base),
                        fi_span: fi_span.rebased(base),
                    },
                    IfSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    } => IfSyntax::Brace {
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.then_branch, base);
                for (condition, body) in &mut command.elif_branches {
                    Self::rebase_stmt_seq(condition, base);
                    Self::rebase_stmt_seq(body, base);
                }
                if let Some(else_branch) = &mut command.else_branch {
                    Self::rebase_stmt_seq(else_branch, base);
                }
            }
            CompoundCommand::For(command) => {
                command.span = command.span.rebased(base);
                for target in &mut command.targets {
                    target.span = target.span.rebased(base);
                }
                if let Some(words) = &mut command.words {
                    Self::rebase_words(words, base);
                }
                command.syntax = match command.syntax {
                    ForSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    } => ForSyntax::InDoDone {
                        in_span: in_span.map(|span| span.rebased(base)),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    ForSyntax::InBrace {
                        in_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForSyntax::InBrace {
                        in_span: in_span.map(|span| span.rebased(base)),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                    ForSyntax::ParenDoDone {
                        left_paren_span,
                        right_paren_span,
                        do_span,
                        done_span,
                    } => ForSyntax::ParenDoDone {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    ForSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForSyntax::ParenBrace {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Repeat(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_word(&mut command.count, base);
                command.syntax = match command.syntax {
                    RepeatSyntax::DoDone { do_span, done_span } => RepeatSyntax::DoDone {
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                    RepeatSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    } => RepeatSyntax::Brace {
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Foreach(command) => {
                command.span = command.span.rebased(base);
                command.variable_span = command.variable_span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                command.syntax = match command.syntax {
                    ForeachSyntax::ParenBrace {
                        left_paren_span,
                        right_paren_span,
                        left_brace_span,
                        right_brace_span,
                    } => ForeachSyntax::ParenBrace {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
                        left_brace_span: left_brace_span.rebased(base),
                        right_brace_span: right_brace_span.rebased(base),
                    },
                    ForeachSyntax::InDoDone {
                        in_span,
                        do_span,
                        done_span,
                    } => ForeachSyntax::InDoDone {
                        in_span: in_span.rebased(base),
                        do_span: do_span.rebased(base),
                        done_span: done_span.rebased(base),
                    },
                };
                Self::rebase_stmt_seq(&mut command.body, base);
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
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::While(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Until(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.condition, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Case(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_word(&mut command.word, base);
                for case in &mut command.cases {
                    Self::rebase_patterns(&mut case.patterns, base);
                    Self::rebase_stmt_seq(&mut case.body, base);
                }
            }
            CompoundCommand::Select(command) => {
                command.span = command.span.rebased(base);
                command.variable_span = command.variable_span.rebased(base);
                Self::rebase_words(&mut command.words, base);
                Self::rebase_stmt_seq(&mut command.body, base);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::rebase_stmt_seq(commands, base);
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
                    Self::rebase_stmt(inner.as_mut(), base);
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
                Self::rebase_stmt(command.body.as_mut(), base);
            }
            CompoundCommand::Always(command) => {
                command.span = command.span.rebased(base);
                Self::rebase_stmt_seq(&mut command.body, base);
                Self::rebase_stmt_seq(&mut command.always_body, base);
            }
        }
    }

    fn rebase_words(words: &mut [Word], base: Position) {
        for word in words {
            Self::rebase_word(word, base);
        }
    }

    fn rebase_patterns(patterns: &mut [Pattern], base: Position) {
        for pattern in patterns {
            Self::rebase_pattern(pattern, base);
        }
    }

    fn rebase_word(word: &mut Word, base: Position) {
        word.span = word.span.rebased(base);
        for brace in &mut word.brace_syntax {
            brace.span = brace.span.rebased(base);
        }
        Self::rebase_word_parts(&mut word.parts, base);
    }

    fn rebase_pattern(pattern: &mut Pattern, base: Position) {
        pattern.span = pattern.span.rebased(base);
        Self::rebase_pattern_parts(&mut pattern.parts, base);
    }

    fn rebase_word_parts(parts: &mut [WordPartNode], base: Position) {
        for part in parts {
            Self::rebase_word_part(part, base);
        }
    }

    fn rebase_pattern_parts(parts: &mut [PatternPartNode], base: Position) {
        for part in parts {
            part.span = part.span.rebased(base);
            match &mut part.kind {
                PatternPart::CharClass(text) => text.rebased(base),
                PatternPart::Group { patterns, .. } => Self::rebase_patterns(patterns, base),
                PatternPart::Word(word) => Self::rebase_word(word, base),
                PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
            }
        }
    }

    fn rebase_word_part(part: &mut WordPartNode, base: Position) {
        part.span = part.span.rebased(base);
        match &mut part.kind {
            WordPart::ZshQualifiedGlob(glob) => Self::rebase_zsh_qualified_glob(glob, base),
            WordPart::SingleQuoted { value, .. } => value.rebased(base),
            WordPart::DoubleQuoted { parts, .. } => Self::rebase_word_parts(parts, base),
            WordPart::Parameter(parameter) => {
                parameter.span = parameter.span.rebased(base);
                parameter.raw_body.rebased(base);
                Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                match operator {
                    ParameterOp::RemovePrefixShort { pattern }
                    | ParameterOp::RemovePrefixLong { pattern }
                    | ParameterOp::RemoveSuffixShort { pattern }
                    | ParameterOp::RemoveSuffixLong { pattern } => {
                        Self::rebase_pattern(pattern, base);
                    }
                    ParameterOp::ReplaceFirst {
                        pattern,
                        replacement,
                    }
                    | ParameterOp::ReplaceAll {
                        pattern,
                        replacement,
                    } => {
                        Self::rebase_pattern(pattern, base);
                        replacement.rebased(base);
                    }
                    ParameterOp::UseDefault
                    | ParameterOp::AssignDefault
                    | ParameterOp::UseReplacement
                    | ParameterOp::Error
                    | ParameterOp::UpperFirst
                    | ParameterOp::UpperAll
                    | ParameterOp::LowerFirst
                    | ParameterOp::LowerAll => {}
                }
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
            }
            WordPart::ArrayAccess(reference)
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => Self::rebase_var_ref(reference, base),
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
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
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => Self::rebase_stmt_seq(body, base),
            WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {}
        }
    }

    fn rebase_zsh_qualified_glob(glob: &mut ZshQualifiedGlob, base: Position) {
        glob.span = glob.span.rebased(base);
        for segment in &mut glob.segments {
            Self::rebase_zsh_glob_segment(segment, base);
        }
        if let Some(qualifiers) = &mut glob.qualifiers {
            Self::rebase_zsh_glob_qualifier_group(qualifiers, base);
        }
    }

    fn rebase_zsh_glob_segment(segment: &mut ZshGlobSegment, base: Position) {
        match segment {
            ZshGlobSegment::Pattern(pattern) => Self::rebase_pattern(pattern, base),
            ZshGlobSegment::InlineControl(control) => {
                Self::rebase_zsh_inline_glob_control(control, base)
            }
        }
    }

    fn rebase_zsh_inline_glob_control(control: &mut ZshInlineGlobControl, base: Position) {
        match control {
            ZshInlineGlobControl::CaseInsensitive { span }
            | ZshInlineGlobControl::Backreferences { span } => {
                *span = span.rebased(base);
            }
        }
    }

    fn rebase_zsh_glob_qualifier_group(group: &mut ZshGlobQualifierGroup, base: Position) {
        group.span = group.span.rebased(base);
        for fragment in &mut group.fragments {
            match fragment {
                ZshGlobQualifier::Negation { span } | ZshGlobQualifier::Flag { span, .. } => {
                    *span = span.rebased(base);
                }
                ZshGlobQualifier::LetterSequence { text, span } => {
                    *span = span.rebased(base);
                    text.rebased(base);
                }
                ZshGlobQualifier::NumericArgument { span, start, end } => {
                    *span = span.rebased(base);
                    start.rebased(base);
                    if let Some(end) = end {
                        end.rebased(base);
                    }
                }
            }
        }
    }

    fn rebase_parameter_expansion_syntax(syntax: &mut ParameterExpansionSyntax, base: Position) {
        match syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    Self::rebase_var_ref(reference, base);
                }
                BourneParameterExpansion::Indirect { operand, .. } => {
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset,
                    offset_ast,
                    length,
                    length_ast,
                } => {
                    Self::rebase_var_ref(reference, base);
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
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    Self::rebase_var_ref(reference, base);
                    Self::rebase_parameter_operator(operator, base);
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &mut syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        Self::rebase_var_ref(reference, base)
                    }
                    ZshExpansionTarget::Nested(parameter) => {
                        parameter.span = parameter.span.rebased(base);
                        parameter.raw_body.rebased(base);
                        Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
                    }
                    ZshExpansionTarget::Empty => {}
                }
                for modifier in &mut syntax.modifiers {
                    modifier.span = modifier.span.rebased(base);
                    if let Some(argument) = &mut modifier.argument {
                        argument.rebased(base);
                    }
                }
                if let Some(operation) = &mut syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation { operand, .. }
                        | ZshExpansionOperation::Defaulting { operand, .. }
                        | ZshExpansionOperation::TrimOperation { operand, .. }
                        | ZshExpansionOperation::Unknown(operand) => operand.rebased(base),
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            replacement,
                            ..
                        } => {
                            pattern.rebased(base);
                            if let Some(replacement) = replacement {
                                replacement.rebased(base);
                            }
                        }
                        ZshExpansionOperation::Slice { offset, length } => {
                            offset.rebased(base);
                            if let Some(length) = length {
                                length.rebased(base);
                            }
                        }
                    }
                }
            }
        }
    }

    fn rebase_parameter_operator(operator: &mut ParameterOp, base: Position) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                Self::rebase_pattern(pattern, base);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => {
                Self::rebase_pattern(pattern, base);
                replacement.rebased(base);
            }
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
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
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                Self::rebase_word(word, base);
            }
            ConditionalExpr::Pattern(pattern) => Self::rebase_pattern(pattern, base),
            ConditionalExpr::VarRef(var_ref) => Self::rebase_var_ref(var_ref, base),
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

    fn subscript_source_text(&self, raw: &str, span: Span) -> (SourceText, Option<SourceText>) {
        if raw.len() >= 2
            && ((raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\'')))
        {
            let raw_text = raw.to_string();
            let raw = if self.source_matches(span, raw) {
                SourceText::source(span)
            } else {
                SourceText::cooked(span, raw_text.clone())
            };
            let cooked = raw_text[1..raw_text.len() - 1].to_string();
            return (self.source_text(cooked, span.start, span.end), Some(raw));
        }

        let text = if self.source_matches(span, raw) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, raw.to_string())
        };
        (text, None)
    }

    fn subscript_from_source_text(
        &self,
        text: SourceText,
        raw: Option<SourceText>,
        interpretation: SubscriptInterpretation,
    ) -> Subscript {
        let kind = match text.slice(self.input).trim() {
            "@" => SubscriptKind::Selector(SubscriptSelector::At),
            "*" => SubscriptKind::Selector(SubscriptSelector::Star),
            _ => SubscriptKind::Ordinary,
        };
        let arithmetic_ast = if matches!(kind, SubscriptKind::Ordinary) {
            self.simple_subscript_arithmetic_ast(&text)
                .or_else(|| self.maybe_parse_source_text_as_arithmetic(&text))
        } else {
            None
        };
        Subscript {
            text,
            raw,
            kind,
            interpretation,
            arithmetic_ast,
        }
    }

    fn simple_subscript_arithmetic_ast(&self, text: &SourceText) -> Option<ArithmeticExprNode> {
        if !text.is_source_backed() {
            return None;
        }

        let raw = text.slice(self.input);
        if raw.is_empty() || raw.trim() != raw {
            return None;
        }

        let span = text.span();
        if raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Some(ArithmeticExprNode::new(
                ArithmeticExpr::Number(SourceText::source(span)),
                span,
            ));
        }

        if Self::is_valid_identifier(raw) {
            return Some(ArithmeticExprNode::new(
                ArithmeticExpr::Variable(Name::from(raw)),
                span,
            ));
        }

        None
    }

    fn subscript_from_text(
        &self,
        raw: &str,
        span: Span,
        interpretation: SubscriptInterpretation,
    ) -> Subscript {
        let (text, raw) = self.subscript_source_text(raw, span);
        self.subscript_from_source_text(text, raw, interpretation)
    }

    fn var_ref(
        &self,
        name: impl Into<Name>,
        name_span: Span,
        subscript: Option<Subscript>,
        span: Span,
    ) -> VarRef {
        VarRef {
            name: name.into(),
            name_span,
            subscript,
            span,
        }
    }

    fn parameter_var_ref(
        &self,
        part_start: Position,
        prefix: &str,
        name: &str,
        subscript: Option<Subscript>,
        part_end: Position,
    ) -> VarRef {
        let name_start = part_start.advanced_by(prefix);
        let name_span = Span::from_positions(name_start, name_start.advanced_by(name));
        self.var_ref(
            Name::from(name),
            name_span,
            subscript,
            Span::from_positions(part_start, part_end),
        )
    }

    fn parameter_word_part_from_legacy(
        &self,
        part: WordPart,
        part_start: Position,
        part_end: Position,
        source_backed: bool,
    ) -> WordPart {
        let span = Span::from_positions(part_start, part_end);
        let raw_body = self.parameter_raw_body_from_legacy(&part, span, source_backed);
        let raw_body_text = raw_body.slice(self.input).to_string();

        let syntax = match part {
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                colon_variant,
            } => Some(BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                colon_variant,
            }),
            WordPart::Length(reference) | WordPart::ArrayLength(reference) => {
                Some(BourneParameterExpansion::Length { reference })
            }
            WordPart::ArrayAccess(reference) => {
                Some(BourneParameterExpansion::Access { reference })
            }
            WordPart::ArrayIndices(reference) => {
                Some(BourneParameterExpansion::Indices { reference })
            }
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
            } => Some(BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                length,
                length_ast,
            }),
            WordPart::IndirectExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => Some(BourneParameterExpansion::Indirect {
                name,
                operator,
                operand,
                colon_variant,
            }),
            WordPart::PrefixMatch { prefix, kind } => {
                Some(BourneParameterExpansion::PrefixMatch { prefix, kind })
            }
            WordPart::Transformation {
                reference,
                operator,
            } => Some(BourneParameterExpansion::Transformation {
                reference,
                operator,
            }),
            WordPart::Variable(name) if raw_body_text == name.as_str() => {
                Some(BourneParameterExpansion::Access {
                    reference: self.parameter_var_ref(
                        part_start,
                        "${",
                        name.as_str(),
                        None,
                        part_end,
                    ),
                })
            }
            other => return other,
        };

        WordPart::Parameter(ParameterExpansion {
            syntax: ParameterExpansionSyntax::Bourne(syntax.expect("matched Some above")),
            span,
            raw_body,
        })
    }

    fn parameter_raw_body_from_legacy(
        &self,
        part: &WordPart,
        span: Span,
        source_backed: bool,
    ) -> SourceText {
        if source_backed && span.end.offset <= self.input.len() {
            let syntax = span.slice(self.input);
            if let Some(body) = syntax
                .strip_prefix("${")
                .and_then(|syntax| syntax.strip_suffix('}'))
            {
                let start = span.start.advanced_by("${");
                let end = start.advanced_by(body);
                return SourceText::source(Span::from_positions(start, end));
            }
        }

        let mut syntax = String::new();
        self.push_word_part_syntax(&mut syntax, part, span);
        let body = syntax
            .strip_prefix("${")
            .and_then(|syntax| syntax.strip_suffix('}'))
            .unwrap_or(syntax.as_str())
            .to_string();
        SourceText::from(body)
    }

    fn zsh_parameter_word_part(
        &self,
        raw_body: SourceText,
        part_start: Position,
        part_end: Position,
    ) -> WordPart {
        let syntax = self.parse_zsh_parameter_syntax(&raw_body, raw_body.span().start);
        WordPart::Parameter(ParameterExpansion {
            syntax: ParameterExpansionSyntax::Zsh(syntax),
            span: Span::from_positions(part_start, part_end),
            raw_body,
        })
    }

    fn parse_zsh_parameter_syntax(
        &self,
        raw_body: &SourceText,
        base: Position,
    ) -> ZshParameterExpansion {
        let text = raw_body.slice(self.input);
        let mut index = 0;
        let mut modifiers = Vec::new();

        while text[index..].starts_with('(') {
            let Some(close_rel) = text[index + 1..].find(')') else {
                break;
            };
            let close = index + 1 + close_rel;
            let modifier_text = &text[index + 1..close];
            let modifier_start = base.advanced_by(&text[..index]);
            let modifier_span = Span::from_positions(
                modifier_start,
                modifier_start.advanced_by(&text[index..=close]),
            );
            let mut chars = modifier_text.chars();
            let name = chars.next().unwrap_or('?');
            let rest: String = chars.collect();
            let (argument_delimiter, argument) = if rest.is_empty() {
                (None, None)
            } else {
                let mut rest_chars = rest.chars();
                let delimiter = rest_chars.next();
                let arg = rest_chars.as_str();
                (
                    delimiter,
                    (!arg.is_empty()).then(|| SourceText::from(arg.to_string())),
                )
            };
            modifiers.push(ZshModifier {
                name,
                argument,
                argument_delimiter,
                span: modifier_span,
            });
            index = close + 1;
        }

        let (target, operation_index) = if text[index..].starts_with("${") {
            let end = self
                .find_matching_parameter_end(&text[index..])
                .unwrap_or(text.len() - index);
            let nested_text = &text[index..index + end];
            let target = self.parse_nested_parameter_target(nested_text);
            (target, index + end)
        } else if text[index..].starts_with(':') || text[index..].is_empty() {
            (ZshExpansionTarget::Empty, index)
        } else {
            let end = self
                .find_zsh_operation_start(&text[index..])
                .map(|offset| index + offset)
                .unwrap_or(text.len());
            let target_text = text[index..end].trim();
            let target = if target_text.is_empty() {
                ZshExpansionTarget::Empty
            } else {
                ZshExpansionTarget::Reference(self.parse_loose_var_ref(target_text))
            };
            (target, end)
        };

        let operation = (operation_index < text.len()).then(|| {
            self.parse_zsh_parameter_operation(
                &text[operation_index..],
                base.advanced_by(&text[..operation_index]),
            )
        });

        ZshParameterExpansion {
            target,
            modifiers,
            operation,
        }
    }

    fn parse_nested_parameter_target(&self, text: &str) -> ZshExpansionTarget {
        if !(text.starts_with("${") && text.ends_with('}')) {
            return ZshExpansionTarget::Reference(self.parse_loose_var_ref(text));
        }

        let raw_body = SourceText::from(text[2..text.len() - 1].to_string());
        let syntax = if raw_body.slice(self.input).starts_with('(')
            || raw_body.slice(self.input).starts_with(':')
        {
            ParameterExpansionSyntax::Zsh(
                self.parse_zsh_parameter_syntax(&raw_body, Position::new()),
            )
        } else {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: self.parse_loose_var_ref(raw_body.slice(self.input)),
            })
        };

        ZshExpansionTarget::Nested(Box::new(ParameterExpansion {
            syntax,
            span: Span::new(),
            raw_body,
        }))
    }

    fn parse_loose_var_ref(&self, text: &str) -> VarRef {
        let trimmed = text.trim();
        if let Some(open) = trimmed.find('[')
            && trimmed.ends_with(']')
        {
            let name = &trimmed[..open];
            let subscript_text = &trimmed[open + 1..trimmed.len() - 1];
            let subscript = Subscript {
                text: SourceText::from(subscript_text.to_string()),
                raw: None,
                kind: match subscript_text {
                    "@" => SubscriptKind::Selector(SubscriptSelector::At),
                    "*" => SubscriptKind::Selector(SubscriptSelector::Star),
                    _ => SubscriptKind::Ordinary,
                },
                interpretation: SubscriptInterpretation::Contextual,
                arithmetic_ast: None,
            };
            return VarRef {
                name: Name::from(name),
                name_span: Span::new(),
                subscript: Some(subscript),
                span: Span::new(),
            };
        }

        VarRef {
            name: Name::from(trimmed),
            name_span: Span::new(),
            subscript: None,
            span: Span::new(),
        }
    }

    fn find_matching_parameter_end(&self, text: &str) -> Option<usize> {
        let mut depth = 0_i32;
        let mut chars = text.char_indices().peekable();

        while let Some((index, ch)) = chars.next() {
            match ch {
                '$' if chars.peek().is_some_and(|(_, next)| *next == '{') => {
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + ch.len_utf8());
                    }
                }
                _ => {}
            }
        }

        None
    }

    fn find_zsh_operation_start(&self, text: &str) -> Option<usize> {
        let mut bracket_depth = 0_usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for (index, ch) in text.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '[' if !in_single && !in_double => bracket_depth += 1,
                ']' if !in_single && !in_double && bracket_depth > 0 => bracket_depth -= 1,
                ':' if !in_single && !in_double && bracket_depth == 0 => return Some(index),
                '#' | '%' | '/' | '^' | ',' | '~'
                    if !in_single && !in_double && bracket_depth == 0 && index > 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }

        None
    }

    fn zsh_operation_source_text(
        &self,
        text: &str,
        base: Position,
        start: usize,
        end: usize,
    ) -> SourceText {
        self.source_text(
            text[start..end].to_string(),
            base.advanced_by(&text[..start]),
            base.advanced_by(&text[..end]),
        )
    }

    fn find_zsh_top_level_delimiter(&self, text: &str, delimiter: char) -> Option<usize> {
        let mut chars = text.char_indices().peekable();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut brace_depth = 0_usize;
        let mut paren_depth = 0_usize;

        while let Some((index, ch)) = chars.next() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if !in_single => escaped = true,
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '$' if !in_single => {
                    if let Some((_, next)) = chars.peek() {
                        if *next == '{' {
                            brace_depth += 1;
                            chars.next();
                        } else if *next == '(' {
                            paren_depth += 1;
                            chars.next();
                            if let Some((_, after)) = chars.peek()
                                && *after == '('
                            {
                                paren_depth += 1;
                                chars.next();
                            }
                        }
                    }
                }
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                _ if ch == delimiter
                    && !in_single
                    && !in_double
                    && brace_depth == 0
                    && paren_depth == 0 =>
                {
                    return Some(index);
                }
                _ => {}
            }
        }

        None
    }

    fn zsh_slice_candidate(rest: &str) -> bool {
        let Some(first) = rest.chars().next() else {
            return false;
        };

        first.is_ascii_alphanumeric()
            || first == '_'
            || first.is_ascii_whitespace()
            || matches!(first, '$' | '\'' | '"' | '(' | '{')
    }

    fn parse_zsh_parameter_operation(&self, text: &str, base: Position) -> ZshExpansionOperation {
        if let Some(operand) = text.strip_prefix(":#") {
            return ZshExpansionOperation::PatternOperation {
                kind: ZshPatternOp::Filter,
                operand: self.source_text(
                    operand.to_string(),
                    base.advanced_by(":#"),
                    base.advanced_by(text),
                ),
            };
        }

        if let Some((kind, operand)) = text
            .strip_prefix(":-")
            .map(|operand| (ZshDefaultingOp::UseDefault, operand))
            .or_else(|| {
                text.strip_prefix(":=")
                    .map(|operand| (ZshDefaultingOp::AssignDefault, operand))
            })
            .or_else(|| {
                text.strip_prefix(":+")
                    .map(|operand| (ZshDefaultingOp::UseReplacement, operand))
            })
            .or_else(|| {
                text.strip_prefix(":?")
                    .map(|operand| (ZshDefaultingOp::Error, operand))
            })
        {
            return ZshExpansionOperation::Defaulting {
                kind,
                operand: self.source_text(
                    operand.to_string(),
                    base.advanced_by(&text[..2]),
                    base.advanced_by(text),
                ),
                colon_variant: true,
            };
        }

        if let Some((kind, prefix_len)) = [
            ("##", ZshTrimOp::RemovePrefixLong),
            ("#", ZshTrimOp::RemovePrefixShort),
            ("%%", ZshTrimOp::RemoveSuffixLong),
            ("%", ZshTrimOp::RemoveSuffixShort),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| text.starts_with(prefix).then_some((kind, prefix.len())))
        {
            return ZshExpansionOperation::TrimOperation {
                kind,
                operand: self.zsh_operation_source_text(text, base, prefix_len, text.len()),
            };
        }

        if let Some((kind, prefix_len)) = [
            ("//", ZshReplacementOp::ReplaceAll),
            ("/#", ZshReplacementOp::ReplacePrefix),
            ("/%", ZshReplacementOp::ReplaceSuffix),
            ("/", ZshReplacementOp::ReplaceFirst),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| text.starts_with(prefix).then_some((kind, prefix.len())))
        {
            let rest = &text[prefix_len..];
            let separator = self.find_zsh_top_level_delimiter(rest, '/');
            let pattern_end = separator.unwrap_or(rest.len());
            return ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern: self.zsh_operation_source_text(
                    text,
                    base,
                    prefix_len,
                    prefix_len + pattern_end,
                ),
                replacement: separator.map(|separator| {
                    self.zsh_operation_source_text(
                        text,
                        base,
                        prefix_len + separator + 1,
                        text.len(),
                    )
                }),
            };
        }

        if let Some(rest) = text.strip_prefix(':')
            && Self::zsh_slice_candidate(rest)
        {
            let separator = self.find_zsh_top_level_delimiter(rest, ':');
            let offset_end = separator.unwrap_or(rest.len());
            return ZshExpansionOperation::Slice {
                offset: self.zsh_operation_source_text(text, base, 1, 1 + offset_end),
                length: separator.map(|separator| {
                    self.zsh_operation_source_text(text, base, 1 + separator + 1, text.len())
                }),
            };
        }

        ZshExpansionOperation::Unknown(self.source_text(
            text.to_string(),
            base,
            base.advanced_by(text),
        ))
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
            Self::rebase_var_ref(&mut assignment.target, base);
            match &mut assignment.value {
                AssignmentValue::Scalar(word) => Self::rebase_word(word, base),
                AssignmentValue::Compound(array) => Self::rebase_array_expr(array, base),
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

    fn ensure_feature(
        &self,
        enabled: bool,
        feature: &str,
        unsupported_message: &str,
    ) -> Result<()> {
        if enabled {
            Ok(())
        } else {
            Err(self.error(format!("{feature} {unsupported_message}")))
        }
    }

    fn ensure_double_bracket(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().double_bracket,
            "[[ ]] conditionals",
            "are not available in this shell mode",
        )
    }

    fn ensure_arithmetic_for(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().arithmetic_for,
            "c-style for loops",
            "are not available in this shell mode",
        )
    }

    fn ensure_coproc(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().coproc_keyword,
            "coprocess commands",
            "are not available in this shell mode",
        )
    }

    fn ensure_arithmetic_command(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().arithmetic_command,
            "arithmetic commands",
            "are not available in this shell mode",
        )
    }

    fn ensure_select_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().select_loop,
            "select loops",
            "are not available in this shell mode",
        )
    }

    fn ensure_repeat_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().zsh_repeat_loop,
            "repeat loops",
            "are not available in this shell mode",
        )
    }

    fn ensure_foreach_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().zsh_foreach_loop,
            "foreach loops",
            "are not available in this shell mode",
        )
    }

    fn ensure_function_keyword(&self) -> Result<()> {
        self.ensure_feature(
            self.dialect.features().function_keyword,
            "function keyword definitions",
            "are not available in this shell mode",
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

    fn is_empty_stmt_placeholder(command: &Command) -> bool {
        matches!(
            command,
            Command::Simple(SimpleCommand {
                name,
                args,
                redirects,
                assignments,
                ..
            }) if name.render("").is_empty()
                && args.is_empty()
                && redirects.is_empty()
                && assignments.is_empty()
        )
    }

    fn compound_span(compound: &CompoundCommand) -> Span {
        match compound {
            CompoundCommand::If(command) => command.span,
            CompoundCommand::For(command) => command.span,
            CompoundCommand::Repeat(command) => command.span,
            CompoundCommand::Foreach(command) => command.span,
            CompoundCommand::ArithmeticFor(command) => command.span,
            CompoundCommand::While(command) => command.span,
            CompoundCommand::Until(command) => command.span,
            CompoundCommand::Case(command) => command.span,
            CompoundCommand::Select(command) => command.span,
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => body.span,
            CompoundCommand::Arithmetic(command) => command.span,
            CompoundCommand::Time(command) => command.span,
            CompoundCommand::Conditional(command) => command.span,
            CompoundCommand::Coproc(command) => command.span,
            CompoundCommand::Always(command) => command.span,
        }
    }

    fn stmt_seq_with_span(span: Span, stmts: Vec<Stmt>) -> StmtSeq {
        StmtSeq {
            leading_comments: Vec::new(),
            stmts,
            trailing_comments: Vec::new(),
            span,
        }
    }

    fn lower_commands_to_stmt_seq(commands: Vec<Command>, span: Span) -> StmtSeq {
        let mut stmts = Vec::new();
        for command in commands {
            Self::lower_command_into_stmts(command, &mut stmts);
        }
        Self::stmt_seq_with_span(span, stmts)
    }

    fn lower_command_into_stmts(command: Command, stmts: &mut Vec<Stmt>) {
        match command {
            Command::List(list) => Self::lower_list_into_stmts(list, stmts),
            other => stmts.push(Self::lower_non_sequence_command_to_stmt(other)),
        }
    }

    fn lower_list_into_stmts(list: CommandList, stmts: &mut Vec<Stmt>) {
        let CommandList { first, rest, .. } = list;
        let mut current = *first;
        let mut pending = Vec::new();

        for item in rest {
            match item.operator {
                ListOperator::And | ListOperator::Or => pending.push(item),
                ListOperator::Semicolon | ListOperator::Background(_) => {
                    let terminator = match item.operator {
                        ListOperator::Semicolon => StmtTerminator::Semicolon,
                        ListOperator::Background(operator) => StmtTerminator::Background(operator),
                        ListOperator::And | ListOperator::Or => unreachable!(),
                    };
                    let mut stmt =
                        Self::lower_and_or_segment(current, std::mem::take(&mut pending));
                    stmt.terminator = Some(terminator);
                    stmt.terminator_span = Some(item.operator_span);
                    stmts.push(stmt);

                    if Self::is_empty_stmt_placeholder(&item.command) {
                        return;
                    }
                    current = item.command;
                }
            }
        }

        stmts.push(Self::lower_and_or_segment(current, pending));
    }

    fn lower_and_or_segment(first: Command, rest: Vec<CommandListItem>) -> Stmt {
        let mut stmt = Self::lower_non_sequence_command_to_stmt(first);

        for item in rest {
            let op = match item.operator {
                ListOperator::And => BinaryOp::And,
                ListOperator::Or => BinaryOp::Or,
                ListOperator::Semicolon | ListOperator::Background(_) => unreachable!(),
            };
            let right = Self::lower_non_sequence_command_to_stmt(item.command);
            let span = stmt.span.merge(right.span);
            stmt = Stmt {
                leading_comments: Vec::new(),
                command: AstCommand::Binary(BinaryCommand {
                    left: Box::new(stmt),
                    op,
                    op_span: item.operator_span,
                    right: Box::new(right),
                    span,
                }),
                negated: false,
                redirects: Vec::new(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
                span,
            };
        }

        stmt
    }

    fn lower_builtin_command(builtin: BuiltinCommand) -> (AstBuiltinCommand, Vec<Redirect>, Span) {
        match builtin {
            BuiltinCommand::Break(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Break(AstBreakCommand {
                        depth: command.depth,
                        extra_args: command.extra_args,
                        assignments: command.assignments,
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Continue(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Continue(AstContinueCommand {
                        depth: command.depth,
                        extra_args: command.extra_args,
                        assignments: command.assignments,
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Return(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Return(AstReturnCommand {
                        code: command.code,
                        extra_args: command.extra_args,
                        assignments: command.assignments,
                        span,
                    }),
                    redirects,
                    span,
                )
            }
            BuiltinCommand::Exit(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Exit(AstExitCommand {
                        code: command.code,
                        extra_args: command.extra_args,
                        assignments: command.assignments,
                        span,
                    }),
                    redirects,
                    span,
                )
            }
        }
    }

    fn lower_non_sequence_command_to_stmt(command: Command) -> Stmt {
        match command {
            Command::Simple(command) => Stmt {
                leading_comments: Vec::new(),
                command: AstCommand::Simple(AstSimpleCommand {
                    name: command.name,
                    args: command.args,
                    assignments: command.assignments,
                    span: command.span,
                }),
                negated: false,
                redirects: command.redirects,
                terminator: None,
                terminator_span: None,
                inline_comment: None,
                span: command.span,
            },
            Command::Builtin(command) => {
                let (command, redirects, span) = Self::lower_builtin_command(command);
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Builtin(command),
                    negated: false,
                    redirects,
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span,
                }
            }
            Command::Decl(command) => Stmt {
                leading_comments: Vec::new(),
                command: AstCommand::Decl(AstDeclClause {
                    variant: command.variant,
                    variant_span: command.variant_span,
                    operands: command.operands,
                    assignments: command.assignments,
                    span: command.span,
                }),
                negated: false,
                redirects: command.redirects,
                terminator: None,
                terminator_span: None,
                inline_comment: None,
                span: command.span,
            },
            Command::Pipeline(pipeline) => {
                let Pipeline {
                    negated,
                    commands,
                    operators,
                    span,
                } = pipeline;
                let mut commands = commands.into_iter();
                let mut stmt = Self::lower_non_sequence_command_to_stmt(
                    commands
                        .next()
                        .expect("pipeline should contain at least one command"),
                );
                for ((op, op_span), command) in operators.into_iter().zip(commands) {
                    let right = Self::lower_non_sequence_command_to_stmt(command);
                    let binary_span = stmt.span.merge(right.span);
                    stmt = Stmt {
                        leading_comments: Vec::new(),
                        command: AstCommand::Binary(BinaryCommand {
                            left: Box::new(stmt),
                            op,
                            op_span,
                            right: Box::new(right),
                            span: binary_span,
                        }),
                        negated: false,
                        redirects: Vec::new(),
                        terminator: None,
                        terminator_span: None,
                        inline_comment: None,
                        span: binary_span,
                    };
                }
                stmt.negated = negated;
                stmt.span = span;
                stmt
            }
            Command::Compound(compound, redirects) => {
                let span = Self::compound_span(&compound);
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Compound(*compound),
                    negated: false,
                    redirects,
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span,
                }
            }
            Command::Function(function) => Stmt {
                leading_comments: Vec::new(),
                span: function.span,
                command: AstCommand::Function(function),
                negated: false,
                redirects: Vec::new(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
            },
            Command::AnonymousFunction(function, redirects) => Stmt {
                leading_comments: Vec::new(),
                span: function.span,
                command: AstCommand::AnonymousFunction(function),
                negated: false,
                redirects,
                terminator: None,
                terminator_span: None,
                inline_comment: None,
            },
            Command::List(list) => {
                let mut stmts = Vec::new();
                Self::lower_list_into_stmts(list, &mut stmts);
                stmts
                    .into_iter()
                    .next()
                    .expect("command list should lower to at least one statement")
            }
        }
    }

    fn comment_start(comment: Comment) -> usize {
        usize::from(comment.range.start())
    }

    fn is_inline_comment(source: &str, stmt: &Stmt, comment: Comment) -> bool {
        let comment_start = Self::comment_start(comment);
        if comment_start < stmt.span.end.offset {
            return false;
        }
        source
            .get(stmt.span.end.offset..comment_start)
            .is_some_and(|gap| !gap.contains('\n'))
    }

    fn take_comments_before(
        comments: &mut VecDeque<Comment>,
        end_offset: usize,
    ) -> VecDeque<Comment> {
        let mut taken = VecDeque::new();
        while comments
            .front()
            .is_some_and(|comment| Self::comment_start(*comment) < end_offset)
        {
            taken.push_back(
                comments
                    .pop_front()
                    .expect("front comment should exist while draining"),
            );
        }
        taken
    }

    fn attach_comments_to_file(&self, file: &mut File) {
        let mut comments = self.comments.iter().copied().collect::<VecDeque<_>>();
        Self::attach_comments_to_stmt_seq_with_source(self.input, &mut file.body, &mut comments);
        file.body.trailing_comments.extend(comments);
    }

    fn attach_comments_to_stmt_seq_with_source(
        source: &str,
        sequence: &mut StmtSeq,
        comments: &mut VecDeque<Comment>,
    ) {
        if sequence.stmts.is_empty() {
            sequence
                .trailing_comments
                .extend(Self::take_comments_before(
                    comments,
                    sequence.span.end.offset,
                ));
            return;
        }

        for (index, stmt) in sequence.stmts.iter_mut().enumerate() {
            let leading = Self::take_comments_before(comments, stmt.span.start.offset);
            if index == 0 {
                sequence.leading_comments.extend(leading);
            } else {
                stmt.leading_comments.extend(leading);
            }

            let mut nested = Self::take_comments_before(comments, stmt.span.end.offset);
            Self::attach_comments_to_stmt_with_source(source, stmt, &mut nested);
            if !nested.is_empty() {
                stmt.leading_comments.extend(nested);
            }

            if stmt.inline_comment.is_none()
                && comments
                    .front()
                    .is_some_and(|comment| Self::is_inline_comment(source, stmt, *comment))
            {
                stmt.inline_comment = comments.pop_front();
            }
        }

        sequence
            .trailing_comments
            .extend(Self::take_comments_before(
                comments,
                sequence.span.end.offset,
            ));
    }

    fn attach_comments_to_stmt_with_source(
        source: &str,
        stmt: &mut Stmt,
        comments: &mut VecDeque<Comment>,
    ) {
        match &mut stmt.command {
            AstCommand::Binary(binary) => {
                let mut left_comments =
                    Self::take_comments_before(comments, binary.left.span.end.offset);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    binary.left.as_mut(),
                    &mut left_comments,
                );
                if !left_comments.is_empty() {
                    binary.left.leading_comments.extend(left_comments);
                }

                let mut right_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    binary.right.as_mut(),
                    &mut right_comments,
                );
                if !right_comments.is_empty() {
                    binary.right.leading_comments.extend(right_comments);
                }
            }
            AstCommand::Compound(compound) => {
                Self::attach_comments_to_compound_with_source(source, compound, comments);
            }
            AstCommand::Function(function) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    function.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    function.body.leading_comments.extend(body_comments);
                }
            }
            AstCommand::AnonymousFunction(function) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    function.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    function.body.leading_comments.extend(body_comments);
                }
            }
            AstCommand::Simple(_) | AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
        }
    }

    fn attach_comments_to_compound_with_source(
        source: &str,
        command: &mut CompoundCommand,
        comments: &mut VecDeque<Comment>,
    ) {
        match command {
            CompoundCommand::If(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut then_branch =
                    Self::take_comments_before(comments, command.then_branch.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.then_branch,
                    &mut then_branch,
                );
                command.then_branch.trailing_comments.extend(then_branch);

                for (condition_seq, body_seq) in &mut command.elif_branches {
                    let mut elif_condition =
                        Self::take_comments_before(comments, condition_seq.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        condition_seq,
                        &mut elif_condition,
                    );
                    condition_seq.trailing_comments.extend(elif_condition);

                    let mut elif_body =
                        Self::take_comments_before(comments, body_seq.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(source, body_seq, &mut elif_body);
                    body_seq.trailing_comments.extend(elif_body);
                }

                if let Some(else_branch) = &mut command.else_branch {
                    let mut else_comments = std::mem::take(comments);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        else_branch,
                        &mut else_comments,
                    );
                    else_branch.trailing_comments.extend(else_comments);
                }
            }
            CompoundCommand::For(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Repeat(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Foreach(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::ArithmeticFor(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::While(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Until(command) => {
                let mut condition =
                    Self::take_comments_before(comments, command.condition.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.condition,
                    &mut condition,
                );
                command.condition.trailing_comments.extend(condition);

                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Case(command) => {
                for case in &mut command.cases {
                    let mut body_comments =
                        Self::take_comments_before(comments, case.body.span.end.offset);
                    Self::attach_comments_to_stmt_seq_with_source(
                        source,
                        &mut case.body,
                        &mut body_comments,
                    );
                    case.body.trailing_comments.extend(body_comments);
                }
            }
            CompoundCommand::Select(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Subshell(body) | CompoundCommand::BraceGroup(body) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(source, body, &mut body_comments);
                body.trailing_comments.extend(body_comments);
            }
            CompoundCommand::Always(command) => {
                let mut body_comments =
                    Self::take_comments_before(comments, command.body.span.end.offset);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.body,
                    &mut body_comments,
                );
                command.body.trailing_comments.extend(body_comments);

                let mut always_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_seq_with_source(
                    source,
                    &mut command.always_body,
                    &mut always_comments,
                );
                command
                    .always_body
                    .trailing_comments
                    .extend(always_comments);
            }
            CompoundCommand::Time(command) => {
                if let Some(inner) = &mut command.command {
                    let mut inner_comments = std::mem::take(comments);
                    Self::attach_comments_to_stmt_with_source(
                        source,
                        inner.as_mut(),
                        &mut inner_comments,
                    );
                    if !inner_comments.is_empty() {
                        inner.leading_comments.extend(inner_comments);
                    }
                }
            }
            CompoundCommand::Coproc(command) => {
                let mut body_comments = std::mem::take(comments);
                Self::attach_comments_to_stmt_with_source(
                    source,
                    command.body.as_mut(),
                    &mut body_comments,
                );
                if !body_comments.is_empty() {
                    command.body.leading_comments.extend(body_comments);
                }
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
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
            if self
                .current_token
                .as_ref()
                .is_some_and(|token| token.flags.is_synthetic())
            {
                self.expand_next_word = true;
            } else {
                self.maybe_expand_current_alias_chain();
            }
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

    fn current_token_has_leading_whitespace(&self) -> bool {
        self.current_span.start.offset > 0
            && self.input[..self.current_span.start.offset]
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, ' ' | '\t' | '\n'))
    }

    fn current_token_is_tight_to_next_token(&mut self) -> bool {
        let current_end = self.current_span.end.offset;
        self.peek_next()
            .is_some_and(|token| token.span.start.offset == current_end)
    }

    fn current_brace_body_context(&self) -> Option<BraceBodyContext> {
        self.brace_body_stack.last().copied()
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
            b"repeat" => Some(Keyword::Repeat),
            b"foreach" => Some(Keyword::Foreach),
            b"while" => Some(Keyword::While),
            b"until" => Some(Keyword::Until),
            b"case" => Some(Keyword::Case),
            b"select" => Some(Keyword::Select),
            b"time" => Some(Keyword::Time),
            b"coproc" => Some(Keyword::Coproc),
            b"function" => Some(Keyword::Function),
            b"always" => Some(Keyword::Always),
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

    fn looks_like_disabled_repeat_loop(&self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Repeat) {
            return Ok(false);
        }

        let mut probe = self.clone();
        probe.advance();
        if !probe.at_word_like() {
            return Ok(false);
        }
        probe.advance();

        match probe.current_token_kind {
            Some(TokenKind::LeftBrace) => Ok(true),
            Some(TokenKind::Semicolon) => {
                probe.advance();
                probe.skip_newlines()?;
                Ok(probe.current_keyword() == Some(Keyword::Do))
            }
            Some(TokenKind::Newline) => {
                probe.skip_newlines()?;
                Ok(probe.current_keyword() == Some(Keyword::Do))
            }
            _ => Ok(false),
        }
    }

    fn looks_like_disabled_foreach_loop(&self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Foreach) {
            return Ok(false);
        }

        let mut probe = self.clone();
        probe.advance();
        if probe.current_name_token().is_none() {
            return Ok(false);
        }
        probe.advance();

        if probe.at(TokenKind::LeftParen) {
            probe.advance();
            let mut saw_word = false;
            while !probe.at(TokenKind::RightParen) {
                if !probe.at_word_like() {
                    return Ok(false);
                }
                saw_word = true;
                probe.advance();
            }
            if !saw_word {
                return Ok(false);
            }
            probe.advance();
            return Ok(probe.at(TokenKind::LeftBrace));
        }

        if probe.current_keyword() != Some(Keyword::In) {
            return Ok(false);
        }
        probe.advance();

        let mut saw_word = false;
        let saw_separator = loop {
            if probe.current_keyword() == Some(Keyword::Do) {
                break false;
            }

            match probe.current_token_kind {
                Some(kind) if kind.is_word_like() => {
                    saw_word = true;
                    probe.advance();
                }
                Some(TokenKind::Semicolon) => {
                    probe.advance();
                    break true;
                }
                Some(TokenKind::Newline) => {
                    probe.skip_newlines()?;
                    break true;
                }
                _ => break false,
            }
        };

        Ok(saw_word && saw_separator && probe.current_keyword() == Some(Keyword::Do))
    }

    fn skip_newlines_with_flag(&mut self) -> Result<bool> {
        let mut skipped = false;
        while self.at(TokenKind::Newline) {
            self.tick()?;
            self.advance();
            skipped = true;
        }
        Ok(skipped)
    }

    fn skip_newlines(&mut self) -> Result<()> {
        self.skip_newlines_with_flag().map(|_| ())
    }
}
#[cfg(test)]
mod tests;
