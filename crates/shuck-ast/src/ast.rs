//! AST types for parsed bash scripts
//!
//! These types define the abstract syntax tree for bash scripts.
//! All command nodes include source location spans for error messages and $LINENO.

#![allow(dead_code)]

use crate::{
    Name,
    span::{Position, Span, TextRange},
};
use std::{borrow::Cow, fmt};

/// Source-backed text for AST nodes that need stable spans but only occasionally
/// need owned cooked text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    span: Span,
    cooked: Option<Box<str>>,
}

impl SourceText {
    pub fn source(span: Span) -> Self {
        Self { span, cooked: None }
    }

    pub fn cooked(span: Span, text: impl Into<Box<str>>) -> Self {
        Self {
            span,
            cooked: Some(text.into()),
        }
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn slice<'a>(&'a self, source: &'a str) -> &'a str {
        self.cooked
            .as_deref()
            .unwrap_or_else(|| self.span.slice(source))
    }

    pub fn is_source_backed(&self) -> bool {
        self.cooked.is_none()
    }

    pub fn rebased(&mut self, base: Position) {
        self.span = self.span.rebased(base);
    }
}

impl From<Span> for SourceText {
    fn from(span: Span) -> Self {
        Self::source(span)
    }
}

impl From<&str> for SourceText {
    fn from(value: &str) -> Self {
        Self::cooked(Span::new(), value)
    }
}

impl From<String> for SourceText {
    fn from(value: String) -> Self {
        Self::cooked(Span::new(), value)
    }
}

/// Literal text within a word part.
///
/// Most literals can be recovered directly from the containing part node span.
/// Owned text is kept only for cooked or synthetic literals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralText {
    Source,
    Owned(Box<str>),
}

impl LiteralText {
    pub fn source() -> Self {
        Self::Source
    }

    pub fn owned(text: impl Into<Box<str>>) -> Self {
        Self::Owned(text.into())
    }

    pub fn as_str<'a>(&'a self, source: &'a str, span: Span) -> &'a str {
        match self {
            Self::Source => span.slice(source),
            Self::Owned(text) => text.as_ref(),
        }
    }

    pub fn is_source_backed(&self) -> bool {
        matches!(self, Self::Source)
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Owned(text) if text.is_empty())
    }
}

impl From<&str> for LiteralText {
    fn from(value: &str) -> Self {
        Self::owned(value)
    }
}

impl From<String> for LiteralText {
    fn from(value: String) -> Self {
        Self::owned(value)
    }
}

impl PartialEq<str> for LiteralText {
    fn eq(&self, other: &str) -> bool {
        matches!(self, Self::Owned(text) if text.as_ref() == other)
    }
}

impl PartialEq<&str> for LiteralText {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

/// A shell comment located by its byte range in the source.
///
/// The comment text (without the leading `#`) is obtained by slicing the
/// source: `comment.range.slice(source)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    pub range: TextRange,
}

/// A complete bash script.
#[derive(Debug, Clone)]
pub struct Script {
    pub commands: Vec<Command>,
    /// Source span of the entire script
    pub span: Span,
}

/// A single command in the script.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// A simple command (e.g., `echo hello`)
    Simple(SimpleCommand),

    /// A builtin command with a dedicated typed AST node
    Builtin(BuiltinCommand),

    /// A declaration builtin clause (`declare`, `local`, `export`, `readonly`, `typeset`)
    Decl(DeclClause),

    /// A pipeline (e.g., `ls | grep foo`)
    Pipeline(Pipeline),

    /// A command list (e.g., `a && b || c`)
    List(CommandList),

    /// A compound command (if, for, while, case, etc.) with optional redirections
    Compound(CompoundCommand, Vec<Redirect>),

    /// A function definition
    Function(FunctionDef),
}

/// A simple command with arguments and redirections.
#[derive(Debug, Clone)]
pub struct SimpleCommand {
    /// Command name
    pub name: Word,
    /// Command arguments
    pub args: Vec<Word>,
    /// Redirections
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the command
    pub assignments: Vec<Assignment>,
    /// Source span of this command
    pub span: Span,
}

/// A declaration builtin clause such as `declare`, `local`, `export`, `readonly`, or `typeset`.
#[derive(Debug, Clone)]
pub struct DeclClause {
    /// Declaration builtin variant.
    pub variant: Name,
    /// Source span of the declaration builtin name.
    pub variant_span: Span,
    /// Parsed declaration operands.
    pub operands: Vec<DeclOperand>,
    /// Redirections attached to the declaration clause.
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the declaration clause.
    pub assignments: Vec<Assignment>,
    /// Source span of this command.
    pub span: Span,
}

/// A typed operand inside a declaration clause.
#[derive(Debug, Clone)]
pub enum DeclOperand {
    /// A literal option word such as `-a` or `+x`.
    Flag(Word),
    /// A bare variable name or indexed reference.
    Name(VarRef),
    /// A typed assignment operand.
    Assignment(Assignment),
    /// A word whose runtime expansion may produce a flag, name, or assignment.
    Dynamic(Word),
}

/// How a subscript should be interpreted by downstream consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptInterpretation {
    Indexed,
    Associative,
    Contextual,
}

/// The syntactic shape of a parsed subscript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptKind {
    Ordinary,
    Selector(SubscriptSelector),
}

/// Array selector variants like `[@]` and `[*]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptSelector {
    At,
    Star,
}

impl SubscriptSelector {
    pub const fn as_char(self) -> char {
        match self {
            Self::At => '@',
            Self::Star => '*',
        }
    }
}

/// A typed array subscript or selector.
#[derive(Debug, Clone)]
pub struct Subscript {
    pub text: SourceText,
    /// Original subscript syntax when it differs from the cooked semantic text.
    pub raw: Option<SourceText>,
    pub kind: SubscriptKind,
    pub interpretation: SubscriptInterpretation,
    /// Typed arithmetic view of this subscript when it parses as arithmetic.
    pub arithmetic_ast: Option<ArithmeticExprNode>,
}

impl Subscript {
    pub fn span(&self) -> Span {
        self.text.span()
    }

    pub fn syntax_source_text(&self) -> &SourceText {
        self.raw.as_ref().unwrap_or(&self.text)
    }

    pub fn syntax_text<'a>(&'a self, source: &'a str) -> &'a str {
        self.syntax_source_text().slice(source)
    }

    pub fn is_array_selector(&self) -> bool {
        matches!(self.kind, SubscriptKind::Selector(_))
    }

    pub fn selector(&self) -> Option<SubscriptSelector> {
        match self.kind {
            SubscriptKind::Ordinary => None,
            SubscriptKind::Selector(selector) => Some(selector),
        }
    }

    pub fn is_source_backed(&self) -> bool {
        self.syntax_source_text().is_source_backed()
    }
}

/// A variable reference with an optional typed subscript.
#[derive(Debug, Clone)]
pub struct VarRef {
    pub name: Name,
    pub name_span: Span,
    pub subscript: Option<Subscript>,
    pub span: Span,
}

impl VarRef {
    pub fn has_array_selector(&self) -> bool {
        self.subscript
            .as_ref()
            .is_some_and(Subscript::is_array_selector)
    }

    pub fn is_source_backed(&self) -> bool {
        self.subscript
            .as_ref()
            .is_none_or(Subscript::is_source_backed)
    }
}

/// Builtin commands with dedicated AST nodes.
#[derive(Debug, Clone)]
pub enum BuiltinCommand {
    /// `break [N]`
    Break(BreakCommand),
    /// `continue [N]`
    Continue(ContinueCommand),
    /// `return [N]`
    Return(ReturnCommand),
    /// `exit [N]`
    Exit(ExitCommand),
}

/// `break [N]`
#[derive(Debug, Clone)]
pub struct BreakCommand {
    /// Optional loop depth argument
    pub depth: Option<Word>,
    /// Additional operands preserved for fidelity
    pub extra_args: Vec<Word>,
    /// Redirections attached to the builtin
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the builtin
    pub assignments: Vec<Assignment>,
    /// Source span of this command
    pub span: Span,
}

/// `continue [N]`
#[derive(Debug, Clone)]
pub struct ContinueCommand {
    /// Optional loop depth argument
    pub depth: Option<Word>,
    /// Additional operands preserved for fidelity
    pub extra_args: Vec<Word>,
    /// Redirections attached to the builtin
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the builtin
    pub assignments: Vec<Assignment>,
    /// Source span of this command
    pub span: Span,
}

/// `return [N]`
#[derive(Debug, Clone)]
pub struct ReturnCommand {
    /// Optional return code argument
    pub code: Option<Word>,
    /// Additional operands preserved for fidelity
    pub extra_args: Vec<Word>,
    /// Redirections attached to the builtin
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the builtin
    pub assignments: Vec<Assignment>,
    /// Source span of this command
    pub span: Span,
}

/// `exit [N]`
#[derive(Debug, Clone)]
pub struct ExitCommand {
    /// Optional exit code argument
    pub code: Option<Word>,
    /// Additional operands preserved for fidelity
    pub extra_args: Vec<Word>,
    /// Redirections attached to the builtin
    pub redirects: Vec<Redirect>,
    /// Variable assignments before the builtin
    pub assignments: Vec<Assignment>,
    /// Source span of this command
    pub span: Span,
}

/// A pipeline of commands.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Whether the pipeline is negated (!)
    pub negated: bool,
    /// Commands in the pipeline
    pub commands: Vec<Command>,
    /// Source span of this pipeline
    pub span: Span,
}

/// A list of commands with operators.
#[derive(Debug, Clone)]
pub struct CommandList {
    /// First command
    pub first: Box<Command>,
    /// Remaining commands with their operators
    pub rest: Vec<CommandListItem>,
    /// Source span of this command list
    pub span: Span,
}

/// A command following a list operator such as `&&`, `||`, `;`, or `&`.
#[derive(Debug, Clone)]
pub struct CommandListItem {
    pub operator: ListOperator,
    pub operator_span: Span,
    pub command: Command,
}

/// Operators for command lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListOperator {
    /// && - execute next if previous succeeded
    And,
    /// || - execute next if previous failed
    Or,
    /// ; - execute next unconditionally
    Semicolon,
    /// & - execute in background
    Background,
}

/// Compound commands (control structures).
#[derive(Debug, Clone)]
pub enum CompoundCommand {
    /// If statement
    If(IfCommand),
    /// For loop
    For(ForCommand),
    /// C-style for loop: for ((init; cond; step))
    ArithmeticFor(Box<ArithmeticForCommand>),
    /// While loop
    While(WhileCommand),
    /// Until loop
    Until(UntilCommand),
    /// Case statement
    Case(CaseCommand),
    /// Select loop
    Select(SelectCommand),
    /// Subshell (commands in parentheses)
    Subshell(Vec<Command>),
    /// Brace group
    BraceGroup(Vec<Command>),
    /// Arithmetic command ((expression))
    Arithmetic(ArithmeticCommand),
    /// Time command - measure execution time
    Time(TimeCommand),
    /// Conditional expression [[ ... ]]
    Conditional(ConditionalCommand),
    /// Coprocess: `coproc [NAME] command`
    Coproc(CoprocCommand),
}

