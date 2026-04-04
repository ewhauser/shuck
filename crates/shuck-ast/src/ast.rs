//! AST types for parsed bash scripts
//!
//! These types define the abstract syntax tree for bash scripts.
//! All command nodes include source location spans for error messages and $LINENO.

#![allow(dead_code)]

use crate::span::Span;
use std::fmt;

/// A complete bash script.
#[derive(Debug, Clone)]
pub struct Script {
    pub commands: Vec<Command>,
    /// Source span of the entire script
    pub span: Span,
}

/// A single command in the script.
#[derive(Debug, Clone)]
pub enum Command {
    /// A simple command (e.g., `echo hello`)
    Simple(SimpleCommand),

    /// A builtin command with a dedicated typed AST node
    Builtin(BuiltinCommand),

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
    pub rest: Vec<(ListOperator, Command)>,
    /// Source span of this command list
    pub span: Span,
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
    ArithmeticFor(ArithmeticForCommand),
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
    Arithmetic(String),
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
    pub name: String,
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
    Pattern(Word),
    Regex(Word),
}

impl ConditionalExpr {
    /// Source span of this conditional expression node.
    pub fn span(&self) -> Span {
        match self {
            Self::Binary(expr) => expr.span(),
            Self::Unary(expr) => expr.span(),
            Self::Parenthesized(expr) => expr.span(),
            Self::Word(word) | Self::Pattern(word) | Self::Regex(word) => word.span,
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
    pub variable: String,
    pub words: Option<Vec<Word>>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// Select loop.
#[derive(Debug, Clone)]
pub struct SelectCommand {
    pub variable: String,
    pub words: Vec<Word>,
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
}

/// C-style arithmetic for loop: for ((init; cond; step)); do body; done
#[derive(Debug, Clone)]
pub struct ArithmeticForCommand {
    /// Initialization expression
    pub init: String,
    /// Condition expression
    pub condition: String,
    /// Step/update expression
    pub step: String,
    /// Loop body
    pub body: Vec<Command>,
    /// Source span of this command
    pub span: Span,
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
#[derive(Debug, Clone, PartialEq)]
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
    pub patterns: Vec<Word>,
    pub commands: Vec<Command>,
    pub terminator: CaseTerminator,
}

/// Function definition.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub body: Box<Command>,
    /// Source span of this function definition
    pub span: Span,
}

/// A word (potentially with expansions).
#[derive(Debug, Clone)]
pub struct Word {
    pub parts: Vec<WordPart>,
    /// Source spans for each word part in `parts`
    pub part_spans: Vec<Span>,
    /// True if this word came from a quoted source (single or double quotes)
    /// Quoted words should not undergo brace expansion or glob expansion
    pub quoted: bool,
    /// Source span of this word
    pub span: Span,
}

impl Word {
    /// Create a simple literal word.
    pub fn literal(s: impl Into<String>) -> Self {
        Self::literal_with_span(s, Span::new())
    }

    /// Create a simple literal word with an explicit source span.
    pub fn literal_with_span(s: impl Into<String>, span: Span) -> Self {
        Self {
            parts: vec![WordPart::Literal(s.into())],
            part_spans: vec![span],
            quoted: false,
            span,
        }
    }

    /// Create a quoted literal word (no brace/glob expansion).
    pub fn quoted_literal(s: impl Into<String>) -> Self {
        Self::quoted_literal_with_span(s, Span::new())
    }

    /// Create a quoted literal word with an explicit source span.
    pub fn quoted_literal_with_span(s: impl Into<String>, span: Span) -> Self {
        Self {
            parts: vec![WordPart::Literal(s.into())],
            part_spans: vec![span],
            quoted: true,
            span,
        }
    }

    /// Set the source span on an existing word.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        if self.parts.len() == self.part_spans.len() && self.part_spans.len() == 1 {
            self.part_spans[0] = span;
        }
        self
    }

    /// Get the source span for a specific word part.
    pub fn part_span(&self, index: usize) -> Option<Span> {
        self.part_spans.get(index).copied()
    }

