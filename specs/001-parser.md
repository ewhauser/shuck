# 001: Shell Parser

## Status

Implemented

## Summary

The shuck-parser crate is a Bash script parser library that converts shell script source text into an Abstract Syntax Tree (AST). It implements a recursive descent parser with a two-stage architecture: a **Lexer** that tokenizes input into a stream of tokens with source position tracking, and a **Parser** that builds an AST from tokens using recursive descent, handling Bash grammar including pipes, lists, control structures, redirections, heredocs, and word expansions.

## Motivation

A correct, complete Bash parser is essential for the shuck project:

- **Linting**: Enables static analysis of shell scripts by producing a structured AST that lint rules can traverse
- **Error Reporting**: Provides precise source locations (line, column) for all parse errors
- **Expansion Handling**: Parses inline expansions (`$var`, `$(...)`, `${}`, etc.) with correct semantics
- **Security**: Validates deeply nested structures to prevent DoS attacks (stack overflow, infinite loops)

The parser must be fast, correct, and robust to malformed input. Recovery from errors allows analysis of partially-valid scripts without stopping on the first error.

## Design

### Architecture Overview

The parsing pipeline:

```
Input (String)
    -> Lexer: char-by-char tokenization with position tracking
    -> Token Stream (Word, Pipe, Semicolon, Redirect tokens, etc.)
    -> Parser: recursive descent with lookahead and depth/fuel limits
    -> AST (Script containing Commands and nested structures)
```

The parser maintains:
- **Current token** and **lookahead token** for decision making
- **Position tracking** through `Position` (line, column, offset) and `Span` types
- **Depth counters** to enforce recursion limits (prevents stack overflow)
- **Fuel (operation count)** to limit total work (prevents infinite loops)

### Lexer

**Location:** `crates/shuck-parser/src/parser/lexer.rs`

Converts source text into a stream of tokens with position tracking. Handles special characters (pipes, redirects, operators), quoted strings (single, double, ANSI-C quoting), word constructs (variable references, expansions, brace/glob patterns), command substitution with depth tracking, heredocs with arbitrary delimiters, and comments (optionally preserved).

#### State

```rust
pub struct Lexer<'a> {
    input: &'a str,
    position: Position,              // Current line:col:offset
    chars: Peekable<Chars>,
    reinject_buf: VecDeque<char>,   // For heredoc rest-of-line tokens
    max_subst_depth: usize,         // Limit on $(...) nesting
}
```

#### Token Types

Defined in `shuck_ast::Token`:

**Delimiters & Control Flow:**
- `Newline`, `Semicolon` (`;`), `DoubleSemicolon` (`;;`), `SemiAmp` (`;&`), `DoubleSemiAmp` (`;;&`)
- `Pipe` (`|`), `And` (`&&`), `Or` (`||`), `Background` (`&`)

**Redirections:**
- Basic: `RedirectOut` (`>`), `RedirectAppend` (`>>`), `RedirectIn` (`<`)
- Heredocs: `HereDoc` (`<<`), `HereDocStrip` (`<<-`), `HereString` (`<<<`)
- Compound: `RedirectBoth` (`&>`), `Clobber` (`>|`), `DupOutput` (`>&`), `DupInput` (`<&`)
- FD variants: `RedirectFd(i32)`, `RedirectFdAppend(i32)`, `DupFd(i32, i32)`, `DupFdIn(i32, i32)`, `DupFdClose(i32)`, `RedirectFdIn(i32)`
- Process: `ProcessSubIn` (`<(...)`), `ProcessSubOut` (`>(...)`)

**Grouping:**
- Parens: `LeftParen`, `RightParen`, `DoubleLeftParen` (`((`), `DoubleRightParen` (`))`)
- Braces: `LeftBrace`, `RightBrace`
- Brackets: `DoubleLeftBracket` (`[[`), `DoubleRightBracket` (`]]`)