/// Coprocess command - runs a command with bidirectional communication.
///
/// In the sandboxed model, the coprocess runs synchronously and its
/// stdout is buffered for later reading via the NAME array FDs.
/// `NAME[0]` = virtual read FD, `NAME[1]` = virtual write FD, `NAME_PID` = virtual PID.
#[derive(Debug, Clone)]
pub struct CoprocCommand {
    /// Coprocess name (defaults to "COPROC")
    pub name: Name,
    /// Source span of the explicit coprocess name, when present.
    pub name_span: Option<Span>,
    /// The command to run as a coprocess
    pub body: Box<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Time command - wraps a command and measures its execution time.
///
/// Note: Shuck only supports wall-clock time measurement.
/// User/system CPU time is not tracked (always reported as 0).
/// This is a known incompatibility with bash.
#[derive(Debug, Clone)]
pub struct TimeCommand {
    /// Use POSIX output format (-p flag)
    pub posix_format: bool,
    /// The command to time (optional - timing with no command is valid)
    pub command: Option<Box<Command>>,
    /// Source span of this command
    pub span: Span,
}

/// Bash conditional command `[[ ... ]]`.
#[derive(Debug, Clone)]
pub struct ConditionalCommand {
    /// The parsed conditional expression.
    pub expression: ConditionalExpr,
    /// Source span of the full `[[ ... ]]` command.
    pub span: Span,
    /// Source span of the opening `[[`.
    pub left_bracket_span: Span,
    /// Source span of the closing `]]`.
    pub right_bracket_span: Span,
}

/// A node within a `[[ ... ]]` conditional expression.
#[derive(Debug, Clone)]
pub enum ConditionalExpr {
    Binary(ConditionalBinaryExpr),
    Unary(ConditionalUnaryExpr),
    Parenthesized(ConditionalParenExpr),
    Word(Word),
    Pattern(Pattern),
    Regex(Word),
    VarRef(Box<VarRef>),
}

impl ConditionalExpr {
    /// Source span of this conditional expression node.
    pub fn span(&self) -> Span {
        match self {
            Self::Binary(expr) => expr.span(),
            Self::Unary(expr) => expr.span(),
            Self::Parenthesized(expr) => expr.span(),
            Self::Word(word) | Self::Regex(word) => word.span,
            Self::Pattern(pattern) => pattern.span,
            Self::VarRef(var_ref) => var_ref.span,
        }
    }
}

/// A binary `[[ ... ]]` expression like `a == b` or `x && y`.
#[derive(Debug, Clone)]
pub struct ConditionalBinaryExpr {
    pub left: Box<ConditionalExpr>,
    pub op: ConditionalBinaryOp,
    pub op_span: Span,
    pub right: Box<ConditionalExpr>,
}

impl ConditionalBinaryExpr {
    pub fn span(&self) -> Span {
        self.left.span().merge(self.right.span())
    }
}

/// A unary `[[ ... ]]` expression like `! x` or `-n "$x"`.
#[derive(Debug, Clone)]
pub struct ConditionalUnaryExpr {
    pub op: ConditionalUnaryOp,
    pub op_span: Span,
    pub expr: Box<ConditionalExpr>,
}

impl ConditionalUnaryExpr {
    pub fn span(&self) -> Span {
        self.op_span.merge(self.expr.span())
    }
}

/// A parenthesized `[[ ... ]]` sub-expression.
#[derive(Debug, Clone)]
pub struct ConditionalParenExpr {
    pub left_paren_span: Span,
    pub expr: Box<ConditionalExpr>,
    pub right_paren_span: Span,
}

impl ConditionalParenExpr {
    pub fn span(&self) -> Span {
        self.left_paren_span.merge(self.right_paren_span)
    }
}

/// Binary operators allowed inside `[[ ... ]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalBinaryOp {
    RegexMatch,
    NewerThan,
    OlderThan,
    SameFile,
    ArithmeticEq,
    ArithmeticNe,
    ArithmeticLe,
    ArithmeticGe,
    ArithmeticLt,
    ArithmeticGt,
    And,
    Or,
    PatternEqShort,
    PatternEq,
    PatternNe,
    LexicalBefore,
    LexicalAfter,
}

impl ConditionalBinaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RegexMatch => "=~",
            Self::NewerThan => "-nt",
            Self::OlderThan => "-ot",
            Self::SameFile => "-ef",
            Self::ArithmeticEq => "-eq",
            Self::ArithmeticNe => "-ne",
            Self::ArithmeticLe => "-le",
            Self::ArithmeticGe => "-ge",
            Self::ArithmeticLt => "-lt",
            Self::ArithmeticGt => "-gt",
            Self::And => "&&",
            Self::Or => "||",
            Self::PatternEqShort => "=",
            Self::PatternEq => "==",
            Self::PatternNe => "!=",
            Self::LexicalBefore => "<",
            Self::LexicalAfter => ">",
        }
    }
}

/// Unary operators allowed inside `[[ ... ]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalUnaryOp {
    Exists,
    RegularFile,
    Directory,
    CharacterSpecial,
    BlockSpecial,
    NamedPipe,
    Socket,
    Symlink,
    Sticky,
    SetGroupId,
    SetUserId,
    GroupOwned,
    UserOwned,
    Modified,
    Readable,
    Writable,
    Executable,
    NonEmptyFile,
    FdTerminal,
    EmptyString,
    NonEmptyString,
    OptionSet,
    VariableSet,
    ReferenceVariable,
    Not,
}

impl ConditionalUnaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exists => "-e",
            Self::RegularFile => "-f",
            Self::Directory => "-d",
            Self::CharacterSpecial => "-c",
            Self::BlockSpecial => "-b",
            Self::NamedPipe => "-p",
            Self::Socket => "-S",
            Self::Symlink => "-L",
            Self::Sticky => "-k",
            Self::SetGroupId => "-g",
            Self::SetUserId => "-u",
            Self::GroupOwned => "-G",
            Self::UserOwned => "-O",
            Self::Modified => "-N",
            Self::Readable => "-r",
            Self::Writable => "-w",
            Self::Executable => "-x",
            Self::NonEmptyFile => "-s",
            Self::FdTerminal => "-t",
            Self::EmptyString => "-z",
            Self::NonEmptyString => "-n",
            Self::OptionSet => "-o",
            Self::VariableSet => "-v",
            Self::ReferenceVariable => "-R",
            Self::Not => "!",
        }
    }
}

/// If statement.
#[derive(Debug, Clone)]
pub struct IfCommand {
    pub condition: Vec<Command>,
    pub then_branch: Vec<Command>,
    pub elif_branches: Vec<(Vec<Command>, Vec<Command>)>,
    pub else_branch: Option<Vec<Command>>,
    /// Source span of this command
    pub span: Span,
}

/// For loop.
#[derive(Debug, Clone)]
pub struct ForCommand {
    pub variable: Name,
    pub variable_span: Span,
    pub words: Option<Vec<Word>>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Select loop.
#[derive(Debug, Clone)]
pub struct SelectCommand {
    pub variable: Name,
    pub variable_span: Span,
    pub words: Vec<Word>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Arithmetic command `(( expr ))`.
#[derive(Debug, Clone)]
pub struct ArithmeticCommand {
    pub span: Span,
    pub left_paren_span: Span,
    pub expr_span: Option<Span>,
    /// Typed arithmetic view of `expr_span`.
    pub expr_ast: Option<ArithmeticExprNode>,
    pub right_paren_span: Span,
}

/// C-style arithmetic for loop: for ((init; cond; step)); do body; done
#[derive(Debug, Clone)]
pub struct ArithmeticForCommand {
    pub left_paren_span: Span,
    pub init_span: Option<Span>,
    /// Typed arithmetic view of `init_span`.
    pub init_ast: Option<ArithmeticExprNode>,
    pub first_semicolon_span: Span,
    pub condition_span: Option<Span>,
    /// Typed arithmetic view of `condition_span`.
    pub condition_ast: Option<ArithmeticExprNode>,
    pub second_semicolon_span: Span,
    pub step_span: Option<Span>,
    /// Typed arithmetic view of `step_span`.
    pub step_ast: Option<ArithmeticExprNode>,
    pub right_paren_span: Span,
    /// Loop body
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// A typed arithmetic expression plus its source span.
#[derive(Debug, Clone)]
pub struct ArithmeticExprNode {
    pub kind: ArithmeticExpr,
    pub span: Span,
}

impl ArithmeticExprNode {
    pub fn new(kind: ArithmeticExpr, span: Span) -> Self {
        Self { kind, span }
    }
}

/// A typed arithmetic expression used by shell arithmetic contexts.
#[derive(Debug, Clone)]
pub enum ArithmeticExpr {
    /// Numeric literal spelling such as `42`, `16#ff`, or `'a'`.
    Number(SourceText),
    /// Bare arithmetic variable reference such as `i`.
    Variable(Name),
    /// Indexed arithmetic reference such as `arr[i + 1]`.
    Indexed {
        name: Name,
        index: Box<ArithmeticExprNode>,
    },
    /// Shell-evaluated primary such as `$x`, `${x}`, `"3"`, or `$(cmd)`.
    ShellWord(Word),
    Parenthesized {
        expression: Box<ArithmeticExprNode>,
    },
    Unary {
        op: ArithmeticUnaryOp,
        expr: Box<ArithmeticExprNode>,
    },
    Postfix {
        expr: Box<ArithmeticExprNode>,
        op: ArithmeticPostfixOp,
    },
    Binary {
        left: Box<ArithmeticExprNode>,
        op: ArithmeticBinaryOp,
        right: Box<ArithmeticExprNode>,
    },
    Conditional {
        condition: Box<ArithmeticExprNode>,
        then_expr: Box<ArithmeticExprNode>,
        else_expr: Box<ArithmeticExprNode>,
    },
    Assignment {
        target: ArithmeticLvalue,
        op: ArithmeticAssignOp,
        value: Box<ArithmeticExprNode>,
    },
}

/// Assignment target inside arithmetic.
#[derive(Debug, Clone)]
pub enum ArithmeticLvalue {
    Variable(Name),
    Indexed {
        name: Name,
        index: Box<ArithmeticExprNode>,
    },
}

/// Prefix unary arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticUnaryOp {
    PreIncrement,
    PreDecrement,
    Plus,
    Minus,
    LogicalNot,
    BitwiseNot,
}

/// Postfix arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticPostfixOp {
    Increment,
    Decrement,
}

/// Binary arithmetic operators ordered by normal shell arithmetic precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticBinaryOp {
    Comma,
    Power,
    Multiply,
    Divide,
    Modulo,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
}