    /// Iterate over word parts and their spans together.
    pub fn parts_with_spans(&self) -> impl Iterator<Item = (&WordPart, Span)> + '_ {
        self.parts.iter().zip(self.part_spans.iter().copied())
    }
}

impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.parts {
            match part {
                WordPart::Literal(s) => write!(f, "{}", s)?,
                WordPart::Variable(name) => write!(f, "${}", name)?,
                WordPart::CommandSubstitution(cmd) => write!(f, "$({:?})", cmd)?,
                WordPart::ArithmeticExpansion(expr) => write!(f, "$(({}))", expr)?,
                WordPart::ParameterExpansion {
                    name,
                    operator,
                    operand,
                    colon_variant,
                } => match operator {
                    ParameterOp::UseDefault => {
                        let c = if *colon_variant { ":" } else { "" };
                        write!(f, "${{{}{}-{}}}", name, c, operand)?
                    }
                    ParameterOp::AssignDefault => {
                        let c = if *colon_variant { ":" } else { "" };
                        write!(f, "${{{}{}={}}}", name, c, operand)?
                    }
                    ParameterOp::UseReplacement => {
                        let c = if *colon_variant { ":" } else { "" };
                        write!(f, "${{{}{}+{}}}", name, c, operand)?
                    }
                    ParameterOp::Error => {
                        let c = if *colon_variant { ":" } else { "" };
                        write!(f, "${{{}{}?{}}}", name, c, operand)?
                    }
                    ParameterOp::RemovePrefixShort => write!(f, "${{{}#{}}}", name, operand)?,
                    ParameterOp::RemovePrefixLong => write!(f, "${{{}##{}}}", name, operand)?,
                    ParameterOp::RemoveSuffixShort => write!(f, "${{{}%{}}}", name, operand)?,
                    ParameterOp::RemoveSuffixLong => write!(f, "${{{}%%{}}}", name, operand)?,
                    ParameterOp::ReplaceFirst {
                        pattern,
                        replacement,
                    } => write!(f, "${{{}/{}/{}}}", name, pattern, replacement)?,
                    ParameterOp::ReplaceAll {
                        pattern,
                        replacement,
                    } => write!(f, "${{{}///{}/{}}}", name, pattern, replacement)?,
                    ParameterOp::UpperFirst => write!(f, "${{{}^}}", name)?,
                    ParameterOp::UpperAll => write!(f, "${{{}^^}}", name)?,
                    ParameterOp::LowerFirst => write!(f, "{}{{,}}", name)?,
                    ParameterOp::LowerAll => write!(f, "${{{},,}}", name)?,
                },
                WordPart::Length(name) => write!(f, "${{#{}}}", name)?,
                WordPart::ArrayAccess { name, index } => write!(f, "${{{}[{}]}}", name, index)?,
                WordPart::ArrayLength(name) => write!(f, "${{#{}[@]}}", name)?,
                WordPart::ArrayIndices(name) => write!(f, "${{!{}[@]}}", name)?,
                WordPart::Substring {
                    name,
                    offset,
                    length,
                } => {
                    if let Some(len) = length {
                        write!(f, "${{{}:{}:{}}}", name, offset, len)?
                    } else {
                        write!(f, "${{{}:{}}}", name, offset)?
                    }
                }
                WordPart::ArraySlice {
                    name,
                    offset,
                    length,
                } => {
                    if let Some(len) = length {
                        write!(f, "${{{}[@]:{}:{}}}", name, offset, len)?
                    } else {
                        write!(f, "${{{}[@]:{}}}", name, offset)?
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
                        write!(f, "${{!{}{}{}{}}}", name, c, op_char, operand)?
                    } else {
                        write!(f, "${{!{}}}", name)?
                    }
                }
                WordPart::PrefixMatch(prefix) => write!(f, "${{!{}*}}", prefix)?,
                WordPart::ProcessSubstitution { commands, is_input } => {
                    let prefix = if *is_input { "<" } else { ">" };
                    write!(f, "{}({:?})", prefix, commands)?
                }
                WordPart::Transformation { name, operator } => {
                    write!(f, "${{{}@{}}}", name, operator)?
                }
            }
        }
        Ok(())
    }
}

