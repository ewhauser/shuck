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
    AlwaysCommand, ArithmeticCommand, ArithmeticExpansionSyntax, ArithmeticExpr,
    ArithmeticExprNode, ArithmeticForCommand, ArithmeticLvalue, ArrayElem, ArrayExpr, ArrayKind,
    Assignment, AssignmentValue, BackgroundOperator, BinaryCommand, BinaryOp,
    BourneParameterExpansion, BraceExpansionKind, BraceQuoteContext, BraceSyntax, BraceSyntaxKind,
    BreakCommand as AstBreakCommand, BuiltinCommand as AstBuiltinCommand, CaseCommand, CaseItem,
    CaseTerminator, Command as AstCommand, CommandSubstitutionSyntax, Comment, CompoundCommand,
    ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand, ConditionalExpr,
    ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp,
    ContinueCommand as AstContinueCommand, CoprocCommand, DeclClause as AstDeclClause, DeclOperand,
    ExitCommand as AstExitCommand, File, ForCommand, ForeachCommand, ForeachSyntax, FunctionDef,
    FunctionSurface, Heredoc, HeredocDelimiter, IfCommand, IfSyntax, LiteralText, Name,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternGroupKind,
    PatternPart, PatternPartNode, Position, PrefixMatchKind, Redirect, RedirectKind,
    RedirectTarget, RepeatCommand, RepeatSyntax, ReturnCommand as AstReturnCommand, SelectCommand,
    SimpleCommand as AstSimpleCommand, SourceText, Span, Stmt, StmtSeq, StmtTerminator, Subscript,
    SubscriptInterpretation, SubscriptKind, SubscriptSelector, TextSize, TimeCommand, TokenKind,
    UntilCommand, VarRef, WhileCommand, Word, WordPart, WordPartNode, ZshDefaultingOp,
    ZshExpansionOperation, ZshExpansionTarget, ZshGlobQualifier, ZshGlobQualifierGroup,
    ZshModifier, ZshParameterExpansion, ZshPatternOp, ZshQualifiedGlob, ZshReplacementOp,
    ZshTrimOp,
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
struct PatternCursor {
    segment_index: usize,
    literal_offset: usize,
    position: Position,
}

enum PatternSegment<'a> {
    Literal { text: &'a str, span: Span },
    Word(&'a WordPartNode),
}

struct PatternParser<'a> {
    input: &'a str,
    segments: Vec<PatternSegment<'a>>,
    full_span: Span,
}

enum WordTargetBoundary {
    EndOfWord,
    Assignment { append: bool, value_start: Position },
}

struct ParsedWordTarget {
    name: Name,
    name_span: Span,
    subscript: Option<Subscript>,
    boundary: WordTargetBoundary,
}

impl<'a> PatternParser<'a> {
    fn new(input: &'a str, word: &'a Word) -> Self {
        Self::from_word_parts(input, &word.parts, word.span)
    }

    fn from_word_parts(input: &'a str, parts: &'a [WordPartNode], full_span: Span) -> Self {
        let mut segments = Vec::with_capacity(parts.len());

        for part in parts {
            match &part.kind {
                WordPart::Literal(text) => segments.push(PatternSegment::Literal {
                    text: text.as_str(input, part.span),
                    span: part.span,
                }),
                _ => segments.push(PatternSegment::Word(part)),
            }
        }

        Self {
            input,
            segments,
            full_span,
        }
    }

    fn parse(&self) -> Pattern {
        let mut cursor = PatternCursor {
            segment_index: 0,
            literal_offset: 0,
            position: self
                .segments
                .first()
                .map(|segment| self.segment_start(segment))
                .unwrap_or(self.full_span.start),
        };
        let mut pattern = self.parse_until(&mut cursor, false);
        pattern.span = self.full_span;
        pattern
    }

    fn parse_until(&self, cursor: &mut PatternCursor, stop_at_group_delim: bool) -> Pattern {
        let start = cursor.position;
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut literal_start: Option<Position> = None;
        let mut literal_end = start;

        while let Some(segment) = self.segments.get(cursor.segment_index) {
            if stop_at_group_delim && self.peek_group_delimiter(*cursor).is_some() {
                break;
            }

            match segment {
                PatternSegment::Word(part) => {
                    self.flush_literal(&mut parts, &mut literal, &mut literal_start, literal_end);
                    parts.push(PatternPartNode::new(
                        PatternPart::Word(Word {
                            parts: vec![(*part).clone()],
                            span: part.span,
                            brace_syntax: Vec::new(),
                        }),
                        part.span,
                    ));
                    self.advance_to_next_segment(cursor);
                }
                PatternSegment::Literal { .. } => {
                    if self.peek_literal_char(*cursor).is_none() {
                        self.advance_to_next_segment(cursor);
                        continue;
                    }

                    if let Some((group, next_cursor)) = self.try_parse_group(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(group);
                        *cursor = next_cursor;
                        continue;
                    }

                    if let Some((char_class, next_cursor)) = self.try_parse_char_class(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(char_class);
                        *cursor = next_cursor;
                        continue;
                    }

                    if let Some((wildcard, next_cursor)) = self.try_parse_wildcard(*cursor) {
                        self.flush_literal(
                            &mut parts,
                            &mut literal,
                            &mut literal_start,
                            literal_end,
                        );
                        parts.push(wildcard);
                        *cursor = next_cursor;
                        continue;
                    }

                    let Some((ch, span)) = self.consume_literal_char(cursor) else {
                        break;
                    };
                    if literal_start.is_none() {
                        literal_start = Some(span.start);
                    }
                    literal_end = span.end;
                    literal.push(ch);
                }
            }
        }

        self.flush_literal(&mut parts, &mut literal, &mut literal_start, literal_end);

        Pattern {
            span: if let (Some(first), Some(last)) = (parts.first(), parts.last()) {
                first.span.merge(last.span)
            } else {
                Span::from_positions(start, cursor.position)
            },
            parts,
        }
    }

    fn flush_literal(
        &self,
        parts: &mut Vec<PatternPartNode>,
        literal: &mut String,
        literal_start: &mut Option<Position>,
        literal_end: Position,
    ) {
        let Some(start) = literal_start.take() else {
            return;
        };
        let span = Span::from_positions(start, literal_end);
        let text = std::mem::take(literal);
        parts.push(PatternPartNode::new(
            PatternPart::Literal(self.literal_text(span, text)),
            span,
        ));
    }

    fn try_parse_wildcard(
        &self,
        cursor: PatternCursor,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        let ch = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || !matches!(ch, '*' | '?') {
            return None;
        }

        let mut next_cursor = cursor;
        let (_, span) = self.consume_literal_char(&mut next_cursor)?;
        let kind = if ch == '*' {
            PatternPart::AnyString
        } else {
            PatternPart::AnyChar
        };
        Some((PatternPartNode::new(kind, span), next_cursor))
    }

    fn try_parse_char_class(
        &self,
        cursor: PatternCursor,
    ) -> Option<(PatternPartNode, PatternCursor)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        if self.is_escaped(cursor) || !text[cursor.literal_offset..].starts_with('[') {
            return None;
        }

        let end_offset = self.find_char_class_end(text, cursor.literal_offset)?;
        let raw = &text[cursor.literal_offset..end_offset];
        let start = cursor.position;
        let end = start.advanced_by(raw);
        let span = Span::from_positions(start, end);
        let mut next_cursor = cursor;
        next_cursor.literal_offset = end_offset;
        next_cursor.position = end;
        if end_offset == text.len() {
            self.advance_to_next_segment(&mut next_cursor);
        }