/// Arithmetic assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticAssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    AndAssign,
    XorAssign,
    OrAssign,
}

/// While loop.
#[derive(Debug, Clone)]
pub struct WhileCommand {
    pub condition: Vec<Command>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Until loop.
#[derive(Debug, Clone)]
pub struct UntilCommand {
    pub condition: Vec<Command>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Case statement.
#[derive(Debug, Clone)]
pub struct CaseCommand {
    pub word: Word,
    pub cases: Vec<CaseItem>,
    /// Source span of this command
    pub span: Span,
}

/// Terminator for a case item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaseTerminator {
    /// `;;` — stop matching
    Break,
    /// `;&` — fall through to next case body unconditionally
    FallThrough,
    /// `;;&` — continue checking remaining patterns
    Continue,
}

/// A single case item.
#[derive(Debug, Clone)]
pub struct CaseItem {
    pub patterns: Vec<Pattern>,
    pub commands: Vec<Command>,
    pub terminator: CaseTerminator,
}

/// Surface syntax preserved for a function declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FunctionSurface {
    pub function_keyword_span: Option<Span>,
    pub name_parens_span: Option<Span>,
}

impl FunctionSurface {
    pub fn uses_function_keyword(&self) -> bool {
        self.function_keyword_span.is_some()
    }

    pub fn has_name_parens(&self) -> bool {
        self.name_parens_span.is_some()
    }

    pub fn rebased(&mut self, base: Position) {
        if let Some(span) = &mut self.function_keyword_span {
            *span = span.rebased(base);
        }
        if let Some(span) = &mut self.name_parens_span {
            *span = span.rebased(base);
        }
    }
}

/// Function definition.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: Name,
    pub name_span: Span,
    pub surface: FunctionSurface,
    pub body: Box<Command>,
    /// Source span of this function definition
    pub span: Span,
}

impl FunctionDef {
    pub fn uses_function_keyword(&self) -> bool {
        self.surface.uses_function_keyword()
    }

    pub fn has_name_parens(&self) -> bool {
        self.surface.has_name_parens()
    }
}

/// Original syntax form for command substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionSyntax {
    DollarParen,
    Backtick,
}

/// Original syntax form for arithmetic expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticExpansionSyntax {
    DollarParenParen,
    LegacyBracket,
}

/// Selector form for `${!prefix@}` versus `${!prefix*}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixMatchKind {
    At,
    Star,
}

impl PrefixMatchKind {
    pub const fn as_char(self) -> char {
        match self {
            Self::At => '@',
            Self::Star => '*',
        }
    }
}

/// Brace expansion surface form recognized inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraceExpansionKind {
    CommaList,
    Sequence,
}

/// Quoting context for brace-like syntax inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraceQuoteContext {
    Unquoted,
    DoubleQuoted,
    SingleQuoted,
}

/// Parser-owned classification for brace-like syntax inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BraceSyntaxKind {
    Expansion(BraceExpansionKind),
    Literal,
    TemplatePlaceholder,
}

/// A brace-like surface-syntax occurrence inside a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BraceSyntax {
    pub kind: BraceSyntaxKind,
    pub span: Span,
    pub quote_context: BraceQuoteContext,
}

impl BraceSyntax {
    pub const fn expansion_kind(self) -> Option<BraceExpansionKind> {
        match self.kind {
            BraceSyntaxKind::Expansion(kind) => Some(kind),
            BraceSyntaxKind::Literal | BraceSyntaxKind::TemplatePlaceholder => None,
        }
    }

    pub const fn is_recognized_expansion(self) -> bool {
        matches!(self.kind, BraceSyntaxKind::Expansion(_))
    }

    pub const fn expands(self) -> bool {
        self.is_recognized_expansion() && matches!(self.quote_context, BraceQuoteContext::Unquoted)
    }

    pub const fn treated_literally(self) -> bool {
        !self.expands()
    }
}

/// A word part paired with its source span.
#[derive(Debug, Clone)]
pub struct WordPartNode {
    pub kind: WordPart,
    pub span: Span,
}

impl WordPartNode {
    pub fn new(kind: WordPart, span: Span) -> Self {
        Self { kind, span }
    }
}

/// A word (potentially with expansions).
#[derive(Debug, Clone)]
pub struct Word {
    pub parts: Vec<WordPartNode>,
    /// Source span of this word
    pub span: Span,
    /// Parser-owned brace surface classification for this word.
    pub brace_syntax: Vec<BraceSyntax>,
}

impl Word {
    /// Create a simple literal word.
    pub fn literal(s: impl Into<String>) -> Self {
        Self::literal_with_span(s, Span::new())
    }

    /// Create a simple literal word with an explicit source span.
    pub fn literal_with_span(s: impl Into<String>, span: Span) -> Self {
        Self {
            parts: vec![WordPartNode::new(
                WordPart::Literal(LiteralText::owned(s.into())),
                span,
            )],
            span,
            brace_syntax: Vec::new(),
        }
    }

    /// Create a quoted literal word (no brace/glob expansion).
    pub fn quoted_literal(s: impl Into<String>) -> Self {
        Self::quoted_literal_with_span(s, Span::new())
    }

    /// Create a quoted literal word with an explicit source span.
    pub fn quoted_literal_with_span(s: impl Into<String>, span: Span) -> Self {
        Self {
            parts: vec![WordPartNode::new(
                WordPart::SingleQuoted {
                    value: SourceText::cooked(span, s.into()),
                    dollar: false,
                },
                span,
            )],
            span,
            brace_syntax: Vec::new(),
        }
    }

    /// Create a source-backed literal word.
    pub fn source_literal_with_spans(span: Span, part_span: Span) -> Self {
        Self {
            parts: vec![WordPartNode::new(
                WordPart::Literal(LiteralText::source()),
                part_span,
            )],
            span,
            brace_syntax: Vec::new(),
        }
    }

    /// Create a quoted source-backed literal word.
    pub fn quoted_source_literal_with_spans(span: Span, part_span: Span) -> Self {
        Self {
            parts: vec![WordPartNode::new(
                WordPart::SingleQuoted {
                    value: SourceText::source(part_span),
                    dollar: false,
                },
                span,
            )],
            span,
            brace_syntax: Vec::new(),
        }
    }

    /// Set the source span on an existing word.
    pub fn with_span(mut self, span: Span) -> Self {
        let previous_span = self.span;
        self.span = span;
        if let [part] = self.parts.as_mut_slice()
            && part.span == previous_span
        {
            part.span = span;
        }
        self
    }

    /// Get the source span for a specific word part.
    pub fn part_span(&self, index: usize) -> Option<Span> {
        self.parts.get(index).map(|part| part.span)
    }

    /// Get a specific word part.
    pub fn part(&self, index: usize) -> Option<&WordPart> {
        self.parts.get(index).map(|part| &part.kind)
    }

    /// Iterate over word parts and their spans together.
    pub fn parts_with_spans(&self) -> impl Iterator<Item = (&WordPart, Span)> + '_ {
        self.parts.iter().map(|part| (&part.kind, part.span))
    }

    pub fn brace_syntax(&self) -> &[BraceSyntax] {
        &self.brace_syntax
    }

    pub fn has_active_brace_expansion(&self) -> bool {
        self.brace_syntax.iter().copied().any(BraceSyntax::expands)
    }

    pub fn is_fully_quoted(&self) -> bool {
        matches!(self.parts.as_slice(), [part] if part.kind.is_quoted())
    }

    pub fn is_fully_double_quoted(&self) -> bool {
        matches!(
            self.parts.as_slice(),
            [WordPartNode {
                kind: WordPart::DoubleQuoted { .. },
                ..
            }]
        )
    }

    pub fn has_quoted_parts(&self) -> bool {
        self.parts.iter().any(|part| part.kind.is_quoted())
    }

    /// Render this word using exact source slices when available and owned cooked
    /// text only where the parser normalized the input.
    pub fn render(&self, source: &str) -> String {
        self.render_with_mode(Some(source), RenderMode::Decoded)
    }

    /// Render this word as shell syntax, preserving quote delimiters and other
    /// syntactic wrappers when they are represented in the AST.
    pub fn render_syntax(&self, source: &str) -> String {
        self.render_with_mode(Some(source), RenderMode::Syntax)
    }

    fn render_with_mode(&self, source: Option<&str>, mode: RenderMode) -> String {
        let mut rendered = String::new();
        self.fmt_with_source_mode(&mut rendered, source, mode)
            .expect("writing into a String should not fail");
        rendered
    }

    fn fmt_with_source_mode(
        &self,
        f: &mut impl fmt::Write,
        source: Option<&str>,
        mode: RenderMode,
    ) -> fmt::Result {
        if matches!(mode, RenderMode::Syntax)
            && let Some(source) = source
            && word_prefers_whole_source_slice_in_syntax(self)
            && let Some(slice) = syntax_source_slice(self.span, source)
        {
            f.write_str(trim_unescaped_trailing_whitespace(slice))?;
            return Ok(());
        }

        for (part, span) in self.parts_with_spans() {
            fmt_word_part_with_source_mode(f, part, span, source, mode)?;
        }

        Ok(())
    }
}

impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_source_mode(f, None, RenderMode::Decoded)
    }
}

/// A shell pattern in a pattern-sensitive context such as `case`, `[[ ... == ... ]]`,
/// or parameter pattern operators.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub parts: Vec<PatternPartNode>,
    pub span: Span,
}

impl Pattern {
    /// Get the source span for a specific pattern part.
    pub fn part_span(&self, index: usize) -> Option<Span> {
        self.parts.get(index).map(|part| part.span)
    }

    pub fn is_source_backed(&self) -> bool {
        self.parts
            .iter()
            .all(|part| pattern_part_is_source_backed(&part.kind))
    }