**Words:**
- `Word(String)` -- regular word with possible expansions
- `LiteralWord(String)` -- single-quoted, no expansions
- `QuotedWord(String)` -- double-quoted, may contain expansions
- `Comment(String)` -- line comment body (only with `next_token_with_comments()`)
- `Error(String)` -- lexer error (unterminated quote, etc.)

#### Quoting Rules

- **Single Quotes:** `'...'` -- No expansions, no escaping. Produces `LiteralWord`.
- **Double Quotes:** `"..."` -- Variable expansions active, backslash escaping. Produces `QuotedWord` or `Word` if followed by unquoted content.
- **ANSI-C:** `$'...'` -- Escape sequences processed: `\n`, `\t`, `\r`, `\a`, `\b`, `\e`, `\\`, `\'`. Processed at parse time.
- **Locale:** `$"..."` -- Treated as double quotes (synonym).

#### Special Handling

**Brace Expansion vs. Brace Group:**
- `{a,b,c}` or `{1..5}` -> `Word` token (brace expansion in lexer)
- `{ cmd; }` -> `LeftBrace` token (brace group, requires space after `{`)

**Bracket Expression vs. Test:**
- `[abc]` (glob) -> bracket word token
- `[ -f file ]` (test) -> `Word("[")` token
- Heuristic: `[` followed by whitespace or quote -> test command

**File Descriptor Redirects:**
- `2>`, `2>>`, `2>&1` -> tokenized with fd info via lookahead
- Lookahead to distinguish `23` (number in word) from `2>` (redirect)

**Heredoc Delimiter Quoting:**
- Quoted: `<<"EOF"` or `<<'EOF'` -> content is literal (no expansions)
- Unquoted: `<<EOF` -> content has expansions (variables, substitutions)

**Command Substitution Depth:**
- Max nesting: `max_subst_depth` default 50
- Prevents stack overflow from deeply nested `$(...)`
- When exceeded, content consumed but error emitted

#### Key Methods

- `new(input)` / `with_max_subst_depth(input, max_depth)` -- Create lexer
- `next_token() -> Option<Token>` -- Get next token (skips whitespace & comments)
- `next_spanned_token() -> Option<SpannedToken>` -- Token with source span
- `position() -> Position` -- Current position
- `read_heredoc(delimiter) -> String` -- Read heredoc content until delimiter

### Parser

**Location:** `crates/shuck-parser/src/parser/mod.rs`

Recursive descent parser that builds an AST from the token stream.

#### State

```rust
pub struct Parser<'a> {
    input: &'a str,
    lexer: Lexer<'a>,
    current_token: Option<Token>,
    current_span: Span,
    peeked_token: Option<SpannedToken>,   // One-token lookahead
    max_depth: usize,      // Hard clamped to 100 (HARD_MAX_AST_DEPTH)
    current_depth: usize,  // Current recursion depth
    fuel: usize,           // Remaining operations
    max_fuel: usize,       // Maximum operations
}
```

#### Safety Limits

| Limit | Default | Hard Cap | Purpose |
|-------|---------|----------|---------|
| AST Depth | 100 | 100 | Prevent stack overflow from deep nesting |
| Parser Ops | 100,000 | None | Prevent infinite loops |
| Subst Depth | 50 | 50 | Prevent stack overflow in lexer |

- **Depth:** Checked via `push_depth()` / `pop_depth()` on each recursive call
- **Fuel:** Decremented on each `tick()` call; errors if exhausted
- **Enforcement:** Depth is clamped to hard cap on construction to prevent misconfiguration

#### Entry Points

```rust
pub fn parse(self) -> Result<Script>              // Strict mode
pub fn parse_recovered(self) -> RecoveredParse    // Recovery mode
pub fn parse_word_string(input: &str) -> Word     // Parse isolated word
```

#### Recursive Descent Methods