        Some((
            PatternPartNode::new(
                PatternPart::CharClass(self.source_text(span, raw.to_string())),
                span,
            ),
            next_cursor,
        ))
    }

    fn try_parse_group(&self, cursor: PatternCursor) -> Option<(PatternPartNode, PatternCursor)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        let opener = self.peek_literal_char(cursor)?;
        if self.is_escaped(cursor) || !matches!(opener, '?' | '*' | '+' | '@' | '!') {
            return None;
        }
        let mut chars = text[cursor.literal_offset..].chars();
        chars.next()?;
        if chars.next()? != '(' {
            return None;
        }

        let kind = match opener {
            '?' => PatternGroupKind::ZeroOrOne,
            '*' => PatternGroupKind::ZeroOrMore,
            '+' => PatternGroupKind::OneOrMore,
            '@' => PatternGroupKind::ExactlyOne,
            '!' => PatternGroupKind::NoneOf,
            _ => return None,
        };

        let start = cursor.position;
        let mut next_cursor = cursor;
        self.consume_literal_char(&mut next_cursor)?;
        self.consume_literal_char(&mut next_cursor)?;

        let mut patterns = Vec::new();
        loop {
            patterns.push(self.parse_until(&mut next_cursor, true));
            match self.peek_group_delimiter(next_cursor) {
                Some('|') => {
                    self.consume_literal_char(&mut next_cursor)?;
                }
                Some(')') => {
                    let (_, close_span) = self.consume_literal_char(&mut next_cursor)?;
                    return Some((
                        PatternPartNode::new(
                            PatternPart::Group { kind, patterns },
                            Span::from_positions(start, close_span.end),
                        ),
                        next_cursor,
                    ));
                }
                _ => return None,
            }
        }
    }

    fn find_char_class_end(&self, text: &str, start_offset: usize) -> Option<usize> {
        let mut cursor = start_offset + '['.len_utf8();
        let mut chars = text[cursor..].chars();

        if matches!(chars.next(), Some('!') | Some('^')) {
            cursor += 1;
        }
        if text[cursor..].starts_with(']') {
            cursor += 1;
        }

        while cursor < text.len() {
            let rest = &text[cursor..];
            let ch = rest.chars().next()?;

            if ch == '\\' {
                cursor += ch.len_utf8();
                if let Some(next) = text[cursor..].chars().next() {
                    cursor += next.len_utf8();
                }
                continue;
            }

            if ch == '['
                && let Some(class_kind) = text[cursor + 1..].chars().next()
                && matches!(class_kind, ':' | '.' | '=')
            {
                cursor += '['.len_utf8() + class_kind.len_utf8();
                loop {
                    let rest = &text[cursor..];
                    let inner = rest.chars().next()?;
                    cursor += inner.len_utf8();
                    if inner == class_kind && text[cursor..].starts_with(']') {
                        cursor += ']'.len_utf8();
                        break;
                    }
                }
                continue;
            }

            cursor += ch.len_utf8();
            if ch == ']' {
                return Some(cursor);
            }
        }

        None
    }

    fn peek_group_delimiter(&self, cursor: PatternCursor) -> Option<char> {
        let ch = self.peek_literal_char(cursor)?;
        (!self.is_escaped(cursor) && matches!(ch, '|' | ')')).then_some(ch)
    }

    fn peek_literal_char(&self, cursor: PatternCursor) -> Option<char> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        text[cursor.literal_offset..].chars().next()
    }

    fn is_escaped(&self, cursor: PatternCursor) -> bool {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index).unwrap()
        else {
            return false;
        };
        let mut backslashes = 0;
        let mut offset = cursor.literal_offset;
        while offset > 0 {
            offset -= 1;
            if text.as_bytes()[offset] != b'\\' {
                break;
            }
            backslashes += 1;
        }
        backslashes % 2 == 1
    }

    fn consume_literal_char(&self, cursor: &mut PatternCursor) -> Option<(char, Span)> {
        let PatternSegment::Literal { text, .. } = self.segments.get(cursor.segment_index)? else {
            return None;
        };
        let ch = text[cursor.literal_offset..].chars().next()?;
        let start = cursor.position;
        cursor.literal_offset += ch.len_utf8();
        cursor.position.advance(ch);
        let span = Span::from_positions(start, cursor.position);

        if cursor.literal_offset == text.len() {
            self.advance_to_next_segment(cursor);
        }

        Some((ch, span))
    }

    fn advance_to_next_segment(&self, cursor: &mut PatternCursor) {
        cursor.segment_index += 1;
        cursor.literal_offset = 0;
        cursor.position = self
            .segments
            .get(cursor.segment_index)
            .map(|segment| self.segment_start(segment))
            .unwrap_or(self.full_span.end);
    }

    fn segment_start(&self, segment: &PatternSegment<'_>) -> Position {
        match segment {
            PatternSegment::Literal { span, .. } => span.start,
            PatternSegment::Word(part) => part.span.start,
        }
    }

    fn literal_text(&self, span: Span, text: String) -> LiteralText {
        if self.source_matches(span, &text) {
            LiteralText::source()
        } else {
            LiteralText::owned(text)
        }
    }

    fn source_text(&self, span: Span, text: String) -> SourceText {
        if self.source_matches(span, &text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, text)
        }
    }

    fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == text
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
                    self.collect_brace_syntax_from_pattern(&glob.pattern, quote_context, out);
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

        let (base_len, qualifiers) =
            self.parse_zsh_glob_qualifier_group(text, span.start, source_backed)?;
        if base_len == 0 {
            return None;
        }

        let base_text = &text[..base_len];
        let base_span = Span::from_positions(span.start, qualifiers.span.start);
        let pattern_word =
            self.decode_word_text(base_text, base_span, base_span.start, source_backed);
        let pattern = self.pattern_from_word(&pattern_word);

        if !Self::pattern_has_glob_syntax(&pattern) {
            return None;
        }

        Some(self.word_with_parts(
            vec![WordPartNode::new(
                WordPart::ZshQualifiedGlob(ZshQualifiedGlob {
                    span,
                    pattern,
                    qualifiers,
                }),
                span,
            )],
            span,
        ))
    }

    fn parse_zsh_glob_qualifier_group(
        &self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> Option<(usize, ZshGlobQualifierGroup)> {
        text.strip_suffix(')')?;

        let mut in_bracket = false;
        let mut paren_depth = 0usize;
        let mut group_count = 0usize;
        let mut group_start = None;
        let mut group_end = None;

        for (index, ch) in text.char_indices() {
            if in_bracket {
                if ch == ']' {
                    in_bracket = false;
                }
                continue;
            }

            match ch {
                '[' => in_bracket = true,
                '(' => {
                    if paren_depth == 0 {
                        group_count += 1;
                        group_start = Some(index);
                    }
                    paren_depth += 1;
                    if paren_depth > 1 {
                        return None;
                    }
                }
                ')' => {
                    if paren_depth == 0 {
                        return None;
                    }
                    paren_depth -= 1;
                    if paren_depth == 0 {
                        let end = index + ch.len_utf8();
                        if end != text.len() {
                            return None;
                        }
                        group_end = Some(end);
                    }
                }
                _ => {}
            }
        }

        if in_bracket || paren_depth != 0 || group_count != 1 {
            return None;
        }

        let group_start = group_start?;
        let group_end = group_end?;
        let inner_start = group_start + "(".len();
        let inner_end = group_end - ")".len();
        let fragments = self.parse_zsh_glob_qualifier_fragments(
            &text[inner_start..inner_end],
            Self::text_position(base, text, inner_start),
            source_backed,
        )?;

        Some((
            group_start,
            ZshGlobQualifierGroup {
                span: Span::from_positions(
                    Self::text_position(base, text, group_start),
                    Self::text_position(base, text, group_end),
                ),
                fragments,
            },
        ))
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
                function.name_span = function.name_span.rebased(base);
                Self::rebase_stmt(function.body.as_mut(), base);
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
                command.variable_span = command.variable_span.rebased(base);
                if let Some(words) = &mut command.words {
                    Self::rebase_words(words, base);
                }
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
        Self::rebase_pattern(&mut glob.pattern, base);
        glob.qualifiers.span = glob.qualifiers.span.rebased(base);
        for fragment in &mut glob.qualifiers.fragments {
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

    fn pattern_from_word(&self, word: &Word) -> Pattern {
        PatternParser::new(self.input, word).parse()
    }

    fn pattern_from_source_text(&mut self, text: &SourceText) -> Pattern {
        let span = text.span();
        let mut parts = Vec::new();
        self.decode_word_parts_into_with_quote_fragments(
            text.slice(self.input),
            span.start,
            text.is_source_backed(),
            true,
            &mut parts,
        );
        PatternParser::from_word_parts(self.input, &parts, span).parse()
    }

    fn single_literal_word_text<'b>(&'b self, word: &'b Word) -> Option<&'b str> {
        if word.is_fully_quoted() || word.parts.len() != 1 {
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

    fn source_text_needs_quote_preserving_decode(text: &str) -> bool {
        text.contains(['\'', '"'])
    }

    fn decode_word_text_preserving_quotes_if_needed(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        if !Self::source_text_needs_quote_preserving_decode(s)
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(s, span, source_backed)
        {
            return word;
        }

        if Self::source_text_needs_quote_preserving_decode(s) {
            self.decode_fragment_word_text(s, span, base, source_backed)
        } else {
            self.decode_word_text(s, span, base, source_backed)
        }
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
        let quoted_parts = word.has_quoted_parts();

        if let Some((text, token_quoted)) = self.current_static_token_text() {
            let quoted = quoted_parts || token_quoted || raw_text != text;
            return Some((word, text, quoted));
        }

        let text = self.literal_word_text(&word)?;
        let quoted = quoted_parts || raw_text != text;
        Some((word, text, quoted))
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
                | TokenKind::BackgroundPipe
                | TokenKind::BackgroundBang
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

        let file_span =
            Span::from_positions(Position::new(), Position::new().advanced_by(self.input));
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

        let mut file = File {
            body: Self::lower_commands_to_stmt_seq(commands, file_span),
            span: file_span,
        };
        self.attach_comments_to_file(&mut file);
        Ok(ParseOutput { file })
    }

    /// Parse the input while recovering at top-level command boundaries.
    pub fn parse_recovered(mut self) -> RecoveredParse {
        let file_span =
            Span::from_positions(Position::new(), Position::new().advanced_by(self.input));
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

        let mut file = File {
            body: Self::lower_commands_to_stmt_seq(commands, file_span),
            span: file_span,
        };
        self.attach_comments_to_file(&mut file);
        RecoveredParse { file, diagnostics }
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

    /// Parse a command list (commands connected by && or ||)
    fn parse_command_list(&mut self) -> Result<Option<Command>> {
        self.tick()?;
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
                Some(TokenKind::Background) => {
                    (ListOperator::Background(BackgroundOperator::Plain), true)
                }
                Some(TokenKind::BackgroundPipe) => {
                    (ListOperator::Background(BackgroundOperator::Pipe), true)
                }
                Some(TokenKind::BackgroundBang) => {
                    (ListOperator::Background(BackgroundOperator::Bang), true)
                }
                _ => break,
            };
            let operator_span = self.current_span;
            self.advance();

            self.skip_newlines()?;
            if allow_empty_tail && self.current_token.is_none() {
                rest.push(CommandListItem {
                    operator: op,
                    operator_span,
                    command: Command::Simple(SimpleCommand {
                        name: Word::literal(""),
                        args: vec![],
                        redirects: vec![],
                        assignments: vec![],
                        span: self.current_span,
                    }),
                });
                break;
            }

            if let Some(cmd) = self.parse_pipeline()? {
                rest.push(CommandListItem {
                    operator: op,
                    operator_span,
                    command: cmd,
                });
            } else if allow_empty_tail {
                rest.push(CommandListItem {
                    operator: op,
                    operator_span,
                    command: Command::Simple(SimpleCommand {
                        name: Word::literal(""),
                        args: vec![],
                        redirects: vec![],
                        assignments: vec![],
                        span: self.current_span,
                    }),
                });
                break;
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
        let mut operators = Vec::with_capacity(1);

        while self.at_in_set(PIPE_OPERATOR_TOKENS) {
            let op = if self.at(TokenKind::PipeBoth) {
                BinaryOp::PipeAll
            } else {
                BinaryOp::Pipe
            };
            let operator_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            if let Some(cmd) = self.parse_command()? {
                operators.push((op, operator_span));
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
                operators,
                span: start_span.merge(self.current_span),
            })))
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
            self.decode_word_text_preserving_quotes_if_needed(
                &content,
                content_span,
                content_span.start,
                !strip_tabs,
            )
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
        Ok(Some(Command::Compound(Box::new(compound), redirects)))
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
                operands: self.classify_decl_operands(args),
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

        if !self.dialect.features().zsh_repeat_loop && self.looks_like_disabled_repeat_loop()? {
            self.ensure_repeat_loop()?;
        }
        if !self.dialect.features().zsh_foreach_loop && self.looks_like_disabled_foreach_loop()? {
            self.ensure_foreach_loop()?;
        }

        // Check for compound commands and function keyword
        match self.current_keyword() {
            Some(Keyword::If) => return self.parse_compound_with_redirects(|s| s.parse_if()),
            Some(Keyword::For) => return self.parse_compound_with_redirects(|s| s.parse_for()),
            Some(Keyword::Repeat) if self.dialect.features().zsh_repeat_loop => {
                return self.parse_compound_with_redirects(|s| s.parse_repeat());
            }
            Some(Keyword::Foreach) if self.dialect.features().zsh_foreach_loop => {
                return self.parse_compound_with_redirects(|s| s.parse_foreach());
            }
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
                return Ok(Some(Command::Compound(Box::new(compound), redirects)));
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
        let condition_start = self.current_span.start;
        let allow_brace_syntax = self.dialect.features().zsh_brace_if;
        let condition = self.parse_if_condition_until_body_start(allow_brace_syntax)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::lower_commands_to_stmt_seq(condition, condition_span);

        let (mut syntax, then_branch, brace_style) =
            if allow_brace_syntax && self.at(TokenKind::LeftBrace) {
                let (then_branch, left_brace_span, right_brace_span) =
                    self.parse_brace_enclosed_stmt_seq("syntax error: empty then clause")?;
                (
                    IfSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    },
                    then_branch,
                    true,
                )
            } else {
                let then_span = self.current_span;
                self.expect_keyword(Keyword::Then)?;
                self.skip_newlines()?;

                let then_start = self.current_span.start;
                let then_branch = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
                let then_branch_span = Span::from_positions(then_start, self.current_span.start);

                if then_branch.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty then clause"));
                }

                (
                    IfSyntax::ThenFi {
                        then_span,
                        fi_span: Span::new(),
                    },
                    Self::lower_commands_to_stmt_seq(then_branch, then_branch_span),
                    false,
                )
            };

        // Parse elif branches
        let mut elif_branches = Vec::new();
        while self.is_keyword(Keyword::Elif) {
            self.advance(); // consume 'elif'
            self.skip_newlines()?;

            let elif_condition_start = self.current_span.start;
            let elif_condition = self.parse_if_condition_until_body_start(brace_style)?;
            let elif_condition_span =
                Span::from_positions(elif_condition_start, self.current_span.start);
            let elif_condition =
                Self::lower_commands_to_stmt_seq(elif_condition, elif_condition_span);

            let elif_body = if brace_style {
                if !self.at(TokenKind::LeftBrace) {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start elif clause"));
                }
                self.parse_brace_enclosed_stmt_seq("syntax error: empty elif clause")?
                    .0
            } else {
                self.expect_keyword(Keyword::Then)?;
                self.skip_newlines()?;

                let elif_body_start = self.current_span.start;
                let elif_body = self.parse_compound_list_until(IF_BODY_TERMINATORS)?;
                let elif_body_span = Span::from_positions(elif_body_start, self.current_span.start);

                if elif_body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty elif clause"));
                }

                Self::lower_commands_to_stmt_seq(elif_body, elif_body_span)
            };

            elif_branches.push((elif_condition, elif_body));
        }

        // Parse else branch
        let else_branch = if self.is_keyword(Keyword::Else) {
            self.advance(); // consume 'else'
            self.skip_newlines()?;
            if brace_style {
                if !self.at(TokenKind::LeftBrace) {
                    self.pop_depth();
                    return Err(self.error("expected '{' to start else clause"));
                }
                Some(
                    self.parse_brace_enclosed_stmt_seq("syntax error: empty else clause")?
                        .0,
                )
            } else {
                let else_start = self.current_span.start;
                let branch = self.parse_compound_list(Keyword::Fi)?;
                let else_span = Span::from_positions(else_start, self.current_span.start);

                if branch.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty else clause"));
                }

                Some(Self::lower_commands_to_stmt_seq(branch, else_span))
            }
        } else {
            None
        };

        if !brace_style {
            self.expect_keyword(Keyword::Fi)?;
            if let IfSyntax::ThenFi { then_span, .. } = syntax {
                syntax = IfSyntax::ThenFi {
                    then_span,
                    fi_span: self.current_span,
                };
            }
        }

        self.pop_depth();
        Ok(CompoundCommand::If(IfCommand {
            condition,
            then_branch,
            elif_branches,
            else_branch,
            syntax,
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
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty for loop body"));
        }
        let body = Self::lower_commands_to_stmt_seq(body, body_span);

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

    /// Parse a zsh repeat loop.
    fn parse_repeat(&mut self) -> Result<CompoundCommand> {
        self.ensure_repeat_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'repeat'

        let count = match self.current_token_kind {
            Some(kind) if kind.is_word_like() => self.expect_word()?,
            _ => {
                self.pop_depth();
                return Err(self.error("expected loop count in repeat"));
            }
        };

        let (syntax, body, end_span) = match self.current_token_kind {
            Some(TokenKind::LeftBrace) => {
                let (body, left_brace_span, right_brace_span) =
                    self.parse_brace_enclosed_stmt_seq("syntax error: empty repeat loop body")?;
                (
                    RepeatSyntax::Brace {
                        left_brace_span,
                        right_brace_span,
                    },
                    body,
                    right_brace_span,
                )
            }
            Some(TokenKind::Semicolon) => {
                self.advance();
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::lower_commands_to_stmt_seq(body, body_span),
                    done_span,
                )
            }
            Some(TokenKind::Newline) => {
                self.skip_newlines()?;
                if !self.is_keyword(Keyword::Do) {
                    self.pop_depth();
                    return Err(self.error("expected 'do' after repeat count"));
                }
                let do_span = self.current_span;
                self.advance();
                self.skip_newlines()?;

                let body_start = self.current_span.start;
                let body = self.parse_compound_list(Keyword::Done)?;
                let body_span = Span::from_positions(body_start, self.current_span.start);
                if body.is_empty() {
                    self.pop_depth();
                    return Err(self.error("syntax error: empty repeat loop body"));
                }
                if !self.is_keyword(Keyword::Done) {
                    self.pop_depth();
                    return Err(self.error("expected 'done'"));
                }
                let done_span = self.current_span;
                self.advance();
                (
                    RepeatSyntax::DoDone { do_span, done_span },
                    Self::lower_commands_to_stmt_seq(body, body_span),
                    done_span,
                )
            }
            _ => {
                self.pop_depth();
                return Err(self.error("expected ';' or '{' after repeat count"));
            }
        };

        self.pop_depth();
        Ok(CompoundCommand::Repeat(RepeatCommand {
            count,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse a zsh foreach loop.
    fn parse_foreach(&mut self) -> Result<CompoundCommand> {
        self.ensure_foreach_loop()?;
        let start_span = self.current_span;
        self.push_depth()?;
        self.advance(); // consume 'foreach'

        let (variable, variable_span) = match self.current_name_token() {
            Some(pair) => pair,
            _ => {
                self.pop_depth();
                return Err(self.error("expected variable name in foreach"));
            }
        };
        self.advance();

        let (words, body, syntax, end_span) = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance();

            let mut words = Vec::new();
            while !self.at(TokenKind::RightParen) {
                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .current_word()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                        self.advance();
                    }
                    Some(_) | None => {
                        self.pop_depth();
                        return Err(self.error("expected ')' after foreach word list"));
                    }
                }
            }
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }

            let right_paren_span = self.current_span;
            self.advance();
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after foreach word list"));
            }

            let (body, left_brace_span, right_brace_span) =
                self.parse_brace_enclosed_stmt_seq("syntax error: empty foreach loop body")?;
            (
                words,
                body,
                ForeachSyntax::ParenBrace {
                    left_paren_span,
                    right_paren_span,
                    left_brace_span,
                    right_brace_span,
                },
                right_brace_span,
            )
        } else if self.is_keyword(Keyword::In) {
            let in_span = self.current_span;
            self.advance();

            let mut words = Vec::new();
            let saw_separator = loop {
                match self.current_token_kind {
                    _ if self.current_keyword() == Some(Keyword::Do) => break false,
                    Some(kind) if kind.is_word_like() => {
                        let word = self
                            .current_word()
                            .ok_or_else(|| self.error("expected foreach word"))?;
                        words.push(word);
                        self.advance();
                    }
                    Some(TokenKind::Semicolon) => {
                        self.advance();
                        break true;
                    }
                    Some(TokenKind::Newline) => {
                        self.skip_newlines()?;
                        break true;
                    }
                    _ => break false,
                }
            };
            if words.is_empty() {
                self.pop_depth();
                return Err(self.error("expected word list in foreach"));
            }
            if !saw_separator {
                self.pop_depth();
                return Err(self.error("expected ';' or newline before 'do' in foreach"));
            }
            if !self.is_keyword(Keyword::Do) {
                self.pop_depth();
                return Err(self.error("expected 'do' in foreach"));
            }
            let do_span = self.current_span;
            self.advance();
            self.skip_newlines()?;

            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);
            if body.is_empty() {
                self.pop_depth();
                return Err(self.error("syntax error: empty foreach loop body"));
            }
            if !self.is_keyword(Keyword::Done) {
                self.pop_depth();
                return Err(self.error("expected 'done'"));
            }
            let done_span = self.current_span;
            self.advance();
            (
                words,
                Self::lower_commands_to_stmt_seq(body, body_span),
                ForeachSyntax::InDoDone {
                    in_span,
                    do_span,
                    done_span,
                },
                done_span,
            )
        } else {
            self.pop_depth();
            return Err(self.error("expected '(' or 'in' after foreach variable"));
        };

        self.pop_depth();
        Ok(CompoundCommand::Foreach(ForeachCommand {
            variable,
            variable_span,
            words,
            body,
            syntax,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse select loop: select var in list; do body; done
    fn parse_select(&mut self) -> Result<CompoundCommand> {
        self.ensure_select_loop()?;
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
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty select loop body"));
        }
        let body = Self::lower_commands_to_stmt_seq(body, body_span);

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
        self.ensure_arithmetic_for()?;
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
            let span = Self::compound_span(&body);
            (
                Self::lower_commands_to_stmt_seq(
                    vec![Command::Compound(Box::new(body), Vec::new())],
                    span,
                ),
                self.current_span,
            )
        } else {
            // Expect 'do'
            self.expect_keyword(Keyword::Do)?;
            self.skip_newlines()?;

            // Parse body
            let body_start = self.current_span.start;
            let body = self.parse_compound_list(Keyword::Done)?;
            let body_span = Span::from_positions(body_start, self.current_span.start);

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
            (Self::lower_commands_to_stmt_seq(body, body_span), done_span)
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
        let condition_start = self.current_span.start;
        let condition = self.parse_compound_list(Keyword::Do)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::lower_commands_to_stmt_seq(condition, condition_span);

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty while loop body"));
        }
        let body = Self::lower_commands_to_stmt_seq(body, body_span);

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
        let condition_start = self.current_span.start;
        let condition = self.parse_compound_list(Keyword::Do)?;
        let condition_span = Span::from_positions(condition_start, self.current_span.start);
        let condition = Self::lower_commands_to_stmt_seq(condition, condition_span);

        // Expect 'do'
        self.expect_keyword(Keyword::Do)?;
        self.skip_newlines()?;

        // Parse body
        let body_start = self.current_span.start;
        let body = self.parse_compound_list(Keyword::Done)?;
        let body_span = Span::from_positions(body_start, self.current_span.start);

        // Bash requires at least one command in loop body
        if body.is_empty() {
            self.pop_depth();
            return Err(self.error("syntax error: empty until loop body"));
        }
        let body = Self::lower_commands_to_stmt_seq(body, body_span);

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
                    patterns.push(self.pattern_from_word(&word));
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
            let body_start = self.current_span.start;
            let mut commands = Vec::new();
            while !self.is_case_terminator()
                && !self.is_keyword(Keyword::Esac)
                && self.current_token.is_some()
            {
                commands.push(self.parse_command_list_required()?);
                self.skip_newlines()?;
            }

            let terminator = self.parse_case_terminator();
            let body_span = Span::from_positions(body_start, self.current_span.start);
            cases.push(CaseItem {
                patterns,
                body: Self::lower_commands_to_stmt_seq(commands, body_span),
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
        let command = self
            .parse_pipeline()?
            .map(|command| Box::new(Self::lower_non_sequence_command_to_stmt(command)));

        Ok(CompoundCommand::Time(TimeCommand {
            posix_format,
            command,
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
        self.ensure_coproc()?;
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
        let body = Self::lower_non_sequence_command_to_stmt(body);

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

        let body_start = self.current_span.start;
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
        Ok(CompoundCommand::Subshell(Self::lower_commands_to_stmt_seq(
            commands,
            Span::from_positions(body_start, self.current_span.start),
        )))
    }

    /// Parse a brace group
    fn parse_brace_group(&mut self) -> Result<CompoundCommand> {
        self.push_depth()?;
        let (body, left_brace_span, right_brace_span) =
            self.parse_brace_enclosed_stmt_seq("syntax error: empty brace group")?;

        let compound = if self.dialect.features().zsh_always && self.is_keyword(Keyword::Always) {
            self.advance();
            self.skip_newlines()?;
            if !self.at(TokenKind::LeftBrace) {
                self.pop_depth();
                return Err(self.error("expected '{' after always"));
            }
            let (always_body, _, always_right_brace_span) =
                self.parse_brace_enclosed_stmt_seq("syntax error: empty always clause")?;
            CompoundCommand::Always(AlwaysCommand {
                body,
                always_body,
                span: left_brace_span.merge(always_right_brace_span),
            })
        } else {
            let _ = right_brace_span;
            CompoundCommand::BraceGroup(body)
        };

        self.pop_depth();
        Ok(compound)
    }

    fn parse_brace_enclosed_stmt_seq(
        &mut self,
        empty_error: &str,
    ) -> Result<(StmtSeq, Span, Span)> {
        let left_brace_span = self.current_span;
        self.advance();
        self.skip_newlines()?;

        let body_start = self.current_span.start;
        let mut commands = Vec::new();
        while !matches!(self.current_token_kind, Some(TokenKind::RightBrace) | None) {
            self.skip_newlines()?;
            if self.at(TokenKind::RightBrace) {
                break;
            }
            commands.push(self.parse_command_list_required()?);
        }

        if !self.at(TokenKind::RightBrace) {
            return Err(Error::parse(
                "expected '}' to close brace group".to_string(),
            ));
        }

        if commands.is_empty() {
            return Err(self.error(empty_error));
        }

        let right_brace_span = self.current_span;
        self.advance();
        Ok((
            Self::lower_commands_to_stmt_seq(
                commands,
                Span::from_positions(body_start, right_brace_span.start),
            ),
            left_brace_span,
            right_brace_span,
        ))
    }

    fn parse_if_condition_until_body_start(
        &mut self,
        allow_brace_body: bool,
    ) -> Result<Vec<Command>> {
        let mut commands = Vec::with_capacity(2);

        loop {
            self.skip_newlines()?;

            if self.is_keyword(Keyword::Then) || (allow_brace_body && self.at(TokenKind::LeftBrace))
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

    /// Parse arithmetic command ((expression))
    /// Parse [[ conditional expression ]]
    fn parse_conditional(&mut self) -> Result<CompoundCommand> {
        self.ensure_double_bracket()?;
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
                if matches!(
                    op,
                    ConditionalUnaryOp::VariableSet | ConditionalUnaryOp::ReferenceVariable
                ) {
                    let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                    self.conditional_var_ref_expr(word)
                } else {
                    let word = self.parse_conditional_operand_word()?;
                    ConditionalExpr::Word(word)
                }
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
            | ConditionalBinaryOp::PatternNe => {
                let word = self.collect_conditional_context_word(stop_at_right_paren)?;
                ConditionalExpr::Pattern(self.pattern_from_word(&word))
            }
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

    fn conditional_var_ref_expr(&self, word: Word) -> ConditionalExpr {
        self.parse_var_ref_from_word(&word, SubscriptInterpretation::Contextual)
            .map(Box::new)
            .map(ConditionalExpr::VarRef)
            .unwrap_or(ConditionalExpr::Word(word))
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
                let gap_text = gap_span.slice(self.input);
                if Self::source_text_needs_quote_preserving_decode(gap_text) {
                    let gap_word = self.decode_word_text_preserving_quotes_if_needed(
                        gap_text,
                        gap_span,
                        gap_span.start,
                        true,
                    );
                    parts.extend(gap_word.parts);
                } else {
                    parts.push(WordPartNode::new(
                        WordPart::Literal(LiteralText::source()),
                        gap_span,
                    ));
                }
                composite = true;
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

        Ok(self.word_with_parts(parts, Span::from_positions(start, end)))
    }

    fn parse_arithmetic_command(&mut self) -> Result<CompoundCommand> {
        self.ensure_arithmetic_command()?;
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

    fn parse_function_body_command(&mut self, allow_bare_compound: bool) -> Result<Command> {
        let compound = match self.current_keyword() {
            Some(Keyword::If) if allow_bare_compound => self.parse_if()?,
            Some(Keyword::For) if allow_bare_compound => self.parse_for()?,
            Some(Keyword::Repeat)
                if allow_bare_compound && self.dialect.features().zsh_repeat_loop =>
            {
                self.parse_repeat()?
            }
            Some(Keyword::Foreach)
                if allow_bare_compound && self.dialect.features().zsh_foreach_loop =>
            {
                self.parse_foreach()?
            }
            Some(Keyword::While) if allow_bare_compound => self.parse_while()?,
            Some(Keyword::Until) if allow_bare_compound => self.parse_until()?,
            Some(Keyword::Case) if allow_bare_compound => self.parse_case()?,
            Some(Keyword::Select) if allow_bare_compound => self.parse_select()?,
            _ => match self.current_token_kind {
                Some(TokenKind::LeftBrace) => self.parse_brace_group()?,
                Some(TokenKind::LeftParen) => self.parse_subshell()?,
                Some(TokenKind::DoubleLeftBracket) if allow_bare_compound => {
                    self.parse_conditional()?
                }
                Some(TokenKind::DoubleLeftParen) if allow_bare_compound => {
                    if self.looks_like_command_style_double_paren() {
                        self.split_current_double_left_paren();
                        self.parse_subshell()?
                    } else {
                        let mut arithmetic_probe = self.clone();
                        if let Ok(compound) = arithmetic_probe.parse_arithmetic_command() {
                            *self = arithmetic_probe;
                            compound
                        } else {
                            self.split_current_double_left_paren();
                            self.parse_subshell()?
                        }
                    }
                }
                _ => {
                    return Err(Error::parse(
                        "expected compound command for function body".to_string(),
                    ));
                }
            },
        };
        let redirects = self.parse_trailing_redirects();
        Ok(Command::Compound(Box::new(compound), redirects))
    }

    /// Parse function definition with 'function' keyword: function name { body }
    fn parse_function_keyword(&mut self) -> Result<Command> {
        self.ensure_function_keyword()?;
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
        let saw_newline_after_name = self.skip_newlines_with_flag()?;

        // Optional () after name
        let mut name_parens_span = None;
        let allow_bare_compound = if self.at(TokenKind::LeftParen) {
            let left_paren_span = self.current_span;
            self.advance(); // consume '('
            if !self.at(TokenKind::RightParen) {
                return Err(Error::parse(
                    "expected ')' in function definition".to_string(),
                ));
            }
            let right_paren_span = self.current_span;
            self.advance(); // consume ')'
            name_parens_span = Some(left_paren_span.merge(right_paren_span));
            self.skip_newlines_with_flag()?
        } else {
            saw_newline_after_name
        };

        let body = self.parse_function_body_command(allow_bare_compound)?;
        let body = Self::lower_non_sequence_command_to_stmt(body);

        Ok(Command::Function(FunctionDef {
            name,
            name_span,
            surface: FunctionSurface {
                function_keyword_span: Some(start_span),
                name_parens_span,
            },
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
        let left_paren_span = self.current_span;
        self.advance(); // consume '('

        if !self.at(TokenKind::RightParen) {
            return Err(self.error("expected ')' in function definition"));
        }
        let right_paren_span = self.current_span;
        self.advance(); // consume ')'
        self.skip_newlines()?;

        let body = self.parse_function_body_command(true)?;
        let body = Self::lower_non_sequence_command_to_stmt(body);

        Ok(Command::Function(FunctionDef {
            name,
            name_span,
            surface: FunctionSurface {
                function_keyword_span: None,
                name_parens_span: Some(left_paren_span.merge(right_paren_span)),
            },
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

    /// Check if a word is an assignment (NAME=value, NAME+=value, or NAME[index]=value)
    /// Returns (name, optional_index, value, is_append)
    fn is_assignment(word: &str) -> Option<(&str, Option<&str>, &str, bool)> {
        if !word.contains('=') {
            return None;
        }

        let mut ident_end = 0;
        let mut chars = word.char_indices();
        let (_, first) = chars.next()?;
        if !first.is_ascii_alphabetic() && first != '_' {
            return None;
        }
        ident_end += first.len_utf8();
        for (index, ch) in chars {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ident_end = index + ch.len_utf8();
            } else {
                break;
            }
        }

        let name = &word[..ident_end];
        let mut cursor = ident_end;
        let mut index = None;

        if word[cursor..].starts_with('[') {
            let mut close_index = None;
            let mut bracket_depth = 0_i32;
            let mut brace_depth = 0_i32;
            let mut paren_depth = 0_i32;
            let mut in_single = false;
            let mut in_double = false;
            let mut escaped = false;

            for (relative, ch) in word[cursor + 1..].char_indices() {
                let absolute = cursor + 1 + relative;
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
                        if bracket_depth == 0 && brace_depth == 0 && paren_depth == 0 {
                            close_index = Some(absolute);
                            break;
                        }
                        bracket_depth -= 1;
                    }
                    '{' if !in_single && !in_double => brace_depth += 1,
                    '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                    '(' if !in_single && !in_double => paren_depth += 1,
                    ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                    _ => {}
                }
            }

            let close_index = close_index?;
            index = Some(&word[cursor + 1..close_index]);
            cursor = close_index + 1;
        }

        let (is_append, value) = if word[cursor..].starts_with("+=") {
            (true, &word[cursor + 2..])
        } else if word[cursor..].starts_with('=') {
            (false, &word[cursor + 1..])
        } else {
            return None;
        };

        Some((name, index, value, is_append))
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
        let assignment = self.parse_assignment_from_text(
            &text,
            span,
            None,
            SubscriptInterpretation::Contextual,
        )?;

        while self.current_token.is_some() && self.current_span.start.offset < end.offset {
            self.advance();
        }

        Some(assignment)
    }

    fn infer_array_expr_kind(
        explicit_kind: Option<ArrayKind>,
        elements: &[ArrayElem],
    ) -> ArrayKind {
        explicit_kind.unwrap_or_else(|| {
            if elements
                .iter()
                .any(|element| !matches!(element, ArrayElem::Sequential(_)))
            {
                ArrayKind::Contextual
            } else {
                ArrayKind::Indexed
            }
        })
    }

    fn subscript_interpretation_from_array_kind(
        explicit_kind: Option<ArrayKind>,
    ) -> SubscriptInterpretation {
        match explicit_kind {
            Some(ArrayKind::Indexed) => SubscriptInterpretation::Indexed,
            Some(ArrayKind::Associative) => SubscriptInterpretation::Associative,
            _ => SubscriptInterpretation::Contextual,
        }
    }

    fn word_from_raw_text(&mut self, raw: &str, span: Span) -> Word {
        if raw.is_empty() {
            return Word::literal_with_span("", span);
        }

        self.parse_word_with_context(raw, span, span.start, self.source_matches(span, raw))
    }

    fn split_compound_array_elements(&self, inner: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut start: Option<usize> = None;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut paren_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for (index, ch) in inner.char_indices() {
            if start.is_none() {
                if ch.is_whitespace() {
                    continue;
                }
                start = Some(index);
            }

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
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                ch if ch.is_whitespace()
                    && !in_single
                    && !in_double
                    && bracket_depth == 0
                    && brace_depth == 0
                    && paren_depth == 0 =>
                {
                    if let Some(start) = start.take() {
                        ranges.push((start, index));
                    }
                }
                _ => {}
            }
        }

        if let Some(start) = start {
            ranges.push((start, inner.len()));
        }

        ranges
    }

    fn split_compound_array_key_value<'b>(
        &self,
        raw: &'b str,
    ) -> Option<(&'b str, &'b str, bool, usize, usize)> {
        if !raw.starts_with('[') {
            return None;
        }

        let mut close_index = None;
        let mut bracket_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut paren_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        for (index, ch) in raw.char_indices().skip(1) {
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
                    if bracket_depth == 0 && brace_depth == 0 && paren_depth == 0 {
                        close_index = Some(index);
                        break;
                    }
                    bracket_depth -= 1;
                }
                '{' if !in_single && !in_double => brace_depth += 1,
                '}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
                '(' if !in_single && !in_double => paren_depth += 1,
                ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
                _ => {}
            }
        }

        let close_index = close_index?;
        let tail = &raw[close_index + 1..];
        let (append, value_offset) = if tail.starts_with("+=") {
            (true, 2)
        } else if tail.starts_with('=') {
            (false, 1)
        } else {
            return None;
        };

        Some((
            &raw[1..close_index],
            &tail[value_offset..],
            append,
            close_index,
            value_offset,
        ))
    }

    fn parse_compound_array_element(
        &mut self,
        raw: &str,
        span: Span,
        interpretation: SubscriptInterpretation,
    ) -> ArrayElem {
        if let Some((key_raw, value_raw, append, close_index, value_offset)) =
            self.split_compound_array_key_value(raw)
        {
            let key_start = span.start.advanced_by("[");
            let key_end = span.start.advanced_by(&raw[..close_index]);
            let key = self.subscript_from_text(
                key_raw,
                Span::from_positions(key_start, key_end),
                interpretation,
            );
            let value_start = span
                .start
                .advanced_by(&raw[..close_index + 1 + value_offset]);
            let value_span = Span::from_positions(value_start, span.end);
            let value = self.word_from_raw_text(value_raw, value_span);
            return if append {
                ArrayElem::KeyedAppend { key, value }
            } else {
                ArrayElem::Keyed { key, value }
            };
        }

        ArrayElem::Sequential(self.word_from_raw_text(raw, span))
    }

    fn parse_array_expr_from_text(
        &mut self,
        inner: &str,
        base: Position,
        explicit_kind: Option<ArrayKind>,
    ) -> ArrayExpr {
        let interpretation = Self::subscript_interpretation_from_array_kind(explicit_kind);
        let mut cursor = 0;
        let mut cursor_pos = base;
        let elements = self
            .split_compound_array_elements(inner)
            .into_iter()
            .map(|(start, end)| {
                if start > cursor {
                    cursor_pos = cursor_pos.advanced_by(&inner[cursor..start]);
                    cursor = start;
                }

                let start_pos = cursor_pos;
                let end_pos = start_pos.advanced_by(&inner[start..end]);
                let span = Span::from_positions(start_pos, end_pos);
                cursor = end;
                cursor_pos = end_pos;

                self.parse_compound_array_element(&inner[start..end], span, interpretation)
            })
            .collect::<Vec<_>>();

        let span = if let (Some(first), Some(last)) = (elements.first(), elements.last()) {
            first.span().merge(last.span())
        } else {
            Span::from_positions(base, base)
        };

        ArrayExpr {
            kind: Self::infer_array_expr_kind(explicit_kind, &elements),
            elements,
            span,
        }
    }

    /// Parse a simple command with redirections.
    fn collect_compound_array(
        &mut self,
        open_paren_span: Span,
        explicit_kind: Option<ArrayKind>,
    ) -> (ArrayExpr, Span) {
        let inner_start = open_paren_span.end;
        let mut closing_span = Span::new();
        let mut fallback = String::new();

        loop {
            match self.current_token_kind {
                Some(TokenKind::RightParen) => {
                    closing_span = self.current_span;
                    self.advance();
                    break;
                }
                Some(kind) if kind.is_word_like() => {
                    if !fallback.is_empty() {
                        fallback.push(' ');
                    }
                    if let Some(word) = self.current_source_like_word_text() {
                        fallback.push_str(&word);
                    }
                    self.advance();
                }
                None => break,
                _ => self.advance(),
            }
        }

        let inner = if closing_span != Span::new()
            && inner_start.offset <= closing_span.start.offset
            && closing_span.start.offset <= self.input.len()
        {
            self.input[inner_start.offset..closing_span.start.offset].to_string()
        } else {
            fallback
        };

        let mut array = self.parse_array_expr_from_text(&inner, inner_start, explicit_kind);
        array.span = if closing_span == Span::new() {
            open_paren_span
        } else {
            open_paren_span.merge(closing_span)
        };
        (array, closing_span)
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

    fn split_word_at(&self, word: Word, start: Position) -> Word {
        let value_span = Span::from_positions(start, word.span.end);
        let mut parts = Vec::new();

        for part in word.parts {
            if let Some((kind, span)) = self.trim_word_part_prefix(part.kind, part.span, start) {
                parts.push(WordPartNode::new(kind, span));
            }
        }

        self.word_with_parts(parts, value_span)
    }

    fn word_syntax_is_source_backed(&self, word: &Word) -> bool {
        word.span.end.offset <= self.input.len()
            && word
                .parts
                .first()
                .is_none_or(|part| part.span.start == word.span.start)
            && word
                .parts
                .last()
                .is_none_or(|part| part.span.end == word.span.end)
            && word
                .parts
                .iter()
                .all(|part| self.word_part_syntax_is_source_backed(&part.kind, part.span))
    }

    fn word_part_syntax_is_source_backed(&self, part: &WordPart, span: Span) -> bool {
        span.end.offset <= self.input.len()
            && match part {
                WordPart::Literal(text) => text.is_source_backed(),
                WordPart::ZshQualifiedGlob(glob) => {
                    glob.pattern.is_source_backed()
                        && self.zsh_glob_qualifier_group_is_source_backed(&glob.qualifiers)
                }
                WordPart::SingleQuoted { value, .. } => value.is_source_backed(),
                WordPart::DoubleQuoted { parts, .. } => parts
                    .iter()
                    .all(|part| self.word_part_syntax_is_source_backed(&part.kind, part.span)),
                WordPart::Variable(_)
                | WordPart::CommandSubstitution { .. }
                | WordPart::ProcessSubstitution { .. }
                | WordPart::PrefixMatch { .. } => true,
                WordPart::ArithmeticExpansion { expression, .. } => expression.is_source_backed(),
                WordPart::Parameter(parameter) => parameter.raw_body.is_source_backed(),
                WordPart::ParameterExpansion {
                    reference,
                    operator,
                    operand,
                    ..
                } => {
                    reference.is_source_backed()
                        && self.parameter_operator_is_source_backed(operator)
                        && operand.as_ref().is_none_or(SourceText::is_source_backed)
                }
                WordPart::Length(reference)
                | WordPart::ArrayAccess(reference)
                | WordPart::ArrayLength(reference)
                | WordPart::ArrayIndices(reference)
                | WordPart::Transformation { reference, .. } => reference.is_source_backed(),
                WordPart::Substring {
                    reference,
                    offset,
                    length,
                    ..
                }
                | WordPart::ArraySlice {
                    reference,
                    offset,
                    length,
                    ..
                } => {
                    reference.is_source_backed()
                        && offset.is_source_backed()
                        && length.as_ref().is_none_or(SourceText::is_source_backed)
                }
                WordPart::IndirectExpansion {
                    operator, operand, ..
                } => {
                    operator.is_none() && operand.as_ref().is_none_or(SourceText::is_source_backed)
                }
            }
    }

    fn parameter_operator_is_source_backed(&self, operator: &ParameterOp) -> bool {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => pattern.is_source_backed(),
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => pattern.is_source_backed() && replacement.is_source_backed(),
            _ => true,
        }
    }

    fn zsh_glob_qualifier_group_is_source_backed(&self, group: &ZshGlobQualifierGroup) -> bool {
        group
            .fragments
            .iter()
            .all(Self::zsh_glob_qualifier_is_source_backed)
    }

    fn zsh_glob_qualifier_is_source_backed(fragment: &ZshGlobQualifier) -> bool {
        match fragment {
            ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => true,
            ZshGlobQualifier::LetterSequence { text, .. } => text.is_source_backed(),
            ZshGlobQualifier::NumericArgument { start, end, .. } => {
                start.is_source_backed() && end.as_ref().is_none_or(SourceText::is_source_backed)
            }
        }
    }

    fn word_part_syntax_text<'b>(&'b self, part: &'b WordPartNode) -> Cow<'b, str> {
        if self.word_part_syntax_is_source_backed(&part.kind, part.span) {
            Cow::Borrowed(part.span.slice(self.input))
        } else {
            let mut syntax = String::new();
            self.push_word_part_syntax(&mut syntax, &part.kind, part.span);
            Cow::Owned(syntax)
        }
    }

    fn compound_array_inner_text<'b>(&'b self, word: &'b Word) -> Option<(Cow<'b, str>, Position)> {
        let first = word.parts.first()?;
        let last = word.parts.last()?;
        let first_syntax = self.word_part_syntax_text(first);
        let last_syntax = self.word_part_syntax_text(last);

        if !first_syntax.starts_with('(') || !last_syntax.ends_with(')') {
            return None;
        }

        let inner_start = word.span.start.advanced_by("(");
        if self.word_syntax_is_source_backed(word) {
            let syntax = word.span.slice(self.input);
            return Some((
                Cow::Borrowed(&syntax[1..syntax.len().saturating_sub(1)]),
                inner_start,
            ));
        }

        let mut inner = String::new();
        for (index, part) in word.parts.iter().enumerate() {
            let syntax = self.word_part_syntax_text(part);
            let start = if index == 0 { 1 } else { 0 };
            let end = syntax.len() - usize::from(index + 1 == word.parts.len());
            if start < end {
                inner.push_str(&syntax[start..end]);
            }
        }

        Some((Cow::Owned(inner), inner_start))
    }

    fn push_word_part_syntax(&self, out: &mut String, part: &WordPart, span: Span) {
        if self.word_part_syntax_is_source_backed(part, span) {
            out.push_str(span.slice(self.input));
            return;
        }

        match part {
            WordPart::Literal(text) => out.push_str(text.as_str(self.input, span)),
            WordPart::ZshQualifiedGlob(glob) => {
                self.push_pattern_syntax(out, &glob.pattern);
                self.push_zsh_glob_qualifier_group_syntax(out, &glob.qualifiers);
            }
            WordPart::SingleQuoted { value, dollar } => {
                if *dollar {
                    out.push('$');
                }
                out.push('\'');
                out.push_str(value.slice(self.input));
                out.push('\'');
            }
            WordPart::DoubleQuoted { parts, dollar } => {
                if *dollar {
                    out.push('$');
                }
                out.push('"');
                for part in parts {
                    self.push_word_part_syntax(out, &part.kind, part.span);
                }
                out.push('"');
            }
            WordPart::Variable(name) => {
                out.push('$');
                out.push_str(name.as_str());
            }
            WordPart::CommandSubstitution { syntax, .. } => match syntax {
                CommandSubstitutionSyntax::DollarParen => out.push_str("$()"),
                CommandSubstitutionSyntax::Backtick => out.push_str("``"),
            },
            WordPart::ArithmeticExpansion {
                expression, syntax, ..
            } => match syntax {
                ArithmeticExpansionSyntax::DollarParenParen => {
                    out.push_str("$((");
                    out.push_str(expression.slice(self.input));
                    out.push_str("))");
                }
                ArithmeticExpansionSyntax::LegacyBracket => {
                    out.push_str("$[");
                    out.push_str(expression.slice(self.input));
                    out.push(']');
                }
            },
            WordPart::Parameter(parameter) => {
                out.push_str("${");
                out.push_str(parameter.raw_body.slice(self.input));
                out.push('}');
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                colon_variant,
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                self.push_parameter_operator_syntax(
                    out,
                    operator,
                    operand.as_ref(),
                    *colon_variant,
                );
                out.push('}');
            }
            WordPart::Length(reference) => {
                out.push_str("${#");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayAccess(reference) => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayLength(reference) => {
                out.push_str("${#");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::ArrayIndices(reference) => {
                out.push_str("${!");
                self.push_var_ref_syntax(out, reference);
                out.push('}');
            }
            WordPart::Substring {
                reference,
                offset,
                length,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                length,
                ..
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push(':');
                out.push_str(offset.slice(self.input));
                if let Some(length) = length {
                    out.push(':');
                    out.push_str(length.slice(self.input));
                }
                out.push('}');
            }
            WordPart::IndirectExpansion {
                name,
                operator,
                operand,
                colon_variant,
            } => {
                out.push_str("${!");
                out.push_str(name.as_str());
                if let Some(operator) = operator {
                    self.push_parameter_operator_syntax(
                        out,
                        operator,
                        operand.as_ref(),
                        *colon_variant,
                    );
                }
                out.push('}');
            }
            WordPart::PrefixMatch { prefix, kind } => {
                out.push_str("${!");
                out.push_str(prefix.as_str());
                out.push(kind.as_char());
                out.push('}');
            }
            WordPart::ProcessSubstitution { is_input, .. } => {
                out.push(if *is_input { '<' } else { '>' });
                out.push_str("()");
            }
            WordPart::Transformation {
                reference,
                operator,
            } => {
                out.push_str("${");
                self.push_var_ref_syntax(out, reference);
                out.push('@');
                out.push(*operator);
                out.push('}');
            }
        }
    }

    fn push_zsh_glob_qualifier_group_syntax(
        &self,
        out: &mut String,
        group: &ZshGlobQualifierGroup,
    ) {
        out.push('(');
        for fragment in &group.fragments {
            match fragment {
                ZshGlobQualifier::Negation { .. } => out.push('^'),
                ZshGlobQualifier::Flag { name, .. } => out.push(*name),
                ZshGlobQualifier::LetterSequence { text, .. } => {
                    out.push_str(text.slice(self.input));
                }
                ZshGlobQualifier::NumericArgument { start, end, .. } => {
                    out.push('[');
                    out.push_str(start.slice(self.input));
                    if let Some(end) = end {
                        out.push(',');
                        out.push_str(end.slice(self.input));
                    }
                    out.push(']');
                }
            }
        }
        out.push(')');
    }

    fn push_var_ref_syntax(&self, out: &mut String, reference: &VarRef) {
        out.push_str(reference.name.as_str());
        if let Some(subscript) = &reference.subscript {
            out.push('[');
            out.push_str(subscript.syntax_text(self.input));
            out.push(']');
        }
    }

    fn push_parameter_operator_syntax(
        &self,
        out: &mut String,
        operator: &ParameterOp,
        operand: Option<&SourceText>,
        colon_variant: bool,
    ) {
        let colon = if colon_variant { ":" } else { "" };
        match operator {
            ParameterOp::UseDefault => {
                out.push_str(colon);
                out.push('-');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::AssignDefault => {
                out.push_str(colon);
                out.push('=');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::UseReplacement => {
                out.push_str(colon);
                out.push('+');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::Error => {
                out.push_str(colon);
                out.push('?');
                if let Some(operand) = operand {
                    out.push_str(operand.slice(self.input));
                }
            }
            ParameterOp::RemovePrefixShort { pattern } => {
                out.push('#');
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemovePrefixLong { pattern } => {
                out.push_str("##");
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemoveSuffixShort { pattern } => {
                out.push('%');
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::RemoveSuffixLong { pattern } => {
                out.push_str("%%");
                self.push_pattern_syntax(out, pattern);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            } => {
                out.push('/');
                self.push_pattern_syntax(out, pattern);
                out.push('/');
                out.push_str(replacement.slice(self.input));
            }
            ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => {
                out.push_str("//");
                self.push_pattern_syntax(out, pattern);
                out.push('/');
                out.push_str(replacement.slice(self.input));
            }
            ParameterOp::UpperFirst => out.push('^'),
            ParameterOp::UpperAll => out.push_str("^^"),
            ParameterOp::LowerFirst => out.push(','),
            ParameterOp::LowerAll => out.push_str(",,"),
        }
    }

    fn push_pattern_syntax(&self, out: &mut String, pattern: &Pattern) {
        if pattern.is_source_backed() && pattern.span.end.offset <= self.input.len() {
            out.push_str(pattern.span.slice(self.input));
            return;
        }

        for part in &pattern.parts {
            self.push_pattern_part_syntax(out, &part.kind, part.span);
        }
    }

    fn push_pattern_part_syntax(&self, out: &mut String, part: &PatternPart, span: Span) {
        match part {
            PatternPart::Literal(text) => out.push_str(text.as_str(self.input, span)),
            PatternPart::AnyString => out.push('*'),
            PatternPart::AnyChar => out.push('?'),
            PatternPart::CharClass(text) => out.push_str(text.slice(self.input)),
            PatternPart::Group { kind, patterns } => {
                out.push(kind.prefix());
                out.push('(');
                for (index, pattern) in patterns.iter().enumerate() {
                    if index > 0 {
                        out.push('|');
                    }
                    self.push_pattern_syntax(out, pattern);
                }
                out.push(')');
            }
            PatternPart::Word(word) => {
                for part in &word.parts {
                    self.push_word_part_syntax(out, &part.kind, part.span);
                }
            }
        }
    }

    fn parse_assignment_from_word(
        &mut self,
        word: Word,
        explicit_array_kind: Option<ArrayKind>,
        subscript_interpretation: SubscriptInterpretation,
    ) -> Option<Assignment> {
        let assignment_span = word.span;
        let ParsedWordTarget {
            name,
            name_span,
            subscript,
            boundary,
        } = self.parse_word_target(&word, subscript_interpretation, true)?;
        let WordTargetBoundary::Assignment {
            append,
            value_start,
        } = boundary
        else {
            return None;
        };
        let target = self.var_ref(name, name_span, subscript, assignment_span);
        let value_word = self.split_word_at(word, value_start);

        let value = if value_word.parts.is_empty() {
            AssignmentValue::Scalar(Word::literal_with_span(
                "",
                Span::from_positions(value_start, assignment_span.end),
            ))
        } else if let Some((inner, inner_start)) = self
            .compound_array_inner_text(&value_word)
            .map(|(inner, inner_start)| (inner.into_owned(), inner_start))
        {
            AssignmentValue::Compound(self.parse_array_expr_from_text(
                &inner,
                inner_start,
                explicit_array_kind,
            ))
        } else {
            AssignmentValue::Scalar(value_word)
        };

        Some(Assignment {
            target,
            value,
            append,
            span: assignment_span,
        })
    }

    fn parse_assignment_from_text(
        &mut self,
        w: &str,
        assignment_span: Span,
        explicit_array_kind: Option<ArrayKind>,
        subscript_interpretation: SubscriptInterpretation,
    ) -> Option<Assignment> {
        let source_backed = assignment_span.end.offset <= self.input.len()
            && assignment_span.slice(self.input) == w;
        let word = self.decode_word_text_preserving_quotes_if_needed(
            w,
            assignment_span,
            assignment_span.start,
            source_backed,
        );
        self.parse_assignment_from_word(word, explicit_array_kind, subscript_interpretation)
    }

    fn parse_word_target(
        &self,
        word: &Word,
        interpretation: SubscriptInterpretation,
        allow_assignment: bool,
    ) -> Option<ParsedWordTarget> {
        let first_part = word.parts.first()?;
        let WordPart::Literal(first_literal) = &first_part.kind else {
            return None;
        };
        let first_text = first_literal.as_str(self.input, first_part.span);
        let mut name_end = 0;
        for (offset, ch) in first_text.char_indices() {
            if (offset == 0 && (ch.is_ascii_alphabetic() || ch == '_'))
                || (offset > 0 && (ch.is_ascii_alphanumeric() || ch == '_'))
            {
                name_end = offset + ch.len_utf8();
            } else {
                break;
            }
        }
        if name_end == 0 {
            return None;
        }

        let name_text = &first_text[..name_end];
        let name = Name::from(name_text);
        let name_span =
            Span::from_positions(word.span.start, word.span.start.advanced_by(name_text));
        let mut after_name = name_end;
        let mut in_subscript = false;
        let mut bracket_depth = 0usize;
        let mut subscript_start = None;
        let mut subscript_end = None;
        let mut subscript_text = String::new();

        for (part_index, part) in word.parts.iter().enumerate() {
            match &part.kind {
                WordPart::Literal(text) => {
                    let text = text.as_str(self.input, part.span);
                    let mut offset = if part_index == 0 { after_name } else { 0 };
                    while offset < text.len() {
                        let ch = text[offset..].chars().next()?;
                        let next_offset = offset + ch.len_utf8();
                        let ch_start = part.span.start.advanced_by(&text[..offset]);
                        let ch_end = part.span.start.advanced_by(&text[..next_offset]);

                        if in_subscript {
                            match ch {
                                '[' => {
                                    bracket_depth += 1;
                                    subscript_text.push(ch);
                                }
                                ']' if bracket_depth == 0 => {
                                    subscript_end = Some(ch_start);
                                    in_subscript = false;
                                }
                                ']' => {
                                    bracket_depth -= 1;
                                    subscript_text.push(ch);
                                }
                                _ => subscript_text.push(ch),
                            }
                            offset = next_offset;
                            continue;
                        }

                        match ch {
                            '[' if subscript_start.is_none() => {
                                subscript_start = Some(ch_end);
                                in_subscript = true;
                            }
                            '=' if allow_assignment => {
                                return Some(ParsedWordTarget {
                                    name,
                                    name_span,
                                    subscript: self.build_target_subscript(
                                        subscript_text,
                                        subscript_start.zip(subscript_end),
                                        interpretation,
                                    )?,
                                    boundary: WordTargetBoundary::Assignment {
                                        append: false,
                                        value_start: ch_end,
                                    },
                                });
                            }
                            '+' if allow_assignment && text[next_offset..].starts_with('=') => {
                                return Some(ParsedWordTarget {
                                    name,
                                    name_span,
                                    subscript: self.build_target_subscript(
                                        subscript_text,
                                        subscript_start.zip(subscript_end),
                                        interpretation,
                                    )?,
                                    boundary: WordTargetBoundary::Assignment {
                                        append: true,
                                        value_start: part
                                            .span
                                            .start
                                            .advanced_by(&text[..next_offset + '='.len_utf8()]),
                                    },
                                });
                            }
                            _ => return None,
                        }
                        offset = next_offset;
                    }
                }
                _ => {
                    if !in_subscript {
                        return None;
                    }
                    subscript_text.push_str(self.word_part_syntax_text(part).as_ref());
                }
            }
            after_name = 0;
        }

        if in_subscript {
            return None;
        }

        Some(ParsedWordTarget {
            name,
            name_span,
            subscript: self.build_target_subscript(
                subscript_text,
                subscript_start.zip(subscript_end),
                interpretation,
            )?,
            boundary: WordTargetBoundary::EndOfWord,
        })
    }

    fn build_target_subscript(
        &self,
        text: String,
        span: Option<(Position, Position)>,
        interpretation: SubscriptInterpretation,
    ) -> Option<Option<Subscript>> {
        let Some((start, end)) = span else {
            return Some(None);
        };
        let subscript_span = Span::from_positions(start, end);
        let (text, raw) = self.subscript_source_text(&text, subscript_span);
        Some(Some(self.subscript_from_source_text(
            text,
            raw,
            interpretation,
        )))
    }

    fn parse_var_ref_from_word(
        &self,
        word: &Word,
        interpretation: SubscriptInterpretation,
    ) -> Option<VarRef> {
        let ParsedWordTarget {
            name,
            name_span,
            subscript,
            boundary,
        } = self.parse_word_target(word, interpretation, false)?;
        matches!(boundary, WordTargetBoundary::EndOfWord)
            .then(|| self.var_ref(name, name_span, subscript, word.span))
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

    fn is_literal_flag_text(text: &str) -> bool {
        if text.contains('=') {
            return false;
        }

        let Some(first) = text.chars().next() else {
            return false;
        };
        if first != '-' && first != '+' {
            return false;
        }

        true
    }

    fn classify_decl_operand(
        &mut self,
        word: Word,
        explicit_array_kind: Option<ArrayKind>,
    ) -> DeclOperand {
        let interpretation = Self::subscript_interpretation_from_array_kind(explicit_array_kind);

        if self
            .single_literal_word_text(&word)
            .is_some_and(Self::is_literal_flag_text)
        {
            return DeclOperand::Flag(word);
        }

        if let Some(assignment) =
            self.parse_assignment_from_word(word.clone(), explicit_array_kind, interpretation)
        {
            return DeclOperand::Assignment(assignment);
        }

        if let Some(name) = self.parse_var_ref_from_word(&word, interpretation) {
            return DeclOperand::Name(name);
        }

        DeclOperand::Dynamic(word)
    }

    fn explicit_array_kind_from_flag_text(text: &str) -> Option<ArrayKind> {
        if !text.starts_with('-') {
            return None;
        }

        let mut explicit_kind = None;
        for flag in text.chars().skip(1) {
            match flag {
                'a' => explicit_kind = Some(ArrayKind::Indexed),
                'A' => explicit_kind = Some(ArrayKind::Associative),
                _ => {}
            }
        }
        explicit_kind
    }

    fn classify_decl_operands(&mut self, words: Vec<Word>) -> Vec<DeclOperand> {
        let mut explicit_array_kind = None;
        let mut operands = Vec::with_capacity(words.len());

        for word in words {
            if let Some(text) = self.single_literal_word_text(&word)
                && Self::is_literal_flag_text(text)
            {
                explicit_array_kind =
                    Self::explicit_array_kind_from_flag_text(text).or(explicit_array_kind);
                operands.push(DeclOperand::Flag(word));
                continue;
            }

            operands.push(self.classify_decl_operand(word, explicit_array_kind));
        }

        operands
    }

    /// Parse the value side of an assignment (`VAR=value`).
    /// Returns `Some((Assignment, needs_advance))` if the current word is an assignment.
    /// The bool indicates whether the caller must call `self.advance()` afterward.
    fn try_parse_assignment(&mut self, raw: &str) -> Option<(Assignment, bool)> {
        let (_, _, value_str, _) = Self::is_assignment(raw)?;

        // Empty value — check for arr=(...) syntax with separate tokens
        if value_str.is_empty() {
            let assignment_span = self.current_span;
            let (target, is_append, value_start) = self.current_word().and_then(|word| {
                let ParsedWordTarget {
                    name,
                    name_span,
                    subscript,
                    boundary,
                } = self.parse_word_target(&word, SubscriptInterpretation::Contextual, true)?;
                let WordTargetBoundary::Assignment {
                    append,
                    value_start,
                } = boundary
                else {
                    return None;
                };
                Some((
                    self.var_ref(name, name_span, subscript, assignment_span),
                    append,
                    value_start,
                ))
            })?;
            self.advance();
            if self.at(TokenKind::LeftParen) {
                let open_paren_span = self.current_span;
                self.advance(); // consume '('
                let (array, close_span) = self.collect_compound_array(open_paren_span, None);
                return Some((
                    Assignment {
                        target,
                        value: AssignmentValue::Compound(array),
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
            let value_span = Span::from_positions(value_start, assignment_span.end);
            return Some((
                Assignment {
                    target,
                    value: AssignmentValue::Scalar(Word::literal_with_span("", value_span)),
                    append: is_append,
                    span: assignment_span,
                },
                false,
            ));
        }

        self.current_word()
            .and_then(|word| {
                self.parse_assignment_from_word(word, None, SubscriptInterpretation::Contextual)
            })
            .or_else(|| {
                self.parse_assignment_from_text(
                    raw,
                    self.current_span,
                    None,
                    SubscriptInterpretation::Contextual,
                )
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
                let body = self.nested_stmt_seq_from_current_input(inner_start, close_span.start);

                Ok(self.word_with_parts(
                    vec![WordPartNode::new(
                        WordPart::ProcessSubstitution { body, is_input },
                        process_span.merge(close_span),
                    )],
                    process_span.merge(close_span),
                ))
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
        self.decode_word_parts_into_with_quote_fragments(s, base, source_backed, false, parts);
    }

    fn decode_word_parts_into_with_quote_fragments(
        &mut self,
        s: &str,
        base: Position,
        source_backed: bool,
        preserve_quote_fragments: bool,
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

            if preserve_quote_fragments && ch == '\'' {
                self.flush_literal_part(parts, &mut current, current_start, part_start);

                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut closed = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if c == '\'' {
                        closed = true;
                        break;
                    }
                    if let Some(content) = content.as_mut() {
                        content.push(c);
                    }
                    content_end = cursor;
                }

                if !closed {
                    if current.is_empty() {
                        current_start = part_start;
                    }
                    let fragment = if source_backed {
                        Span::from_positions(part_start, cursor)
                            .slice(self.input)
                            .to_string()
                    } else {
                        let mut fragment = String::from("'");
                        fragment.push_str(content.as_deref().unwrap_or_default());
                        fragment
                    };
                    current.push_str(&fragment);
                    continue;
                }

                Self::push_word_part(
                    parts,
                    WordPart::SingleQuoted {
                        value: if source_backed {
                            SourceText::source(Span::from_positions(content_start, content_end))
                        } else {
                            self.source_text(
                                content.unwrap_or_default(),
                                content_start,
                                content_end,
                            )
                        },
                        dollar: false,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if preserve_quote_fragments && ch == '"' {
                self.flush_literal_part(parts, &mut current, current_start, part_start);

                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut escaped = false;
                let mut closed = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if escaped {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        escaped = false;
                        continue;
                    }

                    match c {
                        '\\' => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                            escaped = true;
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        _ => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                        }
                    }
                }

                if !closed {
                    if current.is_empty() {
                        current_start = part_start;
                    }
                    let fragment = if source_backed {
                        Span::from_positions(part_start, cursor)
                            .slice(self.input)
                            .to_string()
                    } else {
                        let mut fragment = String::from("\"");
                        fragment.push_str(content.as_deref().unwrap_or_default());
                        fragment
                    };
                    current.push_str(&fragment);
                    continue;
                }

                let inner_span = Span::from_positions(content_start, content_end);
                let inner = if source_backed {
                    self.decode_word_text(
                        inner_span.slice(self.input),
                        inner_span,
                        content_start,
                        true,
                    )
                } else {
                    let content = content.unwrap_or_default();
                    self.decode_word_text(&content, inner_span, content_start, false)
                };

                Self::push_word_part(
                    parts,
                    WordPart::DoubleQuoted {
                        parts: inner.parts,
                        dollar: false,
                    },
                    part_start,
                    cursor,
                );
                current_start = cursor;
                continue;
            }

            if ch == '`' {
                self.flush_literal_part(parts, &mut current, current_start, part_start);

                let inner_start = cursor;
                let body = if source_backed {
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
                    self.nested_stmt_seq_from_current_input(inner_start, inner_end)
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
                    self.nested_stmt_seq_from_source(&cmd_str, inner_start)
                };

                Self::push_word_part(
                    parts,
                    WordPart::CommandSubstitution {
                        body,
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

            if chars.peek() == Some(&'"') {
                Self::next_word_char_unwrap(&mut chars, &mut cursor);
                let content_start = cursor;
                let mut content = (!source_backed).then(String::new);
                let mut content_end = content_start;
                let mut escaped = false;

                while let Some(c) = Self::next_word_char(&mut chars, &mut cursor) {
                    if escaped {
                        if let Some(content) = content.as_mut() {
                            content.push(c);
                        }
                        content_end = cursor;
                        escaped = false;
                        continue;
                    }

                    match c {
                        '\\' => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                            escaped = true;
                        }
                        '"' => break,
                        _ => {
                            if let Some(content) = content.as_mut() {
                                content.push(c);
                            }
                            content_end = cursor;
                        }
                    }
                }

                let inner_span = Span::from_positions(content_start, content_end);
                let inner = if source_backed {
                    self.decode_word_text(
                        inner_span.slice(self.input),
                        inner_span,
                        content_start,
                        true,
                    )
                } else {
                    let content = content.unwrap_or_default();
                    self.decode_word_text(&content, inner_span, content_start, false)
                };

                Self::push_word_part(
                    parts,
                    WordPart::DoubleQuoted {
                        parts: inner.parts,
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
                    let body = if source_backed {
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
                        self.nested_stmt_seq_from_current_input(inner_start, inner_end)
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
                        self.nested_stmt_seq_from_source(&cmd_str, inner_start)
                    };
                    Self::push_word_part(
                        parts,
                        WordPart::CommandSubstitution {
                            body,
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
                let brace_body_start = cursor;

                if self.dialect.features().zsh_parameter_modifiers
                    && matches!(chars.peek(), Some(&'(') | Some(&':'))
                {
                    let raw_body = self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                    let parameter = self.zsh_parameter_word_part(raw_body, part_start, cursor);
                    Self::push_word_part(parts, parameter, part_start, cursor);
                    current_start = cursor;
                    continue;
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '#') {
                    let var_name =
                        Self::read_word_while(&mut chars, &mut cursor, |c| c != '}' && c != '[');
                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let (index, raw_index) =
                            self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let subscript = self.subscript_from_source_text(
                            index,
                            raw_index,
                            SubscriptInterpretation::Contextual,
                        );
                        let reference = self.parameter_var_ref(
                            part_start,
                            "${#",
                            &var_name,
                            Some(subscript),
                            cursor,
                        );
                        let part = if reference
                            .subscript
                            .as_ref()
                            .and_then(Subscript::selector)
                            .is_some()
                        {
                            WordPart::ArrayLength(reference)
                        } else {
                            WordPart::Length(reference)
                        };
                        let part = self.parameter_word_part_from_legacy(
                            part,
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else {
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::Length(
                                self.parameter_var_ref(part_start, "${#", &var_name, None, cursor),
                            ),
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
                    }
                    current_start = cursor;
                    continue;
                }

                if Self::consume_word_char_if(&mut chars, &mut cursor, '!') {
                    let var_name = Self::read_word_while(&mut chars, &mut cursor, |c| {
                        !matches!(c, '}' | '[' | '*' | '@' | ':' | '-' | '=' | '+' | '?')
                    });

                    if Self::consume_word_char_if(&mut chars, &mut cursor, '[') {
                        let (index, raw_index) =
                            self.read_array_index(&mut chars, &mut cursor, source_backed);
                        Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                        let subscript = self.subscript_from_source_text(
                            index,
                            raw_index,
                            SubscriptInterpretation::Contextual,
                        );
                        let reference = self.parameter_var_ref(
                            part_start,
                            "${!",
                            &var_name,
                            Some(subscript),
                            cursor,
                        );
                        let part = if reference
                            .subscript
                            .as_ref()
                            .and_then(Subscript::selector)
                            .is_some()
                        {
                            WordPart::ArrayIndices(reference)
                        } else {
                            WordPart::Variable(
                                format!(
                                    "!{}[{}]",
                                    var_name,
                                    reference
                                        .subscript
                                        .as_ref()
                                        .map(|subscript| subscript.syntax_text(self.input))
                                        .unwrap_or_default()
                                )
                                .into(),
                            )
                        };
                        Self::push_word_part(parts, part, part_start, cursor);
                    } else if Self::consume_word_char_if(&mut chars, &mut cursor, '}') {
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::IndirectExpansion {
                                name: var_name.into(),
                                operator: None,
                                operand: None,
                                colon_variant: false,
                            },
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
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
                            let part = self.parameter_word_part_from_legacy(
                                WordPart::IndirectExpansion {
                                    name: var_name.into(),
                                    operator: Some(operator),
                                    operand: Some(operand),
                                    colon_variant: true,
                                },
                                part_start,
                                cursor,
                                source_backed,
                            );
                            Self::push_word_part(parts, part, part_start, cursor);
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
                        let part = self.parameter_word_part_from_legacy(
                            WordPart::IndirectExpansion {
                                name: var_name.into(),
                                operator: Some(operator),
                                operand: Some(operand),
                                colon_variant: false,
                            },
                            part_start,
                            cursor,
                            source_backed,
                        );
                        Self::push_word_part(parts, part, part_start, cursor);
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
                            let kind = if suffix.ends_with('@') {
                                PrefixMatchKind::At
                            } else {
                                PrefixMatchKind::Star
                            };
                            WordPart::PrefixMatch {
                                prefix: format!("{}{}", var_name, &suffix[..suffix.len() - 1])
                                    .into(),
                                kind,
                            }
                        } else {
                            WordPart::Variable(format!("!{}{}", var_name, suffix).into())
                        };
                        let part = self.parameter_word_part_from_legacy(
                            part,
                            part_start,
                            cursor,
                            source_backed,
                        );
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
                    let (index, raw_index) =
                        self.read_array_index(&mut chars, &mut cursor, source_backed);
                    let subscript = self.subscript_from_source_text(
                        index,
                        raw_index,
                        SubscriptInterpretation::Contextual,
                    );

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
                                    reference: self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
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
                                    reference: self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
                                    offset,
                                    offset_ast,
                                    length,
                                    length_ast,
                                }
                            }
                        } else if matches!(next_c, '-' | '+' | '=' | '?') {
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
                                reference: self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ),
                                operator,
                                operand: Some(operand),
                                colon_variant: false,
                            }
                        } else if next_c == '@' {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            if chars.peek().is_some() {
                                let operator = Self::next_word_char_unwrap(&mut chars, &mut cursor);
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::Transformation {
                                    reference: self.parameter_var_ref(
                                        part_start,
                                        "${",
                                        &var_name,
                                        Some(subscript),
                                        cursor,
                                    ),
                                    operator,
                                }
                            } else {
                                Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                                WordPart::ArrayAccess(self.parameter_var_ref(
                                    part_start,
                                    "${",
                                    &var_name,
                                    Some(subscript),
                                    cursor,
                                ))
                            }
                        } else {
                            Self::consume_word_char_if(&mut chars, &mut cursor, '}');
                            WordPart::ArrayAccess(self.parameter_var_ref(
                                part_start,
                                "${",
                                &var_name,
                                Some(subscript),
                                cursor,
                            ))
                        }
                    } else {
                        WordPart::ArrayAccess(self.parameter_var_ref(
                            part_start,
                            "${",
                            &var_name,
                            Some(subscript),
                            cursor,
                        ))
                    };

                    let part = self.parameter_word_part_from_legacy(
                        part,
                        part_start,
                        cursor,
                        source_backed,
                    );
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
                                        reference: self.parameter_var_ref(
                                            part_start, "${", &var_name, None, cursor,
                                        ),
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
                                        reference: self.parameter_var_ref(
                                            part_start, "${", &var_name, None, cursor,
                                        ),
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
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                operand: Some(operand),
                                colon_variant: false,
                            }
                        }
                        '#' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let longest = Self::consume_word_char_if(&mut chars, &mut cursor, '#');
                            let operand_text =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let pattern = self.pattern_from_source_text(&operand_text);
                            let operator = if longest {
                                ParameterOp::RemovePrefixLong { pattern }
                            } else {
                                ParameterOp::RemovePrefixShort { pattern }
                            };
                            WordPart::ParameterExpansion {
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                operand: None,
                                colon_variant: false,
                            }
                        }
                        '%' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let longest = Self::consume_word_char_if(&mut chars, &mut cursor, '%');
                            let operand_text =
                                self.read_brace_operand(&mut chars, &mut cursor, source_backed);
                            let pattern = self.pattern_from_source_text(&operand_text);
                            let operator = if longest {
                                ParameterOp::RemoveSuffixLong { pattern }
                            } else {
                                ParameterOp::RemoveSuffixShort { pattern }
                            };
                            WordPart::ParameterExpansion {
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
                                operator,
                                operand: None,
                                colon_variant: false,
                            }
                        }
                        '/' => {
                            Self::next_word_char_unwrap(&mut chars, &mut cursor);
                            let replace_all =
                                Self::consume_word_char_if(&mut chars, &mut cursor, '/');
                            let pattern_text = self.read_replacement_pattern(
                                &mut chars,
                                &mut cursor,
                                source_backed,
                            );
                            let pattern = self.pattern_from_source_text(&pattern_text);
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
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
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
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
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
                                reference: self
                                    .parameter_var_ref(part_start, "${", &var_name, None, cursor),
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
                                    reference: self.parameter_var_ref(
                                        part_start, "${", &var_name, None, cursor,
                                    ),
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

                let part = if cursor.offset > brace_body_start.offset {
                    self.parameter_word_part_from_legacy(part, part_start, cursor, source_backed)
                } else {
                    part
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
    ) -> (SourceText, Option<SourceText>) {
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

        let span = Span::from_positions(start, end);
        if source_backed {
            self.subscript_source_text(span.slice(self.input), span)
        } else {
            let text = text.unwrap_or_default();
            self.subscript_source_text(&text, span)
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
        self.word_with_parts(parts, span)
    }

    fn decode_fragment_word_text(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        let mut parts = Vec::new();
        self.decode_word_parts_into_with_quote_fragments(s, base, source_backed, true, &mut parts);
        self.word_with_parts(parts, span)
    }

    fn parse_word_with_context(
        &mut self,
        s: &str,
        span: Span,
        base: Position,
        source_backed: bool,
    ) -> Word {
        self.decode_word_text_preserving_quotes_if_needed(s, span, base, source_backed)
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
        BackgroundOperator, BinaryCommand, BourneParameterExpansion,
        BuiltinCommand as AstBuiltinCommand, Command as AstCommand,
        CompoundCommand as AstCompoundCommand, ForeachSyntax, FunctionDef as AstFunctionDef,
        IfSyntax, Name, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, PrefixMatchKind,
        RepeatSyntax, SimpleCommand as AstSimpleCommand, SourceText, StmtTerminator,
        ZshDefaultingOp, ZshExpansionOperation, ZshExpansionTarget, ZshPatternOp, ZshReplacementOp,
        ZshTrimOp,
    };

    fn is_fully_quoted(word: &Word) -> bool {
        word.is_fully_quoted()
    }

    fn pattern_part_slices<'a>(pattern: &'a Pattern, input: &'a str) -> Vec<&'a str> {
        pattern
            .parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect()
    }

    fn top_level_part_slices<'a>(word: &'a Word, input: &'a str) -> Vec<&'a str> {
        word.parts
            .iter()
            .map(|part| part.span.slice(input))
            .collect()
    }

    fn brace_slices<'a>(word: &'a Word, input: &'a str) -> Vec<&'a str> {
        word.brace_syntax
            .iter()
            .map(|brace| brace.span.slice(input))
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

    fn collect_file_comments(file: &File) -> Vec<Comment> {
        let mut comments = Vec::new();
        collect_stmt_seq_comments(&file.body, &mut comments);
        comments
    }

    fn collect_stmt_seq_comments(sequence: &StmtSeq, comments: &mut Vec<Comment>) {
        comments.extend(sequence.leading_comments.iter().copied());
        for stmt in &sequence.stmts {
            collect_stmt_comments(stmt, comments);
        }
        comments.extend(sequence.trailing_comments.iter().copied());
    }

    fn collect_stmt_comments(stmt: &Stmt, comments: &mut Vec<Comment>) {
        comments.extend(stmt.leading_comments.iter().copied());
        if let Some(comment) = stmt.inline_comment {
            comments.push(comment);
        }
        collect_command_comments(&stmt.command, comments);
    }

    fn collect_command_comments(command: &AstCommand, comments: &mut Vec<Comment>) {
        match command {
            AstCommand::Binary(command) => {
                collect_stmt_comments(&command.left, comments);
                collect_stmt_comments(&command.right, comments);
            }
            AstCommand::Compound(command) => collect_compound_comments(command, comments),
            AstCommand::Function(function) => collect_stmt_comments(&function.body, comments),
            AstCommand::Simple(_) | AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
        }
    }

    fn collect_compound_comments(command: &AstCompoundCommand, comments: &mut Vec<Comment>) {
        match command {
            AstCompoundCommand::If(command) => {
                collect_stmt_seq_comments(&command.condition, comments);
                collect_stmt_seq_comments(&command.then_branch, comments);
                for branch in &command.elif_branches {
                    collect_stmt_seq_comments(&branch.0, comments);
                    collect_stmt_seq_comments(&branch.1, comments);
                }
                if let Some(body) = &command.else_branch {
                    collect_stmt_seq_comments(body, comments);
                }
            }
            AstCompoundCommand::For(command) => {
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::Select(command) => {
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::ArithmeticFor(command) => {
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::While(command) => {
                collect_stmt_seq_comments(&command.condition, comments);
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::Until(command) => {
                collect_stmt_seq_comments(&command.condition, comments);
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::Case(command) => {
                for item in &command.cases {
                    collect_stmt_seq_comments(&item.body, comments);
                }
            }
            AstCompoundCommand::Subshell(body) | AstCompoundCommand::BraceGroup(body) => {
                collect_stmt_seq_comments(body, comments);
            }
            AstCompoundCommand::Always(command) => {
                collect_stmt_seq_comments(&command.body, comments);
                collect_stmt_seq_comments(&command.always_body, comments);
            }
            AstCompoundCommand::Repeat(command) => {
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::Foreach(command) => {
                collect_stmt_seq_comments(&command.body, comments);
            }
            AstCompoundCommand::Conditional(_)
            | AstCompoundCommand::Arithmetic(_)
            | AstCompoundCommand::Time(_)
            | AstCompoundCommand::Coproc(_) => {}
        }
    }

    fn expect_function(stmt: &Stmt) -> &AstFunctionDef {
        let AstCommand::Function(function) = &stmt.command else {
            panic!("expected function definition");
        };
        function
    }

    fn expect_compound(stmt: &Stmt) -> (&AstCompoundCommand, &[Redirect]) {
        let AstCommand::Compound(compound) = &stmt.command else {
            panic!("expected compound command");
        };
        (compound, stmt.redirects.as_slice())
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

    fn expect_subscript<'a>(reference: &'a VarRef, input: &str, expected: &str) -> &'a Subscript {
        let subscript = reference
            .subscript
            .as_ref()
            .expect("expected subscripted reference");
        assert_eq!(subscript.text.slice(input), expected);
        subscript
    }

    fn expect_subscript_syntax<'a>(
        reference: &'a VarRef,
        input: &str,
        expected_syntax: &str,
        expected_cooked: &str,
    ) -> &'a Subscript {
        let subscript = expect_subscript(reference, input, expected_cooked);
        assert_eq!(subscript.syntax_text(input), expected_syntax);
        subscript
    }

    fn array_access_reference(part: &WordPart) -> Option<&VarRef> {
        match part {
            WordPart::ArrayAccess(reference) => Some(reference),
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                    reference,
                }) => Some(reference),
                _ => None,
            },
            _ => None,
        }
    }

    fn expect_array_access(word: &Word) -> &VarRef {
        let [part] = word.parts.as_slice() else {
            panic!("expected single expansion part");
        };
        array_access_reference(&part.kind)
            .unwrap_or_else(|| panic!("expected array access part, got {:?}", part.kind))
    }

    fn expect_parameter(word: &Word) -> &ParameterExpansion {
        let [part] = word.parts.as_slice() else {
            panic!("expected single parameter part");
        };
        let WordPart::Parameter(parameter) = &part.kind else {
            panic!("expected parameter part, got {:?}", part.kind);
        };
        parameter
    }

    fn expect_zsh_qualified_glob(word: &Word) -> &ZshQualifiedGlob {
        let [part] = word.parts.as_slice() else {
            panic!("expected single qualified glob part");
        };
        let WordPart::ZshQualifiedGlob(glob) = &part.kind else {
            panic!("expected qualified glob part, got {:?}", part.kind);
        };
        glob
    }

    fn expect_array_length_part(part: &WordPart) -> &VarRef {
        match part {
            WordPart::ArrayLength(reference) => reference,
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Length {
                    reference,
                }) => reference,
                _ => panic!("expected array length part, got {:?}", part),
            },
            _ => panic!("expected array length part, got {:?}", part),
        }
    }

    fn expect_array_indices_part(part: &WordPart) -> &VarRef {
        match part {
            WordPart::ArrayIndices(reference) => reference,
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Indices {
                    reference,
                }) => reference,
                _ => panic!("expected array indices part, got {:?}", part),
            },
            _ => panic!("expected array indices part, got {:?}", part),
        }
    }

    fn expect_substring_part(
        part: &WordPart,
    ) -> (
        &VarRef,
        &Option<ArithmeticExprNode>,
        &Option<ArithmeticExprNode>,
    ) {
        match part {
            WordPart::Substring {
                reference,
                offset_ast,
                length_ast,
                ..
            } => (reference, offset_ast, length_ast),
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                }) if !reference.has_array_selector() => (reference, offset_ast, length_ast),
                _ => panic!("expected substring part, got {:?}", part),
            },
            _ => panic!("expected substring part, got {:?}", part),
        }
    }

    fn expect_array_slice_part(
        part: &WordPart,
    ) -> (
        &VarRef,
        &Option<ArithmeticExprNode>,
        &Option<ArithmeticExprNode>,
    ) {
        match part {
            WordPart::ArraySlice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => (reference, offset_ast, length_ast),
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                }) if reference.has_array_selector() => (reference, offset_ast, length_ast),
                _ => panic!("expected array slice part, got {:?}", part),
            },
            _ => panic!("expected array slice part, got {:?}", part),
        }
    }

    fn expect_parameter_operation_part(
        part: &WordPart,
    ) -> (&VarRef, &ParameterOp, Option<&SourceText>) {
        match part {
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                ..
            } => (reference, operator, operand.as_ref()),
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    ..
                }) => (reference, operator, operand.as_ref()),
                _ => panic!("expected parameter operation part, got {:?}", part),
            },
            _ => panic!("expected parameter operation part, got {:?}", part),
        }
    }

    fn expect_prefix_match_part(part: &WordPart) -> (&Name, PrefixMatchKind) {
        match part {
            WordPart::PrefixMatch { prefix, kind } => (prefix, *kind),
            WordPart::Parameter(parameter) => match &parameter.syntax {
                ParameterExpansionSyntax::Bourne(BourneParameterExpansion::PrefixMatch {
                    prefix,
                    kind,
                }) => (prefix, *kind),
                _ => panic!("expected prefix match part, got {:?}", part),
            },
            _ => panic!("expected prefix match part, got {:?}", part),
        }
    }

    fn expect_simple(stmt: &Stmt) -> &AstSimpleCommand {
        let AstCommand::Simple(command) = &stmt.command else {
            panic!("expected simple command");
        };
        command
    }

    fn expect_binary(stmt: &Stmt) -> &BinaryCommand {
        let AstCommand::Binary(command) = &stmt.command else {
            panic!("expected binary command");
        };
        command
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
        let script = parser.parse().unwrap().file;

        assert_eq!(script.body.len(), 1);

        if let AstCommand::Simple(cmd) = &script.body[0].command {
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
        let script = parser.parse().unwrap().file;

        let AstCommand::Builtin(AstBuiltinCommand::Break(command)) = &script.body[0].command else {
            panic!("expected break builtin");
        };

        assert_eq!(command.depth.as_ref().unwrap().render(input), "2");
        assert!(command.extra_args.is_empty());
    }

    #[test]
    fn test_parse_continue_preserves_extra_args() {
        let input = "continue 1 extra";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().file;

        let AstCommand::Builtin(AstBuiltinCommand::Continue(command)) = &script.body[0].command
        else {
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
        let script = parser.parse().unwrap().file;

        let AstCommand::Builtin(AstBuiltinCommand::Return(command)) = &script.body[0].command
        else {
            panic!("expected return builtin");
        };

        assert_eq!(command.code.as_ref().unwrap().render(input), "42");
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].target.name, "FOO");
        assert_eq!(script.body[0].redirects.len(), 1);
        assert_eq!(
            redirect_word_target(&script.body[0].redirects[0]).render(input),
            "out.txt"
        );
    }

    #[test]
    fn test_parse_exit_as_typed_builtin() {
        let input = "exit 1";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().file;

        let AstCommand::Builtin(AstBuiltinCommand::Exit(command)) = &script.body[0].command else {
            panic!("expected exit builtin");
        };

        assert_eq!(command.code.as_ref().unwrap().render(input), "1");
        assert!(command.extra_args.is_empty());
    }

    #[test]
    fn test_parse_quoted_flow_control_name_stays_simple_command() {
        let input = "'break' 2";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = parser.parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = parser.parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = parser.parse().unwrap().file;

        if let AstCommand::Simple(cmd) = &script.body[0].command {
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
        let script = parser.parse().unwrap().file;

        if let AstCommand::Simple(cmd) = &script.body[0].command {
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

        assert_eq!(recovered.file.body.len(), 2);
        assert_eq!(recovered.diagnostics.len(), 1);
        assert_eq!(recovered.diagnostics[0].message, "expected word");
        assert_eq!(recovered.diagnostics[0].span.start.line, 2);

        let first = expect_simple(&recovered.file.body[0]);
        assert_eq!(first.name.render(input), "echo");
        assert_eq!(first.args[0].render(input), "one");

        let second = expect_simple(&recovered.file.body[1]);
        assert_eq!(second.name.render(input), "echo");
        assert_eq!(second.args[0].render(input), "two");
    }

    #[test]
    fn test_parse_pipeline() {
        let parser = Parser::new("echo hello | cat");
        let script = parser.parse().unwrap().file;

        assert_eq!(script.body.len(), 1);
        let pipeline = expect_binary(&script.body[0]);
        assert_eq!(pipeline.op, BinaryOp::Pipe);
        assert_eq!(
            expect_simple(&pipeline.left)
                .name
                .render("echo hello | cat"),
            "echo"
        );
        assert_eq!(
            expect_simple(&pipeline.right)
                .name
                .render("echo hello | cat"),
            "cat"
        );
    }

    #[test]
    fn test_parse_pipe_both_pipeline() {
        let input = "echo hello |& cat";
        let script = Parser::new(input).parse().unwrap().file;

        let pipeline = expect_binary(&script.body[0]);
        assert_eq!(pipeline.op, BinaryOp::PipeAll);
        assert_eq!(expect_simple(&pipeline.left).name.render(input), "echo");
        assert_eq!(expect_simple(&pipeline.right).name.render(input), "cat");
    }

    #[test]
    fn test_parse_redirect_out() {
        let input = "echo hello > /tmp/out";
        let parser = Parser::new(input);
        let script = parser.parse().unwrap().file;
        let stmt = &script.body[0];
        let cmd = expect_simple(stmt);

        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
        assert_eq!(
            redirect_word_target(&stmt.redirects[0]).render(input),
            "/tmp/out"
        );
    }

    #[test]
    fn test_parse_redirect_both_append() {
        let input = "echo hello &>> /tmp/out";
        let script = Parser::new(input).parse().unwrap().file;
        let stmt = &script.body[0];
        let cmd = expect_simple(stmt);

        assert_eq!(cmd.name.render(input), "echo");
        assert_eq!(stmt.redirects.len(), 2);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::Append);
        assert_eq!(
            redirect_word_target(&stmt.redirects[0]).render(input),
            "/tmp/out"
        );
        assert_eq!(stmt.redirects[1].fd, Some(2));
        assert_eq!(stmt.redirects[1].kind, RedirectKind::DupOutput);
        assert_eq!(redirect_word_target(&stmt.redirects[1]).render(input), "1");
    }

    #[test]
    fn test_parse_redirect_append() {
        let parser = Parser::new("echo hello >> /tmp/out");
        let script = parser.parse().unwrap().file;
        let stmt = &script.body[0];

        assert_eq!(
            expect_simple(stmt).name.render("echo hello >> /tmp/out"),
            "echo"
        );
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::Append);
    }

    #[test]
    fn test_parse_redirect_in() {
        let parser = Parser::new("cat < /tmp/in");
        let script = parser.parse().unwrap().file;
        let stmt = &script.body[0];

        assert_eq!(expect_simple(stmt).name.render("cat < /tmp/in"), "cat");
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::Input);
    }

    #[test]
    fn test_parse_redirect_read_write() {
        let input = "exec 8<> /tmp/rw";
        let script = Parser::new(input).parse().unwrap().file;
        let stmt = &script.body[0];
        let cmd = expect_simple(stmt);

        assert_eq!(cmd.name.render(input), "exec");
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].fd, Some(8));
        assert_eq!(stmt.redirects[0].kind, RedirectKind::ReadWrite);
        assert_eq!(
            redirect_word_target(&stmt.redirects[0]).render(input),
            "/tmp/rw"
        );
    }

    #[test]
    fn test_parse_named_fd_redirect_read_write() {
        let input = "exec {rw}<> /tmp/rw";
        let script = Parser::new(input).parse().unwrap().file;
        let stmt = &script.body[0];
        let cmd = expect_simple(stmt);

        assert_eq!(cmd.name.render(input), "exec");
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].fd_var.as_deref(), Some("rw"));
        assert_eq!(stmt.redirects[0].kind, RedirectKind::ReadWrite);
        assert_eq!(
            redirect_word_target(&stmt.redirects[0]).render(input),
            "/tmp/rw"
        );
    }

    #[test]
    fn test_parse_command_list_and() {
        let parser = Parser::new("true && echo success");
        let script = parser.parse().unwrap().file;

        assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::And);
    }

    #[test]
    fn test_parse_command_list_or() {
        let parser = Parser::new("false || echo fallback");
        let script = parser.parse().unwrap().file;

        assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::Or);
    }

    #[test]
    fn test_parse_command_list_preserves_operator_spans() {
        let input = "true && false || echo fallback";
        let script = Parser::new(input).parse().unwrap().file;

        let outer = expect_binary(&script.body[0]);
        assert_eq!(outer.op, BinaryOp::Or);
        assert_eq!(outer.op_span.slice(input), "||");
        let inner = expect_binary(&outer.left);
        assert_eq!(inner.op, BinaryOp::And);
        assert_eq!(inner.op_span.slice(input), "&&");
    }

    #[test]
    fn test_heredoc_pipe() {
        let parser = Parser::new("cat <<EOF | sort\nc\na\nb\nEOF\n");
        let script = parser.parse().unwrap().file;
        assert!(
            matches!(&script.body[0].command, AstCommand::Binary(_)),
            "heredoc with pipe should parse as a binary pipe"
        );
    }

    #[test]
    fn test_prefix_heredoc_before_command_in_pipeline_parses() {
        let input = "<<EOF tac | tr '\\n' 'X'\none\ntwo\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let pipeline = expect_binary(&script.body[0]);
        assert_eq!(pipeline.op, BinaryOp::Pipe);
        let command = expect_simple(&pipeline.left);
        assert_eq!(command.name.render(input), "tac");
        assert_eq!(pipeline.left.redirects.len(), 1);
        assert_eq!(pipeline.left.redirects[0].kind, RedirectKind::HereDoc);
    }

    #[test]
    fn test_redirect_only_command_parses() {
        let input = ">myfile\n";
        let script = Parser::new(input).parse().unwrap().file;
        let stmt = &script.body[0];
        let command = expect_simple(stmt);

        assert!(command.name.render(input).is_empty());
        assert_eq!(stmt.redirects.len(), 1);
        assert_eq!(stmt.redirects[0].kind, RedirectKind::Output);
        assert_eq!(
            redirect_word_target(&stmt.redirects[0]).render(input),
            "myfile"
        );
    }

    #[test]
    fn test_function_definition_absorbs_trailing_heredoc_redirect() {
        let input = "f() { cat; } <<EOF\nhello\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (_, redirects) = expect_compound(function.body.as_ref());
        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert_eq!(redirects.len(), 1);
        assert_eq!(redirects[0].kind, RedirectKind::HereDoc);
    }

    #[test]
    fn test_function_body_command_with_heredoc_parses() {
        let input = "f() {\n  read head << EOF\nref: refs/heads/dev/andy\nEOF\n}\nf\n";
        let script = Parser::new(input).parse().unwrap().file;

        assert_eq!(script.body.len(), 2);

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::BraceGroup(body) = compound else {
            panic!("expected brace-group function body");
        };
        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_function_keyword_without_parens_preserves_surface_form() {
        let input = "function inc { :; }\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::BraceGroup(body) = compound else {
            panic!("expected brace-group function body");
        };

        assert!(function.uses_function_keyword());
        assert!(!function.has_name_parens());
        assert_eq!(
            function
                .surface
                .function_keyword_span
                .map(|span| span.slice(input)),
            Some("function")
        );
        assert_eq!(function.surface.name_parens_span, None);
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_function_keyword_with_parens_preserves_surface_form() {
        let input = "function inc() { :; }\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());

        assert!(function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert_eq!(
            function
                .surface
                .function_keyword_span
                .map(|span| span.slice(input)),
            Some("function")
        );
        assert_eq!(
            function
                .surface
                .name_parens_span
                .map(|span| span.slice(input)),
            Some("()")
        );
        assert!(matches!(compound, AstCompoundCommand::BraceGroup(_)));
        assert!(redirects.is_empty());
    }

    #[test]
    fn test_posix_function_with_brace_group_preserves_surface_form() {
        let input = "inc() { :; }\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());

        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert_eq!(function.surface.function_keyword_span, None);
        assert_eq!(
            function
                .surface
                .name_parens_span
                .map(|span| span.slice(input)),
            Some("()")
        );
        assert!(matches!(compound, AstCompoundCommand::BraceGroup(_)));
        assert!(redirects.is_empty());
    }

    #[test]
    fn test_posix_function_allows_subshell_body() {
        let input = "inc_subshell() ( j=$((j+5)); )\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::Subshell(body) = compound else {
            panic!("expected subshell function body");
        };
        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_function_keyword_allows_subshell_body() {
        let input = "function inc_subshell() ( j=$((j+5)); )\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::Subshell(body) = compound else {
            panic!("expected subshell function body");
        };
        assert!(function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_posix_function_allows_conditional_body() {
        let input = "f() [[ -n \"$x\" ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional function body");
        };

        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(input), "[[ -n \"$x\" ]]");
    }

    #[test]
    fn test_posix_function_allows_arithmetic_body() {
        let input = "f() (( x + 1 ))\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic function body");
        };

        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(input), "(( x + 1 ))");
    }

    #[test]
    fn test_posix_function_allows_if_body() {
        let input = "f() if true; then :; fi\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        assert!(matches!(compound, AstCompoundCommand::If(_)));

        assert!(!function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
    }

    #[test]
    fn test_function_keyword_allows_newline_conditional_body() {
        let input = "function f()\n[[ -n x ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional function body");
        };

        assert!(function.uses_function_keyword());
        assert!(function.has_name_parens());
        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(input), "[[ -n x ]]");
    }

    #[test]
    fn test_function_keyword_rejects_same_line_conditional_body() {
        let parser = Parser::new("function f() [[ -n x ]]\n");
        assert!(
            parser.parse().is_err(),
            "same-line conditional body should be rejected for function keyword definitions"
        );
    }

    #[test]
    fn test_function_body_rejects_simple_command() {
        let parser = Parser::new("f() echo hi\n");
        assert!(
            parser.parse().is_err(),
            "simple command should not be accepted as a function body"
        );
    }

    #[test]
    fn test_function_body_rejects_time_command() {
        let parser = Parser::new("f() time { :; }\n");
        assert!(
            parser.parse().is_err(),
            "time command should not be accepted as a function body"
        );
    }

    #[test]
    fn test_function_body_rejects_coproc_command() {
        let parser = Parser::new("f() coproc cat\n");
        assert!(
            parser.parse().is_err(),
            "coproc command should not be accepted as a function body"
        );
    }

    #[test]
    fn test_function_conditional_body_absorbs_trailing_redirect() {
        let input = "f() [[ -n x ]] >out\n";
        let script = Parser::new(input).parse().unwrap().file;

        let function = expect_function(&script.body[0]);
        let (compound, redirects) = expect_compound(function.body.as_ref());
        assert!(matches!(compound, AstCompoundCommand::Conditional(_)));

        assert_eq!(redirects.len(), 1);
        assert_eq!(redirects[0].kind, RedirectKind::Output);
        assert_eq!(redirect_word_target(&redirects[0]).render(input), "out");
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
        let script = parser.parse().unwrap().file;
        assert_eq!(script.body.len(), 1);
        let (compound, _) = expect_compound(&script.body[0]);
        if let AstCompoundCommand::While(w) = compound {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        assert!(redirects.is_empty());
        let AstCompoundCommand::While(command) = compound else {
            panic!("expected while command");
        };
        assert_eq!(command.condition.len(), 1);
        assert_eq!(command.body.len(), 2);
    }

    #[test]
    fn test_heredoc_target_preserves_body_span() {
        let input = "cat <<'EOF'\nhello $name\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let stmt = &script.body[0];
        let _command = expect_simple(stmt);
        assert_eq!(stmt.redirects.len(), 1);

        let redirect = &stmt.redirects[0];
        let heredoc = redirect_heredoc(redirect);
        assert_eq!(heredoc.body.span.slice(input), "hello $name\n");
        assert!(is_fully_quoted(&heredoc.body));
    }

    #[test]
    fn test_heredoc_delimiter_metadata_tracks_flags_and_spans() {
        let input = "cat <<EOF\nhello\nEOF\ncat <<'EOF'\nhello\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let unquoted_stmt = &script.body[0];
        let _unquoted = expect_simple(unquoted_stmt);
        let unquoted_redirect = &unquoted_stmt.redirects[0];
        let unquoted_heredoc = redirect_heredoc(unquoted_redirect);
        assert_eq!(unquoted_redirect.span.slice(input), "<<EOF");
        assert_eq!(unquoted_heredoc.delimiter.span.slice(input), "EOF");
        assert_eq!(unquoted_heredoc.delimiter.raw.span.slice(input), "EOF");
        assert_eq!(unquoted_heredoc.delimiter.cooked, "EOF");
        assert!(!unquoted_heredoc.delimiter.quoted);
        assert!(unquoted_heredoc.delimiter.expands_body);
        assert!(!unquoted_heredoc.delimiter.strip_tabs);

        let quoted_stmt = &script.body[1];
        let _quoted = expect_simple(quoted_stmt);
        let quoted_redirect = &quoted_stmt.redirects[0];
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
        let script = Parser::new(input).parse().unwrap().file;

        let stmt = &script.body[0];
        let _command = expect_simple(stmt);
        let redirect = &stmt.redirects[0];
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
        let script = Parser::new(input).parse().unwrap().file;

        let stmt = &script.body[0];
        let _command = expect_simple(stmt);
        let redirect = &stmt.redirects[0];
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
        let script = Parser::new(input).parse().unwrap().file;

        let stmt = &script.body[0];
        let _command = expect_simple(stmt);
        let redirect = &stmt.redirects[0];
        let heredoc = redirect_heredoc(redirect);

        assert_eq!(redirect.span.slice(input), "<<-EOF");
        assert!(heredoc.delimiter.strip_tabs);
        assert!(heredoc.delimiter.expands_body);
        assert_eq!(heredoc.delimiter.cooked, "EOF");
    }

    #[test]
    fn test_heredoc_targets_preserve_quoted_and_unquoted_decode_behavior() {
        let input = "cat <<EOF\nhello $name\nEOF\ncat <<'EOF'\nhello $name\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let unquoted_target = &redirect_heredoc(&script.body[0].redirects[0]).body;
        assert!(!is_fully_quoted(unquoted_target));
        assert_eq!(unquoted_target.render(input), "hello $name\n");
        let unquoted_slices = top_level_part_slices(unquoted_target, input);
        assert_eq!(unquoted_slices, vec!["hello ", "$name", "\n"]);
        assert!(matches!(
            unquoted_target.parts[1].kind,
            WordPart::Variable(_)
        ));

        let quoted_target = &redirect_heredoc(&script.body[1].redirects[0]).body;
        assert!(is_fully_quoted(quoted_target));
        assert_eq!(quoted_target.render(input), "hello $name\n");
        assert!(matches!(
            quoted_target.parts.as_slice(),
            [part] if matches!(&part.kind, WordPart::SingleQuoted { .. })
        ));
    }

    #[test]
    fn test_unquoted_heredoc_body_preserves_multiple_quoted_fragments() {
        let input = "cat <<EOF\nbefore '$HOME' and \"$USER\"\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let body = &redirect_heredoc(&script.body[0].redirects[0]).body;

        assert!(!is_fully_quoted(body));
        assert_eq!(
            top_level_part_slices(body, input),
            vec!["before ", "'$HOME'", " and ", "\"$USER\"", "\n"]
        );
        assert!(matches!(body.parts[1].kind, WordPart::SingleQuoted { .. }));
        assert!(matches!(body.parts[3].kind, WordPart::DoubleQuoted { .. }));
    }

    #[test]
    fn test_unquoted_heredoc_body_leaves_unmatched_single_quote_literal() {
        let input = "cat <<EOF\n'$HOME\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let body = &redirect_heredoc(&script.body[0].redirects[0]).body;

        assert!(
            !body
                .parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::SingleQuoted { .. }))
        );
        assert_eq!(body.render_syntax(input), "'$HOME\n");
    }

    #[test]
    fn test_strip_tabs_heredoc_body_preserves_single_quoted_fragments() {
        let input = "cat <<-EOF\n\t'$HOME'\nEOF\n";
        let script = Parser::new(input).parse().unwrap().file;

        let heredoc = redirect_heredoc(&script.body[0].redirects[0]);

        assert!(heredoc.delimiter.strip_tabs);
        assert!(matches!(
            heredoc.body.parts[0].kind,
            WordPart::SingleQuoted { .. }
        ));
        assert_eq!(heredoc.body.render_syntax(input), "'$HOME'\n");
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
        let script = parser.parse().unwrap().file;
        assert_eq!(script.body.len(), 1);
        if let AstCommand::Simple(cmd) = &script.body[0].command {
            assert_eq!(cmd.name.render(input), "echo");
            assert_eq!(cmd.args.len(), 1);
            // The arg should contain an ArrayAccess with the full nested index
            let arg = &cmd.args[0];
            let has_array_access = arg.parts.iter().any(|p| {
                array_access_reference(&p.kind).is_some_and(|reference| {
                    reference.name == "arr"
                        && reference.subscript.as_ref().is_some_and(|subscript| {
                            subscript.text.slice(input).contains("${#arr[@]}")
                        })
                })
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].target.name, "a");
        expect_subscript(&command.assignments[0].target, input, "1 + 2");
        assert!(command.name.render(input).is_empty());
    }

    #[test]
    fn test_parenthesized_indexed_assignment_is_not_function_definition() {
        let input = "a[(1+2)*3]=9\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].target.name, "a");
        expect_subscript(&command.assignments[0].target, input, "(1+2)*3");
        assert!(command.name.render(input).is_empty());
    }

    #[test]
    fn test_assignment_index_ast_tracks_arithmetic_subscripts() {
        let input = "a[i + 1]=x\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let assignment = &command.assignments[0];
        let subscript_ast = assignment
            .target
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.arithmetic_ast.as_ref());
        let expr = subscript_ast.expect("expected arithmetic subscript AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive subscript");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_variable(left.as_ref(), "i");
        expect_number(right.as_ref(), input, "1");
    }

    #[test]
    fn test_decl_name_and_array_access_attach_arithmetic_index_asts() {
        let input = "declare foo[1+2]\necho ${arr[i+1]}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
            panic!("expected declaration command");
        };
        let DeclOperand::Name(name) = &command.operands[0] else {
            panic!("expected declaration name operand");
        };
        let subscript_ast = name
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.arithmetic_ast.as_ref());
        let expr = subscript_ast.expect("expected declaration index AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive expression in declaration index");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_number(left.as_ref(), input, "1");
        expect_number(right.as_ref(), input, "2");

        let AstCommand::Simple(command) = &script.body[1].command else {
            panic!("expected simple command");
        };
        let reference = expect_array_access(&command.args[0]);
        expect_subscript(reference, input, "i+1");
        let expr = reference
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.arithmetic_ast.as_ref())
            .expect("expected array access index AST");
        let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
            panic!("expected additive array index");
        };
        assert_eq!(*op, ArithmeticBinaryOp::Add);
        expect_variable(left.as_ref(), "i");
        expect_number(right.as_ref(), input, "1");
    }

    #[test]
    fn test_substring_and_array_slice_attach_arithmetic_companion_asts() {
        let input = "echo ${s:i+1:len*2} ${arr[@]:i:j}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        let (_, offset_ast, length_ast) = expect_substring_part(&command.args[0].parts[0].kind);
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

        let (_, offset_ast, length_ast) = expect_array_slice_part(&command.args[1].parts[0].kind);
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        let reference = expect_array_access(&command.args[0]);
        let subscript = reference
            .subscript
            .as_ref()
            .expect("expected first array subscript");
        assert_eq!(subscript.selector(), Some(SubscriptSelector::At));
        assert!(subscript.arithmetic_ast.is_none());

        let reference = expect_array_access(&command.args[1]);
        let subscript = reference
            .subscript
            .as_ref()
            .expect("expected second array subscript");
        assert_eq!(subscript.selector(), Some(SubscriptSelector::Star));
        assert!(subscript.arithmetic_ast.is_none());

        let reference = expect_array_access(&command.args[2]);
        let subscript = reference
            .subscript
            .as_ref()
            .expect("expected third array subscript");
        assert_eq!(subscript.selector(), None);
        assert!(subscript.arithmetic_ast.is_none());
    }

    #[test]
    fn test_parameter_forms_preserve_selector_kinds() {
        let input = "echo ${arr[@]} ${arr[*]} ${#arr[@]} ${!arr[*]} ${arr[@]:1:2}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        let reference = expect_array_access(&command.args[0]);
        assert_eq!(
            reference.subscript.as_ref().and_then(Subscript::selector),
            Some(SubscriptSelector::At)
        );

        let reference = expect_array_access(&command.args[1]);
        assert_eq!(
            reference.subscript.as_ref().and_then(Subscript::selector),
            Some(SubscriptSelector::Star)
        );

        let reference = expect_array_length_part(&command.args[2].parts[0].kind);
        assert_eq!(
            reference.subscript.as_ref().and_then(Subscript::selector),
            Some(SubscriptSelector::At)
        );

        let reference = expect_array_indices_part(&command.args[3].parts[0].kind);
        assert_eq!(
            reference.subscript.as_ref().and_then(Subscript::selector),
            Some(SubscriptSelector::Star)
        );

        let (reference, _, _) = expect_array_slice_part(&command.args[4].parts[0].kind);
        assert_eq!(
            reference.subscript.as_ref().and_then(Subscript::selector),
            Some(SubscriptSelector::At)
        );
    }

    #[test]
    fn test_compound_array_assignment_preserves_mixed_element_kinds() {
        let input = "arr=(one [two]=2 [three]+=3 four)\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let assignment = &command.assignments[0];
        let AssignmentValue::Compound(array) = &assignment.value else {
            panic!("expected compound array assignment");
        };

        assert_eq!(array.kind, ArrayKind::Contextual);
        assert_eq!(array.elements.len(), 4);

        let ArrayElem::Sequential(first) = &array.elements[0] else {
            panic!("expected first sequential element");
        };
        assert_eq!(first.span.slice(input), "one");

        let ArrayElem::Keyed { key, value } = &array.elements[1] else {
            panic!("expected keyed element");
        };
        assert_eq!(key.text.slice(input), "two");
        assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
        assert_eq!(value.span.slice(input), "2");

        let ArrayElem::KeyedAppend { key, value } = &array.elements[2] else {
            panic!("expected keyed append element");
        };
        assert_eq!(key.text.slice(input), "three");
        assert_eq!(key.interpretation, SubscriptInterpretation::Contextual);
        assert_eq!(value.span.slice(input), "3");

        let ArrayElem::Sequential(last) = &array.elements[3] else {
            panic!("expected trailing sequential element");
        };
        assert_eq!(last.span.slice(input), "four");
    }

    #[test]
    fn test_assignment_append_and_keyed_append_stay_distinct() {
        let input = "arr+=one\nassoc=(one [key]+=value)\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(first) = &script.body[0].command else {
            panic!("expected first simple command");
        };
        assert!(first.assignments[0].append);

        let AstCommand::Simple(second) = &script.body[1].command else {
            panic!("expected second simple command");
        };
        assert!(!second.assignments[0].append);
        let AssignmentValue::Compound(array) = &second.assignments[0].value else {
            panic!("expected compound assignment");
        };
        assert!(matches!(array.elements[1], ArrayElem::KeyedAppend { .. }));
    }

    #[test]
    fn test_assignment_target_mixed_subscript_and_compound_value_stay_structured() {
        let input = "assoc[\"$key\"-suffix]=(\"$value\" plain)\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let assignment = &command.assignments[0];
        assert_eq!(assignment.target.name.as_str(), "assoc");
        let subscript = assignment
            .target
            .subscript
            .as_ref()
            .expect("expected target subscript");
        assert_eq!(subscript.text.slice(input), "\"$key\"-suffix");

        let AssignmentValue::Compound(array) = &assignment.value else {
            panic!("expected compound assignment value");
        };
        assert_eq!(array.elements.len(), 2);
        let ArrayElem::Sequential(first) = &array.elements[0] else {
            panic!("expected first sequential element");
        };
        assert_eq!(first.span.slice(input), "\"$value\"");
        let ArrayElem::Sequential(second) = &array.elements[1] else {
            panic!("expected second sequential element");
        };
        assert_eq!(second.span.slice(input), "plain");
    }

    #[test]
    fn test_leaf_spans_track_words_assignments_and_redirects() {
        let script = Parser::new("foo=bar echo hi > out\n").parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(command.assignments[0].span.start.line, 1);
        assert_eq!(command.assignments[0].span.start.column, 1);
        assert_eq!(command.name.span.start.column, 9);
        assert_eq!(command.args[0].span.start.column, 14);
        assert_eq!(script.body[0].redirects[0].span.start.column, 17);
        assert_eq!(
            redirect_word_target(&script.body[0].redirects[0])
                .span
                .start
                .column,
            19
        );
    }

    #[test]
    fn test_word_part_spans_track_mixed_expansions() {
        let input = "echo pre${name:-fallback}$(printf hi)$((1+2))post\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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

        let WordPart::CommandSubstitution {
            body: commands,
            syntax,
        } = &parts[1].kind
        else {
            panic!("expected command substitution");
        };
        assert_eq!(*syntax, CommandSubstitutionSyntax::Backtick);

        let inner = expect_simple(&commands[0]);
        assert_eq!(inner.name.render(input), "printf");
        assert_eq!(inner.args[0].render(input), "hi");
    }

    #[test]
    fn test_escaped_backticks_inside_double_quotes_stay_literal() {
        let input = "echo \"pre \\`pwd\\` post\"\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.render(input), "pre `pwd` post");

        let WordPart::DoubleQuoted { parts, .. } = &word.parts[0].kind else {
            panic!("expected double-quoted word");
        };
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part.kind, WordPart::CommandSubstitution { .. }))
        );
    }

    #[test]
    fn test_brace_syntax_marks_unquoted_expansion_candidates() {
        let list_input = "{a,b}";
        let list = Parser::parse_word_string(list_input);
        assert_eq!(brace_slices(&list, list_input), vec!["{a,b}"]);
        assert_eq!(
            list.brace_syntax(),
            &[BraceSyntax {
                kind: BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList),
                span: list.span,
                quote_context: BraceQuoteContext::Unquoted,
            }]
        );
        assert!(list.has_active_brace_expansion());

        let sequence_input = "{1..3}";
        let sequence = Parser::parse_word_string(sequence_input);
        assert_eq!(brace_slices(&sequence, sequence_input), vec!["{1..3}"]);
        assert_eq!(
            sequence.brace_syntax()[0].kind,
            BraceSyntaxKind::Expansion(BraceExpansionKind::Sequence)
        );
        assert!(sequence.brace_syntax()[0].expands());
    }

    #[test]
    fn test_brace_syntax_marks_literal_and_quoted_brace_forms() {
        let literal_input = "HEAD@{1}";
        let literal = Parser::parse_word_string(literal_input);
        assert_eq!(brace_slices(&literal, literal_input), vec!["{1}"]);
        assert_eq!(literal.brace_syntax()[0].kind, BraceSyntaxKind::Literal);
        assert!(literal.brace_syntax()[0].treated_literally());
        assert!(!literal.has_active_brace_expansion());

        let quoted_input = "\"{a,b}\"";
        let quoted = Parser::parse_word_string(quoted_input);
        assert_eq!(brace_slices(&quoted, quoted_input), vec!["{a,b}"]);
        assert_eq!(
            quoted.brace_syntax()[0].kind,
            BraceSyntaxKind::Expansion(BraceExpansionKind::CommaList)
        );
        assert_eq!(
            quoted.brace_syntax()[0].quote_context,
            BraceQuoteContext::DoubleQuoted
        );
        assert!(quoted.brace_syntax()[0].treated_literally());
        assert!(!quoted.has_active_brace_expansion());
    }

    #[test]
    fn test_brace_syntax_marks_template_placeholders_inside_quotes() {
        let input = "\"$root/pkg/{{name}}/bin/{{cmd}}\"";
        let word = Parser::parse_word_string(input);

        assert_eq!(brace_slices(&word, input), vec!["{{name}}", "{{cmd}}"]);
        assert_eq!(word.brace_syntax().len(), 2);
        assert!(
            word.brace_syntax()
                .iter()
                .all(|brace| brace.kind == BraceSyntaxKind::TemplatePlaceholder)
        );
        assert!(
            word.brace_syntax()
                .iter()
                .all(|brace| brace.quote_context == BraceQuoteContext::DoubleQuoted)
        );
    }

    #[test]
    fn test_brace_syntax_ignores_escaped_unquoted_braces() {
        let word = Parser::parse_word_string("\\{a,b\\}");
        assert!(word.brace_syntax().is_empty());
        assert!(!word.has_active_brace_expansion());
    }

    #[test]
    fn test_dollar_quoted_words_preserve_quote_variants() {
        let input = "printf $'line\\n' $\"prefix $HOME\"\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        assert_eq!(word.parts.len(), 1);
        assert_eq!(
            word.part_span(0).unwrap().slice(input),
            "${arr[$RANDOM % ${#arr[@]}]}"
        );

        let reference = array_access_reference(&word.parts[0].kind).expect("expected array access");
        let subscript = reference.subscript.as_ref().expect("expected subscript");
        assert!(subscript.is_source_backed());
        assert_eq!(subscript.text.slice(input), "$RANDOM % ${#arr[@]}");
    }

    #[test]
    fn test_word_part_spans_track_parenthesized_arithmetic_expansion() {
        let input = "echo $((a <= (1 || 2)))\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, _, operand) = expect_parameter_operation_part(&word.parts[0].kind);
        let operand = operand.expect("expected operand");
        assert!(operand.is_source_backed());
        assert_eq!(operand.slice(input), "$(pwd)");
    }

    #[test]
    fn test_parameter_expansion_trim_operand_accepts_literal_left_brace_after_multiline_quote() {
        let input = "dns_servercow_info='ServerCow.de\nSite: ServerCow.de\n'\n\nf(){\n  if true; then\n    txtvalue_old=${response#*{\\\"name\\\":\\\"\"$_sub_domain\"\\\",\\\"ttl\\\":20,\\\"type\\\":\\\"TXT\\\",\\\"content\\\":\\\"}\n  fi\n}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Function(function) = &script.body[1].command else {
            panic!("expected function definition");
        };
        let (compound, redirects) = expect_compound(function.body.as_ref());
        let AstCompoundCommand::BraceGroup(body) = compound else {
            panic!("expected brace-group function body");
        };
        assert!(redirects.is_empty());
        let (if_compound, redirects) = expect_compound(&body[0]);
        let AstCompoundCommand::If(if_command) = if_compound else {
            panic!("expected if command");
        };
        assert!(redirects.is_empty());
        let command = expect_simple(&if_command.then_branch[0]);
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::RemovePrefixShort { pattern } = operator else {
            panic!("expected short-prefix trim operator");
        };
        assert!(pattern.render(input).contains("$_sub_domain"));
        assert!(pattern.parts.iter().any(|part| {
            matches!(
                &part.kind,
                PatternPart::Word(word)
                    if word.parts.iter().any(
                        |word_part| matches!(&word_part.kind, WordPart::Variable(name) if name == "_sub_domain")
                    )
            )
        }));
    }

    #[test]
    fn test_parameter_expansion_trim_operand_tracks_nested_parameter_expansions() {
        let input = "echo ${var#${prefix:-fallback}}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::RemovePrefixShort { pattern } = operator else {
            panic!("expected short-prefix trim operator");
        };
        assert_eq!(pattern.render(input), "${prefix:-fallback}");
        assert!(matches!(
            &pattern.parts[..],
            [PatternPartNode {
                kind: PatternPart::Word(word),
                ..
            }] if matches!(
                &word.parts[..],
                [WordPartNode {
                    kind: WordPart::Parameter(_) | WordPart::ParameterExpansion { .. },
                    ..
                }]
            )
        ));
    }

    #[test]
    fn test_parameter_replacement_pattern_stays_source_backed() {
        let input = "echo ${var/foo/bar}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } = operator
        else {
            panic!("expected replace-first operator");
        };

        assert_eq!(pattern.render(input), "foo");
        assert_eq!(pattern.parts.len(), 1);
        assert!(matches!(
            &pattern.parts[0].kind,
            PatternPart::Literal(text) if text.is_source_backed()
        ));
        assert!(replacement.is_source_backed());
        assert_eq!(replacement.slice(input), "bar");
    }

    #[test]
    fn test_parameter_trim_pattern_preserves_quoted_fragments_around_expansions() {
        let input = "echo ${var#\"pre\"$suffix'-'*}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::RemovePrefixShort { pattern } = operator else {
            panic!("expected short-prefix trim operator");
        };

        assert!(matches!(
            &pattern.parts[..],
            [
                PatternPartNode {
                    kind: PatternPart::Word(first),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Word(second),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Word(third),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::AnyString,
                    ..
                }
            ] if first.is_fully_quoted()
                && matches!(
                    &second.parts[..],
                    [WordPartNode {
                        kind: WordPart::Variable(name),
                        ..
                    }] if name.as_str() == "suffix"
                )
                && third.is_fully_quoted()
        ));
    }

    #[test]
    fn test_parameter_replacement_pattern_preserves_mixed_quote_fragments() {
        let input = "echo ${var//\"pre\"$suffix'-'/x}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::ReplaceAll {
            pattern,
            replacement,
        } = operator
        else {
            panic!("expected replace-all operator");
        };

        assert_eq!(
            pattern_part_slices(pattern, input),
            vec!["\"pre\"", "$suffix", "'-'"]
        );
        assert_eq!(replacement.slice(input), "x");
        assert!(matches!(
            &pattern.parts[..],
            [
                PatternPartNode {
                    kind: PatternPart::Word(first),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Word(second),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Word(third),
                    ..
                }
            ] if first.is_fully_quoted()
                && matches!(
                    &second.parts[..],
                    [WordPartNode {
                        kind: WordPart::Variable(name),
                        ..
                    }] if name.as_str() == "suffix"
                )
                && third.is_fully_quoted()
        ));
    }

    #[test]
    fn test_parameter_replacement_pattern_cooks_escaped_slash() {
        let input = r#"echo ${var/foo\/bar/baz}"#;
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let word = &command.args[0];

        let (_, operator, _) = expect_parameter_operation_part(&word.parts[0].kind);
        let ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        } = operator
        else {
            panic!("expected replace-first operator");
        };

        assert_eq!(pattern.render(input), "foo/bar");
        assert_eq!(pattern.parts.len(), 1);
        assert!(matches!(
            &pattern.parts[0].kind,
            PatternPart::Literal(text) if !text.is_source_backed() && text == "foo/bar"
        ));
        assert!(replacement.is_source_backed());
        assert_eq!(replacement.slice(input), "baz");
    }

    #[test]
    fn test_parse_arithmetic_command_preserves_exact_spans() {
        let input = "(( 1 +\n 2 <= 3 ))\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
            panic!("expected arithmetic compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.expr_span.unwrap().slice(input), "   ");
        assert!(command.expr_ast.is_none());
    }

    #[test]
    fn test_parse_arithmetic_command_with_nested_parens_and_double_right_paren() {
        let input = "(( (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) ))\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Arithmetic(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Subshell(commands) = compound else {
            panic!("expected outer subshell");
        };
        assert!(redirects.is_empty());
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0].command,
            AstCommand::Compound(AstCompoundCommand::Subshell(_))
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.condition_span.unwrap().slice(input), " n<(3-(1))");
    }

    #[test]
    fn test_parse_arithmetic_for_treats_spaced_less_than_left_paren_as_arithmetic() {
        let input = "for (( n=0; n<(3- (1)); n++ )) ; do echo $n; done\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.condition_span.unwrap().slice(input), " n<(3- (1))");
    }

    #[test]
    fn test_parse_arithmetic_for_accepts_brace_group_body() {
        let input = "for ((a=1; a <= 3; a++)) {\n  echo $a\n}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, redirects) = expect_compound(&script.body[0]);
        let AstCompoundCommand::ArithmeticFor(command) = compound else {
            panic!("expected arithmetic for compound command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.body.len(), 1);

        let (body_compound, body_redirects) = expect_compound(&command.body[0]);
        let AstCompoundCommand::BraceGroup(body) = body_compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Function(function) = &script.body[0].command else {
            panic!("expected function definition");
        };
        assert_eq!(function.name_span.slice(input), "my_fn");

        let (compound, _) = expect_compound(&script.body[1]);
        let AstCompoundCommand::For(command) = compound else {
            panic!("expected for loop");
        };
        assert_eq!(command.variable_span.slice(input), "item");

        let (compound, _) = expect_compound(&script.body[2]);
        let AstCompoundCommand::Select(command) = compound else {
            panic!("expected select loop");
        };
        assert_eq!(command.variable_span.slice(input), "choice");

        let AstCommand::Simple(command) = &script.body[3].command else {
            panic!("expected assignment-only simple command");
        };
        assert_eq!(command.assignments[0].target.name_span.slice(input), "foo");
        expect_subscript(&command.assignments[0].target, input, "10");

        let _command = expect_simple(&script.body[4]);
        assert_eq!(
            script.body[4].redirects[0]
                .fd_var_span
                .unwrap()
                .slice(input),
            "myfd"
        );

        let (compound, _) = expect_compound(&script.body[5]);
        let AstCompoundCommand::Coproc(command) = compound else {
            panic!("expected coproc command");
        };
        assert_eq!(command.name_span.unwrap().slice(input), "worker");
    }

    #[test]
    fn test_for_loop_words_consume_segmented_tokens_directly() {
        let input = "for item in foo\"bar\" 'baz'qux; do echo \"$item\"; done";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::For(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Case(command) = compound else {
            panic!("expected case command");
        };

        let patterns = &command.cases[0].patterns;
        assert_eq!(patterns.len(), 2);

        assert_eq!(patterns[0].render(input), "foobar");
        assert_eq!(patterns[0].parts.len(), 2);
        assert_eq!(
            pattern_part_slices(&patterns[0], input),
            vec!["foo", "\"bar\""]
        );
        assert!(matches!(
            &patterns[0].parts[1].kind,
            PatternPart::Word(word) if is_fully_quoted(word)
        ));

        assert_eq!(patterns[1].render(input), "bazqux");
        assert_eq!(patterns[1].parts.len(), 2);
        assert_eq!(
            pattern_part_slices(&patterns[1], input),
            vec!["'baz'", "qux"]
        );
        assert!(matches!(
            &patterns[1].parts[0].kind,
            PatternPart::Word(word) if is_fully_quoted(word)
        ));
    }

    #[test]
    fn test_parse_conditional_builds_structured_logical_ast() {
        let script = Parser::new("[[ ! (foo && bar) ]]\n").parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
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
        let input = "[[ foo == @(bar|baz)* ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        assert_eq!(binary.op, ConditionalBinaryOp::PatternEq);

        let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
            panic!("expected pattern rhs");
        };
        assert_eq!(pattern.render(input), "@(bar|baz)*");
        assert!(matches!(
            &pattern.parts[0].kind,
            PatternPart::Group {
                kind: PatternGroupKind::ExactlyOne,
                ..
            }
        ));
        assert!(matches!(&pattern.parts[1].kind, PatternPart::AnyString));
    }

    #[test]
    fn test_parse_conditional_var_ref_operand() {
        let input = "[[ -v assoc[$key] ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        assert_eq!(unary.op, ConditionalUnaryOp::VariableSet);

        let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
            panic!("expected typed var-ref operand");
        };
        assert_eq!(var_ref.name.as_str(), "assoc");
        assert_eq!(var_ref.name_span.slice(input), "assoc");
        expect_subscript(var_ref, input, "$key");
    }

    #[test]
    fn test_parse_conditional_var_ref_operand_preserves_quoted_subscript_syntax() {
        let input = "[[ -v assoc[\"key\"] ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
            panic!("expected typed var-ref operand");
        };

        let subscript = expect_subscript_syntax(var_ref, input, "\"key\"", "key");
        assert!(matches!(subscript.kind, SubscriptKind::Ordinary));
    }

    #[test]
    fn test_parse_conditional_var_ref_operand_preserves_spaced_zero_subscript() {
        let input = "[[ -v assoc[ 0 ] ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
            panic!("expected typed var-ref operand");
        };

        let subscript = expect_subscript(var_ref, input, " 0 ");
        assert!(matches!(
            subscript.arithmetic_ast.as_ref().map(|expr| &expr.kind),
            Some(ArithmeticExpr::Number(_))
        ));
    }

    #[test]
    fn test_parse_conditional_var_ref_operand_preserves_nested_arithmetic_subscript() {
        let input = "[[ -v assoc[$((0))] ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        let ConditionalExpr::VarRef(var_ref) = unary.expr.as_ref() else {
            panic!("expected typed var-ref operand");
        };

        let subscript = expect_subscript(var_ref, input, "$((0))");
        assert!(subscript.arithmetic_ast.is_some());
    }

    #[test]
    fn test_parse_conditional_non_direct_var_ref_falls_back_to_word() {
        let input = "[[ -v prefix$var ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Unary(unary) = &command.expression else {
            panic!("expected unary conditional");
        };
        let ConditionalExpr::Word(word) = unary.expr.as_ref() else {
            panic!("expected word fallback");
        };
        assert_eq!(word.render(input), "prefix$var");
    }

    #[test]
    fn test_parse_pattern_preserves_dynamic_fragments_inside_extglob() {
        let input = "[[ value == --@($choice|$prefix-'x') ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
            panic!("expected conditional compound command");
        };

        let ConditionalExpr::Binary(binary) = &command.expression else {
            panic!("expected binary conditional");
        };
        let ConditionalExpr::Pattern(pattern) = binary.right.as_ref() else {
            panic!("expected pattern rhs");
        };

        assert_eq!(pattern.render(input), "--@($choice|$prefix-x)");
        let PatternPart::Group { patterns, .. } = &pattern.parts[1].kind else {
            panic!("expected extglob group");
        };
        assert!(matches!(
            &patterns[0].parts[..],
            [PatternPartNode {
                kind: PatternPart::Word(word),
                ..
            }] if matches!(
                &word.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }]
                if name == "choice"
            )
        ));
        assert!(matches!(
            &patterns[1].parts[..],
            [
                PatternPartNode {
                    kind: PatternPart::Word(variable),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Literal(text),
                    ..
                },
                PatternPartNode {
                    kind: PatternPart::Word(quoted),
                    ..
                }
            ] if matches!(
                &variable.parts[..],
                [WordPartNode {
                    kind: WordPart::Variable(name),
                    ..
                }]
                if name == "prefix"
            ) && text.as_str(input, patterns[1].parts[1].span) == "-" && is_fully_quoted(quoted)
        ));
    }

    #[test]
    fn test_parse_conditional_regex_rhs_preserves_structure() {
        let input = "[[ foo =~ [ab](c|d) ]]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let (compound, _) = expect_compound(&script.body[0]);
        let AstCompoundCommand::Conditional(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[hello\"]\"");
    }

    #[test]
    fn test_parse_glob_word_with_command_sub_in_bracket_expression_stays_single_arg() {
        let input = "echo [$(echo abc)]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[$(echo abc)]");
    }

    #[test]
    fn test_parse_glob_word_with_extglob_chars_stays_single_arg() {
        let input = "echo [+()]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[+()]");
    }

    #[test]
    fn test_parse_glob_word_with_trailing_literal_right_paren_stays_single_arg() {
        let input = "echo [+(])\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0].span.slice(input), "[+(])");
    }

    #[test]
    fn test_parse_glob_of_unescaped_double_left_bracket_stays_word() {
        let input = "echo [[z] []z]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
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

        let script = Parser::new(input).parse().unwrap().file;
        assert_eq!(script.body.len(), 6);
    }

    #[test]
    fn test_command_substitution_spans_are_absolute() {
        let script = Parser::new("out=$(\n  printf '%s\\n' $x\n)\n")
            .parse()
            .unwrap()
            .file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let AssignmentValue::Scalar(word) = &command.assignments[0].value else {
            panic!("expected scalar assignment");
        };
        let WordPart::CommandSubstitution {
            body: commands,
            syntax,
        } = &word.parts[0].kind
        else {
            panic!("expected command substitution");
        };
        assert_eq!(*syntax, CommandSubstitutionSyntax::DollarParen);
        let inner = expect_simple(&commands[0]);

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
            .file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };
        let WordPart::ProcessSubstitution {
            body: commands,
            is_input,
        } = &command.args[0].parts[0].kind
        else {
            panic!("expected process substitution");
        };
        assert!(*is_input);

        let inner = expect_simple(&commands[0]);
        assert_eq!(inner.name.span.start.line, 2);
        assert_eq!(inner.name.span.start.column, 3);
        assert_eq!(inner.args[1].span.start.column, 17);
    }

    #[test]
    fn test_parse_declare_clause_classifies_operands_and_prefix_assignments() {
        let input = "FOO=1 declare -a arr=(\"hello world\" two) bar >out\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
            panic!("expected declaration clause");
        };

        assert_eq!(command.variant, "declare");
        assert_eq!(command.variant_span.slice(input), "declare");
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].target.name, "FOO");
        assert_eq!(script.body[0].redirects.len(), 1);
        assert_eq!(
            redirect_word_target(&script.body[0].redirects[0])
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
        assert_eq!(assignment.target.name, "arr");
        let AssignmentValue::Compound(array) = &assignment.value else {
            panic!("expected compound array assignment");
        };
        assert_eq!(array.kind, ArrayKind::Indexed);
        assert_eq!(array.elements.len(), 2);
        let ArrayElem::Sequential(first) = &array.elements[0] else {
            panic!("expected first sequential element");
        };
        assert!(is_fully_quoted(first));
        assert_eq!(first.span.slice(input), "\"hello world\"");
        let ArrayElem::Sequential(second) = &array.elements[1] else {
            panic!("expected second sequential element");
        };
        assert_eq!(second.span.slice(input), "two");

        let DeclOperand::Name(name) = &command.operands[2] else {
            panic!("expected bare name operand");
        };
        assert_eq!(name.name, "bar");
    }

    #[test]
    fn test_parse_declare_a_threads_associative_kind_into_compound_array() {
        let input = "declare -A assoc=(one [foo]=bar [bar]+=baz two)\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
            panic!("expected declaration clause");
        };

        let DeclOperand::Assignment(assignment) = &command.operands[1] else {
            panic!("expected assignment operand, got {:#?}", command.operands);
        };
        let AssignmentValue::Compound(array) = &assignment.value else {
            panic!("expected compound array assignment");
        };

        assert_eq!(array.kind, ArrayKind::Associative);
        assert_eq!(array.elements.len(), 4);
        assert!(matches!(array.elements[0], ArrayElem::Sequential(_)));

        let ArrayElem::Keyed { key, .. } = &array.elements[1] else {
            panic!("expected keyed element");
        };
        assert_eq!(key.text.slice(input), "foo");
        assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

        let ArrayElem::KeyedAppend { key, .. } = &array.elements[2] else {
            panic!("expected keyed append element");
        };
        assert_eq!(key.text.slice(input), "bar");
        assert_eq!(key.interpretation, SubscriptInterpretation::Associative);

        assert!(matches!(array.elements[3], ArrayElem::Sequential(_)));
    }

    #[test]
    fn test_parse_parameter_expansion_preserves_quoted_associative_subscripts() {
        let input = "printf '%s\\n' ${assoc[\"key\"]} ${assoc['k']}\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        let first = expect_array_access(&command.args[1]);
        let second = expect_array_access(&command.args[2]);

        let first_subscript = expect_subscript_syntax(first, input, "\"key\"", "key");
        assert!(matches!(first_subscript.kind, SubscriptKind::Ordinary));
        assert_eq!(command.args[1].render_syntax(input), "${assoc[\"key\"]}");

        let second_subscript = expect_subscript_syntax(second, input, "'k'", "k");
        assert!(matches!(second_subscript.kind, SubscriptKind::Ordinary));
        assert_eq!(command.args[2].render_syntax(input), "${assoc['k']}");
    }

    #[test]
    fn test_parse_prefix_match_preserves_selector_kind() {
        let input = "printf '%s\\n' \"${!prefix@}\" \"${!prefix*}\"\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Simple(command) = &script.body[0].command else {
            panic!("expected simple command");
        };

        let first = &command.args[1];
        let second = &command.args[2];

        let [first_part] = first.parts.as_slice() else {
            panic!("expected quoted prefix match");
        };
        let WordPart::DoubleQuoted {
            parts: first_inner, ..
        } = &first_part.kind
        else {
            panic!("expected double-quoted prefix match");
        };
        let (prefix, kind) = expect_prefix_match_part(&first_inner[0].kind);
        assert_eq!(prefix.as_str(), "prefix");
        assert_eq!(kind, PrefixMatchKind::At);

        let [second_part] = second.parts.as_slice() else {
            panic!("expected quoted prefix match");
        };
        let WordPart::DoubleQuoted {
            parts: second_inner,
            ..
        } = &second_part.kind
        else {
            panic!("expected double-quoted prefix match");
        };
        let (prefix, kind) = expect_prefix_match_part(&second_inner[0].kind);
        assert_eq!(prefix.as_str(), "prefix");
        assert_eq!(kind, PrefixMatchKind::Star);
        assert_eq!(first.render_syntax(input), "\"${!prefix@}\"");
        assert_eq!(second.render_syntax(input), "\"${!prefix*}\"");
    }

    #[test]
    fn test_parse_declare_a_preserves_quoted_associative_keys() {
        let input = "declare -A assoc=([\"key\"]=bar ['alt']+=baz)\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
            panic!("expected declaration clause");
        };

        let DeclOperand::Assignment(assignment) = &command.operands[1] else {
            panic!("expected assignment operand");
        };
        let AssignmentValue::Compound(array) = &assignment.value else {
            panic!("expected compound array assignment");
        };

        let ArrayElem::Keyed { key, .. } = &array.elements[0] else {
            panic!("expected keyed element");
        };
        assert_eq!(key.text.slice(input), "key");
        assert_eq!(key.syntax_text(input), "\"key\"");

        let ArrayElem::KeyedAppend { key, .. } = &array.elements[1] else {
            panic!("expected keyed append element");
        };
        assert_eq!(key.text.slice(input), "alt");
        assert_eq!(key.syntax_text(input), "'alt'");
    }

    #[test]
    fn test_parse_export_uses_dynamic_operand_for_invalid_assignment() {
        let script = Parser::new("export foo-bar=(one two)\n")
            .parse()
            .unwrap()
            .file;

        let AstCommand::Decl(command) = &script.body[0].command else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
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
        assert_eq!(assignment.target.name, "VAR");
        assert!(
            matches!(&assignment.value, AssignmentValue::Scalar(value) if value.span.slice(input) == "value")
        );

        let DeclOperand::Name(name) = &command.operands[2] else {
            panic!("expected bare name operand");
        };
        assert_eq!(name.name, "other");
    }

    #[test]
    fn test_parse_declaration_name_operand_preserves_nested_arithmetic_subscript() {
        let input = "declare assoc[$((0))]\n";
        let script = Parser::new(input).parse().unwrap().file;

        let AstCommand::Decl(command) = &script.body[0].command else {
            panic!("expected declaration clause");
        };

        let DeclOperand::Name(name) = &command.operands[0] else {
            panic!("expected declaration name operand");
        };
        let subscript = expect_subscript(name, input, "$((0))");
        assert!(subscript.arithmetic_ast.is_some());
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
        let script = Parser::new(input).parse().unwrap().file;

        let Some(command) = script.body.last() else {
            panic!("expected final command to be a for loop");
        };
        let (compound, _) = expect_compound(command);
        let AstCompoundCommand::For(command) = compound else {
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
        let script = Parser::new(input).parse().unwrap().file;

        let Some(command) = script.body.last() else {
            panic!("expected final command to be a brace group");
        };
        let (compound, _) = expect_compound(command);
        let AstCompoundCommand::BraceGroup(commands) = compound else {
            panic!("expected final command to be a brace group");
        };
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0].command, AstCommand::Simple(_)));
        assert!(matches!(commands[1].command, AstCommand::Simple(_)));
    }

    #[test]
    fn test_alias_expansion_can_open_a_subshell() {
        let input = "\
shopt -s expand_aliases
alias LEFT='('
LEFT echo one; echo two )
";
        let script = Parser::new(input).parse().unwrap().file;

        let Some(command) = script.body.last() else {
            panic!("expected final command to be a subshell");
        };
        let (compound, _) = expect_compound(command);
        let AstCompoundCommand::Subshell(commands) = compound else {
            panic!("expected final command to be a subshell");
        };
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0].command, AstCommand::Simple(_)));
        assert!(matches!(commands[1].command, AstCommand::Simple(_)));
    }

    #[test]
    fn test_alias_expansion_with_trailing_space_expands_next_word() {
        let input = "\
shopt -s expand_aliases
alias greet='echo '
alias subject='hello'
greet subject
";
        let script = Parser::new(input).parse().unwrap().file;

        let Some(stmt) = script.body.last() else {
            panic!("expected final command to be a simple command");
        };
        let command = expect_simple(stmt);

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
        let script = Parser::new(input).parse().unwrap().file;

        let Some(stmt) = script.body.last() else {
            panic!("expected final command to be a simple command");
        };
        let command = expect_simple(stmt);

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
        let comments = collect_file_comments(&output.file);
        for (i, comment) in comments.iter().enumerate() {
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
        assert_eq!(collect_file_comments(&output.file).len(), 3);
        assert_comment_ranges_valid(source, &output);
    }

    #[test]
    fn test_comment_ranges_with_unicode() {
        let source = "# café résumé\necho ok\n# 你好世界\n";
        let output = Parser::new(source).parse().unwrap();
        assert_eq!(collect_file_comments(&output.file).len(), 2);
        assert_comment_ranges_valid(source, &output);
    }

    #[test]
    fn test_comment_ranges_heredoc_no_false_comments() {
        // Lines with # inside a heredoc must NOT produce Comment entries
        let source = "cat <<EOF\n# not a comment\nline two\nEOF\n# real\n";
        let output = Parser::new(source).parse().unwrap();
        assert_comment_ranges_valid(source, &output);
        // Only the real comment after EOF should be collected
        let texts: Vec<&str> = collect_file_comments(&output.file)
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
        let texts: Vec<&str> = collect_file_comments(&output.file)
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
        Parser::with_dialect("[[ foo == bar ]]\n", ShellDialect::Zsh)
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

    #[test]
    fn test_zsh_dialect_accepts_c_style_for_loops() {
        Parser::with_dialect(
            "for ((i=0; i<2; i++)); do echo hi; done\n",
            ShellDialect::Zsh,
        )
        .parse()
        .unwrap();
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_parses_star_dot() {
        let source = "print *(.)\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let glob = expect_zsh_qualified_glob(&command.args[0]);

        assert_eq!(glob.span.slice(source), "*(.)");
        assert_eq!(command.args[0].span.slice(source), "*(.)");
        assert_eq!(glob.pattern.render_syntax(source), "*");
        assert_eq!(glob.qualifiers.span.slice(source), "(.)");
        assert!(matches!(
            glob.qualifiers.fragments.as_slice(),
            [ZshGlobQualifier::Flag { name: '.', span }] if span.slice(source) == "."
        ));
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_parses_star_slash() {
        let source = "print *(/)\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let glob = expect_zsh_qualified_glob(&command.args[0]);

        assert_eq!(glob.span.slice(source), "*(/)");
        assert_eq!(glob.pattern.render_syntax(source), "*");
        assert_eq!(glob.qualifiers.span.slice(source), "(/)");
        assert!(matches!(
            glob.qualifiers.fragments.as_slice(),
            [ZshGlobQualifier::Flag { name: '/', span }] if span.slice(source) == "/"
        ));
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_parses_star_n() {
        let source = "print *(N)\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let glob = expect_zsh_qualified_glob(&command.args[0]);

        assert_eq!(glob.span.slice(source), "*(N)");
        assert_eq!(glob.pattern.render_syntax(source), "*");
        assert_eq!(glob.qualifiers.span.slice(source), "(N)");
        assert!(matches!(
            glob.qualifiers.fragments.as_slice(),
            [ZshGlobQualifier::Flag { name: 'N', span }] if span.slice(source) == "N"
        ));
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_parses_recursive_pattern_with_letter_sequence_and_range() {
        let source = "print **/*(.om[1,3])\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let glob = expect_zsh_qualified_glob(&command.args[0]);

        assert_eq!(glob.span.slice(source), "**/*(.om[1,3])");
        assert_eq!(glob.pattern.render_syntax(source), "**/*");
        assert_eq!(glob.qualifiers.span.slice(source), "(.om[1,3])");

        let [
            ZshGlobQualifier::Flag {
                name: '.',
                span: dot_span,
            },
            ZshGlobQualifier::LetterSequence {
                text,
                span: letters_span,
            },
            ZshGlobQualifier::NumericArgument {
                span: range_span,
                start,
                end: Some(end),
            },
        ] = glob.qualifiers.fragments.as_slice()
        else {
            panic!("expected dot, letter sequence, and numeric range qualifiers");
        };

        assert_eq!(dot_span.slice(source), ".");
        assert_eq!(letters_span.slice(source), "om");
        assert_eq!(text.slice(source), "om");
        assert_eq!(range_span.slice(source), "[1,3]");
        assert_eq!(start.slice(source), "1");
        assert_eq!(end.slice(source), "3");
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_parses_prefixed_glob_with_negation() {
        let source = "print foo*(^-)\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };
        let glob = expect_zsh_qualified_glob(&command.args[0]);

        assert_eq!(glob.span.slice(source), "foo*(^-)");
        assert_eq!(glob.pattern.render_syntax(source), "foo*");
        assert_eq!(glob.qualifiers.span.slice(source), "(^-)");
        let [
            ZshGlobQualifier::Negation {
                span: negation_span,
            },
            ZshGlobQualifier::Flag {
                name: '-',
                span: flag_span,
            },
        ] = glob.qualifiers.fragments.as_slice()
        else {
            panic!("expected negation and dash flag qualifiers");
        };
        assert_eq!(negation_span.slice(source), "^");
        assert_eq!(flag_span.slice(source), "-");
    }

    #[test]
    fn test_zsh_trailing_glob_qualifier_falls_back_for_out_of_scope_group() {
        let source = "print *(#q.)\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let AstCommand::Simple(command) = &output.file.body[0].command else {
            panic!("expected simple command");
        };

        assert_eq!(command.args[0].span.slice(source), "*(#q.)");
        assert!(!matches!(
            command.args[0].parts.as_slice(),
            [WordPartNode {
                kind: WordPart::ZshQualifiedGlob(_),
                ..
            }]
        ));
    }

    #[test]
    fn test_non_zsh_dialects_do_not_special_case_trailing_glob_qualifiers() {
        let source = "print **/*(.om[1,3])\n";

        for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
            let output = Parser::with_dialect(source, dialect).parse().unwrap();
            let AstCommand::Simple(command) = &output.file.body[0].command else {
                panic!("expected simple command");
            };

            assert_eq!(command.args[0].span.slice(source), "**/*(.om[1,3])");
            assert!(!matches!(
                command.args[0].parts.as_slice(),
                [WordPartNode {
                    kind: WordPart::ZshQualifiedGlob(_),
                    ..
                }]
            ));
        }
    }

    #[test]
    fn test_zsh_repeat_do_done_preserves_structure_and_spans() {
        let source = "repeat 3; do echo hi; done\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let (compound, redirects) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected repeat command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(source), "repeat 3; do echo hi; done");
        assert_eq!(command.count.span.slice(source), "3");
        assert_eq!(command.body.len(), 1);
        assert_eq!(command.body.span.slice(source), "echo hi; ");

        match command.syntax {
            RepeatSyntax::DoDone { do_span, done_span } => {
                assert_eq!(do_span.slice(source), "do");
                assert_eq!(done_span.slice(source), "done");
            }
            RepeatSyntax::Brace { .. } => panic!("expected do/done repeat syntax"),
        }

        let body_command = expect_simple(&command.body[0]);
        assert_eq!(body_command.name.render(source), "echo");
        assert_eq!(body_command.args[0].render(source), "hi");
    }

    #[test]
    fn test_zsh_repeat_brace_preserves_structure_and_spans() {
        let source = "repeat 3 { echo hi; }\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let (compound, redirects) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::Repeat(command) = compound else {
            panic!("expected repeat command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(source), "repeat 3 { echo hi; }");
        assert_eq!(command.count.span.slice(source), "3");
        assert_eq!(command.body.len(), 1);
        assert_eq!(command.body.span.slice(source), "echo hi; ");

        match command.syntax {
            RepeatSyntax::Brace {
                left_brace_span,
                right_brace_span,
            } => {
                assert_eq!(left_brace_span.slice(source), "{");
                assert_eq!(right_brace_span.slice(source), "}");
            }
            RepeatSyntax::DoDone { .. } => panic!("expected brace repeat syntax"),
        }

        let body_command = expect_simple(&command.body[0]);
        assert_eq!(body_command.name.render(source), "echo");
        assert_eq!(body_command.args[0].render(source), "hi");
    }

    #[test]
    fn test_zsh_foreach_paren_brace_preserves_structure_and_spans() {
        let source = "foreach x (a b c) { echo $x; }\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let (compound, redirects) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::Foreach(command) = compound else {
            panic!("expected foreach command");
        };

        assert!(redirects.is_empty());
        assert_eq!(command.span.slice(source), "foreach x (a b c) { echo $x; }");
        assert_eq!(command.variable.as_str(), "x");
        assert_eq!(command.variable_span.slice(source), "x");
        assert_eq!(
            command
                .words
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(command.body.len(), 1);
        assert_eq!(command.body.span.slice(source), "echo $x; ");

        match command.syntax {
            ForeachSyntax::ParenBrace {
                left_paren_span,
                right_paren_span,
                left_brace_span,
                right_brace_span,
            } => {
                assert_eq!(left_paren_span.slice(source), "(");
                assert_eq!(right_paren_span.slice(source), ")");
                assert_eq!(left_brace_span.slice(source), "{");
                assert_eq!(right_brace_span.slice(source), "}");
            }
            ForeachSyntax::InDoDone { .. } => panic!("expected paren/brace foreach syntax"),
        }

        let body_command = expect_simple(&command.body[0]);
        assert_eq!(body_command.name.render(source), "echo");
        assert_eq!(body_command.args[0].render(source), "$x");
    }

    #[test]
    fn test_zsh_foreach_in_do_done_preserves_structure_and_spans() {
        let source = "foreach x in a b c; do echo $x; done\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let (compound, redirects) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::Foreach(command) = compound else {
            panic!("expected foreach command");
        };

        assert!(redirects.is_empty());
        assert_eq!(
            command.span.slice(source),
            "foreach x in a b c; do echo $x; done"
        );
        assert_eq!(command.variable.as_str(), "x");
        assert_eq!(command.variable_span.slice(source), "x");
        assert_eq!(
            command
                .words
                .iter()
                .map(|word| word.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(command.body.len(), 1);
        assert_eq!(command.body.span.slice(source), "echo $x; ");

        match command.syntax {
            ForeachSyntax::InDoDone {
                in_span,
                do_span,
                done_span,
            } => {
                assert_eq!(in_span.slice(source), "in");
                assert_eq!(do_span.slice(source), "do");
                assert_eq!(done_span.slice(source), "done");
            }
            ForeachSyntax::ParenBrace { .. } => panic!("expected in/do/done foreach syntax"),
        }

        let body_command = expect_simple(&command.body[0]);
        assert_eq!(body_command.name.render(source), "echo");
        assert_eq!(body_command.args[0].render(source), "$x");
    }

    #[test]
    fn test_non_zsh_dialects_reject_repeat_and_foreach_forms() {
        for dialect in [ShellDialect::Bash, ShellDialect::Posix, ShellDialect::Mksh] {
            for source in [
                "repeat 3; do echo hi; done\n",
                "repeat 3 { echo hi; }\n",
                "foreach x (a b c) { echo $x; }\n",
                "foreach x in a b c; do echo $x; done\n",
            ] {
                let error = Parser::with_dialect(source, dialect).parse().unwrap_err();
                assert!(
                    matches!(error, Error::Parse { .. }),
                    "expected parse error for {dialect:?} on {source:?}, got {error:?}"
                );
            }
        }
    }

    #[test]
    fn test_zsh_parameter_modifier_records_modifier_and_target() {
        let source = "print ${(m)foo} ${(%):-%x}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        let first = expect_parameter(&command.args[0]);
        assert_eq!(first.raw_body.slice(source), "(m)foo");
        let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert_eq!(
            first
                .modifiers
                .iter()
                .map(|modifier| modifier.name)
                .collect::<Vec<_>>(),
            vec!['m']
        );
        let ZshExpansionTarget::Reference(reference) = &first.target else {
            panic!("expected direct zsh reference target");
        };
        assert_eq!(reference.name.as_str(), "foo");
        assert!(first.operation.is_none());

        let second = expect_parameter(&command.args[1]);
        assert_eq!(second.raw_body.slice(source), "(%):-%x");
        let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert_eq!(
            second
                .modifiers
                .iter()
                .map(|modifier| modifier.name)
                .collect::<Vec<_>>(),
            vec!['%']
        );
        assert!(matches!(second.target, ZshExpansionTarget::Empty));
        assert!(matches!(
            second.operation,
            Some(ZshExpansionOperation::Defaulting {
                kind: ZshDefaultingOp::UseDefault,
                ref operand,
                colon_variant: true,
            }) if operand.slice(source) == "%x"
        ));
    }

    #[test]
    fn test_zsh_nested_parameter_modifier_records_nested_target_and_pattern_operation() {
        let source = "print ${(M)${(k)parameters[@]}:#__gitcomp_builtin_*}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);
        let parameter = expect_parameter(&command.args[0]);

        let ParameterExpansionSyntax::Zsh(parameter) = &parameter.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert_eq!(
            parameter
                .modifiers
                .iter()
                .map(|modifier| modifier.name)
                .collect::<Vec<_>>(),
            vec!['M']
        );
        let ZshExpansionTarget::Nested(inner) = &parameter.target else {
            panic!("expected nested zsh parameter target");
        };
        let ParameterExpansionSyntax::Zsh(inner) = &inner.syntax else {
            panic!("expected nested zsh syntax");
        };
        assert_eq!(
            inner
                .modifiers
                .iter()
                .map(|modifier| modifier.name)
                .collect::<Vec<_>>(),
            vec!['k']
        );
        let ZshExpansionTarget::Reference(reference) = &inner.target else {
            panic!("expected nested reference target");
        };
        assert_eq!(reference.name.as_str(), "parameters");
        assert!(reference.has_array_selector());
        assert!(matches!(
            parameter.operation,
            Some(ZshExpansionOperation::PatternOperation {
                kind: ZshPatternOp::Filter,
                ref operand,
            }) if operand.slice(source) == "__gitcomp_builtin_*"
        ));
    }

    #[test]
    fn test_zsh_parameter_supported_operations_are_typed_and_preserve_source_spans() {
        let source = "print ${(m)foo#${needle}} ${(S)foo//\"pre\"$suffix/$replacement} ${(m)foo:$offset:${length}} ${(m)foo:^other}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        let trim = expect_parameter(&command.args[0]);
        assert_eq!(trim.raw_body.slice(source), "(m)foo#${needle}");
        let ParameterExpansionSyntax::Zsh(trim) = &trim.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            trim.operation,
            Some(ZshExpansionOperation::TrimOperation {
                kind: ZshTrimOp::RemovePrefixShort,
                ref operand,
            }) if operand.is_source_backed() && operand.slice(source) == "${needle}"
        ));

        let replacement = expect_parameter(&command.args[1]);
        assert_eq!(
            replacement.raw_body.slice(source),
            "(S)foo//\"pre\"$suffix/$replacement"
        );
        let ParameterExpansionSyntax::Zsh(replacement) = &replacement.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            replacement.operation,
            Some(ZshExpansionOperation::ReplacementOperation {
                kind: ZshReplacementOp::ReplaceAll,
                ref pattern,
                replacement: Some(ref replacement),
            }) if pattern.is_source_backed()
                && pattern.slice(source) == "\"pre\"$suffix"
                && replacement.is_source_backed()
                && replacement.slice(source) == "$replacement"
        ));

        let slice = expect_parameter(&command.args[2]);
        assert_eq!(slice.raw_body.slice(source), "(m)foo:$offset:${length}");
        let ParameterExpansionSyntax::Zsh(slice) = &slice.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            slice.operation,
            Some(ZshExpansionOperation::Slice {
                ref offset,
                length: Some(ref length),
            }) if offset.is_source_backed()
                && offset.slice(source) == "$offset"
                && length.is_source_backed()
                && length.slice(source) == "${length}"
        ));

        let unknown = expect_parameter(&command.args[3]);
        assert_eq!(unknown.raw_body.slice(source), "(m)foo:^other");
        let ParameterExpansionSyntax::Zsh(unknown) = &unknown.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            unknown.operation,
            Some(ZshExpansionOperation::Unknown(ref operand))
                if operand.is_source_backed() && operand.slice(source) == ":^other"
        ));
    }

    #[test]
    fn test_zsh_parameter_operation_kinds_cover_long_trim_and_anchored_replacement() {
        let source = "print ${(m)foo##pre*} ${(m)foo%%post*} ${(S)foo/#$prefix/$replacement} ${(S)foo/%$suffix/$replacement}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let command = expect_simple(&output.file.body[0]);

        let first = expect_parameter(&command.args[0]);
        let ParameterExpansionSyntax::Zsh(first) = &first.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            first.operation,
            Some(ZshExpansionOperation::TrimOperation {
                kind: ZshTrimOp::RemovePrefixLong,
                ref operand,
            }) if operand.slice(source) == "pre*"
        ));

        let second = expect_parameter(&command.args[1]);
        let ParameterExpansionSyntax::Zsh(second) = &second.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            second.operation,
            Some(ZshExpansionOperation::TrimOperation {
                kind: ZshTrimOp::RemoveSuffixLong,
                ref operand,
            }) if operand.slice(source) == "post*"
        ));

        let third = expect_parameter(&command.args[2]);
        let ParameterExpansionSyntax::Zsh(third) = &third.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            third.operation,
            Some(ZshExpansionOperation::ReplacementOperation {
                kind: ZshReplacementOp::ReplacePrefix,
                ref pattern,
                replacement: Some(ref replacement),
            }) if pattern.slice(source) == "$prefix" && replacement.slice(source) == "$replacement"
        ));

        let fourth = expect_parameter(&command.args[3]);
        let ParameterExpansionSyntax::Zsh(fourth) = &fourth.syntax else {
            panic!("expected zsh parameter syntax");
        };
        assert!(matches!(
            fourth.operation,
            Some(ZshExpansionOperation::ReplacementOperation {
                kind: ZshReplacementOp::ReplaceSuffix,
                ref pattern,
                replacement: Some(ref replacement),
            }) if pattern.slice(source) == "$suffix" && replacement.slice(source) == "$replacement"
        ));
    }

    #[test]
    fn test_zsh_brace_if_records_brace_syntax() {
        let source = "if [[ -n $foo ]] { print foo; } elif [[ -n $bar ]] { print bar; } else { print baz; }\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let (compound, _) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::If(command) = compound else {
            panic!("expected if command");
        };

        assert!(matches!(
            command.syntax,
            IfSyntax::Brace {
                left_brace_span,
                right_brace_span,
            } if left_brace_span.slice(source) == "{" && right_brace_span.slice(source) == "}"
        ));
        assert_eq!(command.elif_branches.len(), 1);
        assert!(command.else_branch.is_some());
    }

    #[test]
    fn test_zsh_always_and_background_operators_preserve_surface_forms() {
        let source = "\
{ print body; } always { print cleanup; }
print quiet &|
print hidden &!
";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();

        let (compound, _) = expect_compound(&output.file.body[0]);
        let AstCompoundCommand::Always(command) = compound else {
            panic!("expected always compound command");
        };
        assert_eq!(command.body.len(), 1);
        assert_eq!(command.always_body.len(), 1);

        assert_eq!(
            output.file.body[1].terminator,
            Some(StmtTerminator::Background(BackgroundOperator::Pipe))
        );
        assert_eq!(
            output.file.body[2].terminator,
            Some(StmtTerminator::Background(BackgroundOperator::Bang))
        );
    }
}