    /// Iterate over pattern parts and their spans together.
    pub fn parts_with_spans(&self) -> impl Iterator<Item = (&PatternPart, Span)> + '_ {
        self.parts.iter().map(|part| (&part.kind, part.span))
    }

    /// Render this pattern using exact source slices when available and owned cooked
    /// text only where the parser normalized the input.
    pub fn render(&self, source: &str) -> String {
        self.render_with_mode(Some(source), RenderMode::Decoded)
    }

    /// Render this pattern as shell syntax, preserving quoted fragments when
    /// they are represented in the AST.
    pub fn render_syntax(&self, source: &str) -> String {
        self.render_with_mode(Some(source), RenderMode::Syntax)
    }

    fn render_with_mode(&self, source: Option<&str>, mode: RenderMode) -> String {
        let mut rendered = String::new();
        self.fmt_with_source_mode(&mut rendered, source, mode)
            .expect("writing into a String should not fail");
        rendered
    }

    fn fmt_with_source_mode(
        &self,
        f: &mut impl fmt::Write,
        source: Option<&str>,
        mode: RenderMode,
    ) -> fmt::Result {
        if matches!(mode, RenderMode::Syntax)
            && let Some(source) = source
            && pattern_prefers_whole_source_slice_in_syntax(self)
            && let Some(slice) = syntax_source_slice(self.span, source)
        {
            f.write_str(trim_unescaped_trailing_whitespace(slice))?;
            return Ok(());
        }

        for (part, span) in self.parts_with_spans() {
            fmt_pattern_part_with_source_mode(f, part, span, source, mode)?;
        }

        Ok(())
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_source_mode(f, None, RenderMode::Decoded)
    }
}

/// The extglob operator for a pattern group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternGroupKind {
    ZeroOrOne,
    ZeroOrMore,
    OneOrMore,
    ExactlyOne,
    NoneOf,
}

impl PatternGroupKind {
    pub fn prefix(self) -> char {
        match self {
            Self::ZeroOrOne => '?',
            Self::ZeroOrMore => '*',
            Self::OneOrMore => '+',
            Self::ExactlyOne => '@',
            Self::NoneOf => '!',
        }
    }
}

/// A pattern part paired with its source span.
#[derive(Debug, Clone)]
pub struct PatternPartNode {
    pub kind: PatternPart,
    pub span: Span,
}

impl PatternPartNode {
    pub fn new(kind: PatternPart, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Parts of a pattern.
#[derive(Debug, Clone)]
pub enum PatternPart {
    Literal(LiteralText),
    AnyString,
    AnyChar,
    CharClass(SourceText),
    Group {
        kind: PatternGroupKind,
        patterns: Vec<Pattern>,
    },
    Word(Word),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Decoded,
    Syntax,
}

fn syntax_source_slice<'a>(span: Span, source: &'a str) -> Option<&'a str> {
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn word_prefers_whole_source_slice_in_syntax(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if part.span == word.span && top_level_word_part_prefers_source_slice_in_syntax(&part.kind)
    )
}

fn top_level_word_part_prefers_source_slice_in_syntax(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(text) => text.is_source_backed(),
        WordPart::SingleQuoted { value, .. } => value.is_source_backed(),
        WordPart::DoubleQuoted { parts, .. } => parts.iter().all(|part| match &part.kind {
            WordPart::Literal(_) => true,
            other => part_prefers_source_slice_in_syntax(other) && part_is_source_backed(other),
        }),
        _ => part_prefers_source_slice_in_syntax(part) && part_is_source_backed(part),
    }
}

fn pattern_prefers_whole_source_slice_in_syntax(pattern: &Pattern) -> bool {
    !pattern.parts.is_empty()
        && pattern
            .parts
            .iter()
            .all(|part| top_level_pattern_part_prefers_source_slice_in_syntax(&part.kind))
}

fn top_level_pattern_part_prefers_source_slice_in_syntax(part: &PatternPart) -> bool {
    match part {
        PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => true,
        PatternPart::CharClass(text) => text.is_source_backed(),
        PatternPart::Group { patterns, .. } => patterns
            .iter()
            .all(pattern_prefers_whole_source_slice_in_syntax),
        PatternPart::Word(word) => word_prefers_whole_source_slice_in_syntax(word),
    }
}

fn display_source_text<'a>(text: Option<&'a SourceText>, source: Option<&'a str>) -> &'a str {
    match (text, source) {
        (Some(text), Some(source)) => text.slice(source),
        (
            Some(SourceText {
                cooked: Some(text), ..
            }),
            None,
        ) => text.as_ref(),
        (Some(_), None) => "...",
        (None, _) => "",
    }
}

fn display_subscript_text<'a>(subscript: &'a Subscript, source: Option<&'a str>) -> Cow<'a, str> {
    match (source, subscript.selector()) {
        (Some(source), _) => Cow::Borrowed(subscript.syntax_text(source)),
        (None, Some(selector)) => Cow::Owned(selector.as_char().to_string()),
        (None, None) => Cow::Borrowed(display_source_text(
            Some(subscript.syntax_source_text()),
            source,
        )),
    }
}

fn fmt_var_ref_with_source(
    f: &mut impl fmt::Write,
    reference: &VarRef,
    source: Option<&str>,
) -> fmt::Result {
    write!(f, "{}", reference.name)?;
    if let Some(subscript) = &reference.subscript {
        write!(f, "[{}]", display_subscript_text(subscript, source))?;
    }
    Ok(())
}

/// Parts of a word.
#[derive(Debug, Clone)]
pub enum WordPart {
    /// Literal text
    Literal(LiteralText),
    /// Single-quoted literal content, including `$'...'` ANSI-C quoting.
    SingleQuoted { value: SourceText, dollar: bool },
    /// Double-quoted content with nested expansions.
    DoubleQuoted {
        parts: Vec<WordPartNode>,
        dollar: bool,
    },
    /// Variable expansion ($VAR or ${VAR})
    Variable(Name),
    /// Command substitution ($(...)) or legacy backticks.
    CommandSubstitution {
        commands: Vec<Command>,
        syntax: CommandSubstitutionSyntax,
    },
    /// Arithmetic expansion ($((...)) or legacy $[...]).
    ArithmeticExpansion {
        expression: SourceText,
        /// Typed arithmetic view of `expression`.
        expression_ast: Option<ArithmeticExprNode>,
        syntax: ArithmeticExpansionSyntax,
    },
    /// Parameter expansion with operator ${var:-default}, ${var:=default}, etc.
    /// `colon_variant` distinguishes `:-` (unset-or-empty) from `-` (unset-only).
    ParameterExpansion {
        reference: VarRef,
        operator: ParameterOp,
        operand: Option<SourceText>,
        colon_variant: bool,
    },
    /// Length expansion ${#var}
    Length(VarRef),
    /// Array element access `${arr[index]}` or `${arr[@]}` or `${arr[*]}`
    ArrayAccess(VarRef),
    /// Array length `${#arr[@]}` or `${#arr[*]}`
    ArrayLength(VarRef),
    /// Array indices `${!arr[@]}` or `${!arr[*]}`
    ArrayIndices(VarRef),
    /// Substring extraction `${var:offset}` or `${var:offset:length}`
    Substring {
        reference: VarRef,
        offset: SourceText,
        /// Typed arithmetic view of `offset` when it parses as arithmetic.
        offset_ast: Option<ArithmeticExprNode>,
        length: Option<SourceText>,
        /// Typed arithmetic view of `length` when it parses as arithmetic.
        length_ast: Option<ArithmeticExprNode>,
    },
    /// Array slice `${arr[@]:offset:length}`
    ArraySlice {
        reference: VarRef,
        offset: SourceText,
        /// Typed arithmetic view of `offset` when it parses as arithmetic.
        offset_ast: Option<ArithmeticExprNode>,
        length: Option<SourceText>,
        /// Typed arithmetic view of `length` when it parses as arithmetic.
        length_ast: Option<ArithmeticExprNode>,
    },
    /// Indirect expansion `${!var}` - expands to value of variable named by var's value
    /// Optionally composed with an operator: `${!var:-default}`, `${!var:=val}`, etc.
    IndirectExpansion {
        name: Name,
        operator: Option<ParameterOp>,
        operand: Option<SourceText>,
        colon_variant: bool,
    },
    /// Prefix matching `${!prefix*}` or `${!prefix@}` - names of variables with given prefix
    PrefixMatch { prefix: Name, kind: PrefixMatchKind },
    /// Process substitution <(cmd) or >(cmd)
    ProcessSubstitution {
        /// The commands to run
        commands: Vec<Command>,
        /// True for <(cmd), false for >(cmd)
        is_input: bool,
    },
    /// Parameter transformation `${var@op}` where op is Q, E, P, A, K, a, u, U, L
    Transformation { reference: VarRef, operator: char },
}

impl WordPart {
    pub fn is_quoted(&self) -> bool {
        matches!(self, Self::SingleQuoted { .. } | Self::DoubleQuoted { .. })
    }
}

/// Compound array literal assigned with `(...)`.
#[derive(Debug, Clone)]
pub struct ArrayExpr {
    pub kind: ArrayKind,
    pub elements: Vec<ArrayElem>,
    pub span: Span,
}

/// The array flavor implied by the current parse context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayKind {
    Indexed,
    Associative,
    Contextual,
}

/// An element inside a compound array literal.
#[derive(Debug, Clone)]
pub enum ArrayElem {
    Sequential(Word),
    Keyed { key: Subscript, value: Word },
    KeyedAppend { key: Subscript, value: Word },
}

impl ArrayElem {
    pub fn span(&self) -> Span {
        match self {
            Self::Sequential(word) => word.span,
            Self::Keyed { key, value } | Self::KeyedAppend { key, value } => {
                key.span().merge(value.span)
            }
        }
    }
}

fn fmt_literal_text(
    f: &mut impl fmt::Write,
    text: &LiteralText,
    span: Span,
    source: Option<&str>,
) -> fmt::Result {
    match source {
        Some(source) => f.write_str(text.as_str(source, span)),
        None => match text {
            LiteralText::Source => f.write_str("<source>"),
            LiteralText::Owned(text) => f.write_str(text),
        },
    }
}

fn fmt_pattern_part_with_source_mode(
    f: &mut impl fmt::Write,
    part: &PatternPart,
    span: Span,
    source: Option<&str>,
    mode: RenderMode,
) -> fmt::Result {
    match part {
        PatternPart::Literal(text) => fmt_literal_text(f, text, span, source)?,
        PatternPart::AnyString => f.write_str("*")?,
        PatternPart::AnyChar => f.write_str("?")?,
        PatternPart::CharClass(text) => match source {
            Some(source) if span.end.offset <= source.len() => f.write_str(span.slice(source))?,
            _ => f.write_str(display_source_text(Some(text), source))?,
        },
        PatternPart::Group { kind, patterns } => {
            write!(f, "{}(", kind.prefix())?;
            let mut patterns = patterns.iter();
            if let Some(pattern) = patterns.next() {
                pattern.fmt_with_source_mode(f, source, mode)?;
                for pattern in patterns {
                    f.write_str("|")?;
                    pattern.fmt_with_source_mode(f, source, mode)?;
                }
            }
            f.write_str(")")?;
        }
        PatternPart::Word(word) => word.fmt_with_source_mode(f, source, mode)?,
    }

    Ok(())
}