**Top-level parsing:**
- `parse()` -- Entry point; parse script and return `Script` or error
- `parse_command_list()` -> `Option<Command>` -- Commands with `&&`, `||`, `;`, `&`
- `parse_pipeline()` -> `Option<Command>` -- Commands with pipes and negation (`!`)
- `parse_command()` -> `Option<Command>` -- Dispatch to compound or simple
- `parse_simple_command()` -> `Option<SimpleCommand>` -- Command with args and redirects

**Compound command parsers:**
- `parse_if()` -- `if cond; then body; [elif cond; then body;]* [else body;] fi`
- `parse_for()` -- `for var [in words]; do body; done` or C-style `for ((init; cond; step))`
- `parse_while()` -- `while cond; do body; done`
- `parse_until()` -- `until cond; do body; done`
- `parse_case()` -- `case word in pattern) body;; [pattern) body;;]* esac`
- `parse_select()` -- `select var in words; do body; done`
- `parse_subshell()` -- `( commands )`
- `parse_brace_group()` -- `{ commands; }`
- `parse_conditional()` -- `[[ test expression ]]`
- `parse_arithmetic_command()` -- `(( arithmetic expression ))`
- `parse_time()` -- `time [-p] command`
- `parse_coproc()` -- `coproc [name] command`
- `parse_function_keyword()` / `parse_function_posix()` -- Function definitions

**Helper methods:**
- `parse_word(s: String) -> Word` -- Parse word string with expansions
- `parse_compound_list(terminator) -> Vec<Command>` -- Commands until keyword
- `parse_compound_list_until(terminators) -> Vec<Command>` -- Until any of keywords
- `parse_trailing_redirects() -> Vec<Redirect>` -- Redirects after compound command
- `parse_heredoc_redirect(strip_tabs) -> Result<()>` -- Parse and consume heredoc
- `expect_word() -> Result<Word>` -- Require word token

#### Reserved Words

Words that are **only** special in command position:

```rust
["then", "else", "elif", "fi", "do", "done", "esac", "in"]
```

These terminate compound command bodies. In argument position, they are regular words.

Example: `echo then` prints "then"; `for x in a then b` iterates over `[a, then, b]`.

#### Assignment Recognition

Assignments in command-leading position:
- Simple: `VAR=value`
- Append: `VAR+=value`
- Indexed: `VAR[expr]=value`
- Compound: `arr=(a b c)`

Validation: Variable name must match `[a-zA-Z_][a-zA-Z0-9_]*`

#### Redirection Handling

Redirections collected in `SimpleCommand` and compound commands:

```rust
pub struct Redirect {
    pub fd: Option<i32>,         // File descriptor (1 = stdout, 0 = stdin)
    pub fd_var: Option<Name>,    // For {var}>file syntax
    pub fd_var_span: Option<Span>,
    pub kind: RedirectKind,      // Input, Output, Append, HereDoc, etc.
    pub span: Span,
    pub target: Word,            // Filename or FD number
}

pub enum RedirectKind {
    Output,       // >
    Clobber,      // >| (force overwrite)
    Append,       // >>
    Input,        // <
    HereDoc,      // <<
    HereDocStrip, // <<- (strip tabs)
    HereString,   // <<<
    DupOutput,    // >&
    DupInput,     // <&
    OutputBoth,   // &>
}
```

Redirects can appear:
- After simple command args: `echo hello > file`
- After compound commands: `if true; fi > file`
- Multiple on same line: `cat <<EOF > file 2>&1`
- With fd variables: `exec {fd}>file` (`fd_var` stores the compact name and `fd_var_span` points at the exact identifier)

#### Word Expansion Parsing

The lexer produces word tokens with raw expansion syntax. The parser's `parse_word_with_context()` method then parses expansions into `WordPart` variants:

