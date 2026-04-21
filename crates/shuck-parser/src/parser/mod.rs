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
use memchr::{memchr, memchr2, memchr3};
use smallvec::SmallVec;

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
    ForeachSyntax, FunctionDef, FunctionHeader, FunctionHeaderEntry, Heredoc, HeredocBody,
    HeredocBodyMode, HeredocBodyPart, HeredocBodyPartNode, HeredocDelimiter, IfCommand, IfSyntax,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStatus {
    Clean,
    Recovered,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZshCaseGroupPart {
    pub pattern_part_index: usize,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxFacts {
    pub zsh_brace_if_spans: Vec<Span>,
    pub zsh_always_spans: Vec<Span>,
    pub zsh_case_group_parts: Vec<ZshCaseGroupPart>,
}

/// The result of parsing a script, including any recovery diagnostics and
/// syntax facts collected along the way.
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub file: File,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub status: ParseStatus,
    pub terminal_error: Option<Error>,
    pub syntax_facts: SyntaxFacts,
}

impl ParseResult {
    pub fn is_ok(&self) -> bool {
        self.status == ParseStatus::Clean
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }

    pub fn strict_error(&self) -> Error {
        self.terminal_error.clone().unwrap_or_else(|| {
            let diagnostic = self
                .diagnostics
                .first()
                .expect("non-clean parse result should include a diagnostic or terminal error");
            Error::parse_at(
                diagnostic.message.clone(),
                diagnostic.span.start.line,
                diagnostic.span.start.column,
            )
        })
    }

    pub fn unwrap(self) -> Self {
        if self.is_ok() {
            self
        } else {
            panic!(
                "called `ParseResult::unwrap()` on a non-clean parse: {}",
                self.strict_error()
            )
        }
    }

    pub fn expect(self, message: &str) -> Self {
        if self.is_ok() {
            self
        } else {
            panic!("{message}: {}", self.strict_error())
        }
    }

    pub fn unwrap_err(self) -> Error {
        if self.is_err() {
            self.strict_error()
        } else {
            panic!("called `ParseResult::unwrap_err()` on a clean parse")
        }
    }

    pub fn expect_err(self, message: &str) -> Error {
        if self.is_err() {
            self.strict_error()
        } else {
            panic!("{message}")
        }
    }
}

#[cfg(feature = "benchmarking")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct ParserBenchmarkCounters {
    pub lexer_current_position_calls: u64,
    pub parser_set_current_spanned_calls: u64,
    pub parser_advance_raw_calls: u64,
}