fn fmt_word_part_with_source_mode(
    f: &mut impl fmt::Write,
    part: &WordPart,
    span: Span,
    source: Option<&str>,
    mode: RenderMode,
) -> fmt::Result {
    if matches!(mode, RenderMode::Syntax)
        && let Some(source) = source
        && part_prefers_source_slice_in_syntax(part)
        && part_is_source_backed(part)
        && span.end.offset <= source.len()
    {
        f.write_str(span.slice(source))?;
        return Ok(());
    }

    match part {
        WordPart::Literal(text) => match (mode, source) {
            (RenderMode::Syntax, Some(source))
                if text.is_source_backed() && span.end.offset <= source.len() =>
            {
                f.write_str(trim_unescaped_trailing_whitespace(span.slice(source)))?;
            }
            _ => fmt_literal_text(f, text, span, source)?,
        },
        WordPart::SingleQuoted { value, dollar } => match mode {
            RenderMode::Decoded => f.write_str(display_source_text(Some(value), source))?,
            RenderMode::Syntax => match source {
                Some(source)
                    if value.is_source_backed()
                        && part_is_source_backed(part)
                        && span.end.offset <= source.len() =>
                {
                    f.write_str(span.slice(source))?;
                }
                _ => {
                    if *dollar {
                        f.write_str("$")?;
                    }
                    f.write_str("'")?;
                    f.write_str(display_source_text(Some(value), source))?;
                    f.write_str("'")?;
                }
            },
        },
        WordPart::DoubleQuoted { parts, dollar } => match mode {
            RenderMode::Decoded => {
                for part in parts {
                    fmt_word_part_with_source_mode(f, &part.kind, part.span, source, mode)?;
                }
            }
            RenderMode::Syntax => match source {
                Some(source) if part_is_source_backed(part) && span.end.offset <= source.len() => {
                    f.write_str(span.slice(source))?;
                }
                _ => {
                    if *dollar {
                        f.write_str("$")?;
                    }
                    f.write_str("\"")?;
                    for part in parts {
                        fmt_word_part_with_source_mode(f, &part.kind, part.span, source, mode)?;
                    }
                    f.write_str("\"")?;
                }
            },
        },
        WordPart::Variable(name) => write!(f, "${}", name)?,
        WordPart::CommandSubstitution { commands, syntax } => match source {
            Some(source) if span.end.offset <= source.len() => f.write_str(span.slice(source))?,
            _ => match syntax {
                CommandSubstitutionSyntax::DollarParen => write!(f, "$({:?})", commands)?,
                CommandSubstitutionSyntax::Backtick => write!(f, "`{:?}`", commands)?,
            },
        },
        WordPart::ArithmeticExpansion {
            expression, syntax, ..
        } => match source {
            Some(source) if expression.is_source_backed() && span.end.offset <= source.len() => {
                f.write_str(span.slice(source))?
            }
            _ => match syntax {
                ArithmeticExpansionSyntax::DollarParenParen => {
                    write!(f, "$(({}))", display_source_text(Some(expression), source))?
                }
                ArithmeticExpansionSyntax::LegacyBracket => {
                    write!(f, "$[{}]", display_source_text(Some(expression), source))?
                }
            },
        },
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            colon_variant,
        } => match operator {
            ParameterOp::UseDefault => {
                let c = if *colon_variant { ":" } else { "" };
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    "{}-{}}}",
                    c,
                    display_source_text(operand.as_ref(), source)
                )?
            }
            ParameterOp::AssignDefault => {
                let c = if *colon_variant { ":" } else { "" };
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    "{}={}}}",
                    c,
                    display_source_text(operand.as_ref(), source)
                )?
            }
            ParameterOp::UseReplacement => {
                let c = if *colon_variant { ":" } else { "" };
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    "{}+{}}}",
                    c,
                    display_source_text(operand.as_ref(), source)
                )?
            }
            ParameterOp::Error => {
                let c = if *colon_variant { ":" } else { "" };
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    "{}?{}}}",
                    c,
                    display_source_text(operand.as_ref(), source)
                )?
            }
            ParameterOp::RemovePrefixShort { pattern } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("#")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                f.write_str("}")?;
            }
            ParameterOp::RemovePrefixLong { pattern } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("##")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                f.write_str("}")?;
            }
            ParameterOp::RemoveSuffixShort { pattern } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("%")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                f.write_str("}")?;
            }
            ParameterOp::RemoveSuffixLong { pattern } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("%%")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                f.write_str("}")?;
            }
            ParameterOp::ReplaceFirst {
                pattern,
                replacement,
            } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("/")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                write!(f, "/{}}}", display_source_text(Some(replacement), source))?;
            }
            ParameterOp::ReplaceAll {
                pattern,
                replacement,
            } => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("//")?;
                pattern.fmt_with_source_mode(f, source, mode)?;
                write!(f, "/{}}}", display_source_text(Some(replacement), source))?;
            }
            ParameterOp::UpperFirst => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("^}")?;
            }
            ParameterOp::UpperAll => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str("^^}")?;
            }
            ParameterOp::LowerFirst => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str(",}")?;
            }
            ParameterOp::LowerAll => {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                f.write_str(",,}")?;
            }
        },
        WordPart::Length(reference) => {
            write!(f, "${{#")?;
            fmt_var_ref_with_source(f, reference, source)?;
            f.write_str("}")?;
        }
        WordPart::ArrayAccess(reference) => {
            write!(f, "${{")?;
            fmt_var_ref_with_source(f, reference, source)?;
            f.write_str("}")?;
        }
        WordPart::ArrayLength(reference) => {
            write!(f, "${{#")?;
            fmt_var_ref_with_source(f, reference, source)?;
            f.write_str("}")?;
        }
        WordPart::ArrayIndices(reference) => {
            write!(f, "${{!")?;
            fmt_var_ref_with_source(f, reference, source)?;
            f.write_str("}")?;
        }
        WordPart::Substring {
            reference,
            offset,
            length,
            ..
        } => {
            if let Some(length) = length {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    ":{}:{}}}",
                    display_source_text(Some(offset), source),
                    display_source_text(Some(length), source)
                )?
            } else {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(f, ":{}}}", display_source_text(Some(offset), source))?
            }
        }
        WordPart::ArraySlice {
            reference,
            offset,
            length,
            ..
        } => {
            if let Some(length) = length {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(
                    f,
                    ":{}:{}}}",
                    display_source_text(Some(offset), source),
                    display_source_text(Some(length), source)
                )?
            } else {
                write!(f, "${{")?;
                fmt_var_ref_with_source(f, reference, source)?;
                write!(f, ":{}}}", display_source_text(Some(offset), source))?
            }
        }
        WordPart::IndirectExpansion {
            name,
            operator,
            operand,
            colon_variant,
        } => {
            if let Some(op) = operator {
                let c = if *colon_variant { ":" } else { "" };
                let op_char = match op {
                    ParameterOp::UseDefault => "-",
                    ParameterOp::AssignDefault => "=",
                    ParameterOp::UseReplacement => "+",
                    ParameterOp::Error => "?",
                    _ => "",
                };
                write!(
                    f,
                    "${{!{}{}{}{}}}",
                    name,
                    c,
                    op_char,
                    display_source_text(operand.as_ref(), source)
                )?
            } else {
                write!(f, "${{!{}}}", name)?
            }
        }
        WordPart::PrefixMatch { prefix, kind } => write!(f, "${{!{}{}}}", prefix, kind.as_char())?,
        WordPart::ProcessSubstitution { commands, is_input } => match source {
            Some(source) if span.end.offset <= source.len() => f.write_str(span.slice(source))?,
            _ => {
                let prefix = if *is_input { "<" } else { ">" };
                write!(f, "{}({:?})", prefix, commands)?
            }
        },
        WordPart::Transformation {
            reference,
            operator,
        } => {
            write!(f, "${{")?;
            fmt_var_ref_with_source(f, reference, source)?;
            write!(f, "@{}}}", operator)?;
        }
    }

    Ok(())
}

fn part_prefers_source_slice_in_syntax(part: &WordPart) -> bool {
    matches!(
        part,
        WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
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
            | WordPart::Transformation { .. }
    )
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text[..whitespace_start]
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}

