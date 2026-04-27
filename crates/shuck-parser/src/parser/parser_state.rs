use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use smallvec::SmallVec;

use shuck_ast::{
    AnonymousFunctionCommand, Assignment, Comment, CompoundCommand, DeclOperand, FunctionDef, Name,
    Position, Redirect, Span, TokenKind, Word,
};

use super::{
    Keyword, LexedToken, Lexer, ShellDialect, ShellProfile, SyntaxFacts, ZshOptionTimeline,
};

#[cfg(feature = "benchmarking")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct ParserBenchmarkCounters {
    /// Number of lexer current-position lookups performed while parsing.
    pub lexer_current_position_calls: u64,
    /// Number of parser calls that updated the current spanned token.
    pub parser_set_current_spanned_calls: u64,
    /// Number of raw token-advance operations performed by the parser.
    pub parser_advance_raw_calls: u64,
}

#[derive(Debug, Clone)]
pub(super) struct SimpleCommand {
    pub(super) name: Word,
    pub(super) args: SmallVec<[Word; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct BreakCommand {
    pub(super) depth: Option<Word>,
    pub(super) extra_args: SmallVec<[Word; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct ContinueCommand {
    pub(super) depth: Option<Word>,
    pub(super) extra_args: SmallVec<[Word; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct ReturnCommand {
    pub(super) code: Option<Word>,
    pub(super) extra_args: SmallVec<[Word; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct ExitCommand {
    pub(super) code: Option<Word>,
    pub(super) extra_args: SmallVec<[Word; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) enum BuiltinCommand {
    Break(BreakCommand),
    Continue(ContinueCommand),
    Return(ReturnCommand),
    Exit(ExitCommand),
}

#[derive(Debug, Clone)]
pub(super) struct DeclClause {
    pub(super) variant: Name,
    pub(super) variant_span: Span,
    pub(super) operands: SmallVec<[DeclOperand; 2]>,
    pub(super) redirects: SmallVec<[Redirect; 1]>,
    pub(super) assignments: SmallVec<[Assignment; 1]>,
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) enum Command {
    Simple(SimpleCommand),
    Builtin(BuiltinCommand),
    Decl(Box<DeclClause>),
    Compound(Box<CompoundCommand>, SmallVec<[Redirect; 1]>),
    Function(FunctionDef),
    AnonymousFunction(AnonymousFunctionCommand, SmallVec<[Redirect; 1]>),
}

/// Parser for bash scripts.
#[derive(Clone)]
pub struct Parser<'a> {
    pub(super) input: &'a str,
    pub(super) lexer: Lexer<'a>,
    pub(super) synthetic_tokens: VecDeque<SyntheticToken>,
    pub(super) alias_replays: Vec<AliasReplay>,
    pub(super) current_token: Option<LexedToken<'a>>,
    pub(super) current_word_cache: Option<Word>,
    pub(super) current_token_kind: Option<TokenKind>,
    pub(super) current_keyword: Option<Keyword>,
    /// Span of the current token
    pub(super) current_span: Span,
    /// Lookahead token for function parsing
    pub(super) peeked_token: Option<LexedToken<'a>>,
    /// Maximum allowed AST nesting depth
    pub(super) max_depth: usize,
    /// Current nesting depth
    pub(super) current_depth: usize,
    /// Remaining fuel for parsing operations
    pub(super) fuel: usize,
    /// Maximum fuel (for error reporting)
    pub(super) max_fuel: usize,
    /// Depth of reparsing source-text operands as patterns.
    pub(super) source_text_pattern_depth: usize,
    /// Comments collected during parsing.
    pub(super) comments: Vec<Comment>,
    /// Known aliases declared earlier in the current parse stream.
    pub(super) aliases: HashMap<String, AliasDefinition>,
    /// Whether alias expansion is currently enabled.
    pub(super) expand_aliases: bool,
    /// Whether the next fetched word is eligible for alias expansion because
    /// the previous alias expansion ended with trailing whitespace.
    pub(super) expand_next_word: bool,
    /// Nesting depth of active brace-delimited statement sequences.
    pub(super) brace_group_depth: usize,
    /// Active brace-body parsing contexts, used to distinguish compact zsh
    /// closers from literal `}` arguments.
    pub(super) brace_body_stack: Vec<BraceBodyContext>,
    pub(super) syntax_facts: SyntaxFacts,
    pub(super) shell_profile: ShellProfile,
    pub(super) zsh_timeline: Option<Arc<ZshOptionTimeline>>,
    pub(super) dialect: ShellDialect,
    #[cfg(feature = "benchmarking")]
    pub(super) benchmark_counters: Option<ParserBenchmarkCounters>,
}

#[derive(Clone)]
pub(super) struct ParserCheckpoint<'a> {
    pub(super) lexer: Lexer<'a>,
    pub(super) synthetic_tokens: VecDeque<SyntheticToken>,
    pub(super) alias_replays: Vec<AliasReplay>,
    pub(super) current_token: Option<LexedToken<'a>>,
    pub(super) current_token_kind: Option<TokenKind>,
    pub(super) current_keyword: Option<Keyword>,
    pub(super) current_span: Span,
    pub(super) peeked_token: Option<LexedToken<'a>>,
    pub(super) current_depth: usize,
    pub(super) source_text_pattern_depth: usize,
    pub(super) fuel: usize,
    pub(super) comments: Vec<Comment>,
    pub(super) expand_next_word: bool,
    pub(super) brace_group_depth: usize,
    pub(super) brace_body_stack: Vec<BraceBodyContext>,
    pub(super) syntax_facts: SyntaxFacts,
    #[cfg(feature = "benchmarking")]
    pub(super) benchmark_counters: Option<ParserBenchmarkCounters>,
}

#[derive(Debug, Clone)]
pub(super) struct AliasDefinition {
    pub(super) tokens: Arc<[LexedToken<'static>]>,
    pub(super) expands_next_word: bool,
}

#[derive(Debug, Clone)]
pub(super) struct AliasReplay {
    pub(super) tokens: Arc<[LexedToken<'static>]>,
    pub(super) next_index: usize,
    pub(super) base: Position,
}

impl AliasReplay {
    pub(super) fn new(alias: &AliasDefinition, base: Position) -> Self {
        Self {
            tokens: Arc::clone(&alias.tokens),
            next_index: 0,
            base,
        }
    }

    pub(super) fn next_token<'b>(&mut self) -> Option<LexedToken<'b>> {
        let token = self.tokens.get(self.next_index)?.clone();
        self.next_index += 1;
        Some(token.into_owned().rebased(self.base).with_synthetic_flag())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SyntheticToken {
    pub(super) kind: TokenKind,
    pub(super) span: Span,
}

impl SyntheticToken {
    pub(super) const fn punctuation(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub(super) fn materialize<'b>(self) -> LexedToken<'b> {
        LexedToken::punctuation(self.kind).with_span(self.span)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FlowControlBuiltinKind {
    Break,
    Continue,
    Return,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BraceBodyContext {
    Ordinary,
    Function,
    IfClause,
}