```rust
pub struct WordPartNode {
    pub kind: WordPart,
    pub span: Span,
}

pub struct Word {
    pub parts: Vec<WordPartNode>,  // Literal, quoted fragments, expansions, etc.
    pub span: Span,
}

pub enum CommandSubstitutionSyntax {
    DollarParen,
    Backtick,
}

pub enum ArithmeticExpansionSyntax {
    DollarParenParen,
    LegacyBracket,
}

pub enum WordPart {
    Literal(LiteralText),
    SingleQuoted { value: SourceText, dollar: bool },
    DoubleQuoted { parts: Vec<WordPartNode>, dollar: bool },
    Variable(Name),                             // $VAR, ${VAR}
    CommandSubstitution {
        commands: Vec<Command>,
        syntax: CommandSubstitutionSyntax,
    },
    ArithmeticExpansion {
        expression: SourceText,
        syntax: ArithmeticExpansionSyntax,
    },
    ParameterExpansion { name, operator, operand, colon_variant },
    Length(Name),                               // ${#var}
    ArrayAccess { name, index },                // ${arr[idx]}, ${arr[@]}
    ArrayLength(Name),                          // ${#arr[@]}
    ArrayIndices(Name),                         // ${!arr[@]}
    Substring { name, offset, length },         // ${var:offset:len}
    ArraySlice { name, offset, length },        // ${arr[@]:offset:len}
    IndirectExpansion { name, operator, operand, colon_variant },
    PrefixMatch(Name),                          // ${!prefix*}
    ProcessSubstitution { commands, is_input }, // <(cmd), >(cmd)
    Transformation { name, operator },          // ${var@Q}, ${var@E}, etc.
}

pub enum ParameterOp {
    UseDefault,                                  // :-
    AssignDefault,                               // :=
    UseReplacement,                              // :+
    Error,                                       // :?
    RemovePrefixShort, RemovePrefixLong,         // # ##
    RemoveSuffixShort, RemoveSuffixLong,         // % %%
    ReplaceFirst { pattern, replacement },       // /pat/repl
    ReplaceAll { pattern, replacement },         // //pat/repl
    UpperFirst, UpperAll, LowerFirst, LowerAll,
}
```

Identifier-like fields in `WordPart` use `Name`, a compact owned string type backed by `compact_str::CompactString`. Source-backed text uses `LiteralText` or `SourceText` so ordinary parsing can preserve original slices:

- `Name`: variable names, parameter names, loop/function/coproc names, fd-variable redirect names
- `LiteralText`: unquoted literal fragments
- `SourceText`: quoted bodies, parameter operands, array indices and slices, arithmetic-expansion text

**Algorithm:** Character-by-character iteration:
1. Collect literal characters
2. On `$`, dispatch to expansion-specific parser
3. Read expansion until terminator (`}`, `))`, `[`, etc.)
4. For command substitutions, recursively parse nested commands
5. Build a `WordPartNode` and add it to the word's parts list

### Abstract Syntax Tree

All types defined in the `shuck-ast` crate (`crates/shuck-ast/src/ast.rs`).

#### Script & Commands

```rust
pub struct Script {
    pub commands: Vec<Command>,
    pub span: Span,
}

pub enum Command {
    Simple(SimpleCommand),
    Builtin(BuiltinCommand),
    Pipeline(Pipeline),
    List(CommandList),
    Compound(CompoundCommand, Vec<Redirect>),
    Function(FunctionDef),
}

pub struct SimpleCommand {
    pub name: Word,
    pub args: Vec<Word>,
    pub redirects: Vec<Redirect>,
    pub assignments: Vec<Assignment>,
    pub span: Span,
}

pub struct Pipeline {
    pub negated: bool,              // ! prefix
    pub commands: Vec<Command>,
    pub span: Span,
}

pub struct CommandList {
    pub first: Box<Command>,
    pub rest: Vec<(ListOperator, Command)>,
    pub span: Span,
}

pub enum ListOperator {
    And,        // &&
    Or,         // ||
    Semicolon,  // ;
    Background, // &
}

pub enum BuiltinCommand {
    Break(BreakCommand),
    Continue(ContinueCommand),
    Return(ReturnCommand),
    Exit(ExitCommand),
}
```

#### Compound Commands