fn part_is_source_backed(part: &WordPart) -> bool {
    match part {
        WordPart::Literal(text) => text.is_source_backed(),
        WordPart::SingleQuoted { value, .. } => value.is_source_backed(),
        WordPart::DoubleQuoted { parts, .. } => {
            parts.iter().all(|part| part_is_source_backed(&part.kind))
        }
        WordPart::ArithmeticExpansion { expression, .. } => expression.is_source_backed(),
        WordPart::ParameterExpansion {
            reference,
            operand,
            operator,
            ..
        } => {
            reference.is_source_backed()
                && operator_is_source_backed(operator)
                && operand.as_ref().is_none_or(SourceText::is_source_backed)
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => reference.is_source_backed(),
        WordPart::Substring {
            reference,
            offset: index,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset: index,
            ..
        } => reference.is_source_backed() && index.is_source_backed(),
        WordPart::IndirectExpansion {
            operand, operator, ..
        } => operator.is_none() && operand.as_ref().is_none_or(SourceText::is_source_backed),
        WordPart::CommandSubstitution { .. }
        | WordPart::Variable(_)
        | WordPart::PrefixMatch { .. }
        | WordPart::ProcessSubstitution { .. } => true,
    }
}

fn pattern_part_is_source_backed(part: &PatternPart) -> bool {
    match part {
        PatternPart::Literal(text) => text.is_source_backed(),
        PatternPart::AnyString | PatternPart::AnyChar => true,
        PatternPart::CharClass(text) => text.is_source_backed(),
        PatternPart::Group { patterns, .. } => patterns.iter().all(Pattern::is_source_backed),
        PatternPart::Word(word) => word
            .parts
            .iter()
            .all(|part| part_is_source_backed(&part.kind)),
    }
}

fn operator_is_source_backed(operator: &ParameterOp) -> bool {
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

/// Parameter expansion operators
#[derive(Debug, Clone)]
pub enum ParameterOp {
    /// :- use default if unset/empty
    UseDefault,
    /// := assign default if unset/empty
    AssignDefault,
    /// :+ use replacement if set
    UseReplacement,
    /// :? error if unset/empty
    Error,
    /// # remove prefix (shortest)
    RemovePrefixShort { pattern: Pattern },
    /// ## remove prefix (longest)
    RemovePrefixLong { pattern: Pattern },
    /// % remove suffix (shortest)
    RemoveSuffixShort { pattern: Pattern },
    /// %% remove suffix (longest)
    RemoveSuffixLong { pattern: Pattern },
    /// / pattern replacement (first occurrence)
    ReplaceFirst {
        pattern: Pattern,
        replacement: SourceText,
    },
    /// // pattern replacement (all occurrences)
    ReplaceAll {
        pattern: Pattern,
        replacement: SourceText,
    },
    /// ^ uppercase first char
    UpperFirst,
    /// ^^ uppercase all chars
    UpperAll,
    /// , lowercase first char
    LowerFirst,
    /// ,, lowercase all chars
    LowerAll,
}

/// I/O redirection.
#[derive(Debug, Clone)]
pub struct Redirect {
    /// File descriptor (default: 1 for output, 0 for input)
    pub fd: Option<i32>,
    /// Variable name for `{var}` fd-variable redirects (e.g. `exec {myfd}>&-`)
    pub fd_var: Option<Name>,
    /// Source span of `{name}` in fd-variable redirects.
    pub fd_var_span: Option<Span>,
    /// Type of redirection
    pub kind: RedirectKind,
    /// Source span of this redirection
    pub span: Span,
    /// Redirect payload.
    pub target: RedirectTarget,
}

impl Redirect {
    /// Returns the word target for non-heredoc redirects.
    pub fn word_target(&self) -> Option<&Word> {
        match &self.target {
            RedirectTarget::Word(word) => Some(word),
            RedirectTarget::Heredoc(_) => None,
        }
    }

    /// Returns the mutable word target for non-heredoc redirects.
    pub fn word_target_mut(&mut self) -> Option<&mut Word> {
        match &mut self.target {
            RedirectTarget::Word(word) => Some(word),
            RedirectTarget::Heredoc(_) => None,
        }
    }

    /// Returns heredoc metadata and body when this redirect is a heredoc.
    pub fn heredoc(&self) -> Option<&Heredoc> {
        match &self.target {
            RedirectTarget::Word(_) => None,
            RedirectTarget::Heredoc(heredoc) => Some(heredoc),
        }
    }

    /// Returns mutable heredoc metadata and body when this redirect is a heredoc.
    pub fn heredoc_mut(&mut self) -> Option<&mut Heredoc> {
        match &mut self.target {
            RedirectTarget::Word(_) => None,
            RedirectTarget::Heredoc(heredoc) => Some(heredoc),
        }
    }
}

/// Redirect payload.
#[derive(Debug, Clone)]
pub enum RedirectTarget {
    /// Standard redirect operand like a path or file descriptor.
    Word(Word),
    /// Heredoc delimiter metadata plus decoded body.
    Heredoc(Heredoc),
}

/// Heredoc delimiter metadata and decoded body.
#[derive(Debug, Clone)]
pub struct Heredoc {
    pub delimiter: HeredocDelimiter,
    pub body: Word,
}

/// Parsed heredoc delimiter metadata.
#[derive(Debug, Clone)]
pub struct HeredocDelimiter {
    /// Raw delimiter word with original quoting preserved.
    pub raw: Word,
    /// Cooked delimiter string after quote removal.
    pub cooked: String,
    /// Source span of the delimiter token.
    pub span: Span,
    /// Whether the delimiter used shell quoting.
    pub quoted: bool,
    /// Whether the body should be decoded for expansions.
    pub expands_body: bool,
    /// Whether `<<-` tab stripping applies.
    pub strip_tabs: bool,
}

/// Types of redirections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RedirectKind {
    /// > - redirect output
    Output,
    /// >| - force redirect output (clobber, bypasses noclobber)
    Clobber,
    /// >> - append output
    Append,
    /// < - redirect input
    Input,
    /// <> - redirect input and output
    ReadWrite,
    /// << - here document
    HereDoc,
    /// <<- - here document with leading tab stripping
    HereDocStrip,
    /// <<< - here string
    HereString,
    /// >& - duplicate output fd
    DupOutput,
    /// <& - duplicate input fd
    DupInput,
    /// &> - redirect both stdout and stderr
    OutputBoth,
}

/// Variable assignment.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub target: VarRef,
    pub value: AssignmentValue,
    /// Whether this is an append assignment (+=)
    pub append: bool,
    /// Source span of this assignment
    pub span: Span,
}

/// Value in an assignment - scalar or array
#[derive(Debug, Clone)]
pub enum AssignmentValue {
    /// Scalar value: VAR=value
    Scalar(Word),
    /// Array value: VAR=(a b c)
    Compound(ArrayExpr),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(parts: Vec<WordPart>) -> Word {
        let span = Span::new();
        Word {
            parts: parts
                .into_iter()
                .map(|part| WordPartNode::new(part, span))
                .collect(),
            span,
            brace_syntax: Vec::new(),
        }
    }

    fn pattern(parts: Vec<PatternPart>) -> Pattern {
        let span = Span::new();
        Pattern {
            parts: parts
                .into_iter()
                .map(|part| PatternPartNode::new(part, span))
                .collect(),
            span,
        }
    }

    fn plain_ref(name: &str) -> VarRef {
        let span = Span::new();
        VarRef {
            name: name.into(),
            name_span: span,
            subscript: None,
            span,
        }
    }

    fn indexed_ref(name: &str, index: &str) -> VarRef {
        let span = Span::new();
        VarRef {
            name: name.into(),
            name_span: span,
            subscript: Some(Subscript {
                text: index.into(),
                raw: None,
                kind: SubscriptKind::Ordinary,
                interpretation: SubscriptInterpretation::Contextual,
                arithmetic_ast: None,
            }),
            span,
        }
    }

    fn selector_ref(name: &str, selector: SubscriptSelector) -> VarRef {
        let span = Span::new();
        VarRef {
            name: name.into(),
            name_span: span,
            subscript: Some(Subscript {
                text: selector.as_char().to_string().into(),
                raw: None,
                kind: SubscriptKind::Selector(selector),
                interpretation: SubscriptInterpretation::Contextual,
                arithmetic_ast: None,
            }),
            span,
        }
    }

    fn assignment(target: VarRef, value: AssignmentValue) -> Assignment {
        Assignment {
            target,
            value,
            append: false,
            span: Span::new(),
        }
    }

    fn span_for_source(source: &str) -> Span {
        Span::from_positions(
            Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            Position {
                line: 1,
                column: source.chars().count() + 1,
                offset: source.len(),
            },
        )
    }

    // --- Word ---

    #[test]
    fn word_literal_creates_unquoted_word() {
        let w = Word::literal("hello");
        assert_eq!(w.parts.len(), 1);
        assert!(matches!(w.part(0), Some(WordPart::Literal(s)) if s == "hello"));
    }

    #[test]
    fn word_literal_empty_string() {
        let w = Word::literal("");
        assert!(matches!(w.part(0), Some(WordPart::Literal(s)) if s.is_empty()));
    }

    #[test]
    fn word_quoted_literal_creates_single_quoted_part() {
        let w = Word::quoted_literal("world");
        assert_eq!(w.parts.len(), 1);
        assert!(matches!(
            w.part(0),
            Some(WordPart::SingleQuoted { dollar: false, .. })
        ));
        assert_eq!(format!("{w}"), "world");
    }

    #[test]
    fn word_display_literal() {
        let w = Word::literal("echo");
        assert_eq!(format!("{w}"), "echo");
    }

    #[test]
    fn word_render_syntax_preserves_cooked_double_quoted_literal() {
        let w = word(vec![WordPart::DoubleQuoted {
            parts: vec![WordPartNode::new(
                WordPart::Literal(LiteralText::owned("hello".to_string())),
                Span::new(),
            )],
            dollar: false,
        }]);
        assert_eq!(w.render_syntax(""), "\"hello\"");
    }

    #[test]
    fn word_render_syntax_preserves_source_backed_braced_variable() {
        let span = Span::from_positions(
            Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            Position {
                line: 1,
                column: 5,
                offset: 4,
            },
        );
        let w = Word {
            parts: vec![WordPartNode::new(WordPart::Variable("1".into()), span)],
            span,
            brace_syntax: Vec::new(),
        };

        assert_eq!(w.render_syntax("${1}"), "${1}");
    }

    #[test]
    fn word_render_syntax_trims_source_backed_literal_delimiters() {
        let span = Span::from_positions(
            Position {
                line: 1,
                column: 1,
                offset: 0,
            },
            Position {
                line: 1,
                column: 5,
                offset: 4,
            },
        );
        let w = Word {
            parts: vec![WordPartNode::new(
                WordPart::Literal(LiteralText::source()),
                span,
            )],
            span,
            brace_syntax: Vec::new(),
        };

        assert_eq!(w.render_syntax("foo "), "foo");
    }

    #[test]
    fn word_render_syntax_prefers_whole_word_source_slice() {
        let source = "\"source \\\"$fzf_base/shell/completion.${shell}\\\"\"";
        let span = span_for_source(source);
        let w = Word {
            parts: vec![WordPartNode::new(
                WordPart::DoubleQuoted {
                    parts: vec![WordPartNode::new(
                        WordPart::Literal(LiteralText::owned(
                            "source \"$fzf_base/shell/completion.${shell}\"".to_string(),
                        )),
                        span,
                    )],
                    dollar: false,
                },
                span,
            )],
            span,
            brace_syntax: Vec::new(),
        };

        assert_eq!(w.render_syntax(source), source);
    }

    #[test]
    fn word_display_variable() {
        let w = word(vec![WordPart::Variable("HOME".into())]);
        assert_eq!(format!("{w}"), "$HOME");
    }

    #[test]
    fn word_display_arithmetic_expansion() {
        let w = word(vec![WordPart::ArithmeticExpansion {
            expression: "1+2".into(),
            expression_ast: None,
            syntax: ArithmeticExpansionSyntax::DollarParenParen,
        }]);
        assert_eq!(format!("{w}"), "$((1+2))");
    }

    #[test]
    fn word_display_length() {
        let w = word(vec![WordPart::Length(plain_ref("var"))]);
        assert_eq!(format!("{w}"), "${#var}");
    }

    #[test]
    fn word_display_array_access() {
        let w = word(vec![WordPart::ArrayAccess(indexed_ref("arr", "0"))]);
        assert_eq!(format!("{w}"), "${arr[0]}");
    }

    #[test]
    fn word_display_array_length() {
        let w = word(vec![WordPart::ArrayLength(selector_ref(
            "arr",
            SubscriptSelector::At,
        ))]);
        assert_eq!(format!("{w}"), "${#arr[@]}");
    }

    #[test]
    fn word_display_array_indices() {
        let w = word(vec![WordPart::ArrayIndices(selector_ref(
            "arr",
            SubscriptSelector::At,
        ))]);
        assert_eq!(format!("{w}"), "${!arr[@]}");
    }