#[derive(Debug, Clone)]
struct SimpleCommand {
    name: Word,
    args: SmallVec<[Word; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
    span: Span,
}

#[derive(Debug, Clone)]
struct BreakCommand {
    depth: Option<Word>,
    extra_args: SmallVec<[Word; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ContinueCommand {
    depth: Option<Word>,
    extra_args: SmallVec<[Word; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ReturnCommand {
    code: Option<Word>,
    extra_args: SmallVec<[Word; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
    span: Span,
}

#[derive(Debug, Clone)]
struct ExitCommand {
    code: Option<Word>,
    extra_args: SmallVec<[Word; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
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
    operands: SmallVec<[DeclOperand; 2]>,
    redirects: SmallVec<[Redirect; 1]>,
    assignments: SmallVec<[Assignment; 1]>,
    span: Span,
}

#[derive(Debug, Clone)]
enum Command {
    Simple(SimpleCommand),
    Builtin(BuiltinCommand),
    Decl(Box<DeclClause>),
    Compound(Box<CompoundCommand>, SmallVec<[Redirect; 1]>),
    Function(FunctionDef),
    AnonymousFunction(AnonymousFunctionCommand, SmallVec<[Redirect; 1]>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellDialect {
    Posix,
    Mksh,
    #[default]
    Bash,
    Zsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OptionValue {
    On,
    Off,
    #[default]
    Unknown,
}

impl OptionValue {
    pub const fn is_definitely_on(self) -> bool {
        matches!(self, Self::On)
    }

    pub const fn is_definitely_off(self) -> bool {
        matches!(self, Self::Off)
    }

    pub const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::On, Self::On) => Self::On,
            (Self::Off, Self::Off) => Self::Off,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZshEmulationMode {
    Zsh,
    Sh,
    Ksh,
    Csh,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZshOptionState {
    pub sh_word_split: OptionValue,
    pub glob_subst: OptionValue,
    pub rc_expand_param: OptionValue,
    pub glob: OptionValue,
    pub nomatch: OptionValue,
    pub null_glob: OptionValue,
    pub csh_null_glob: OptionValue,
    pub extended_glob: OptionValue,
    pub ksh_glob: OptionValue,
    pub sh_glob: OptionValue,
    pub bare_glob_qual: OptionValue,
    pub glob_dots: OptionValue,
    pub equals: OptionValue,
    pub magic_equal_subst: OptionValue,
    pub sh_file_expansion: OptionValue,
    pub glob_assign: OptionValue,
    pub ignore_braces: OptionValue,
    pub ignore_close_braces: OptionValue,
    pub brace_ccl: OptionValue,
    pub ksh_arrays: OptionValue,
    pub ksh_zero_subscript: OptionValue,
    pub short_loops: OptionValue,
    pub short_repeat: OptionValue,
    pub rc_quotes: OptionValue,
    pub interactive_comments: OptionValue,
    pub c_bases: OptionValue,
    pub octal_zeroes: OptionValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ZshOptionField {
    ShWordSplit,
    GlobSubst,
    RcExpandParam,
    Glob,
    Nomatch,
    NullGlob,
    CshNullGlob,
    ExtendedGlob,
    KshGlob,
    ShGlob,
    BareGlobQual,
    GlobDots,
    Equals,
    MagicEqualSubst,
    ShFileExpansion,
    GlobAssign,
    IgnoreBraces,
    IgnoreCloseBraces,
    BraceCcl,
    KshArrays,
    KshZeroSubscript,
    ShortLoops,
    ShortRepeat,
    RcQuotes,
    InteractiveComments,
    CBases,
    OctalZeroes,
}

impl ZshOptionState {
    pub const fn zsh_default() -> Self {
        Self {
            sh_word_split: OptionValue::Off,
            glob_subst: OptionValue::Off,
            rc_expand_param: OptionValue::Off,
            glob: OptionValue::On,
            nomatch: OptionValue::On,
            null_glob: OptionValue::Off,
            csh_null_glob: OptionValue::Off,
            extended_glob: OptionValue::Off,
            ksh_glob: OptionValue::Off,
            sh_glob: OptionValue::Off,
            bare_glob_qual: OptionValue::On,
            glob_dots: OptionValue::Off,
            equals: OptionValue::On,
            magic_equal_subst: OptionValue::Off,
            sh_file_expansion: OptionValue::Off,
            glob_assign: OptionValue::Off,
            ignore_braces: OptionValue::Off,
            ignore_close_braces: OptionValue::Off,
            brace_ccl: OptionValue::Off,
            ksh_arrays: OptionValue::Off,
            ksh_zero_subscript: OptionValue::Off,
            short_loops: OptionValue::On,
            short_repeat: OptionValue::On,
            rc_quotes: OptionValue::Off,
            interactive_comments: OptionValue::On,
            c_bases: OptionValue::Off,
            octal_zeroes: OptionValue::Off,
        }
    }

    pub fn for_emulate(mode: ZshEmulationMode) -> Self {
        let mut state = Self::zsh_default();
        match mode {
            ZshEmulationMode::Zsh => {}
            ZshEmulationMode::Sh => {
                state.sh_word_split = OptionValue::On;
                state.glob_subst = OptionValue::On;
                state.sh_glob = OptionValue::On;
                state.sh_file_expansion = OptionValue::On;
                state.bare_glob_qual = OptionValue::Off;
                state.ksh_arrays = OptionValue::Off;
            }
            ZshEmulationMode::Ksh => {
                state.sh_word_split = OptionValue::On;
                state.glob_subst = OptionValue::On;
                state.ksh_glob = OptionValue::On;
                state.ksh_arrays = OptionValue::On;
                state.sh_glob = OptionValue::On;
                state.bare_glob_qual = OptionValue::Off;
            }
            ZshEmulationMode::Csh => {
                state.csh_null_glob = OptionValue::On;
                state.sh_word_split = OptionValue::Off;
                state.glob_subst = OptionValue::Off;
            }
        }
        state
    }

    pub fn apply_setopt(&mut self, name: &str) -> bool {
        self.apply_named_option(name, true)
    }

    pub fn apply_unsetopt(&mut self, name: &str) -> bool {
        self.apply_named_option(name, false)
    }

    fn set_field(&mut self, field: ZshOptionField, value: OptionValue) {
        match field {
            ZshOptionField::ShWordSplit => self.sh_word_split = value,
            ZshOptionField::GlobSubst => self.glob_subst = value,
            ZshOptionField::RcExpandParam => self.rc_expand_param = value,
            ZshOptionField::Glob => self.glob = value,
            ZshOptionField::Nomatch => self.nomatch = value,
            ZshOptionField::NullGlob => self.null_glob = value,
            ZshOptionField::CshNullGlob => self.csh_null_glob = value,
            ZshOptionField::ExtendedGlob => self.extended_glob = value,
            ZshOptionField::KshGlob => self.ksh_glob = value,
            ZshOptionField::ShGlob => self.sh_glob = value,
            ZshOptionField::BareGlobQual => self.bare_glob_qual = value,
            ZshOptionField::GlobDots => self.glob_dots = value,
            ZshOptionField::Equals => self.equals = value,
            ZshOptionField::MagicEqualSubst => self.magic_equal_subst = value,
            ZshOptionField::ShFileExpansion => self.sh_file_expansion = value,
            ZshOptionField::GlobAssign => self.glob_assign = value,
            ZshOptionField::IgnoreBraces => self.ignore_braces = value,
            ZshOptionField::IgnoreCloseBraces => self.ignore_close_braces = value,
            ZshOptionField::BraceCcl => self.brace_ccl = value,
            ZshOptionField::KshArrays => self.ksh_arrays = value,
            ZshOptionField::KshZeroSubscript => self.ksh_zero_subscript = value,
            ZshOptionField::ShortLoops => self.short_loops = value,
            ZshOptionField::ShortRepeat => self.short_repeat = value,
            ZshOptionField::RcQuotes => self.rc_quotes = value,
            ZshOptionField::InteractiveComments => self.interactive_comments = value,
            ZshOptionField::CBases => self.c_bases = value,
            ZshOptionField::OctalZeroes => self.octal_zeroes = value,
        }
    }

    fn field(&self, field: ZshOptionField) -> OptionValue {
        match field {
            ZshOptionField::ShWordSplit => self.sh_word_split,
            ZshOptionField::GlobSubst => self.glob_subst,
            ZshOptionField::RcExpandParam => self.rc_expand_param,
            ZshOptionField::Glob => self.glob,
            ZshOptionField::Nomatch => self.nomatch,
            ZshOptionField::NullGlob => self.null_glob,
            ZshOptionField::CshNullGlob => self.csh_null_glob,
            ZshOptionField::ExtendedGlob => self.extended_glob,
            ZshOptionField::KshGlob => self.ksh_glob,
            ZshOptionField::ShGlob => self.sh_glob,
            ZshOptionField::BareGlobQual => self.bare_glob_qual,
            ZshOptionField::GlobDots => self.glob_dots,
            ZshOptionField::Equals => self.equals,
            ZshOptionField::MagicEqualSubst => self.magic_equal_subst,
            ZshOptionField::ShFileExpansion => self.sh_file_expansion,
            ZshOptionField::GlobAssign => self.glob_assign,
            ZshOptionField::IgnoreBraces => self.ignore_braces,
            ZshOptionField::IgnoreCloseBraces => self.ignore_close_braces,
            ZshOptionField::BraceCcl => self.brace_ccl,
            ZshOptionField::KshArrays => self.ksh_arrays,
            ZshOptionField::KshZeroSubscript => self.ksh_zero_subscript,
            ZshOptionField::ShortLoops => self.short_loops,
            ZshOptionField::ShortRepeat => self.short_repeat,
            ZshOptionField::RcQuotes => self.rc_quotes,
            ZshOptionField::InteractiveComments => self.interactive_comments,
            ZshOptionField::CBases => self.c_bases,
            ZshOptionField::OctalZeroes => self.octal_zeroes,
        }
    }

    pub fn merge(&self, other: &Self) -> Self {
        let mut merged = Self::zsh_default();
        for field in ZshOptionField::ALL {
            merged.set_field(field, self.field(field).merge(other.field(field)));
        }
        merged
    }

    fn apply_named_option(&mut self, name: &str, enable: bool) -> bool {
        let Some((field, value)) = parse_zsh_option_assignment(name, enable) else {
            return false;
        };
        self.set_field(
            field,
            if value {
                OptionValue::On
            } else {
                OptionValue::Off
            },
        );
        true
    }
}

impl ZshOptionField {
    const ALL: [Self; 27] = [
        Self::ShWordSplit,
        Self::GlobSubst,
        Self::RcExpandParam,
        Self::Glob,
        Self::Nomatch,
        Self::NullGlob,
        Self::CshNullGlob,
        Self::ExtendedGlob,
        Self::KshGlob,
        Self::ShGlob,
        Self::BareGlobQual,
        Self::GlobDots,
        Self::Equals,
        Self::MagicEqualSubst,
        Self::ShFileExpansion,
        Self::GlobAssign,
        Self::IgnoreBraces,
        Self::IgnoreCloseBraces,
        Self::BraceCcl,
        Self::KshArrays,
        Self::KshZeroSubscript,
        Self::ShortLoops,
        Self::ShortRepeat,
        Self::RcQuotes,
        Self::InteractiveComments,
        Self::CBases,
        Self::OctalZeroes,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShellProfile {
    pub dialect: ShellDialect,
    pub options: Option<ZshOptionState>,
}

impl ShellProfile {
    pub fn native(dialect: ShellDialect) -> Self {
        Self {
            dialect,
            options: (dialect == ShellDialect::Zsh).then(ZshOptionState::zsh_default),
        }
    }

    pub fn with_zsh_options(dialect: ShellDialect, options: ZshOptionState) -> Self {
        Self {
            dialect,
            options: (dialect == ShellDialect::Zsh).then_some(options),
        }
    }

    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.options.as_ref()
    }
}

fn parse_zsh_option_assignment(name: &str, enable: bool) -> Option<(ZshOptionField, bool)> {
    let mut normalized = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '_' | '-') {
            continue;
        }
        normalized.push(ch.to_ascii_lowercase());
    }

    let (normalized, invert) = if let Some(rest) = normalized.strip_prefix("no") {
        (rest, true)
    } else {
        (normalized.as_str(), false)
    };

    let field = match normalized {
        "shwordsplit" => ZshOptionField::ShWordSplit,
        "globsubst" => ZshOptionField::GlobSubst,
        "rcexpandparam" => ZshOptionField::RcExpandParam,
        "glob" | "noglob" => ZshOptionField::Glob,
        "nomatch" => ZshOptionField::Nomatch,
        "nullglob" => ZshOptionField::NullGlob,
        "cshnullglob" => ZshOptionField::CshNullGlob,
        "extendedglob" => ZshOptionField::ExtendedGlob,
        "kshglob" => ZshOptionField::KshGlob,
        "shglob" => ZshOptionField::ShGlob,
        "bareglobqual" => ZshOptionField::BareGlobQual,
        "globdots" => ZshOptionField::GlobDots,
        "equals" => ZshOptionField::Equals,
        "magicequalsubst" => ZshOptionField::MagicEqualSubst,
        "shfileexpansion" => ZshOptionField::ShFileExpansion,
        "globassign" => ZshOptionField::GlobAssign,
        "ignorebraces" => ZshOptionField::IgnoreBraces,
        "ignoreclosebraces" => ZshOptionField::IgnoreCloseBraces,
        "braceccl" => ZshOptionField::BraceCcl,
        "ksharrays" => ZshOptionField::KshArrays,
        "kshzerosubscript" => ZshOptionField::KshZeroSubscript,
        "shortloops" => ZshOptionField::ShortLoops,
        "shortrepeat" => ZshOptionField::ShortRepeat,
        "rcquotes" => ZshOptionField::RcQuotes,
        "interactivecomments" => ZshOptionField::InteractiveComments,
        "cbases" => ZshOptionField::CBases,
        "octalzeroes" => ZshOptionField::OctalZeroes,
        _ => return None,
    };

    Some((field, if invert { !enable } else { enable }))
}

#[derive(Debug, Clone)]
pub(crate) struct ZshOptionTimeline {
    initial: ZshOptionState,
    entries: Arc<[ZshOptionTimelineEntry]>,
}

#[derive(Debug, Clone)]
struct ZshOptionTimelineEntry {
    offset: usize,
    state: ZshOptionState,
}

impl ZshOptionTimeline {
    fn build(input: &str, shell_profile: &ShellProfile) -> Option<Self> {
        let initial = shell_profile.zsh_options()?.clone();
        if !might_mutate_zsh_parser_options(input) {
            return Some(Self {
                initial,
                entries: Arc::from([]),
            });
        }

        let entries = ZshOptionPrescanner::new(input, initial.clone()).scan();
        Some(Self {
            initial,
            entries: entries.into(),
        })
    }

    fn options_at(&self, offset: usize) -> &ZshOptionState {
        let next_index = self.entries.partition_point(|entry| entry.offset <= offset);
        if next_index == 0 {
            &self.initial
        } else {
            &self.entries[next_index - 1].state
        }
    }
}

fn might_mutate_zsh_parser_options(input: &str) -> bool {
    input.contains("setopt")
        || input.contains("unsetopt")
        || input.contains("emulate")
        || input.contains("set -o")
        || input.contains("set +o")
}

#[derive(Debug, Clone)]
struct ZshOptionPrescanner<'a> {
    input: &'a str,
    offset: usize,
    state: ZshOptionState,
    entries: Vec<ZshOptionTimelineEntry>,
}

#[derive(Debug, Clone)]
enum PrescanToken {
    Word {
        text: String,
        end: usize,
    },
    Separator {
        kind: PrescanSeparator,
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanSeparator {
    Newline,
    Semicolon,
    Pipe,
    Ampersand,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
}

#[derive(Debug, Clone)]
struct PrescanLocalScope {
    saved_state: ZshOptionState,
    brace_depth: usize,
    paren_depth: usize,
    compounds: Vec<PrescanCompound>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanCompound {
    If,
    Loop,
    Case,
}

impl PrescanLocalScope {
    fn simple(saved_state: ZshOptionState) -> Self {
        Self {
            saved_state,
            brace_depth: 0,
            paren_depth: 0,
            compounds: Vec::new(),
        }
    }

    fn brace_group(saved_state: ZshOptionState) -> Self {
        Self {
            brace_depth: 1,
            ..Self::simple(saved_state)
        }
    }

    fn subshell(saved_state: ZshOptionState) -> Self {
        Self {
            paren_depth: 1,
            ..Self::simple(saved_state)
        }
    }

    fn update_for_command(&mut self, words: &[String]) {
        let Some(command) = words.first().map(String::as_str) else {
            return;
        };

        match command {
            "if" => self.compounds.push(PrescanCompound::If),
            "case" => self.compounds.push(PrescanCompound::Case),
            "for" | "select" | "while" | "until" => {
                self.compounds.push(PrescanCompound::Loop);
            }
            "repeat" if words.iter().any(|word| word == "do") => {
                self.compounds.push(PrescanCompound::Loop);
            }
            "fi" => self.pop_compound(PrescanCompound::If),
            "done" => self.pop_compound(PrescanCompound::Loop),
            "esac" => self.pop_compound(PrescanCompound::Case),
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.brace_depth == 0 && self.paren_depth == 0 && self.compounds.is_empty()
    }

    fn pop_compound(&mut self, compound: PrescanCompound) {
        if self.compounds.last().copied() == Some(compound) {
            self.compounds.pop();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrescanFunctionHeaderState {
    None,
    AfterWord,
    AfterFunctionKeyword,
    AfterFunctionName,
    AfterWordOpenParen,
    AfterFunctionNameOpenParen,
    ReadyForBrace,
}

impl<'a> ZshOptionPrescanner<'a> {
    fn new(input: &'a str, state: ZshOptionState) -> Self {
        Self {
            input,
            offset: 0,
            state,
            entries: Vec::new(),
        }
    }

    fn scan(mut self) -> Vec<ZshOptionTimelineEntry> {
        let mut words = Vec::new();
        let mut command_end = 0usize;
        let mut local_scopes = Vec::new();
        let mut function_header = PrescanFunctionHeaderState::None;

        while let Some(token) = self.next_token() {
            match token {
                PrescanToken::Word { text, end } => {
                    if is_prescan_function_body_start(function_header) {
                        local_scopes.push(PrescanLocalScope::simple(self.state.clone()));
                        function_header = PrescanFunctionHeaderState::None;
                    }
                    command_end = end;
                    function_header = match function_header {
                        PrescanFunctionHeaderState::None => {
                            if text == "function" {
                                PrescanFunctionHeaderState::AfterFunctionKeyword
                            } else {
                                PrescanFunctionHeaderState::AfterWord
                            }
                        }
                        PrescanFunctionHeaderState::AfterFunctionKeyword => {
                            PrescanFunctionHeaderState::AfterFunctionName
                        }
                        _ => PrescanFunctionHeaderState::None,
                    };
                    words.push(text);
                }
                PrescanToken::Separator { kind, start, end } => {
                    self.finish_command(&words, command_end.max(start));
                    if let Some(scope) = local_scopes.last_mut() {
                        scope.update_for_command(&words);
                    }
                    if matches!(
                        kind,
                        PrescanSeparator::Newline | PrescanSeparator::Semicolon
                    ) {
                        self.restore_completed_local_scopes(&mut local_scopes, end);
                    }
                    words.clear();
                    command_end = end;

                    match kind {
                        PrescanSeparator::Newline => {
                            if !matches!(
                                function_header,
                                PrescanFunctionHeaderState::AfterFunctionName
                                    | PrescanFunctionHeaderState::ReadyForBrace
                            ) {
                                function_header = PrescanFunctionHeaderState::None;
                            }
                        }
                        PrescanSeparator::Semicolon
                        | PrescanSeparator::Pipe
                        | PrescanSeparator::Ampersand => {
                            function_header = PrescanFunctionHeaderState::None;
                        }
                        PrescanSeparator::OpenParen => {
                            if is_prescan_function_body_start(function_header) {
                                local_scopes.push(PrescanLocalScope::subshell(self.state.clone()));
                                function_header = PrescanFunctionHeaderState::None;
                            } else {
                                let next_header = match function_header {
                                    PrescanFunctionHeaderState::AfterWord => {
                                        PrescanFunctionHeaderState::AfterWordOpenParen
                                    }
                                    PrescanFunctionHeaderState::AfterFunctionName => {
                                        PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                                    }
                                    _ => PrescanFunctionHeaderState::None,
                                };
                                if !matches!(
                                    next_header,
                                    PrescanFunctionHeaderState::AfterWordOpenParen
                                        | PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                                ) {
                                    local_scopes
                                        .push(PrescanLocalScope::subshell(self.state.clone()));
                                }
                                function_header = next_header;
                            }
                        }
                        PrescanSeparator::CloseParen => {
                            let closes_function_header = matches!(
                                function_header,
                                PrescanFunctionHeaderState::AfterWordOpenParen
                                    | PrescanFunctionHeaderState::AfterFunctionNameOpenParen
                            );
                            function_header = if closes_function_header {
                                PrescanFunctionHeaderState::ReadyForBrace
                            } else {
                                PrescanFunctionHeaderState::None
                            };
                            if !closes_function_header {
                                if let Some(scope) = local_scopes.last_mut()
                                    && scope.paren_depth > 0
                                {
                                    scope.paren_depth -= 1;
                                }
                                self.restore_completed_local_scopes(&mut local_scopes, end);
                            }
                        }
                        PrescanSeparator::OpenBrace => {
                            if is_prescan_function_body_start(function_header) {
                                local_scopes
                                    .push(PrescanLocalScope::brace_group(self.state.clone()));
                            } else if let Some(scope) = local_scopes.last_mut() {
                                scope.brace_depth += 1;
                            }
                            function_header = PrescanFunctionHeaderState::None;
                        }
                        PrescanSeparator::CloseBrace => {
                            if let Some(scope) = local_scopes.last_mut()
                                && scope.brace_depth > 0
                            {
                                scope.brace_depth -= 1;
                            }
                            self.restore_completed_local_scopes(&mut local_scopes, end);
                            function_header = PrescanFunctionHeaderState::None;
                        }
                    }
                }
            }
        }

        self.finish_command(&words, command_end.max(self.input.len()));
        if let Some(scope) = local_scopes.last_mut() {
            scope.update_for_command(&words);
        }
        self.restore_completed_local_scopes(&mut local_scopes, self.input.len());
        self.entries
    }

    fn finish_command(&mut self, words: &[String], end_offset: usize) {
        let mut next = self.state.clone();
        if !apply_prescan_command_effects(words, &mut next) || next == self.state {
            return;
        }

        self.state = next.clone();
        self.entries.push(ZshOptionTimelineEntry {
            offset: end_offset,
            state: next,
        });
    }

    fn next_token(&mut self) -> Option<PrescanToken> {
        loop {
            self.skip_horizontal_whitespace();
            let ch = self.peek_char()?;

            if ch == '#' && self.state.interactive_comments.is_definitely_on() {
                self.skip_comment();
                continue;
            }

            return match ch {
                '\n' => {
                    let start = self.offset;
                    self.advance_char();
                    Some(PrescanToken::Separator {
                        kind: PrescanSeparator::Newline,
                        start,
                        end: self.offset,
                    })
                }
                ';' | '|' | '&' | '(' | ')' | '{' | '}' => {
                    let start = self.offset;
                    self.advance_char();
                    if matches!(ch, '|' | '&' | ';') && self.peek_char() == Some(ch) {
                        self.advance_char();
                    }
                    let kind = match ch {
                        ';' => PrescanSeparator::Semicolon,
                        '|' => PrescanSeparator::Pipe,
                        '&' => PrescanSeparator::Ampersand,
                        '(' => PrescanSeparator::OpenParen,
                        ')' => PrescanSeparator::CloseParen,
                        '{' => PrescanSeparator::OpenBrace,
                        '}' => PrescanSeparator::CloseBrace,
                        _ => unreachable!(),
                    };
                    Some(PrescanToken::Separator {
                        kind,
                        start,
                        end: self.offset,
                    })
                }
                _ => self
                    .read_word()
                    .map(|(text, end)| PrescanToken::Word { text, end }),
            };
        }
    }

    fn skip_horizontal_whitespace(&mut self) {
        while let Some(ch) = self.peek_char() {
            match ch {
                ' ' | '\t' => {
                    self.advance_char();
                }
                '\\' if self.second_char() == Some('\n') => {
                    self.advance_char();
                    self.advance_char();
                }
                _ => break,
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch == '\n' {
                break;
            }
            self.advance_char();
        }
    }

    fn read_word(&mut self) -> Option<(String, usize)> {
        let mut text = String::new();

        while let Some(ch) = self.peek_char() {
            if is_prescan_separator(ch) {
                break;
            }

            match ch {
                ' ' | '\t' => break,
                '\\' => {
                    self.advance_char();
                    match self.peek_char() {
                        Some('\n') => {
                            self.advance_char();
                        }
                        Some(next) => {
                            text.push(next);
                            self.advance_char();
                        }
                        None => text.push('\\'),
                    }
                }
                '\'' => {
                    self.advance_char();
                    while let Some(next) = self.peek_char() {
                        if next == '\'' {
                            if self.state.rc_quotes.is_definitely_on()
                                && self.second_char() == Some('\'')
                            {
                                text.push('\'');
                                self.advance_char();
                                self.advance_char();
                                continue;
                            }
                            self.advance_char();
                            break;
                        }
                        text.push(next);
                        self.advance_char();
                    }
                }
                '"' => {
                    self.advance_char();
                    while let Some(next) = self.peek_char() {
                        if next == '"' {
                            self.advance_char();
                            break;
                        }
                        if next == '\\' {
                            self.advance_char();
                            if let Some(escaped) = self.peek_char() {
                                text.push(escaped);
                                self.advance_char();
                            }
                            continue;
                        }
                        text.push(next);
                        self.advance_char();
                    }
                }
                _ => {
                    text.push(ch);
                    self.advance_char();
                }
            }
        }

        (!text.is_empty()).then_some((text, self.offset))
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn second_char(&self) -> Option<char> {
        let mut chars = self.input[self.offset..].chars();
        chars.next()?;
        chars.next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }

    fn restore_completed_local_scopes(
        &mut self,
        local_scopes: &mut Vec<PrescanLocalScope>,
        offset: usize,
    ) {
        while local_scopes
            .last()
            .is_some_and(PrescanLocalScope::is_complete)
        {
            let scope = local_scopes.pop().expect("scope just matched");
            if self.state != scope.saved_state {
                self.state = scope.saved_state.clone();
                self.entries.push(ZshOptionTimelineEntry {
                    offset,
                    state: scope.saved_state,
                });
            } else {
                self.state = scope.saved_state;
            }
        }
    }
}

fn is_prescan_separator(ch: char) -> bool {
    matches!(ch, '\n' | ';' | '|' | '&' | '(' | ')' | '{' | '}')
}

fn is_prescan_function_body_start(state: PrescanFunctionHeaderState) -> bool {
    matches!(
        state,
        PrescanFunctionHeaderState::AfterFunctionName | PrescanFunctionHeaderState::ReadyForBrace
    )
}

fn apply_prescan_command_effects(words: &[String], state: &mut ZshOptionState) -> bool {
    let Some((command, args_index)) = normalize_prescan_command(words) else {
        return false;
    };

    match command {
        "setopt" => words[args_index..]
            .iter()
            .fold(false, |changed, arg| state.apply_setopt(arg) || changed),
        "unsetopt" => words[args_index..]
            .iter()
            .fold(false, |changed, arg| state.apply_unsetopt(arg) || changed),
        "set" => apply_prescan_set_builtin(&words[args_index..], state),
        "emulate" => apply_prescan_emulate(&words[args_index..], state),
        _ => false,
    }
}

fn normalize_prescan_command(words: &[String]) -> Option<(&str, usize)> {
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        if is_prescan_assignment_word(word) {
            index += 1;
            continue;
        }
        match word.as_str() {
            "noglob" => {
                index += 1;
                continue;
            }
            "command" => {
                index = skip_prescan_command_wrapper_options(words, index + 1)?;
                continue;
            }
            "builtin" => {
                index = skip_prescan_wrapper_options(words, index + 1);
                continue;
            }
            "exec" => {
                index = skip_prescan_exec_wrapper_options(words, index + 1);
                continue;
            }
            _ => {}
        }
        return Some((word.as_str(), index + 1));
    }

    None
}

fn skip_prescan_command_wrapper_options(words: &[String], mut index: usize) -> Option<usize> {
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if word.starts_with('-') && word != "-" {
            if word
                .strip_prefix('-')
                .is_some_and(|flags| flags.chars().any(|flag| matches!(flag, 'v' | 'V')))
            {
                return None;
            }
            index += 1;
            continue;
        }
        break;
    }
    Some(index)
}

fn skip_prescan_wrapper_options(words: &[String], mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if word.starts_with('-') && word != "-" {
            index += 1;
            continue;
        }
        break;
    }
    index
}

fn skip_prescan_exec_wrapper_options(words: &[String], mut index: usize) -> usize {
    while let Some(word) = words.get(index) {
        if word == "--" {
            index += 1;
            break;
        }
        if word == "-a" {
            index = (index + 2).min(words.len());
            continue;
        }
        if word.starts_with('-') && word != "-" {
            index += 1;
            continue;
        }
        break;
    }
    index
}

fn is_prescan_assignment_word(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn apply_prescan_set_builtin(words: &[String], state: &mut ZshOptionState) -> bool {
    let mut changed = false;
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        match word.as_str() {
            "-o" | "+o" => {
                let enable = word.starts_with('-');
                if let Some(name) = words.get(index + 1) {
                    changed = if enable {
                        state.apply_setopt(name)
                    } else {
                        state.apply_unsetopt(name)
                    } || changed;
                }
                index += 2;
            }
            _ => {
                if let Some(name) = word.strip_prefix("-o") {
                    changed = state.apply_setopt(name) || changed;
                } else if let Some(name) = word.strip_prefix("+o") {
                    changed = state.apply_unsetopt(name) || changed;
                }
                index += 1;
            }
        }
    }

    changed
}

fn apply_prescan_emulate(words: &[String], state: &mut ZshOptionState) -> bool {
    let mut changed = false;
    let mut mode = None;
    let mut pending_option: Option<bool> = None;
    let mut explicit_updates = Vec::new();
    let mut index = 0usize;

    while let Some(word) = words.get(index) {
        if let Some(enable) = pending_option.take() {
            explicit_updates.push((word.clone(), enable));
            index += 1;
            continue;
        }

        match word.as_str() {
            "-o" | "+o" => {
                pending_option = Some(word.starts_with('-'));
                index += 1;
                continue;
            }
            "zsh" | "sh" | "ksh" | "csh" if mode.is_none() => {
                mode = Some(match word.as_str() {
                    "zsh" => ZshEmulationMode::Zsh,
                    "sh" => ZshEmulationMode::Sh,
                    "ksh" => ZshEmulationMode::Ksh,
                    "csh" => ZshEmulationMode::Csh,
                    _ => unreachable!(),
                });
                index += 1;
                continue;
            }
            _ if mode.is_none() && word.starts_with('-') => {
                for flag in word[1..].chars() {
                    if flag == 'o' {
                        pending_option = Some(true);
                    }
                }
                index += 1;
                continue;
            }
            _ => {}
        }

        index += 1;
    }

    if let Some(mode) = mode {
        *state = ZshOptionState::for_emulate(mode);
        changed = true;
    }

    for (name, enable) in explicit_updates {
        changed = if enable {
            state.apply_setopt(&name)
        } else {
            state.apply_unsetopt(&name)
        } || changed;
    }

    changed
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
                function_keyword: true,
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
    syntax_facts: SyntaxFacts,
    shell_profile: ShellProfile,
    zsh_timeline: Option<Arc<ZshOptionTimeline>>,
    dialect: ShellDialect,
    #[cfg(feature = "benchmarking")]
    benchmark_counters: Option<ParserBenchmarkCounters>,
}

#[derive(Clone)]
struct ParserCheckpoint<'a> {
    lexer: Lexer<'a>,
    synthetic_tokens: VecDeque<SyntheticToken>,
    alias_replays: Vec<AliasReplay>,
    current_token: Option<LexedToken<'a>>,
    current_word_cache: Option<Word>,
    current_token_kind: Option<TokenKind>,
    current_keyword: Option<Keyword>,
    current_span: Span,
    peeked_token: Option<LexedToken<'a>>,
    current_depth: usize,
    fuel: usize,
    comments: Vec<Comment>,
    expand_next_word: bool,
    brace_group_depth: usize,
    brace_body_stack: Vec<BraceBodyContext>,
    syntax_facts: SyntaxFacts,
    #[cfg(feature = "benchmarking")]
    benchmark_counters: Option<ParserBenchmarkCounters>,
}

/// A parser diagnostic emitted while recovering from invalid input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub message: String,
    pub span: Span,
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
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser for the given input and shell dialect.
    pub fn with_dialect(input: &'a str, dialect: ShellDialect) -> Self {
        Self::with_profile(input, ShellProfile::native(dialect))
    }

    /// Create a new parser for the given input and full shell profile.
    pub fn with_profile(input: &'a str, shell_profile: ShellProfile) -> Self {
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            DEFAULT_MAX_PARSER_OPERATIONS,
            shell_profile,
        )
    }

    /// Create a new parser with a custom maximum AST depth.
    pub fn with_max_depth(input: &'a str, max_depth: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            max_depth,
            DEFAULT_MAX_PARSER_OPERATIONS,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser with a custom fuel limit.
    pub fn with_fuel(input: &'a str, max_fuel: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            DEFAULT_MAX_AST_DEPTH,
            max_fuel,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser with custom depth and fuel limits.
    ///
    /// `max_depth` is clamped to `HARD_MAX_AST_DEPTH` (500)
    /// to prevent stack overflow from misconfiguration. Even if the caller passes
    /// `max_depth = 1_000_000`, the parser will cap it at 500.
    pub fn with_limits(input: &'a str, max_depth: usize, max_fuel: usize) -> Self {
        Self::with_limits_and_profile(
            input,
            max_depth,
            max_fuel,
            ShellProfile::native(ShellDialect::Bash),
        )
    }

    /// Create a new parser with custom depth, fuel, and dialect settings.
    pub fn with_limits_and_dialect(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        dialect: ShellDialect,
    ) -> Self {
        Self::with_limits_and_profile(input, max_depth, max_fuel, ShellProfile::native(dialect))
    }

    pub fn with_limits_and_profile(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        shell_profile: ShellProfile,
    ) -> Self {
        Self::with_limits_and_profile_and_benchmarking(
            input,
            max_depth,
            max_fuel,
            shell_profile,
            false,
        )
    }

    fn with_limits_and_profile_and_benchmarking(
        input: &'a str,
        max_depth: usize,
        max_fuel: usize,
        shell_profile: ShellProfile,
        benchmark_counters_enabled: bool,
    ) -> Self {
        #[cfg(not(feature = "benchmarking"))]
        let _ = benchmark_counters_enabled;

        let zsh_timeline = (shell_profile.dialect == ShellDialect::Zsh)
            .then(|| ZshOptionTimeline::build(input, &shell_profile))
            .flatten()
            .map(Arc::new);
        let mut lexer = Lexer::with_max_subst_depth_and_profile(
            input,
            max_depth.min(HARD_MAX_AST_DEPTH),
            &shell_profile,
            zsh_timeline.clone(),
        );
        #[cfg(feature = "benchmarking")]
        if benchmark_counters_enabled {
            lexer.enable_benchmark_counters();
        }
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
            syntax_facts: SyntaxFacts::default(),
            dialect: shell_profile.dialect,
            shell_profile,
            zsh_timeline,
            #[cfg(feature = "benchmarking")]
            benchmark_counters: benchmark_counters_enabled.then(ParserBenchmarkCounters::default),
        }
    }

    #[cfg(feature = "benchmarking")]
    fn rebuild_with_benchmark_counters(&self) -> Self {
        Self::with_limits_and_profile_and_benchmarking(
            self.input,
            self.max_depth,
            self.max_fuel,
            self.shell_profile.clone(),
            true,
        )
    }

    pub fn dialect(&self) -> ShellDialect {
        self.dialect
    }

    pub fn shell_profile(&self) -> &ShellProfile {
        &self.shell_profile
    }

    fn zsh_options_at_offset(&self, offset: usize) -> Option<&ZshOptionState> {
        self.zsh_timeline
            .as_ref()
            .map(|timeline| timeline.options_at(offset))
            .or_else(|| self.shell_profile.zsh_options())
    }

    fn current_zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options_at_offset(self.current_span.start.offset)
    }

    fn zsh_short_loops_enabled(&self) -> bool {
        self.dialect.features().zsh_foreach_loop
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.short_loops.is_definitely_off())
    }

    fn zsh_short_repeat_enabled(&self) -> bool {
        self.dialect.features().zsh_repeat_loop
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.short_repeat.is_definitely_off())
    }

    fn zsh_brace_bodies_enabled(&self) -> bool {
        self.dialect.features().zsh_brace_if
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.ignore_braces.is_definitely_on())
    }

    fn zsh_brace_if_enabled(&self) -> bool {
        self.zsh_brace_bodies_enabled()
    }

    fn zsh_glob_qualifiers_enabled_at(&self, offset: usize) -> bool {
        self.dialect.features().zsh_glob_qualifiers
            && !self.zsh_options_at_offset(offset).is_some_and(|options| {
                options.ignore_braces.is_definitely_on()
                    || options.bare_glob_qual.is_definitely_off()
            })
    }

    fn brace_syntax_enabled_at(&self, offset: usize) -> bool {
        !self.zsh_options_at_offset(offset).is_some_and(|options| {
            options.ignore_braces.is_definitely_on()
                || options.ignore_close_braces.is_definitely_on()
        })
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
        Self::parse_word_string_with_limits_and_dialect(
            input,
            max_depth,
            max_fuel,
            ShellDialect::Bash,
        )
    }

    /// Parse a word string with caller-configured limits and shell dialect.
    pub fn parse_word_string_with_limits_and_dialect(
        input: &str,
        max_depth: usize,
        max_fuel: usize,
        dialect: ShellDialect,
    ) -> Word {
        let mut parser = Parser::with_limits_and_profile(
            input,
            max_depth,
            max_fuel,
            ShellProfile::native(dialect),
        );
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
        let mut parser = Parser::new(text);
        let source_backed = span.end.offset <= source.len() && span.slice(source) == text;
        let start = Position::new();
        let fragment_span = Span::from_positions(start, start.advanced_by(text));
        let mut word = parser.parse_word_with_context(text, fragment_span, start, source_backed);
        if !source_backed {
            Self::materialize_word_source_backing(&mut word, text);
        }
        Self::rebase_word(&mut word, span.start);
        word.span = span;
        word
    }

    fn maybe_record_comment(&mut self, token: &LexedToken<'_>) {
        if token.kind == TokenKind::Comment && !token.flags.is_synthetic() {
            self.comments.push(Comment {
                range: token.span.to_range(),
            });
        }
    }

    fn record_zsh_brace_if_span(&mut self, span: Span) {
        if !self.syntax_facts.zsh_brace_if_spans.contains(&span) {
            self.syntax_facts.zsh_brace_if_spans.push(span);
        }
    }

    fn record_zsh_always_span(&mut self, span: Span) {
        if !self.syntax_facts.zsh_always_spans.contains(&span) {
            self.syntax_facts.zsh_always_spans.push(span);
        }
    }

    fn record_zsh_case_group_part(&mut self, pattern_part_index: usize, span: Span) {
        if !self
            .syntax_facts
            .zsh_case_group_parts
            .iter()
            .any(|fact| fact.pattern_part_index == pattern_part_index && fact.span == span)
        {
            self.syntax_facts
                .zsh_case_group_parts
                .push(ZshCaseGroupPart {
                    pattern_part_index,
                    span,
                });
        }
    }

    fn word_text_needs_parse(text: &str) -> bool {
        memchr3(b'$', b'`', b'\0', text.as_bytes()).is_some()
    }

    fn word_with_parts(&self, parts: Vec<WordPartNode>, span: Span) -> Word {
        let brace_syntax = self.brace_syntax_from_parts(&parts, span.start.offset);
        Word {
            parts,
            span,
            brace_syntax,
        }
    }

    fn heredoc_body_with_parts(
        &self,
        parts: Vec<HeredocBodyPartNode>,
        span: Span,
        mode: HeredocBodyMode,
        source_backed: bool,
    ) -> HeredocBody {
        HeredocBody {
            mode,
            source_backed,
            parts,
            span,
        }
    }

    fn heredoc_body_part_from_word_part_node(part: WordPartNode) -> HeredocBodyPartNode {
        let kind = match part.kind {
            WordPart::Literal(text) => HeredocBodyPart::Literal(text),
            WordPart::Variable(name) => HeredocBodyPart::Variable(name),
            WordPart::CommandSubstitution { body, syntax } => {
                HeredocBodyPart::CommandSubstitution { body, syntax }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                syntax,
            } => HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                syntax,
            },
            WordPart::Parameter(parameter) => HeredocBodyPart::Parameter(Box::new(parameter)),
            other => panic!("unsupported heredoc body part: {other:?}"),
        };

        HeredocBodyPartNode::new(kind, part.span)
    }

    fn brace_syntax_from_parts(&self, parts: &[WordPartNode], offset: usize) -> Vec<BraceSyntax> {
        if !self.brace_syntax_enabled_at(offset) {
            return Vec::new();
        }
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
        let bytes = text.as_bytes();
        let mut index = 0;
        let mut position = base;

        while index < bytes.len() {
            let next_special = if matches!(quote_context, BraceQuoteContext::Unquoted) {
                memchr2(b'{', b'\\', &bytes[index..]).map(|relative| index + relative)
            } else {
                memchr(b'{', &bytes[index..]).map(|relative| index + relative)
            };

            let Some(next_index) = next_special else {
                return;
            };

            if next_index > index {
                position = position.advanced_by(&text[index..next_index]);
                index = next_index;
            }

            if matches!(quote_context, BraceQuoteContext::Unquoted) && bytes[index] == b'\\' {
                let escaped_start = index;
                index += 1;
                if let Some(next) = text[index..].chars().next() {
                    index += next.len_utf8();
                }
                position = position.advanced_by(&text[escaped_start..index]);
                continue;
            }

            let brace_start = position;
            if let Some(len) = Self::template_placeholder_len(text, index, quote_context) {
                let brace_end = brace_start.advanced_by(&text[index..index + len]);
                out.push(BraceSyntax {
                    kind: BraceSyntaxKind::TemplatePlaceholder,
                    span: Span::from_positions(brace_start, brace_end),
                    quote_context,
                });
                position = brace_end;
                index += len;
                continue;
            }

            if let Some((len, kind)) = Self::brace_construct_len(text, index, quote_context) {
                let brace_end = brace_start.advanced_by(&text[index..index + len]);
                out.push(BraceSyntax {
                    kind,
                    span: Span::from_positions(brace_start, brace_end),
                    quote_context,
                });
                position = brace_end;
                index += len;
                continue;
            }

            position.advance('{');
            index += '{'.len_utf8();
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

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum QuoteState {
            Single,
            Double,
        }

        let mut index = start + '{'.len_utf8();
        let mut depth = 1usize;
        let mut has_comma = false;
        let mut has_dot_dot = false;
        let mut saw_unquoted_whitespace = false;
        let mut prev_char = None;
        let mut quote_state = None;

        while index < text.len() {
            if matches!(quote_context, BraceQuoteContext::Unquoted)
                && quote_state.is_none()
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

            if matches!(quote_context, BraceQuoteContext::Unquoted) {
                match quote_state {
                    None => match ch {
                        '\'' => {
                            quote_state = Some(QuoteState::Single);
                            prev_char = None;
                            continue;
                        }
                        '"' => {
                            quote_state = Some(QuoteState::Double);
                            prev_char = None;
                            continue;
                        }
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                let kind = if saw_unquoted_whitespace {
                                    BraceSyntaxKind::Literal
                                } else if has_comma {
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
                        c if c.is_whitespace() => saw_unquoted_whitespace = true,
                        _ => {}
                    },
                    Some(QuoteState::Single) => {
                        if ch == '\'' {
                            quote_state = None;
                        }
                    }
                    Some(QuoteState::Double) => match ch {
                        '\\' => {
                            if let Some(next) = text[index..].chars().next() {
                                index += next.len_utf8();
                            }
                            prev_char = None;
                            continue;
                        }
                        '"' => quote_state = None,
                        _ => {}
                    },
                }
            } else {
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
        if !self.zsh_glob_qualifiers_enabled_at(span.start.offset)
            || text.is_empty()
            || text.contains('=')
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
        } else if text[start..].starts_with("(#s)") {
            (
                "(#s)".len(),
                ZshInlineGlobControl::StartAnchor {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#s)".len()),
                    ),
                },
            )
        } else if text[start..].starts_with("(#e)") {
            (
                "(#e)".len(),
                ZshInlineGlobControl::EndAnchor {
                    span: Span::from_positions(
                        Self::text_position(base, text, start),
                        Self::text_position(base, text, start + "(#e)".len()),
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

        if self.zsh_glob_qualifiers_enabled_at(span.start.offset)
            && let Some(segment) = word.single_segment()
            && segment.kind() == LexedWordSegmentKind::Plain
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(
                segment.as_str(),
                span,
                segment.span().is_some() && source_backed && segment.text_is_source_backed(),
            )
        {
            return Some(word);
        }
        let mut parts = Vec::new();

        for segment in word.segments() {
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let content_span = Self::segment_content_span(segment, span);
            let raw_text = segment.as_str();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
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
            WordPart::Literal(self.literal_text_from_str(
                text,
                span.start,
                span.end,
                source_backed,
            )),
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
                value: self.source_text_from_str(text, content_span.start, content_span.end),
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

        if word.single_segment().is_none()
            && !token.flags.is_synthetic()
            && let Some(source_text) = token.source_slice(self.input)
        {
            return Some(self.parse_word_with_context(source_text, span, span.start, true));
        }

        if let Some(segment) = word.single_segment() {
            let content_span = Self::segment_content_span(segment, span);
            let wrapper_span = Self::segment_wrapper_span(segment, span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let raw_text = segment.as_str();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
            let decode_text = if source_backed
                && !self.source_matches(content_span, text)
                && matches!(
                    segment.kind(),
                    LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted
                )
                && !text.contains("$(")
            {
                content_span.slice(self.input)
            } else {
                text
            };
            let preserve_escaped_expansion_literals =
                source_backed && self.source_matches(content_span, decode_text);

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
                LexedWordSegmentKind::Plain if Self::word_text_needs_parse(text) => Some(
                    self.decode_word_text_preserving_quotes_if_needed_with_escape_mode(
                        text,
                        span,
                        content_span.start,
                        source_backed,
                        preserve_escaped_expansion_literals,
                    ),
                ),
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted
                    if Self::word_text_needs_parse(text) =>
                {
                    let inner = self.decode_quoted_segment_text(
                        decode_text,
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
            let raw_text = segment.as_str();
            let content_span = if let Some(segment_span) = segment.span() {
                cursor = segment_span.end;
                segment_span
            } else {
                let start = cursor;
                let end = start.advanced_by(raw_text);
                cursor = end;
                Span::from_positions(start, end)
            };
            let wrapper_span = segment.wrapper_span().unwrap_or(content_span);
            let source_backed = segment.span().is_some() && !token.flags.is_synthetic();
            let use_source_slice = source_backed
                && match segment.kind() {
                    LexedWordSegmentKind::Plain => {
                        segment.text_is_source_backed()
                            || raw_text.contains("${") && raw_text.contains('/')
                            || !raw_text.contains("$(")
                    }
                    _ => segment.text_is_source_backed(),
                };
            let text = if use_source_slice {
                content_span.slice(self.input)
            } else {
                raw_text
            };
            let preserve_escaped_expansion_literals = source_backed;

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
                            self.decode_word_text_preserving_quotes_if_needed_with_escape_mode(
                                text,
                                content_span,
                                content_span.start,
                                source_backed,
                                preserve_escaped_expansion_literals,
                            )
                            .parts,
                        );
                    } else {
                        parts.push(self.literal_part_from_text(text, content_span, source_backed));
                    }
                }
                LexedWordSegmentKind::DoubleQuoted | LexedWordSegmentKind::DollarDoubleQuoted => {
                    if Self::word_text_needs_parse(text) {
                        let inner = self.decode_quoted_segment_text(
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

    fn current_word_ref(&mut self) -> Option<&Word> {
        if self.current_word_cache.is_none() {
            self.current_word_cache = self.current_word();
        }

        self.current_word_cache.as_ref()
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

    fn take_current_word(&mut self) -> Option<Word> {
        if let Some(word) = self.current_word_cache.take() {
            return Some(word);
        }

        if let Some(word) = self.current_zsh_glob_word_from_source() {
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
        word
    }

    fn take_current_word_and_advance(&mut self) -> Option<Word> {
        let word = self.take_current_word()?;
        self.advance_past_word(&word);
        Some(word)
    }

    fn current_zsh_glob_word_from_source(&mut self) -> Option<Word> {
        if !matches!(
            self.current_token_kind,
            Some(TokenKind::LeftParen | TokenKind::Word)
        ) {
            return None;
        }

        let start = self.current_span.start;
        if !self.source_word_contains_zsh_glob_control(start) {
            return None;
        }
        let (text, end) = self.scan_source_word(start)?;
        if !text.contains("(#") {
            return None;
        }
        let span = Span::from_positions(start, end);
        if self.zsh_glob_qualifiers_enabled_at(span.start.offset)
            && let Some(word) = self.maybe_parse_zsh_qualified_glob_word(&text, span, true)
        {
            return Some(word);
        }

        Some(self.parse_word_with_context(&text, span, start, true))
    }

    fn source_word_contains_zsh_glob_control(&self, start: Position) -> bool {
        if start.offset >= self.input.len() {
            return false;
        }

        let source = &self.input[start.offset..];
        let mut chars = source.chars().peekable();
        let mut cursor = start;
        let mut paren_depth = 0_i32;
        let mut brace_depth = 0_i32;
        let mut in_single = false;
        let mut in_double = false;
        let mut in_backtick = false;
        let mut escaped = false;
        let mut prev_char = None;

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
            if prev_char == Some('(') && ch == '#' {
                return true;
            }

            if escaped {
                escaped = false;
                prev_char = Some(ch);
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

            prev_char = Some(ch);
        }

        false
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
            .or_else(|| {
                (token.span.start.offset <= token.span.end.offset
                    && token.span.end.offset <= self.input.len())
                .then(|| Cow::Borrowed(&self.input[token.span.start.offset..token.span.end.offset]))
            })
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
        let raw_text = token.word_string()?;
        let text_had_escape_markers = raw_text.contains('\x00');
        let text = if text_had_escape_markers {
            raw_text.replace('\x00', "")
        } else {
            raw_text
        };

        match token.kind {
            TokenKind::LiteralWord => Some((text, true)),
            TokenKind::QuotedWord if !Self::word_text_needs_parse(&text) => Some((text, true)),
            TokenKind::Word if !Self::word_text_needs_parse(&text) => Some((
                text,
                token.flags.has_cooked_text() || text_had_escape_markers,
            )),
            _ => None,
        }
    }

    fn nested_stmt_seq_from_source(&mut self, source: &str, base: Position) -> StmtSeq {
        let remaining_depth = self.max_depth.saturating_sub(self.current_depth);
        let nested_profile = self
            .current_zsh_options()
            .cloned()
            .map(|options| ShellProfile::with_zsh_options(self.dialect, options))
            .unwrap_or_else(|| self.shell_profile.clone());
        let inner_parser =
            Parser::with_limits_and_profile(source, remaining_depth, self.fuel, nested_profile);
        let mut output = inner_parser.parse();
        if output.is_ok() {
            Self::materialize_stmt_seq_source_backing(&mut output.file.body, source);
            Self::rebase_file(&mut output.file, base);
            output.file.body
        } else {
            StmtSeq {
                leading_comments: Vec::new(),
                stmts: Vec::new(),
                trailing_comments: Vec::new(),
                span: Span::from_positions(base, base),
            }
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
        if let Some(word) = &mut subscript.word_ast {
            Self::rebase_word(word, base);
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
                    ForSyntax::InDirect { in_span } => ForSyntax::InDirect {
                        in_span: in_span.map(|span| span.rebased(base)),
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
                    ForSyntax::ParenDirect {
                        left_paren_span,
                        right_paren_span,
                    } => ForSyntax::ParenDirect {
                        left_paren_span: left_paren_span.rebased(base),
                        right_paren_span: right_paren_span.rebased(base),
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
                    RepeatSyntax::Direct => RepeatSyntax::Direct,
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

    fn materialize_stmt_seq_source_backing(sequence: &mut StmtSeq, source: &str) {
        for stmt in &mut sequence.stmts {
            Self::materialize_stmt_source_backing(stmt, source);
        }
    }

    fn materialize_stmt_source_backing(stmt: &mut Stmt, source: &str) {
        Self::materialize_ast_command_source_backing(&mut stmt.command, source);
    }

    fn materialize_ast_command_source_backing(command: &mut AstCommand, source: &str) {
        match command {
            AstCommand::Simple(simple) => {
                Self::materialize_word_source_backing(&mut simple.name, source);
            }
            AstCommand::Builtin(_) | AstCommand::Decl(_) => {}
            AstCommand::Binary(binary) => {
                Self::materialize_stmt_source_backing(binary.left.as_mut(), source);
                Self::materialize_stmt_source_backing(binary.right.as_mut(), source);
            }
            AstCommand::Compound(compound) => {
                Self::materialize_compound_source_backing(compound, source);
            }
            AstCommand::Function(function) => {
                Self::materialize_stmt_source_backing(function.body.as_mut(), source);
            }
            AstCommand::AnonymousFunction(function) => {
                Self::materialize_stmt_source_backing(function.body.as_mut(), source);
            }
        }
    }

    fn materialize_compound_source_backing(compound: &mut CompoundCommand, source: &str) {
        match compound {
            CompoundCommand::If(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.then_branch, source);
                for (condition, body) in &mut command.elif_branches {
                    Self::materialize_stmt_seq_source_backing(condition, source);
                    Self::materialize_stmt_seq_source_backing(body, source);
                }
                if let Some(else_branch) = &mut command.else_branch {
                    Self::materialize_stmt_seq_source_backing(else_branch, source);
                }
            }
            CompoundCommand::For(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Repeat(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Foreach(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::ArithmeticFor(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::While(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
            }
            CompoundCommand::Until(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.condition, source);
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
            }
            CompoundCommand::Case(command) => {
                for case in &mut command.cases {
                    Self::materialize_stmt_seq_source_backing(&mut case.body, source);
                }
            }
            CompoundCommand::Select(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source)
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                Self::materialize_stmt_seq_source_backing(commands, source);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(inner) = &mut command.command {
                    Self::materialize_stmt_source_backing(inner.as_mut(), source);
                }
            }
            CompoundCommand::Conditional(_) => {}
            CompoundCommand::Coproc(command) => {
                Self::materialize_stmt_source_backing(command.body.as_mut(), source);
            }
            CompoundCommand::Always(command) => {
                Self::materialize_stmt_seq_source_backing(&mut command.body, source);
                Self::materialize_stmt_seq_source_backing(&mut command.always_body, source);
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

    fn materialize_literal_text_source_backing(text: &mut LiteralText, span: Span, source: &str) {
        match text {
            LiteralText::Source => {
                *text = LiteralText::owned(span.slice(source).to_string());
            }
            LiteralText::CookedSource(cooked) => {
                *text = LiteralText::owned(cooked.to_string());
            }
            LiteralText::Owned(_) => {}
        }
    }

    fn materialize_source_text_source_backing(text: &mut SourceText, source: &str) {
        if text.is_source_backed() {
            let span = text.span();
            let cooked = text.slice(source).to_string();
            *text = SourceText::cooked(span, cooked);
        }
    }

    fn materialize_word_source_backing(word: &mut Word, source: &str) {
        for part in &mut word.parts {
            Self::materialize_word_part_source_backing(part, source);
        }
    }

    fn materialize_pattern_source_backing(pattern: &mut Pattern, source: &str) {
        for part in &mut pattern.parts {
            Self::materialize_pattern_part_source_backing(part, source);
        }
    }

    fn materialize_pattern_part_source_backing(part: &mut PatternPartNode, source: &str) {
        match &mut part.kind {
            PatternPart::Literal(text) => {
                Self::materialize_literal_text_source_backing(text, part.span, source);
            }
            PatternPart::CharClass(text) => {
                Self::materialize_source_text_source_backing(text, source);
            }
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    Self::materialize_pattern_source_backing(pattern, source);
                }
            }
            PatternPart::Word(word) => Self::materialize_word_source_backing(word, source),
            PatternPart::AnyString | PatternPart::AnyChar => {}
        }
    }

    fn materialize_word_part_source_backing(part: &mut WordPartNode, source: &str) {
        match &mut part.kind {
            WordPart::Literal(text) => {
                Self::materialize_literal_text_source_backing(text, part.span, source);
            }
            WordPart::ZshQualifiedGlob(glob) => {
                Self::materialize_zsh_qualified_glob_source_backing(glob, source);
            }
            WordPart::SingleQuoted { value, .. } => {
                Self::materialize_source_text_source_backing(value, source);
            }
            WordPart::DoubleQuoted { parts, .. } => {
                for part in parts {
                    Self::materialize_word_part_source_backing(part, source);
                }
            }
            WordPart::Parameter(parameter) => {
                Self::materialize_source_text_source_backing(&mut parameter.raw_body, source);
                Self::materialize_parameter_expansion_syntax_source_backing(
                    &mut parameter.syntax,
                    source,
                );
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                Self::materialize_parameter_operator_source_backing(operator, source);
                if let Some(operand) = operand {
                    Self::materialize_source_text_source_backing(operand, source);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
            }
            WordPart::ArrayAccess(reference)
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => {
                Self::materialize_var_ref_source_backing(reference, source);
            }
            WordPart::Substring {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                Self::materialize_source_text_source_backing(offset, source);
                Self::materialize_word_source_backing(offset_word_ast, source);
                if let Some(expr) = offset_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
                if let Some(length) = length {
                    Self::materialize_source_text_source_backing(length, source);
                }
                if let Some(word_ast) = length_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
                if let Some(expr) = length_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::materialize_var_ref_source_backing(reference, source);
                if let Some(operator) = operator {
                    Self::materialize_parameter_operator_source_backing(operator, source);
                }
                if let Some(operand) = operand {
                    Self::materialize_source_text_source_backing(operand, source);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::materialize_word_source_backing(word_ast, source);
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                Self::materialize_source_text_source_backing(expression, source);
                Self::materialize_word_source_backing(expression_word_ast, source);
                if let Some(expr) = expression_ast {
                    Self::materialize_arithmetic_expr_source_backing(expr, source);
                }
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. } => {}
        }
    }

    fn materialize_var_ref_source_backing(reference: &mut VarRef, source: &str) {
        if let Some(subscript) = &mut reference.subscript {
            Self::materialize_subscript_source_backing(subscript, source);
        }
    }

    fn materialize_subscript_source_backing(subscript: &mut Subscript, source: &str) {
        Self::materialize_source_text_source_backing(&mut subscript.text, source);
        if let Some(raw) = &mut subscript.raw {
            Self::materialize_source_text_source_backing(raw, source);
        }
        if let Some(word_ast) = &mut subscript.word_ast {
            Self::materialize_word_source_backing(word_ast, source);
        }
        if let Some(expr) = &mut subscript.arithmetic_ast {
            Self::materialize_arithmetic_expr_source_backing(expr, source);
        }
    }

    fn materialize_zsh_qualified_glob_source_backing(glob: &mut ZshQualifiedGlob, source: &str) {
        for segment in &mut glob.segments {
            match segment {
                ZshGlobSegment::Pattern(pattern) => {
                    Self::materialize_pattern_source_backing(pattern, source);
                }
                ZshGlobSegment::InlineControl(_) => {}
            }
        }
        if let Some(qualifiers) = &mut glob.qualifiers {
            for fragment in &mut qualifiers.fragments {
                match fragment {
                    ZshGlobQualifier::LetterSequence { text, .. } => {
                        Self::materialize_source_text_source_backing(text, source);
                    }
                    ZshGlobQualifier::NumericArgument { start, end, .. } => {
                        Self::materialize_source_text_source_backing(start, source);
                        if let Some(end) = end {
                            Self::materialize_source_text_source_backing(end, source);
                        }
                    }
                    ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => {}
                }
            }
        }
    }

    fn materialize_parameter_expansion_syntax_source_backing(
        syntax: &mut ParameterExpansionSyntax,
        source: &str,
    ) {
        match syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference }
                | BourneParameterExpansion::Length { reference }
                | BourneParameterExpansion::Indices { reference }
                | BourneParameterExpansion::Transformation { reference, .. } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                }
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    if let Some(operator) = operator {
                        Self::materialize_parameter_operator_source_backing(operator, source);
                    }
                    if let Some(operand) = operand {
                        Self::materialize_source_text_source_backing(operand, source);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    Self::materialize_source_text_source_backing(offset, source);
                    Self::materialize_word_source_backing(offset_word_ast, source);
                    if let Some(expr) = offset_ast {
                        Self::materialize_arithmetic_expr_source_backing(expr, source);
                    }
                    if let Some(length) = length {
                        Self::materialize_source_text_source_backing(length, source);
                    }
                    if let Some(word_ast) = length_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                    if let Some(expr) = length_ast {
                        Self::materialize_arithmetic_expr_source_backing(expr, source);
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::materialize_var_ref_source_backing(reference, source);
                    Self::materialize_parameter_operator_source_backing(operator, source);
                    if let Some(operand) = operand {
                        Self::materialize_source_text_source_backing(operand, source);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::materialize_word_source_backing(word_ast, source);
                    }
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &mut syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        Self::materialize_var_ref_source_backing(reference, source);
                    }
                    ZshExpansionTarget::Word(word) => {
                        Self::materialize_word_source_backing(word, source);
                    }
                    ZshExpansionTarget::Nested(parameter) => {
                        Self::materialize_source_text_source_backing(
                            &mut parameter.raw_body,
                            source,
                        );
                        Self::materialize_parameter_expansion_syntax_source_backing(
                            &mut parameter.syntax,
                            source,
                        );
                    }
                    ZshExpansionTarget::Empty => {}
                }
                for modifier in &mut syntax.modifiers {
                    if let Some(argument) = &mut modifier.argument {
                        Self::materialize_source_text_source_backing(argument, source);
                    }
                    if let Some(argument_word_ast) = &mut modifier.argument_word_ast {
                        Self::materialize_word_source_backing(argument_word_ast, source);
                    }
                }
                if let Some(operation) = &mut syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::Defaulting {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::TrimOperation {
                            operand,
                            operand_word_ast,
                            ..
                        } => {
                            Self::materialize_source_text_source_backing(operand, source);
                            Self::materialize_word_source_backing(operand_word_ast, source);
                        }
                        ZshExpansionOperation::Unknown { text, word_ast } => {
                            Self::materialize_source_text_source_backing(text, source);
                            Self::materialize_word_source_backing(word_ast, source);
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            pattern_word_ast,
                            replacement,
                            replacement_word_ast,
                            ..
                        } => {
                            Self::materialize_source_text_source_backing(pattern, source);
                            Self::materialize_word_source_backing(pattern_word_ast, source);
                            if let Some(replacement) = replacement {
                                Self::materialize_source_text_source_backing(replacement, source);
                            }
                            if let Some(replacement_word_ast) = replacement_word_ast {
                                Self::materialize_word_source_backing(replacement_word_ast, source);
                            }
                        }
                        ZshExpansionOperation::Slice {
                            offset,
                            offset_word_ast,
                            length,
                            length_word_ast,
                        } => {
                            Self::materialize_source_text_source_backing(offset, source);
                            Self::materialize_word_source_backing(offset_word_ast, source);
                            if let Some(length) = length {
                                Self::materialize_source_text_source_backing(length, source);
                            }
                            if let Some(length_word_ast) = length_word_ast {
                                Self::materialize_word_source_backing(length_word_ast, source);
                            }
                        }
                    }
                }
            }
        }
    }

    fn materialize_parameter_operator_source_backing(operator: &mut ParameterOp, source: &str) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                Self::materialize_pattern_source_backing(pattern, source);
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                replacement_word_ast,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                replacement_word_ast,
            } => {
                Self::materialize_pattern_source_backing(pattern, source);
                Self::materialize_source_text_source_backing(replacement, source);
                Self::materialize_word_source_backing(replacement_word_ast, source);
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

    fn materialize_arithmetic_expr_source_backing(expr: &mut ArithmeticExprNode, source: &str) {
        match &mut expr.kind {
            ArithmeticExpr::Number(text) => {
                Self::materialize_source_text_source_backing(text, source);
            }
            ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => {
                Self::materialize_arithmetic_expr_source_backing(index, source);
            }
            ArithmeticExpr::ShellWord(word) => {
                Self::materialize_word_source_backing(word, source);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                Self::materialize_arithmetic_expr_source_backing(expression, source);
            }
            ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
                Self::materialize_arithmetic_expr_source_backing(expr, source);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                Self::materialize_arithmetic_expr_source_backing(left, source);
                Self::materialize_arithmetic_expr_source_backing(right, source);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::materialize_arithmetic_expr_source_backing(condition, source);
                Self::materialize_arithmetic_expr_source_backing(then_expr, source);
                Self::materialize_arithmetic_expr_source_backing(else_expr, source);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                Self::materialize_arithmetic_lvalue_source_backing(target, source);
                Self::materialize_arithmetic_expr_source_backing(value, source);
            }
        }
    }

    fn materialize_arithmetic_lvalue_source_backing(target: &mut ArithmeticLvalue, source: &str) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => {
                Self::materialize_arithmetic_expr_source_backing(index, source);
            }
        }
    }

    fn rebase_word(word: &mut Word, base: Position) {
        word.span = word.span.rebased(base);
        for brace in &mut word.brace_syntax {
            brace.span = brace.span.rebased(base);
        }
        Self::rebase_word_parts(&mut word.parts, base);
    }

    fn rebase_heredoc_body(body: &mut HeredocBody, base: Position) {
        body.span = body.span.rebased(base);
        for part in &mut body.parts {
            Self::rebase_heredoc_body_part(part, base);
        }
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

    fn rebase_heredoc_body_part(part: &mut HeredocBodyPartNode, base: Position) {
        part.span = part.span.rebased(base);
        match &mut part.kind {
            HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
            HeredocBodyPart::CommandSubstitution { body, .. } => Self::rebase_stmt_seq(body, base),
            HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                expression.rebased(base);
                Self::rebase_word(expression_word_ast, base);
                if let Some(expr) = expression_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            HeredocBodyPart::Parameter(parameter) => {
                parameter.span = parameter.span.rebased(base);
                parameter.raw_body.rebased(base);
                Self::rebase_parameter_expansion_syntax(&mut parameter.syntax, base);
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
                operand_word_ast,
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
                        ..
                    }
                    | ParameterOp::ReplaceAll {
                        pattern,
                        replacement,
                        ..
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
                if let Some(word_ast) = operand_word_ast {
                    Self::rebase_word(word_ast, base);
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
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                offset.rebased(base);
                Self::rebase_word(offset_word_ast, base);
                if let Some(expr) = offset_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
                if let Some(length) = length {
                    length.rebased(base);
                }
                if let Some(word_ast) = length_word_ast {
                    Self::rebase_word(word_ast, base);
                }
                if let Some(expr) = length_ast {
                    Self::rebase_arithmetic_expr(expr, base);
                }
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                Self::rebase_var_ref(reference, base);
                if let Some(operator) = operator {
                    Self::rebase_parameter_operator(operator, base);
                }
                if let Some(operand) = operand {
                    operand.rebased(base);
                }
                if let Some(word_ast) = operand_word_ast {
                    Self::rebase_word(word_ast, base);
                }
            }
            WordPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                expression.rebased(base);
                Self::rebase_word(expression_word_ast, base);
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
            | ZshInlineGlobControl::Backreferences { span }
            | ZshInlineGlobControl::StartAnchor { span }
            | ZshInlineGlobControl::EndAnchor { span } => {
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
                BourneParameterExpansion::Indirect {
                    reference,
                    operand,
                    operator,
                    operand_word_ast,
                    ..
                } => {
                    Self::rebase_var_ref(reference, base);
                    if let Some(operator) = operator {
                        Self::rebase_parameter_operator(operator, base);
                    }
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset,
                    offset_ast,
                    offset_word_ast,
                    length,
                    length_ast,
                    length_word_ast,
                } => {
                    Self::rebase_var_ref(reference, base);
                    offset.rebased(base);
                    Self::rebase_word(offset_word_ast, base);
                    if let Some(expr) = offset_ast {
                        Self::rebase_arithmetic_expr(expr, base);
                    }
                    if let Some(length) = length {
                        length.rebased(base);
                    }
                    if let Some(word_ast) = length_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                    if let Some(expr) = length_ast {
                        Self::rebase_arithmetic_expr(expr, base);
                    }
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    Self::rebase_var_ref(reference, base);
                    Self::rebase_parameter_operator(operator, base);
                    if let Some(operand) = operand {
                        operand.rebased(base);
                    }
                    if let Some(word_ast) = operand_word_ast {
                        Self::rebase_word(word_ast, base);
                    }
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &mut syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        Self::rebase_var_ref(reference, base)
                    }
                    ZshExpansionTarget::Word(word) => Self::rebase_word(word, base),
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
                    if let Some(argument_word_ast) = &mut modifier.argument_word_ast {
                        Self::rebase_word(argument_word_ast, base);
                    }
                }
                if let Some(length_prefix) = &mut syntax.length_prefix {
                    *length_prefix = length_prefix.rebased(base);
                }
                if let Some(operation) = &mut syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::Defaulting {
                            operand,
                            operand_word_ast,
                            ..
                        }
                        | ZshExpansionOperation::TrimOperation {
                            operand,
                            operand_word_ast,
                            ..
                        } => {
                            operand.rebased(base);
                            Self::rebase_word(operand_word_ast, base);
                        }
                        ZshExpansionOperation::Unknown { text, word_ast } => {
                            text.rebased(base);
                            Self::rebase_word(word_ast, base);
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            pattern_word_ast,
                            replacement,
                            replacement_word_ast,
                            ..
                        } => {
                            pattern.rebased(base);
                            Self::rebase_word(pattern_word_ast, base);
                            if let Some(replacement) = replacement {
                                replacement.rebased(base);
                            }
                            if let Some(replacement_word_ast) = replacement_word_ast {
                                Self::rebase_word(replacement_word_ast, base);
                            }
                        }
                        ZshExpansionOperation::Slice {
                            offset,
                            offset_word_ast,
                            length,
                            length_word_ast,
                        } => {
                            offset.rebased(base);
                            Self::rebase_word(offset_word_ast, base);
                            if let Some(length) = length {
                                length.rebased(base);
                            }
                            if let Some(length_word_ast) = length_word_ast {
                                Self::rebase_word(length_word_ast, base);
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
                replacement_word_ast,
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement,
                replacement_word_ast,
            } => {
                Self::rebase_pattern(pattern, base);
                replacement.rebased(base);
                Self::rebase_word(replacement_word_ast, base);
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
        source_backed: bool,
    ) {
        if !current.is_empty() {
            Self::push_word_part(
                parts,
                WordPart::Literal(self.literal_text(
                    std::mem::take(current),
                    current_start,
                    end,
                    source_backed,
                )),
                current_start,
                end,
            );
        }
    }

    fn literal_text(
        &self,
        text: String,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, &text) {
            LiteralText::source()
        } else if source_backed {
            LiteralText::cooked_source(text)
        } else {
            LiteralText::owned(text)
        }
    }

    fn literal_text_from_str(
        &self,
        text: &str,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        self.literal_text_impl(text, None, start, end, source_backed)
    }

    fn literal_text_impl(
        &self,
        text: &str,
        owned: Option<String>,
        start: Position,
        end: Position,
        source_backed: bool,
    ) -> LiteralText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, text) {
            LiteralText::source()
        } else if source_backed {
            LiteralText::cooked_source(owned.unwrap_or_else(|| text.to_owned()))
        } else {
            LiteralText::owned(owned.unwrap_or_else(|| text.to_owned()))
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

    fn source_text_from_str(&self, text: &str, start: Position, end: Position) -> SourceText {
        self.source_text_impl(text, None, start, end)
    }

    fn source_text_impl(
        &self,
        text: &str,
        owned: Option<String>,
        start: Position,
        end: Position,
    ) -> SourceText {
        let span = Span::from_positions(start, end);
        if self.source_matches(span, text) {
            SourceText::source(span)
        } else {
            SourceText::cooked(span, owned.unwrap_or_else(|| text.to_owned()))
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
        let word_ast = if matches!(kind, SubscriptKind::Ordinary) {
            Some(self.parse_source_text_as_word(raw.as_ref().unwrap_or(&text)))
        } else {
            None
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
            word_ast,
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
                operand_word_ast,
                colon_variant,
            } => Some(BourneParameterExpansion::Operation {
                reference,
                operator: self.enrich_parameter_operator(operator),
                operand,
                operand_word_ast,
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
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            }
            | WordPart::ArraySlice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            } => Some(BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
            }),
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                colon_variant,
            } => Some(BourneParameterExpansion::Indirect {
                reference,
                operator: operator.map(|operator| self.enrich_parameter_operator(operator)),
                operand,
                operand_word_ast,
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

    fn enrich_parameter_operator(&self, operator: ParameterOp) -> ParameterOp {
        match operator {
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
                ..
            } => ParameterOp::ReplaceFirst {
                pattern,
                replacement_word_ast: self.parse_source_text_as_word(&replacement),
                replacement,
            },
            ParameterOp::ReplaceAll {
                pattern,
                replacement,
                ..
            } => ParameterOp::ReplaceAll {
                pattern,
                replacement_word_ast: self.parse_source_text_as_word(&replacement),
                replacement,
            },
            ParameterOp::UseDefault
            | ParameterOp::AssignDefault
            | ParameterOp::UseReplacement
            | ParameterOp::Error
            | ParameterOp::RemovePrefixShort { .. }
            | ParameterOp::RemovePrefixLong { .. }
            | ParameterOp::RemoveSuffixShort { .. }
            | ParameterOp::RemoveSuffixLong { .. }
            | ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => operator,
        }
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
        &mut self,
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

    fn parse_zsh_modifier_group(
        &self,
        text: &str,
        base: Position,
        start: usize,
    ) -> Option<(usize, Vec<ZshModifier>)> {
        let rest = text.get(start..)?;
        if !rest.starts_with('(') {
            return None;
        }

        let close_rel = rest[1..].find(')')?;
        let close = start + 1 + close_rel;
        let group_text = &text[start..=close];
        let inner = &text[start + 1..close];
        let group_start = base.advanced_by(&text[..start]);
        let group_span = Span::from_positions(group_start, group_start.advanced_by(group_text));
        let mut modifiers = Vec::new();
        let mut index = 0usize;

        while index < inner.len() {
            let name = inner[index..].chars().next()?;
            index += name.len_utf8();

            let mut argument_delimiter = None;
            let mut argument = None;
            if matches!(name, 's' | 'j')
                && let Some(delimiter) = inner[index..].chars().next()
            {
                index += delimiter.len_utf8();
                let argument_start = index;
                while index < inner.len() {
                    let ch = inner[index..].chars().next()?;
                    if ch == delimiter {
                        let argument_text = &inner[argument_start..index];
                        let argument_base =
                            group_start.advanced_by(&group_text[..1 + argument_start]);
                        let argument_end = argument_base.advanced_by(argument_text);
                        argument_delimiter = Some(delimiter);
                        argument = Some(self.source_text(
                            argument_text.to_string(),
                            argument_base,
                            argument_end,
                        ));
                        index += delimiter.len_utf8();
                        break;
                    }
                    index += ch.len_utf8();
                }
            }

            let argument_word_ast = argument
                .as_ref()
                .map(|argument| self.parse_source_text_as_word(argument));

            modifiers.push(ZshModifier {
                name,
                argument,
                argument_word_ast,
                argument_delimiter,
                span: group_span,
            });
        }

        Some((close + 1, modifiers))
    }

    fn parse_zsh_parameter_syntax(
        &mut self,
        raw_body: &SourceText,
        base: Position,
    ) -> ZshParameterExpansion {
        let text = raw_body.slice(self.input);
        let mut index = 0;
        let mut modifiers = Vec::new();
        let mut length_prefix = None;
        let source_backed = raw_body.is_source_backed();

        while text[index..].starts_with('(')
            && let Some((next_index, group_modifiers)) =
                self.parse_zsh_modifier_group(text, base, index)
        {
            modifiers.extend(group_modifiers);
            index = next_index;
        }

        while index < text.len() {
            let Some(flag) = text[index..].chars().next() else {
                break;
            };
            match flag {
                '=' | '~' | '^' => {
                    let modifier_start = base.advanced_by(&text[..index]);
                    let modifier_end =
                        modifier_start.advanced_by(&text[index..index + flag.len_utf8()]);
                    modifiers.push(ZshModifier {
                        name: flag,
                        argument: None,
                        argument_word_ast: None,
                        argument_delimiter: None,
                        span: Span::from_positions(modifier_start, modifier_end),
                    });
                    index += flag.len_utf8();
                }
                '#' if length_prefix.is_none() => {
                    let prefix_start = base.advanced_by(&text[..index]);
                    let prefix_end = prefix_start.advanced_by("#");
                    length_prefix = Some(Span::from_positions(prefix_start, prefix_end));
                    index += '#'.len_utf8();
                }
                _ => break,
            }
        }

        let (target, operation_index) = if text[index..].starts_with("${") {
            let end = self
                .find_matching_parameter_end(&text[index..])
                .unwrap_or(text.len() - index);
            let nested_text = &text[index..index + end];
            let target =
                self.parse_nested_parameter_target(nested_text, base.advanced_by(&text[..index]));
            (target, index + end)
        } else if text[index..].starts_with(':') || text[index..].is_empty() {
            (ZshExpansionTarget::Empty, index)
        } else {
            let end = self
                .find_zsh_operation_start(&text[index..])
                .map(|offset| index + offset)
                .unwrap_or(text.len());
            let raw_target = &text[index..end];
            let trimmed = raw_target.trim();
            let target = if trimmed.is_empty() {
                ZshExpansionTarget::Empty
            } else {
                let leading = raw_target
                    .len()
                    .saturating_sub(raw_target.trim_start().len());
                let target_base = base.advanced_by(&text[..index + leading]);
                self.parse_zsh_target_from_text(
                    trimmed,
                    target_base,
                    source_backed && leading == 0 && trimmed.len() == raw_target.len(),
                )
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
            length_prefix,
            operation,
        }
    }

    fn parse_zsh_target_from_text(
        &mut self,
        text: &str,
        base: Position,
        source_backed: bool,
    ) -> ZshExpansionTarget {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return ZshExpansionTarget::Empty;
        }

        if trimmed.starts_with("${") && trimmed.ends_with('}') {
            return self.parse_nested_parameter_target(trimmed, base);
        }

        if let Some(reference) = self.maybe_parse_loose_var_ref_target(trimmed) {
            return ZshExpansionTarget::Reference(reference);
        }

        let span = Span::from_positions(base, base.advanced_by(trimmed));
        let word = self.parse_word_with_context(trimmed, span, base, source_backed);
        if let Some(reference) =
            self.parse_var_ref_from_word(&word, SubscriptInterpretation::Contextual)
        {
            ZshExpansionTarget::Reference(reference)
        } else {
            ZshExpansionTarget::Word(word)
        }
    }

    fn maybe_parse_loose_var_ref_target(&self, text: &str) -> Option<VarRef> {
        let trimmed = text.trim();
        Self::looks_like_plain_parameter_access(trimmed).then(|| self.parse_loose_var_ref(trimmed))
    }

    fn is_plain_special_parameter_name(name: &str) -> bool {
        matches!(name, "#" | "$" | "!" | "*" | "@" | "?" | "-") || name == "0"
    }

    fn looks_like_plain_parameter_access(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }

        let name = if let Some(open) = trimmed.find('[') {
            if !trimmed.ends_with(']') {
                return false;
            }
            &trimmed[..open]
        } else {
            trimmed
        };

        Self::is_valid_identifier(name)
            || name.bytes().all(|byte| byte.is_ascii_digit())
            || Self::is_plain_special_parameter_name(name)
    }

    fn parse_nested_parameter_target(&mut self, text: &str, base: Position) -> ZshExpansionTarget {
        if !(text.starts_with("${") && text.ends_with('}')) {
            return self.parse_zsh_target_from_text(text, base, false);
        }

        let raw_body_start = base.advanced_by("${");
        let raw_body = self.source_text(
            text[2..text.len() - 1].to_string(),
            raw_body_start,
            base.advanced_by(&text[..text.len() - 1]),
        );
        let raw_body_text = raw_body.slice(self.input);
        let has_operation = self.find_zsh_operation_start(raw_body_text).is_some();
        let syntax = if Self::looks_like_plain_parameter_access(raw_body_text) && !has_operation {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: self.parse_loose_var_ref(raw_body_text),
            })
        } else if raw_body_text.starts_with('(')
            || raw_body_text.starts_with(':')
            || raw_body_text.starts_with('=')
            || raw_body_text.starts_with('^')
            || raw_body_text.starts_with('~')
            || raw_body_text.starts_with('.')
            || raw_body_text.starts_with('#')
            || raw_body_text.starts_with('"')
            || raw_body_text.starts_with('\'')
            || raw_body_text.starts_with('$')
            || has_operation
        {
            ParameterExpansionSyntax::Zsh(
                self.parse_zsh_parameter_syntax(&raw_body, raw_body_start),
            )
        } else {
            ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access {
                reference: self.parse_loose_var_ref(raw_body_text),
            })
        };

        ZshExpansionTarget::Nested(Box::new(ParameterExpansion {
            syntax,
            span: Span::from_positions(base, base.advanced_by(text)),
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
            let subscript = self.subscript_from_source_text(
                SourceText::from(subscript_text.to_string()),
                None,
                SubscriptInterpretation::Contextual,
            );
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

    fn zsh_simple_modifier_suffix_segment(segment: &str) -> bool {
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return false;
        };

        match first {
            'a' | 'A' | 'c' | 'e' | 'l' | 'P' | 'q' | 'Q' | 'r' | 'u' => chars.next().is_none(),
            'h' | 't' => chars.all(|ch| ch.is_ascii_digit()),
            _ => false,
        }
    }

    fn zsh_modifier_suffix_candidate(rest: &str) -> bool {
        if rest.is_empty() {
            return false;
        }

        let Some(first) = rest.chars().next() else {
            return false;
        };
        if first.is_ascii_digit()
            || first.is_ascii_whitespace()
            || matches!(first, '$' | '\'' | '"' | '(' | '{')
        {
            return false;
        }

        rest.split(':')
            .all(Self::zsh_simple_modifier_suffix_segment)
    }

    fn zsh_slice_candidate(rest: &str) -> bool {
        let Some(first) = rest.chars().next() else {
            return false;
        };

        !Self::zsh_modifier_suffix_candidate(rest)
            && (first.is_ascii_alphanumeric()
                || first == '_'
                || first.is_ascii_whitespace()
                || matches!(first, '$' | '\'' | '"' | '(' | '{'))
    }

    fn parse_zsh_parameter_operation(&self, text: &str, base: Position) -> ZshExpansionOperation {
        if let Some(operand) = text.strip_prefix(":#") {
            let operand = self.source_text(
                operand.to_string(),
                base.advanced_by(":#"),
                base.advanced_by(text),
            );
            return ZshExpansionOperation::PatternOperation {
                kind: ZshPatternOp::Filter,
                operand_word_ast: self.parse_source_text_as_word(&operand),
                operand,
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
            let operand = self.source_text(
                operand.to_string(),
                base.advanced_by(&text[..2]),
                base.advanced_by(text),
            );
            return ZshExpansionOperation::Defaulting {
                kind,
                operand_word_ast: self.parse_source_text_as_word(&operand),
                operand,
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
            let operand = self.zsh_operation_source_text(text, base, prefix_len, text.len());
            return ZshExpansionOperation::TrimOperation {
                kind,
                operand_word_ast: self.parse_source_text_as_word(&operand),
                operand,
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
            let pattern =
                self.zsh_operation_source_text(text, base, prefix_len, prefix_len + pattern_end);
            let replacement = separator.map(|separator| {
                self.zsh_operation_source_text(text, base, prefix_len + separator + 1, text.len())
            });
            return ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern_word_ast: self.parse_source_text_as_word(&pattern),
                replacement_word_ast: self.parse_optional_source_text_as_word(replacement.as_ref()),
                pattern,
                replacement,
            };
        }

        if let Some(rest) = text.strip_prefix(':') {
            if Self::zsh_modifier_suffix_candidate(rest) {
                let text = self.source_text(text.to_string(), base, base.advanced_by(text));
                return ZshExpansionOperation::Unknown {
                    word_ast: self.parse_source_text_as_word(&text),
                    text,
                };
            }

            if Self::zsh_slice_candidate(rest) {
                let separator = self.find_zsh_top_level_delimiter(rest, ':');
                let offset_end = separator.unwrap_or(rest.len());
                let offset = self.zsh_operation_source_text(text, base, 1, 1 + offset_end);
                let length = separator.map(|separator| {
                    self.zsh_operation_source_text(text, base, 1 + separator + 1, text.len())
                });
                return ZshExpansionOperation::Slice {
                    offset_word_ast: self.parse_source_text_as_word(&offset),
                    length_word_ast: self.parse_optional_source_text_as_word(length.as_ref()),
                    offset,
                    length,
                };
            }
        }

        let text = self.source_text(text.to_string(), base, base.advanced_by(text));
        ZshExpansionOperation::Unknown {
            word_ast: self.parse_source_text_as_word(&text),
            text,
        }
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
            self.dialect,
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
            self.dialect,
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

    fn parse_source_text_as_word(&self, text: &SourceText) -> Word {
        Self::parse_word_fragment(self.input, text.slice(self.input), text.span())
    }

    fn parse_optional_source_text_as_word(&self, text: Option<&SourceText>) -> Option<Word> {
        text.map(|text| self.parse_source_text_as_word(text))
    }

    fn source_matches(&self, span: Span, text: &str) -> bool {
        span.start.offset <= span.end.offset
            && span.end.offset <= self.input.len()
            && span.slice(self.input) == text
    }

    fn checkpoint(&self) -> ParserCheckpoint<'a> {
        ParserCheckpoint {
            lexer: self.lexer.clone(),
            synthetic_tokens: self.synthetic_tokens.clone(),
            alias_replays: self.alias_replays.clone(),
            current_token: self.current_token.clone(),
            current_word_cache: self.current_word_cache.clone(),
            current_token_kind: self.current_token_kind,
            current_keyword: self.current_keyword,
            current_span: self.current_span,
            peeked_token: self.peeked_token.clone(),
            current_depth: self.current_depth,
            fuel: self.fuel,
            comments: self.comments.clone(),
            expand_next_word: self.expand_next_word,
            brace_group_depth: self.brace_group_depth,
            brace_body_stack: self.brace_body_stack.clone(),
            syntax_facts: self.syntax_facts.clone(),
            #[cfg(feature = "benchmarking")]
            benchmark_counters: self.benchmark_counters,
        }
    }

    fn restore(&mut self, checkpoint: ParserCheckpoint<'a>) {
        self.lexer = checkpoint.lexer;
        self.synthetic_tokens = checkpoint.synthetic_tokens;
        self.alias_replays = checkpoint.alias_replays;
        self.current_token = checkpoint.current_token;
        self.current_word_cache = checkpoint.current_word_cache;
        self.current_token_kind = checkpoint.current_token_kind;
        self.current_keyword = checkpoint.current_keyword;
        self.current_span = checkpoint.current_span;
        self.peeked_token = checkpoint.peeked_token;
        self.current_depth = checkpoint.current_depth;
        self.fuel = checkpoint.fuel;
        self.comments = checkpoint.comments;
        self.expand_next_word = checkpoint.expand_next_word;
        self.brace_group_depth = checkpoint.brace_group_depth;
        self.brace_body_stack = checkpoint.brace_body_stack;
        self.syntax_facts = checkpoint.syntax_facts;
        #[cfg(feature = "benchmarking")]
        {
            self.benchmark_counters = checkpoint.benchmark_counters;
        }
    }

    fn set_current_spanned(&mut self, token: LexedToken<'a>) {
        #[cfg(feature = "benchmarking")]
        self.maybe_record_set_current_spanned_call();
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

    fn rebase_redirects(redirects: &mut [Redirect], base: Position) {
        for redirect in redirects {
            redirect.span = redirect.span.rebased(base);
            redirect.fd_var_span = redirect.fd_var_span.map(|span| span.rebased(base));
            match &mut redirect.target {
                RedirectTarget::Word(word) => Self::rebase_word(word, base),
                RedirectTarget::Heredoc(heredoc) => {
                    heredoc.delimiter.span = heredoc.delimiter.span.rebased(base);
                    Self::rebase_word(&mut heredoc.delimiter.raw, base);
                    Self::rebase_heredoc_body(&mut heredoc.body, base);
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
            self.zsh_short_repeat_enabled(),
            "repeat loops",
            "are not available in this shell mode",
        )
    }

    fn ensure_foreach_loop(&self) -> Result<()> {
        self.ensure_feature(
            self.zsh_short_loops_enabled(),
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
            let used = self.max_fuel - self.fuel;
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

    fn binary_stmt(left: Stmt, op: BinaryOp, op_span: Span, right: Stmt) -> Stmt {
        let span = left.span.merge(right.span);
        Stmt {
            leading_comments: Vec::new(),
            command: AstCommand::Binary(BinaryCommand {
                left: Box::new(left),
                op,
                op_span,
                right: Box::new(right),
                span,
            }),
            negated: false,
            redirects: Vec::new(),
            terminator: None,
            terminator_span: None,
            inline_comment: None,
            span,
        }
    }

    fn lower_builtin_command(
        builtin: BuiltinCommand,
    ) -> (AstBuiltinCommand, SmallVec<[Redirect; 1]>, Span) {
        match builtin {
            BuiltinCommand::Break(command) => {
                let span = command.span;
                let redirects = command.redirects;
                (
                    AstBuiltinCommand::Break(AstBreakCommand {
                        depth: command.depth,
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_vec(),
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
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_vec(),
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
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_vec(),
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
                        extra_args: command.extra_args.into_vec(),
                        assignments: command.assignments.into_vec(),
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
                    args: command.args.into_vec(),
                    assignments: command.assignments.into_vec(),
                    span: command.span,
                }),
                negated: false,
                redirects: command.redirects.into_vec(),
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
                    redirects: redirects.into_vec(),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span,
                }
            }
            Command::Decl(command) => {
                let command = *command;
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Decl(AstDeclClause {
                        variant: command.variant,
                        variant_span: command.variant_span,
                        operands: command.operands.into_vec(),
                        assignments: command.assignments.into_vec(),
                        span: command.span,
                    }),
                    negated: false,
                    redirects: command.redirects.into_vec(),
                    terminator: None,
                    terminator_span: None,
                    inline_comment: None,
                    span: command.span,
                }
            }
            Command::Compound(compound, redirects) => {
                let span = Self::compound_span(&compound);
                Stmt {
                    leading_comments: Vec::new(),
                    command: AstCommand::Compound(*compound),
                    negated: false,
                    redirects: redirects.into_vec(),
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
                redirects: redirects.into_vec(),
                terminator: None,
                terminator_span: None,
                inline_comment: None,
            },
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
        #[cfg(feature = "benchmarking")]
        self.maybe_record_advance_raw_call();
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

    #[cfg(feature = "benchmarking")]
    fn maybe_record_set_current_spanned_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.parser_set_current_spanned_calls += 1;
        }
    }

    #[cfg(feature = "benchmarking")]
    fn maybe_record_advance_raw_call(&mut self) {
        if let Some(counters) = &mut self.benchmark_counters {
            counters.parser_advance_raw_calls += 1;
        }
    }

    #[cfg(feature = "benchmarking")]
    fn finish_benchmark_counters(&self) -> ParserBenchmarkCounters {
        let mut counters = self.benchmark_counters.unwrap_or_default();
        counters.lexer_current_position_calls =
            self.lexer.benchmark_counters().current_position_calls;
        counters
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

    fn looks_like_disabled_repeat_loop(&mut self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Repeat) {
            return Ok(false);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if !self.at_word_like() {
            self.restore(checkpoint);
            return Ok(false);
        }
        self.advance();

        let result = match self.current_token_kind {
            Some(TokenKind::LeftBrace) => Ok(true),
            Some(TokenKind::Semicolon) => {
                self.advance();
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                Ok(self.current_keyword() == Some(Keyword::Do))
            }
            Some(TokenKind::Newline) => {
                if let Err(error) = self.skip_newlines() {
                    self.restore(checkpoint);
                    return Err(error);
                }
                Ok(self.current_keyword() == Some(Keyword::Do))
            }
            _ => Ok(false),
        };
        self.restore(checkpoint);
        result
    }

    fn looks_like_disabled_foreach_loop(&mut self) -> Result<bool> {
        if self.current_keyword() != Some(Keyword::Foreach) {
            return Ok(false);
        }

        let checkpoint = self.checkpoint();
        self.advance();
        if self.current_name_token().is_none() {
            self.restore(checkpoint);
            return Ok(false);
        }
        self.advance();

        let result = if self.at(TokenKind::LeftParen) {
            self.advance();
            let mut saw_word = false;
            while !self.at(TokenKind::RightParen) {
                if !self.at_word_like() {
                    self.restore(checkpoint);
                    return Ok(false);
                }
                saw_word = true;
                self.advance();
            }
            if !saw_word {
                self.restore(checkpoint);
                return Ok(false);
            }
            self.advance();
            Ok(self.at(TokenKind::LeftBrace))
        } else {
            if self.current_keyword() != Some(Keyword::In) {
                self.restore(checkpoint);
                return Ok(false);
            }
            self.advance();

            let mut saw_word = false;
            let saw_separator = loop {
                if self.current_keyword() == Some(Keyword::Do) {
                    break false;
                }

                match self.current_token_kind {
                    Some(kind) if kind.is_word_like() => {
                        saw_word = true;
                        self.advance();
                    }
                    Some(TokenKind::Semicolon) => {
                        self.advance();
                        break true;
                    }
                    Some(TokenKind::Newline) => {
                        if let Err(error) = self.skip_newlines() {
                            self.restore(checkpoint);
                            return Err(error);
                        }
                        break true;
                    }
                    _ => break false,
                }
            };

            Ok(saw_word && saw_separator && self.current_keyword() == Some(Keyword::Do))
        };
        self.restore(checkpoint);
        result
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