```rust
pub enum CompoundCommand {
    If(IfCommand),
    For(ForCommand),
    ArithmeticFor(ArithmeticForCommand),
    While(WhileCommand),
    Until(UntilCommand),
    Case(CaseCommand),
    Select(SelectCommand),
    Subshell(Vec<Command>),
    BraceGroup(Vec<Command>),
    Arithmetic(ArithmeticCommand),
    Time(TimeCommand),
    Conditional(ConditionalCommand),
    Coproc(CoprocCommand),
}
```

Selected compound node shapes:

```rust
pub struct ForCommand {
    pub variable: Name,
    pub variable_span: Span,
    pub words: Option<Vec<Word>>,
    pub body: Vec<Command>,
    pub span: Span,
}

pub struct ArithmeticCommand {
    pub span: Span,
    pub left_paren_span: Span,
    pub expr_span: Option<Span>,
    pub right_paren_span: Span,
}

pub struct ArithmeticForCommand {
    pub left_paren_span: Span,
    pub init_span: Option<Span>,
    pub first_semicolon_span: Span,
    pub condition_span: Option<Span>,
    pub second_semicolon_span: Span,
    pub step_span: Option<Span>,
    pub right_paren_span: Span,
    pub body: Vec<Command>,
    pub span: Span,
}

pub struct ConditionalCommand {
    pub expression: ConditionalExpr,
    pub span: Span,
    pub left_bracket_span: Span,
    pub right_bracket_span: Span,
}

pub enum ConditionalExpr {
    Binary(ConditionalBinaryExpr),
    Unary(ConditionalUnaryExpr),
    Parenthesized(ConditionalParenExpr),
    Word(Word),
    Pattern(Word),
    Regex(Word),
}

pub struct FunctionDef {
    pub name: Name,
    pub name_span: Span,
    pub body: Box<Command>,
    pub span: Span,
}
```

Arithmetic commands and arithmetic `for` headers are now source-backed rather than string-backed. The parser preserves exact spans for `((`, `))`, both semicolons, and each arithmetic region. Later lowering can slice the original source text without rebuilding strings.

#### Variables & Assignments

```rust
pub struct Assignment {
    pub name: Name,
    pub name_span: Span,
    pub index: Option<String>,     // For arr[idx]=val
    pub index_span: Option<Span>,
    pub value: AssignmentValue,
    pub append: bool,              // += vs =
    pub span: Span,
}

pub enum AssignmentValue {
    Scalar(Word),
    Array(Vec<Word>),
}
```

### Position & Span Tracking

All AST nodes include source location for error reporting.

```rust
pub struct Position {
    pub line: usize,       // 1-based
    pub column: usize,     // 1-based (byte offset within line)
    pub offset: usize,     // 0-based byte offset from start
}

pub struct Span {
    pub start: Position,   // Inclusive
    pub end: Position,     // Exclusive
}
```

- Lexer maintains `Position` while reading each character
- `advance(ch)` increments offset by UTF-8 len; line/col by character (newline resets col to 1)
- Parser records `Span` for each token and merges spans as it builds nodes
- `Span::slice(&self, source: &str) -> &str` is the canonical way to recover exact source text for span-backed nodes like arithmetic commands and identifier subspans
- Nested command substitutions are parsed with relative positions, then rebased to absolute via `Position::rebased(base)` and `Span::rebased(base)`

### Error Handling

```rust
pub enum Error {
    Parse {
        message: String,
        line: usize,       // 0 = no location
        column: usize,     // 0 = no location
    },
}
```

#### Strict vs. Recovery Mode

**Strict Mode** (`parse() -> Result<Script>`):
- Returns error on first syntax error
- Used when only valid scripts are acceptable

**Recovery Mode** (`parse_recovered() -> RecoveredParse`):
- Collects parse diagnostics and continues
- Returns partial script + list of errors
- Used by shuck-syntax for IDE-style error reporting