/// Parts of a word.
#[derive(Debug, Clone)]
pub enum WordPart {
    /// Literal text
    Literal(String),
    /// Variable expansion ($VAR or ${VAR})
    Variable(String),
    /// Command substitution ($(...))
    CommandSubstitution(Vec<Command>),
    /// Arithmetic expansion ($((...)))
    ArithmeticExpansion(String),
    /// Parameter expansion with operator ${var:-default}, ${var:=default}, etc.
    /// `colon_variant` distinguishes `:-` (unset-or-empty) from `-` (unset-only).
    ParameterExpansion {
        name: String,
        operator: ParameterOp,
        operand: String,
        colon_variant: bool,
    },
    /// Length expansion ${#var}
    Length(String),
    /// Array element access `${arr[index]}` or `${arr[@]}` or `${arr[*]}`
    ArrayAccess { name: String, index: String },
    /// Array length `${#arr[@]}` or `${#arr[*]}`
    ArrayLength(String),
    /// Array indices `${!arr[@]}` or `${!arr[*]}`
    ArrayIndices(String),
    /// Substring extraction `${var:offset}` or `${var:offset:length}`
    Substring {
        name: String,
        offset: String,
        length: Option<String>,
    },
    /// Array slice `${arr[@]:offset:length}`
    ArraySlice {
        name: String,
        offset: String,
        length: Option<String>,
    },
    /// Indirect expansion `${!var}` - expands to value of variable named by var's value
    /// Optionally composed with an operator: `${!var:-default}`, `${!var:=val}`, etc.
    IndirectExpansion {
        name: String,
        operator: Option<ParameterOp>,
        operand: String,
        colon_variant: bool,
    },
    /// Prefix matching `${!prefix*}` or `${!prefix@}` - names of variables with given prefix
    PrefixMatch(String),
    /// Process substitution <(cmd) or >(cmd)
    ProcessSubstitution {
        /// The commands to run
        commands: Vec<Command>,
        /// True for <(cmd), false for >(cmd)
        is_input: bool,
    },
    /// Parameter transformation `${var@op}` where op is Q, E, P, A, K, a, u, U, L
    Transformation { name: String, operator: char },
}