    #[test]
    fn word_display_substring_with_length() {
        let w = word(vec![WordPart::Substring {
            reference: plain_ref("var"),
            offset: "2".into(),
            offset_ast: None,
            length: Some("3".into()),
            length_ast: None,
        }]);
        assert_eq!(format!("{w}"), "${var:2:3}");
    }

    #[test]
    fn word_display_substring_without_length() {
        let w = word(vec![WordPart::Substring {
            reference: plain_ref("var"),
            offset: "2".into(),
            offset_ast: None,
            length: None,
            length_ast: None,
        }]);
        assert_eq!(format!("{w}"), "${var:2}");
    }

    #[test]
    fn word_display_array_slice_with_length() {
        let w = word(vec![WordPart::ArraySlice {
            reference: selector_ref("arr", SubscriptSelector::At),
            offset: "1".into(),
            offset_ast: None,
            length: Some("2".into()),
            length_ast: None,
        }]);
        assert_eq!(format!("{w}"), "${arr[@]:1:2}");
    }

    #[test]
    fn word_display_array_slice_without_length() {
        let w = word(vec![WordPart::ArraySlice {
            reference: selector_ref("arr", SubscriptSelector::At),
            offset: "1".into(),
            offset_ast: None,
            length: None,
            length_ast: None,
        }]);
        assert_eq!(format!("{w}"), "${arr[@]:1}");
    }