```rust
pub struct RecoveredParse {
    pub script: Script,
    pub diagnostics: Vec<ParseDiagnostic>,
}

pub struct ParseDiagnostic {
    pub message: String,
    pub span: Span,
}
```

**Recovery algorithm:** On error, emit diagnostic, scan forward to a command boundary (`Newline`, `Semicolon`, `Background`, `And`, `Or`, `Pipe`, `DoubleSemicolon`, `SemiAmp`, `DoubleSemiAmp`), and resume parsing from that point.

## Alternatives Considered

### LALR(1) Parser Generator

Bash grammar is not context-free due to keyword context sensitivity (e.g., `[` behaves as test command vs. glob depending on context). Hand-written recursive descent allows flexible lookahead and context awareness without the complexity and reduced readability of parser generators.

### Single-Pass Token Stream (No Lookahead)

Lookahead is essential for distinguishing function definitions from arithmetic (`name() { ... }` vs `(( expr ))`), recognizing reserved words only in command position, and tracking brace expansion vs. brace group in the lexer. A lexer with one-token lookahead is simpler than complex lookahead in the parser.

### Unbounded Recursion Depth

Stack overflow is a real risk on resource-limited systems. A hard cap of 100 levels is conservative (bash allows much deeper) but protects against intentional DoS. Configurable limits allow per-use-case tuning.

### No Error Recovery

Tools like IDEs and analyzers benefit from partial parsing. Recovery mode allows reporting multiple errors and building a partial AST for continued analysis -- a better user experience than stopping on the first error.

## Security Considerations

| Threat | Defense |
|--------|---------|
| Deeply nested substitutions `$($($(..))` | Lexer `max_subst_depth`, default 50 |
| Deeply nested syntax `((((((())))))` | Parser `max_depth`, hard capped at 100 |
| Infinite loops in parsing | Fuel system, max 100k operations |
| Stack overflow from recursion | Depth + fuel limits prevent unbounded recursion |

The parser performs **no code execution**, no variable expansion side effects, and no disk I/O. It is safe to parse untrusted input.

## Verification

### Unit Tests

```bash
cargo test -p shuck-parser
```

Key test cases:
- `test_parse_simple_command` -- Basic command parsing
- `test_parse_variable` -- Variable in WordPart
- `test_parse_pipeline` -- Pipe and negation
- `test_parse_conditional_builds_structured_logical_ast` -- Structured `[[ ... ]]`
- `test_parse_arithmetic_command_preserves_exact_spans` -- `(( ... ))` source fidelity
- `test_parse_arithmetic_for_preserves_header_spans` -- `for (( ... ; ... ; ... ))` source fidelity
- `test_identifier_spans_track_function_loop_assignment_and_fd_var_names` -- Exact identifier spans
- `test_parse_recovered_skips_invalid_command_and_continues` -- Recovery mode
- `test_unexpected_top_level_token_errors_in_strict_mode` -- Strict mode errors

### Fuzz Testing

Targets now live under the unified top-level fuzz package at `fuzz/fuzz_targets/`:
- `parser_fuzz.rs` -- General robustness (never panics, handles all input)
- `lexer_fuzz.rs` -- Lexer tokenization
- `arithmetic_fuzz.rs` -- Arithmetic expression parsing
- `glob_fuzz.rs` -- Glob pattern parsing

```bash
cd fuzz && cargo +nightly fuzz run parser_fuzz -- -max_total_time=300
```

Verifies no panics on arbitrary input, no stack overflows (depth/fuel limits enforced), no infinite loops (timeouts catch hangs), and graceful error handling.

### Manual Verification

Test complex constructs parse correctly:

```bash
# Nested structures
{ for x in 1 2 3; do if [ $x = 2 ]; then echo two; fi; done; }

# Expansions
echo ${HOME}/bin/${USER:-nobody}

# Heredocs with tab stripping
cat <<-EOF
	indented
EOF

# Process substitution
diff <(echo a) <(echo b)

# Deep nesting stress test (should fail gracefully, not panic)
echo $((((($(echo x))))))
```