/// Parameter expansion operators
#[derive(Debug, Clone, PartialEq)]
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
    RemovePrefixShort,
    /// ## remove prefix (longest)
    RemovePrefixLong,
    /// % remove suffix (shortest)
    RemoveSuffixShort,
    /// %% remove suffix (longest)
    RemoveSuffixLong,
    /// / pattern replacement (first occurrence)
    ReplaceFirst {
        pattern: String,
        replacement: String,
    },
    /// // pattern replacement (all occurrences)
    ReplaceAll {
        pattern: String,
        replacement: String,
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
    pub fd_var: Option<String>,
    /// Type of redirection
    pub kind: RedirectKind,
    /// Source span of this redirection
    pub span: Span,
    /// Target (file, fd, or heredoc content)
    pub target: Word,
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
    pub name: String,
    /// Optional array index for indexed assignments like `arr[0]=value`
    pub index: Option<String>,
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
    Array(Vec<Word>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(parts: Vec<WordPart>) -> Word {
        let span = Span::new();
        let part_count = parts.len();
        Word {
            parts,
            part_spans: vec![span; part_count],
            quoted: false,
            span,
        }
    }

    // --- Word ---

    #[test]
    fn word_literal_creates_unquoted_word() {
        let w = Word::literal("hello");
        assert!(!w.quoted);
        assert_eq!(w.parts.len(), 1);
        assert!(matches!(&w.parts[0], WordPart::Literal(s) if s == "hello"));
    }

    #[test]
    fn word_literal_empty_string() {
        let w = Word::literal("");
        assert!(!w.quoted);
        assert!(matches!(&w.parts[0], WordPart::Literal(s) if s.is_empty()));
    }

    #[test]
    fn word_quoted_literal_sets_quoted_flag() {
        let w = Word::quoted_literal("world");
        assert!(w.quoted);
        assert_eq!(w.parts.len(), 1);
        assert!(matches!(&w.parts[0], WordPart::Literal(s) if s == "world"));
    }

    #[test]
    fn word_display_literal() {
        let w = Word::literal("echo");
        assert_eq!(format!("{w}"), "echo");
    }

    #[test]
    fn word_display_variable() {
        let w = word(vec![WordPart::Variable("HOME".into())]);
        assert_eq!(format!("{w}"), "$HOME");
    }

    #[test]
    fn word_display_arithmetic_expansion() {
        let w = word(vec![WordPart::ArithmeticExpansion("1+2".into())]);
        assert_eq!(format!("{w}"), "$((1+2))");
    }

    #[test]
    fn word_display_length() {
        let w = word(vec![WordPart::Length("var".into())]);
        assert_eq!(format!("{w}"), "${#var}");
    }

    #[test]
    fn word_display_array_access() {
        let w = word(vec![WordPart::ArrayAccess {
            name: "arr".into(),
            index: "0".into(),
        }]);
        assert_eq!(format!("{w}"), "${arr[0]}");
    }

    #[test]
    fn word_display_array_length() {
        let w = word(vec![WordPart::ArrayLength("arr".into())]);
        assert_eq!(format!("{w}"), "${#arr[@]}");
    }

    #[test]
    fn word_display_array_indices() {
        let w = word(vec![WordPart::ArrayIndices("arr".into())]);
        assert_eq!(format!("{w}"), "${!arr[@]}");
    }

    #[test]
    fn word_display_substring_with_length() {
        let w = word(vec![WordPart::Substring {
            name: "var".into(),
            offset: "2".into(),
            length: Some("3".into()),
        }]);
        assert_eq!(format!("{w}"), "${var:2:3}");
    }

    #[test]
    fn word_display_substring_without_length() {
        let w = word(vec![WordPart::Substring {
            name: "var".into(),
            offset: "2".into(),
            length: None,
        }]);
        assert_eq!(format!("{w}"), "${var:2}");
    }

    #[test]
    fn word_display_array_slice_with_length() {
        let w = word(vec![WordPart::ArraySlice {
            name: "arr".into(),
            offset: "1".into(),
            length: Some("2".into()),
        }]);
        assert_eq!(format!("{w}"), "${arr[@]:1:2}");
    }

    #[test]
    fn word_display_array_slice_without_length() {
        let w = word(vec![WordPart::ArraySlice {
            name: "arr".into(),
            offset: "1".into(),
            length: None,
        }]);
        assert_eq!(format!("{w}"), "${arr[@]:1}");
    }

    #[test]
    fn word_display_indirect_expansion() {
        let w = word(vec![WordPart::IndirectExpansion {
            name: "ref".into(),
            operator: None,
            operand: String::new(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${!ref}");
    }

    #[test]
    fn word_display_prefix_match() {
        let w = word(vec![WordPart::PrefixMatch("MY_".into())]);
        assert_eq!(format!("{w}"), "${!MY_*}");
    }

    #[test]
    fn word_display_transformation() {
        let w = word(vec![WordPart::Transformation {
            name: "var".into(),
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
    fn word_display_parameter_expansion_use_default_colon() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::UseDefault,
            operand: "fallback".into(),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:-fallback}");
    }

    #[test]
    fn word_display_parameter_expansion_use_default_no_colon() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::UseDefault,
            operand: "fallback".into(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var-fallback}");
    }

    #[test]
    fn word_display_parameter_expansion_assign_default() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::AssignDefault,
            operand: "val".into(),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:=val}");
    }

    #[test]
    fn word_display_parameter_expansion_use_replacement() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::UseReplacement,
            operand: "alt".into(),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:+alt}");
    }

    #[test]
    fn word_display_parameter_expansion_error() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::Error,
            operand: "msg".into(),
            colon_variant: true,
        }]);
        assert_eq!(format!("{w}"), "${var:?msg}");
    }

    #[test]
    fn word_display_parameter_expansion_prefix_suffix() {
        // RemovePrefixShort
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::RemovePrefixShort,
            operand: "pat".into(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var#pat}");

        // RemovePrefixLong
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::RemovePrefixLong,
            operand: "pat".into(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var##pat}");

        // RemoveSuffixShort
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::RemoveSuffixShort,
            operand: "pat".into(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var%pat}");

        // RemoveSuffixLong
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::RemoveSuffixLong,
            operand: "pat".into(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var%%pat}");
    }

    #[test]
    fn word_display_parameter_expansion_replace() {
        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::ReplaceFirst {
                pattern: "old".into(),
                replacement: "new".into(),
            },
            operand: String::new(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var/old/new}");

        let w = word(vec![WordPart::ParameterExpansion {
            name: "var".into(),
            operator: ParameterOp::ReplaceAll {
                pattern: "old".into(),
                replacement: "new".into(),
            },
            operand: String::new(),
            colon_variant: false,
        }]);
        assert_eq!(format!("{w}"), "${var///old/new}");
    }

    #[test]
    fn word_display_parameter_expansion_case() {
        let check = |op: ParameterOp, expected: &str| {
            let w = word(vec![WordPart::ParameterExpansion {
                name: "var".into(),
                operator: op,
                operand: String::new(),
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
                kind: RedirectKind::Output,
                span: Span::new(),
                target: Word::literal("out.txt"),
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
            assignments: vec![Assignment {
                name: "FOO".into(),
                index: None,
                value: AssignmentValue::Scalar(Word::literal("bar")),
                append: false,
                span: Span::new(),
            }],
            span: Span::new(),
        };
        assert_eq!(cmd.assignments.len(), 1);
        assert_eq!(cmd.assignments[0].name, "FOO");
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
                kind: RedirectKind::Output,
                span: Span::new(),
                target: Word::literal("out.txt"),
            }],
            assignments: vec![Assignment {
                name: "FOO".into(),
                index: None,
                value: AssignmentValue::Scalar(Word::literal("bar")),
                append: false,
                span: Span::new(),
            }],
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
            rest: vec![(ListOperator::And, second)],
            span: Span::new(),
        };
        assert_eq!(list.rest.len(), 1);
        assert_eq!(list.rest[0].0, ListOperator::And);
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
            kind: RedirectKind::Input,
            span: Span::new(),
            target: Word::literal("input.txt"),
        };
        assert!(r.fd.is_none());
        assert_eq!(r.kind, RedirectKind::Input);
    }

    // --- Assignment ---

    #[test]
    fn assignment_scalar() {
        let a = Assignment {
            name: "X".into(),
            index: None,
            value: AssignmentValue::Scalar(Word::literal("1")),
            append: false,
            span: Span::new(),
        };
        assert_eq!(a.name, "X");
        assert!(a.index.is_none());
        assert!(!a.append);
    }

    #[test]
    fn assignment_array() {
        let a = Assignment {
            name: "ARR".into(),
            index: None,
            value: AssignmentValue::Array(vec![
                Word::literal("a"),
                Word::literal("b"),
                Word::literal("c"),
            ]),
            append: false,
            span: Span::new(),
        };
        if let AssignmentValue::Array(words) = &a.value {
            assert_eq!(words.len(), 3);
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn assignment_append() {
        let a = Assignment {
            name: "PATH".into(),
            index: None,
            value: AssignmentValue::Scalar(Word::literal("/usr/bin")),
            append: true,
            span: Span::new(),
        };
        assert!(a.append);
    }

    #[test]
    fn assignment_indexed() {
        let a = Assignment {
            name: "arr".into(),
            index: Some("0".into()),
            value: AssignmentValue::Scalar(Word::literal("val")),
            append: false,
            span: Span::new(),
        };
        assert_eq!(a.index.as_deref(), Some("0"));
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
            words: Some(vec![Word::literal("1"), Word::literal("2")]),
            body: vec![],
            span: Span::new(),
        };
        assert_eq!(for_cmd.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn arithmetic_for_command() {
        let cmd = ArithmeticForCommand {
            init: "i=0".into(),
            condition: "i<10".into(),
            step: "i++".into(),
            body: vec![],
            span: Span::new(),
        };
        assert_eq!(cmd.init, "i=0");
        assert_eq!(cmd.condition, "i<10");
        assert_eq!(cmd.step, "i++");
    }

    #[test]
    fn function_def_construction() {
        let func = FunctionDef {
            name: "my_func".into(),
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
        let cmd = CompoundCommand::Arithmetic("1+1".into());
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