    #[test]
    fn word_display_indirect_expansion() {
        let w = word(vec![WordPart::IndirectExpansion {
            name: "ref".into(),
            operator: None,
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${!ref}");
    }

    #[test]
    fn word_display_prefix_match() {
        let w = word(vec![WordPart::PrefixMatch {
            prefix: "MY_".into(),
            kind: PrefixMatchKind::Star,
        }]);
        assert_eq!(format!("{w}"), "${!MY_*}");
    }

    #[test]
    fn word_display_prefix_match_at() {
        let w = word(vec![WordPart::PrefixMatch {
            prefix: "MY_".into(),
            kind: PrefixMatchKind::At,
        }]);
        assert_eq!(format!("{w}"), "${!MY_@}");
    }

    #[test]
    fn word_render_syntax_preserves_raw_quoted_subscript() {
        let w = word(vec![WordPart::ArrayAccess(VarRef {
            name: "assoc".into(),
            name_span: Span::new(),
            subscript: Some(Subscript {
                text: "key".into(),
                raw: Some("\"key\"".into()),
                kind: SubscriptKind::Ordinary,
                interpretation: SubscriptInterpretation::Associative,
                arithmetic_ast: None,
            }),
            span: Span::new(),
        })]);
        assert_eq!(format!("{w}"), "${assoc[\"key\"]}");
        assert_eq!(w.render_syntax(""), "${assoc[\"key\"]}");
    }

    #[test]
    fn word_display_transformation() {
        let w = word(vec![WordPart::Transformation {
            reference: plain_ref("var"),
            operator: 'Q',
        }]);
        assert_eq!(format!("{w}"), "${var@Q}");
    }

    #[test]
    fn word_display_multiple_parts() {
        let w = word(vec![
            WordPart::Literal("hello ".into()),
            WordPart::Variable("USER".into()),
        ]);
        assert_eq!(format!("{w}"), "hello $USER");
    }

    #[test]
    fn pattern_display_multiple_parts() {
        let p = pattern(vec![
            PatternPart::Literal("file".into()),
            PatternPart::AnyString,
            PatternPart::CharClass("[[:digit:]]".into()),
        ]);
        assert_eq!(format!("{p}"), "file*[[:digit:]]");
    }

    #[test]
    fn pattern_render_syntax_prefers_whole_pattern_source_slice() {
        let source = "Darwin\\ arm64*";
        let span = span_for_source(source);
        let p = Pattern {
            parts: vec![PatternPartNode::new(
                PatternPart::Literal(LiteralText::owned("Darwin arm64*".to_string())),
                span,
            )],
            span,
        };

        assert_eq!(p.render_syntax(source), source);
    }

    #[test]
    fn pattern_display_extglob_group() {
        let p = pattern(vec![PatternPart::Group {
            kind: PatternGroupKind::ExactlyOne,
            patterns: vec![
                pattern(vec![PatternPart::Literal("foo".into())]),
                pattern(vec![PatternPart::Literal("bar".into())]),
            ],
        }]);
        assert_eq!(format!("{p}"), "@(foo|bar)");
    }

    #[test]
    fn word_display_parameter_expansion_use_default_colon() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::UseDefault,
            operand: Some("fallback".into()),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:-fallback}");
    }

    #[test]
    fn word_display_parameter_expansion_use_default_no_colon() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::UseDefault,
            operand: Some("fallback".into()),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var-fallback}");
    }

    #[test]
    fn word_display_parameter_expansion_assign_default() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::AssignDefault,
            operand: Some("val".into()),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:=val}");
    }

    #[test]
    fn word_display_parameter_expansion_use_replacement() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::UseReplacement,
            operand: Some("alt".into()),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:+alt}");
    }

    #[test]
    fn word_display_parameter_expansion_error() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::Error,
            operand: Some("msg".into()),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:?msg}");
    }

    #[test]
    fn word_display_parameter_expansion_prefix_suffix() {
        // RemovePrefixShort
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::RemovePrefixShort {
                pattern: pattern(vec![PatternPart::Literal("pat".into())]),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var#pat}");

        // RemovePrefixLong
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::RemovePrefixLong {
                pattern: pattern(vec![PatternPart::Literal("pat".into())]),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var##pat}");

        // RemoveSuffixShort
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::RemoveSuffixShort {
                pattern: pattern(vec![PatternPart::Literal("pat".into())]),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var%pat}");

        // RemoveSuffixLong
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::RemoveSuffixLong {
                pattern: pattern(vec![PatternPart::Literal("pat".into())]),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var%%pat}");
    }

    #[test]
    fn word_display_parameter_expansion_replace() {
        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::ReplaceFirst {
                pattern: pattern(vec![PatternPart::Literal("old".into())]),
                replacement: "new".into(),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var/old/new}");

        let w = word(vec![WordPart::ParameterExpansion {
            reference: plain_ref("var"),
            operator: ParameterOp::ReplaceAll {
                pattern: pattern(vec![PatternPart::Literal("old".into())]),
                replacement: "new".into(),
            },
            operand: None,
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var//old/new}");
    }

    #[test]
    fn word_display_parameter_expansion_case() {
        let check = |op: ParameterOp, expected: &str| {
            let w = word(vec![WordPart::ParameterExpansion {
                reference: plain_ref("var"),
                operator: op,
                operand: None,
                colon_variant: false,
            }]);
            assert_eq!(format!("{w}"), expected);
        };
        check(ParameterOp::UpperFirst, "${var^}");
        check(ParameterOp::UpperAll, "${var^^}");
        check(ParameterOp::LowerAll, "${var,,}");
    }

    // --- SimpleCommand ---

    #[test]
    fn simple_command_construction() {
        let cmd = SimpleCommand {
            name: Word::literal("ls"),
            args: vec![Word::literal("-la")],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        };
        assert_eq!(format!("{}", cmd.name), "ls");
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(format!("{}", cmd.args[0]), "-la");
    }

    #[test]
    fn simple_command_with_redirects() {
        let cmd = SimpleCommand {
            name: Word::literal("echo"),
            args: vec![Word::literal("hi")],
            redirects: vec![Redirect {
                fd: Some(1),
                fd_var: None,
                fd_var_span: None,
                kind: RedirectKind::Output,
                span: Span::new(),
                target: RedirectTarget::Word(Word::literal("out.txt")),
            }],
            assignments: vec![],
            span: Span::new(),
        };
        assert_eq!(cmd.redirects.len(), 1);
        assert_eq!(cmd.redirects[0].fd, Some(1));
        assert_eq!(cmd.redirects[0].kind, RedirectKind::Output);
    }

    #[test]
    fn simple_command_with_assignments() {
        let cmd = SimpleCommand {
            name: Word::literal("env"),
            args: vec![],
            redirects: vec![],
            assignments: vec![assignment(
                plain_ref("FOO"),
                AssignmentValue::Scalar(Word::literal("bar")),
            )],
            span: Span::new(),
        };
        assert_eq!(cmd.assignments.len(), 1);
        assert_eq!(cmd.assignments[0].target.name, "FOO");
        assert!(!cmd.assignments[0].append);
    }

    // --- BuiltinCommand ---

    #[test]
    fn builtin_break_command_construction() {
        let cmd = BuiltinCommand::Break(BreakCommand {
            depth: Some(Word::literal("2")),
            extra_args: vec![Word::literal("extra")],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        });

        if let BuiltinCommand::Break(command) = &cmd {
            assert_eq!(command.depth.as_ref().unwrap().to_string(), "2");
            assert_eq!(command.extra_args.len(), 1);
            assert_eq!(command.extra_args[0].to_string(), "extra");
        } else {
            panic!("expected Break builtin");
        }
    }

    #[test]
    fn builtin_return_command_with_redirects_and_assignments() {
        let cmd = BuiltinCommand::Return(ReturnCommand {
            code: Some(Word::literal("42")),
            extra_args: vec![],
            redirects: vec![Redirect {
                fd: None,
                fd_var: None,
                fd_var_span: None,
                kind: RedirectKind::Output,
                span: Span::new(),
                target: RedirectTarget::Word(Word::literal("out.txt")),
            }],
            assignments: vec![assignment(
                plain_ref("FOO"),
                AssignmentValue::Scalar(Word::literal("bar")),
            )],
            span: Span::new(),
        });

        if let BuiltinCommand::Return(command) = &cmd {
            assert_eq!(command.code.as_ref().unwrap().to_string(), "42");
            assert_eq!(command.redirects.len(), 1);
            assert_eq!(command.assignments.len(), 1);
        } else {
            panic!("expected Return builtin");
        }
    }

    // --- Pipeline ---

    #[test]
    fn pipeline_construction() {
        let pipe = Pipeline {
            negated: false,
            commands: vec![
                Command::Simple(SimpleCommand {
                    name: Word::literal("ls"),
                    args: vec![],
                    redirects: vec![],
                    assignments: vec![],
                    span: Span::new(),
                }),
                Command::Simple(SimpleCommand {
                    name: Word::literal("grep"),
                    args: vec![Word::literal("foo")],
                    redirects: vec![],
                    assignments: vec![],
                    span: Span::new(),
                }),
            ],
            span: Span::new(),
        };
        assert!(!pipe.negated);
        assert_eq!(pipe.commands.len(), 2);
    }

    #[test]
    fn pipeline_negated() {
        let pipe = Pipeline {
            negated: true,
            commands: vec![],
            span: Span::new(),
        };
        assert!(pipe.negated);
    }

    // --- CommandList ---

    #[test]
    fn command_list_with_operators() {
        let first = Command::Simple(SimpleCommand {
            name: Word::literal("true"),
            args: vec![],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        });
        let second = Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![Word::literal("ok")],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        });
        let list = CommandList {
            first: Box::new(first),
            rest: vec![CommandListItem {
                operator: ListOperator::And,
                operator_span: Span::new(),
                command: second,
            }],
            span: Span::new(),
        };
        assert_eq!(list.rest.len(), 1);
        assert_eq!(list.rest[0].operator, ListOperator::And);
    }

    // --- ListOperator ---

    #[test]
    fn list_operator_equality() {
        assert_eq!(ListOperator::And, ListOperator::And);
        assert_eq!(ListOperator::Or, ListOperator::Or);
        assert_eq!(ListOperator::Semicolon, ListOperator::Semicolon);
        assert_eq!(ListOperator::Background, ListOperator::Background);
        assert_ne!(ListOperator::And, ListOperator::Or);
    }

    // --- RedirectKind ---

    #[test]
    fn redirect_kind_equality() {
        assert_eq!(RedirectKind::Output, RedirectKind::Output);
        assert_eq!(RedirectKind::Append, RedirectKind::Append);
        assert_eq!(RedirectKind::Input, RedirectKind::Input);
        assert_eq!(RedirectKind::ReadWrite, RedirectKind::ReadWrite);
        assert_eq!(RedirectKind::HereDoc, RedirectKind::HereDoc);
        assert_eq!(RedirectKind::HereDocStrip, RedirectKind::HereDocStrip);
        assert_eq!(RedirectKind::HereString, RedirectKind::HereString);
        assert_eq!(RedirectKind::DupOutput, RedirectKind::DupOutput);
        assert_eq!(RedirectKind::DupInput, RedirectKind::DupInput);
        assert_eq!(RedirectKind::OutputBoth, RedirectKind::OutputBoth);
        assert_ne!(RedirectKind::Output, RedirectKind::Append);
    }

    // --- Redirect ---

    #[test]
    fn redirect_default_fd_none() {
        let r = Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::Input,
            span: Span::new(),
            target: RedirectTarget::Word(Word::literal("input.txt")),
        };
        assert!(r.fd.is_none());
        assert_eq!(r.kind, RedirectKind::Input);
    }

    #[test]
    fn redirect_exposes_word_target() {
        let redirect = Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::Output,
            span: Span::new(),
            target: RedirectTarget::Word(Word::literal("out.txt")),
        };

        assert_eq!(redirect.word_target().unwrap().to_string(), "out.txt");
        assert!(redirect.heredoc().is_none());
    }

    #[test]
    fn redirect_exposes_heredoc_payload() {
        let delimiter = HeredocDelimiter {
            raw: Word::quoted_literal("EOF"),
            cooked: "EOF".to_owned(),
            span: Span::new(),
            quoted: true,
            expands_body: false,
            strip_tabs: false,
        };
        let redirect = Redirect {
            fd: None,
            fd_var: None,
            fd_var_span: None,
            kind: RedirectKind::HereDoc,
            span: Span::new(),
            target: RedirectTarget::Heredoc(Heredoc {
                delimiter,
                body: Word::quoted_literal("body"),
            }),
        };

        let heredoc = redirect.heredoc().expect("expected heredoc payload");
        assert_eq!(heredoc.delimiter.cooked, "EOF");
        assert!(heredoc.delimiter.quoted);
        assert!(redirect.word_target().is_none());
    }

    // --- Assignment ---

    #[test]
    fn assignment_scalar() {
        let a = assignment(plain_ref("X"), AssignmentValue::Scalar(Word::literal("1")));
        assert_eq!(a.target.name, "X");
        assert!(a.target.subscript.is_none());
        assert!(!a.append);
    }

    #[test]
    fn assignment_array() {
        let a = assignment(
            plain_ref("ARR"),
            AssignmentValue::Compound(ArrayExpr {
                kind: ArrayKind::Indexed,
                elements: vec![
                    ArrayElem::Sequential(Word::literal("a")),
                    ArrayElem::Sequential(Word::literal("b")),
                    ArrayElem::Sequential(Word::literal("c")),
                ],
                span: Span::new(),
            }),
        );
        if let AssignmentValue::Compound(array) = &a.value {
            assert_eq!(array.elements.len(), 3);
        } else {
            panic!("expected Compound");
        }
    }

    #[test]
    fn assignment_append() {
        let mut a = assignment(
            plain_ref("PATH"),
            AssignmentValue::Scalar(Word::literal("/usr/bin")),
        );
        a.append = true;
        assert!(a.append);
    }

    #[test]
    fn assignment_indexed() {
        let a = assignment(
            indexed_ref("arr", "0"),
            AssignmentValue::Scalar(Word::literal("val")),
        );
        assert_eq!(
            a.target
                .subscript
                .as_ref()
                .map(|subscript| subscript.syntax_text("")),
            Some("0")
        );
    }

    // --- CaseTerminator ---

    #[test]
    fn case_terminator_equality() {
        assert_eq!(CaseTerminator::Break, CaseTerminator::Break);
        assert_eq!(CaseTerminator::FallThrough, CaseTerminator::FallThrough);
        assert_eq!(CaseTerminator::Continue, CaseTerminator::Continue);
        assert_ne!(CaseTerminator::Break, CaseTerminator::FallThrough);
    }

    // --- Compound commands ---

    #[test]
    fn if_command_construction() {
        let if_cmd = IfCommand {
            condition: vec![],
            then_branch: vec![],
            elif_branches: vec![],
            else_branch: None,
            span: Span::new(),
        };
        assert!(if_cmd.else_branch.is_none());
        assert!(if_cmd.elif_branches.is_empty());
    }

    #[test]
    fn for_command_without_words() {
        let for_cmd = ForCommand {
            variable: "i".into(),
            variable_span: Span::new(),
            words: None,
            body: vec![],
            span: Span::new(),
        };
        assert!(for_cmd.words.is_none());
        assert_eq!(for_cmd.variable, "i");
    }

    #[test]
    fn for_command_with_words() {
        let for_cmd = ForCommand {
            variable: "x".into(),
            variable_span: Span::new(),
            words: Some(vec![Word::literal("1"), Word::literal("2")]),
            body: vec![],
            span: Span::new(),
        };
        assert_eq!(for_cmd.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn arithmetic_for_command() {
        let cmd = ArithmeticForCommand {
            left_paren_span: Span::new(),
            init_span: Some(Span::new()),
            init_ast: None,
            first_semicolon_span: Span::new(),
            condition_span: Some(Span::new()),
            condition_ast: None,
            second_semicolon_span: Span::new(),
            step_span: Some(Span::new()),
            step_ast: None,
            right_paren_span: Span::new(),
            body: vec![],
            span: Span::new(),
        };
        assert!(cmd.init_span.is_some());
        assert!(cmd.condition_span.is_some());
        assert!(cmd.step_span.is_some());
    }

    #[test]
    fn function_def_construction() {
        let func = FunctionDef {
            name: "my_func".into(),
            name_span: Span::new(),
            surface: FunctionSurface::default(),
            body: Box::new(Command::Simple(SimpleCommand {
                name: Word::literal("echo"),
                args: vec![Word::literal("hello")],
                redirects: vec![],
                assignments: vec![],
                span: Span::new(),
            })),
            span: Span::new(),
        };
        assert_eq!(func.name, "my_func");
    }

    // --- Script ---

    #[test]
    fn script_empty() {
        let script = Script {
            commands: vec![],
            span: Span::new(),
        };
        assert!(script.commands.is_empty());
    }

    // --- Command enum variants ---

    #[test]
    fn command_variants_constructible() {
        let simple = Command::Simple(SimpleCommand {
            name: Word::literal("echo"),
            args: vec![],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        });
        assert!(matches!(simple, Command::Simple(_)));

        let pipe = Command::Pipeline(Pipeline {
            negated: false,
            commands: vec![],
            span: Span::new(),
        });
        assert!(matches!(pipe, Command::Pipeline(_)));

        let builtin = Command::Builtin(BuiltinCommand::Exit(ExitCommand {
            code: Some(Word::literal("1")),
            extra_args: vec![],
            redirects: vec![],
            assignments: vec![],
            span: Span::new(),
        }));
        assert!(matches!(builtin, Command::Builtin(_)));

        let compound = Command::Compound(CompoundCommand::BraceGroup(vec![]), vec![]);
        assert!(matches!(compound, Command::Compound(..)));

        let func = Command::Function(FunctionDef {
            name: "f".into(),
            name_span: Span::new(),
            surface: FunctionSurface::default(),
            body: Box::new(Command::Simple(SimpleCommand {
                name: Word::literal("true"),
                args: vec![],
                redirects: vec![],
                assignments: vec![],
                span: Span::new(),
            })),
            span: Span::new(),
        });
        assert!(matches!(func, Command::Function(_)));
    }

    // --- CompoundCommand variants ---

    #[test]
    fn compound_command_subshell() {
        let cmd = CompoundCommand::Subshell(vec![]);
        assert!(matches!(cmd, CompoundCommand::Subshell(_)));
    }

    #[test]
    fn compound_command_arithmetic() {
        let cmd = CompoundCommand::Arithmetic(ArithmeticCommand {
            span: Span::new(),
            left_paren_span: Span::new(),
            expr_span: Some(Span::new()),
            expr_ast: None,
            right_paren_span: Span::new(),
        });
        assert!(matches!(cmd, CompoundCommand::Arithmetic(_)));
    }

    #[test]
    fn compound_command_conditional() {
        let cmd = CompoundCommand::Conditional(ConditionalCommand {
            expression: ConditionalExpr::Unary(ConditionalUnaryExpr {
                op: ConditionalUnaryOp::RegularFile,
                op_span: Span::new(),
                expr: Box::new(ConditionalExpr::Word(Word::literal("file"))),
            }),
            span: Span::new(),
            left_bracket_span: Span::new(),
            right_bracket_span: Span::new(),
        });
        if let CompoundCommand::Conditional(command) = &cmd {
            let ConditionalExpr::Unary(expr) = &command.expression else {
                panic!("expected unary conditional");
            };
            assert_eq!(expr.op, ConditionalUnaryOp::RegularFile);
        } else {
            panic!("expected Conditional");
        }
    }

    #[test]
    fn time_command_construction() {
        let cmd = TimeCommand {
            posix_format: true,
            command: None,
            span: Span::new(),
        };
        assert!(cmd.posix_format);
        assert!(cmd.command.is_none());
    }
}
