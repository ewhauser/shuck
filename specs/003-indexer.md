# 003: Shell Script Indexer

## Status

Proposed

## Summary

A new `shuck-indexer` library crate that pre-computes positional and structural metadata from a parsed shell script, enabling lint rules to answer contextual questions cheaply without re-walking the AST. Inspired by ruff's `ruff_python_index::Indexer`, this crate bridges the gap between raw parsing output (AST + comments) and the queries lint rules actually need — "is this offset inside a heredoc?", "where are the comments?", "is this variable reference inside double quotes?".

The indexer tracks **raw positional data only** — comment ranges, syntactic region ranges, line offsets, and continuation lines. Suppression directive parsing (interpreting comment content as disable/enable directives) and semantic indexing (variable scoping, use-def chains, call graph analysis) are explicitly out of scope — those belong in higher-level linter modules that consume the indexer's output.

## Motivation

Today, `shuck check` only reports parse errors. To support lint rules, we need infrastructure that lets rules query positional context efficiently. Without an indexer, every rule would need to walk the AST independently to answer basic questions like:

- **Is this position inside a heredoc, single-quoted string, or double-quoted string?** Rules that flag unquoted variable expansions need to know quoting context. Rules that check for unused variables need to skip heredoc bodies.
- **Where are the comments?** Higher-level modules (suppression parsing, documentation extraction) need efficient access to comment byte ranges and line numbers without re-scanning the source.
- **What's the line offset table?** Converting between byte offsets and line:column positions must be O(1) for diagnostic reporting and suppression matching.
- **Is this line a continuation line?** Shell line continuations (`\` before newline) affect how rules reason about logical vs physical lines.
- **What type of node is at this position?** Some rules need to quickly check if a byte offset falls inside a specific syntactic region (heredoc, command substitution, arithmetic expression, etc.) without walking the tree each time.

Ruff solves the analogous problem for Python with `ruff_python_index::Indexer`, which pre-computes continuation lines, multiline string ranges, f-string ranges, and comment ranges from a single token-stream pass. Go shuck solves it with `AstIndex` (categorized node collections) and `FileFacts` (line offsets, comments, suppressions). We need the Rust equivalent, tailored to shell-specific constructs.

## Design

### Crate Structure

```
crates/shuck-indexer/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API: Indexer struct + construction
    ├── line_index.rs        # Line offset table
    ├── comment_index.rs     # Comment ranges and position metadata
    └── region_index.rs      # Quoted/heredoc/expansion region tracking
```

**Dependencies:** `shuck-ast` (AST types, spans), `shuck-parser` (for `ParseOutput`)

The crate does **not** depend on `serde` or `serde_json` — it is a pure in-memory index with no serialization.

### Core Type

```rust
/// Pre-computed positional and structural index over a parsed shell script.
pub struct Indexer {
    /// Byte offset of the start of each line (0-indexed line number → byte offset).
    line_index: LineIndex,

    /// Comment ranges and position metadata.
    comment_index: CommentIndex,

    /// Byte ranges of syntactic regions where special rules apply.
    region_index: RegionIndex,

    /// Byte offsets of continuation line starts (lines preceded by `\<newline>`).
    continuation_lines: Vec<TextSize>,
}
```

### Construction

```rust
impl Indexer {
    /// Build an index from parser output and the original source text.
    pub fn new(source: &str, output: &ParseOutput) -> Self
}
```

Construction performs two passes:

1. **Source scan** — Linear scan of the source bytes to build the line offset table and detect continuation lines (`\` immediately before `\n`). This is O(n) in source length and does not require the AST.

2. **AST walk** — Single recursive walk of the AST to collect region ranges (heredocs, quoted strings, command substitutions, arithmetic expressions) and classify comments. This is O(n) in AST node count.

Both passes are single-threaded. For typical shell scripts (< 10K lines), construction should be sub-millisecond.

### LineIndex

Maps between byte offsets and line numbers.

```rust
pub struct LineIndex {
    /// Byte offset of the first character of each line.
    /// `line_starts[0]` is always 0.
    line_starts: Vec<TextSize>,
}

impl LineIndex {
    /// Build from source text.
    pub fn new(source: &str) -> Self;

    /// Return the 1-based line number containing `offset`.
    pub fn line_number(&self, offset: TextSize) -> usize;

    /// Return the byte offset of the start of the given 1-based line.
    pub fn line_start(&self, line: usize) -> Option<TextSize>;

    /// Return the byte range of the given 1-based line (excluding newline).
    pub fn line_range(&self, line: usize, source: &str) -> Option<TextRange>;

    /// Return the total number of lines.
    pub fn line_count(&self) -> usize;
}
```

Uses binary search over `line_starts` for O(log n) offset-to-line lookups. Construction is a single linear scan for `\n` bytes.

### CommentIndex

Stores comment byte ranges with position metadata. The indexer does **not** interpret comment content (e.g., parsing suppression directives) — that responsibility belongs to higher-level linter modules that consume `CommentIndex` as input, mirroring how ruff's `Indexer` provides `comment_ranges()` and the linter's `noqa.rs`/`suppression.rs` modules interpret them.

```rust
pub struct CommentIndex {
    /// All comments, sorted by start offset.
    comments: Vec<IndexedComment>,
}

pub struct IndexedComment {
    /// Byte range of the comment in source (including the `#`).
    pub range: TextRange,

    /// The 1-based line number this comment appears on.
    pub line: usize,

    /// Whether this comment is the only non-whitespace content on its line.
    pub is_own_line: bool,
}
```

Construction: takes the `Vec<Comment>` from `ParseOutput`, resolves each to a line number via `LineIndex`, and checks if it's own-line by examining surrounding source bytes.

**Lookup methods:**

```rust
impl CommentIndex {
    /// All comments in source order.
    pub fn comments(&self) -> &[IndexedComment];

    /// Comments on a specific 1-based line.
    pub fn comments_on_line(&self, line: usize) -> &[IndexedComment];

    /// Whether the given byte offset falls inside a comment.
    pub fn is_comment(&self, offset: TextSize) -> bool;
}
```

The `is_own_line` field is particularly useful for downstream consumers: suppression directives on their own line typically apply to the *next* line, while inline comments apply to the *same* line. But that logic belongs to the suppression parser, not the indexer.

### RegionIndex

Tracks byte ranges of syntactic regions where lint rules need special behavior. Shell scripts have several region types that affect how rules interpret code.

```rust
pub struct RegionIndex {
    /// Ranges of single-quoted strings (no expansion, literal content).
    single_quoted: Vec<TextRange>,

    /// Ranges of double-quoted strings (expansions active).
    double_quoted: Vec<TextRange>,

    /// Ranges of heredoc bodies (content between `<<EOF` and `EOF`).
    heredocs: Vec<TextRange>,

    /// Ranges of command substitutions `$(...)` (nested commands).
    command_substitutions: Vec<TextRange>,

    /// Ranges of arithmetic expressions `$((...))` and `((...))`.
    arithmetic: Vec<TextRange>,

    /// Ranges of conditional expressions `[[ ... ]]`.
    conditionals: Vec<TextRange>,
}
```

All vectors are sorted by start offset, enabling binary search for point-in-range queries.

**Why these regions matter for shell linting:**

| Region | Linting Impact |
|--------|---------------|
| Single-quoted | No variable expansion — skip unquoted-variable rules |
| Double-quoted | Expansions active but word splitting suppressed — different quoting rules apply |
| Heredoc body | Variable expansion depends on delimiter quoting; indentation rules differ |
| Command substitution | Nested command context — rules may recurse or skip |
| Arithmetic | Variables don't need `$` prefix; different operator semantics |
| Conditional | Pattern matching context for `=~` and `==`; different quoting rules |

**Lookup methods:**

```rust
impl RegionIndex {
    /// Return the region kind containing the given byte offset, if any.
    /// When offsets are nested (e.g., a variable inside a double-quoted
    /// heredoc), returns the innermost region.
    pub fn region_at(&self, offset: TextSize) -> Option<RegionKind>;

    /// Check if a byte offset falls inside any quoted region
    /// (single-quoted, double-quoted, or heredoc with quoted delimiter).
    pub fn is_quoted(&self, offset: TextSize) -> bool;

    /// Check if a byte offset falls inside a heredoc body.
    pub fn is_heredoc(&self, offset: TextSize) -> bool;

    /// Check if a byte offset falls inside a command substitution.
    pub fn is_command_substitution(&self, offset: TextSize) -> bool;

    /// Check if a byte offset falls inside an arithmetic context.
    pub fn is_arithmetic(&self, offset: TextSize) -> bool;

    /// All heredoc body ranges.
    pub fn heredoc_ranges(&self) -> &[TextRange];
}

pub enum RegionKind {
    SingleQuoted,
    DoubleQuoted,
    Heredoc,
    CommandSubstitution,
    Arithmetic,
    Conditional,
}
```

**Nesting:** Regions can nest (e.g., a command substitution inside a double-quoted string inside a heredoc). The `region_at` method returns the innermost region. For rules that need to understand the full nesting stack, a `region_stack_at(offset) -> Vec<RegionKind>` method could be added later, but the common case is checking the innermost context.

### Continuation Lines

```rust
impl Indexer {
    /// Byte offsets of the start of each continuation line
    /// (a line whose preceding line ends with `\`).
    pub fn continuation_line_starts(&self) -> &[TextSize];

    /// Whether the given byte offset is on a continuation line.
    pub fn is_continuation(&self, offset: TextSize) -> bool;
}
```

Continuation lines affect how rules reason about logical statements spanning multiple physical lines. For example, a rule checking command length should count the logical line, not each physical line.

### Top-Level API

```rust
impl Indexer {
    pub fn new(source: &str, output: &ParseOutput) -> Self;

    pub fn line_index(&self) -> &LineIndex;
    pub fn comment_index(&self) -> &CommentIndex;
    pub fn region_index(&self) -> &RegionIndex;
    pub fn continuation_line_starts(&self) -> &[TextSize];
    pub fn is_continuation(&self, offset: TextSize) -> bool;
}
```

### Integration Point

The indexer sits between parsing and rule execution in the linting pipeline:

```
Source text
  → shuck-parser: parse() → ParseOutput { script, comments }
  → shuck-indexer: Indexer::new(source, &output) → Indexer
  → Suppression parsing (future linter module): consumes Indexer.comment_index()
  → Rule execution: each rule receives &Script, &Indexer, &str (source)
```

The `Indexer` is constructed once per file and shared immutably across all rules and the suppression layer. It does not own the source text or the AST — it borrows from `ParseOutput` during construction and stores only derived positional data (byte offsets, ranges, line numbers).

## Alternatives Considered

### Embed indexing in a syntax facade

A linter-oriented syntax facade could own dialect/profile management, parse-view
selection, and positional indexing. Rejected because the indexer is a distinct
concern — positional metadata over a parsed AST — and separating it keeps both
crates focused. The indexer has no opinion about dialects or parse modes; it
works on any parser output.

### Build indexes lazily on first query

Instead of pre-computing all indexes in `new()`, we could use `OnceCell` to build each sub-index on first access. Rejected because: (a) construction is cheap (two linear passes), (b) lazy initialization adds complexity and makes performance less predictable, and (c) every lint run will use most sub-indexes anyway (line numbers are always needed for diagnostics, region checks are needed for most rules).

### Store AST node references in the index (like Go shuck's AstIndex)

Go shuck's `AstIndex` stores categorized collections of AST node pointers (`[]*syntax.CallExpr`, `[]*syntax.Assign`, etc.), enabling rules to iterate over specific node types without walking the tree. We could do the same with `Vec<&'a SimpleCommand>`, etc. Rejected because: (a) it ties the index lifetime to the AST borrow, complicating ownership, (b) Rust's recursive AST walking with pattern matching is already ergonomic and fast, (c) the Go approach compensates for Go's lack of pattern matching — Rust `match` on `Command` variants is equivalent, and (d) a future visitor/walker trait is a better fit for node-type iteration than pre-sorted buckets.

### Use the token stream instead of (or in addition to) the AST

Ruff's `Indexer` works from the token stream because Python's tokens carry enough information (string delimiters, comment tokens, etc.). We could build a token-stream-based indexer. Rejected because: shell tokenization is context-sensitive (the same `{` can be a brace group or brace expansion), so the token stream alone doesn't reliably classify regions. The AST, which has already resolved these ambiguities, is the right input. However, the source-scan pass (for line offsets and continuations) operates on raw bytes and doesn't need either tokens or the AST.

### Include suppression directive parsing in the indexer

We could parse comment content into suppression directives (inline disables, file-level disables, shellcheck-compatible directives) as part of the indexer. Rejected because: ruff's architecture demonstrates a clean separation — the `Indexer` provides raw comment ranges and the linter layer (`noqa.rs`, `suppression.rs`) interprets them. This keeps the indexer focused on positional data and avoids coupling it to suppression format details, which may evolve independently (new directive formats, different scoping rules). A future linter module will consume `CommentIndex` to build a `SuppressionIndex`.

### Include statement flow context (loop depth, function scope, subshell)

Go shuck's `StatementFlowIndex` tracks per-statement context: whether it's inside a loop, function, subshell, or block. This is useful for rules like "don't use `exit` inside a function" or "break outside a loop." We could include this. Deferred because: this is closer to semantic analysis than positional indexing. A future `shuck-analyzer` or semantic layer can compute flow context by walking the AST with a scope stack. Keeping it out of the indexer preserves the syntactic/positional focus.

## Verification

### Unit Tests

```bash
cargo test -p shuck-indexer
```

#### LineIndex tests

1. **Empty source** — `line_count() == 1`, `line_number(0) == 1`
2. **Single line** — `line_number(0) == 1`, `line_number(last) == 1`
3. **Multiple lines** — correct line numbers at newline boundaries
4. **Line range extraction** — `line_range(n)` returns correct byte ranges
5. **Unicode** — byte offsets handle multi-byte characters correctly

#### CommentIndex tests

6. **Own-line vs inline** — `# comment` alone on a line vs `echo hi # comment`
7. **Shebang** — `#!/bin/bash` included in comment list with correct range
8. **is_comment** — byte offset inside `# comment` returns true, offset in code returns false
9. **comments_on_line** — returns correct comments for a given line number
10. **Comments inside command substitutions** — correctly indexed with rebased positions

#### RegionIndex tests

11. **Single-quoted region** — offset inside `'hello'` → `SingleQuoted`
12. **Double-quoted region** — offset inside `"hello $x"` → `DoubleQuoted`
13. **Heredoc body** — offset inside heredoc content → `Heredoc`
14. **Command substitution** — offset inside `$(cmd)` → `CommandSubstitution`
15. **Arithmetic** — offset inside `$(( 1 + 2 ))` → `Arithmetic`
16. **Nested regions** — variable inside double-quoted string returns `DoubleQuoted` (innermost)
17. **Outside all regions** — bare command word returns `None`

#### Continuation line tests

18. **Backslash continuation** — line after `\<newline>` detected
19. **Backslash inside quotes** — `"foo\<newline>bar"` is NOT a continuation (it's inside a string)
20. **No continuations** — simple script returns empty continuation list

### Integration Test

21. **Round-trip with parse pipeline** — Parse a non-trivial script with `shuck-parser`, build an `Indexer`, verify that comment ranges, region ranges, and line numbers are all consistent and correct. This validates the full pipeline from source → parse → index.
