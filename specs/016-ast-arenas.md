# 016: AST Arenas

## Status

Partially Implemented

## Summary

Migrate Shuck's parsed shell syntax from a recursive `Vec`/`Box` tree to an owned, ID-backed `AstStore` while preserving the existing parser result surface during the transition. This extends the arena direction from [015-indexed-arenas.md](015-indexed-arenas.md): the fact arena work has packed high-churn linter data, and the next milestone attacks parser and AST allocation by giving parsed files stable node IDs, contiguous child storage, and view adapters for existing consumers.

## Motivation

The current `shuck_ast::File` is direct and pleasant to pattern-match, but it allocates throughout the syntax tree. `StmtSeq`, `Stmt`, command variants, words, word parts, heredoc bodies, patterns, arithmetic expressions, and zsh extras all own nested `Vec` or `Box` values. That shape has three costs:

- Parsing large scripts creates many small heap allocations.
- Downstream crates identify syntax by borrowed node addresses or repeated traversal, rather than by stable IDs.
- Derived stores such as linter facts can be compact, but still need to point back to borrowed AST nodes until the AST itself has stable identity.

The goal is not to make every AST consumer switch in one patch. The goal is to introduce the store and IDs in a way that lets parser, indexer, semantic analysis, lint facts, and formatter move independently without changing observable diagnostics or formatted output.

## Implementation Status

The first implementation pass adds an `ArenaFile` sidecar to `ParseResult`, backed by `AstStore` and borrowed view types in `shuck-ast`. The parser now moves the parsed root `StmtSeq` into `ArenaFile::from_body` after comment attachment and materializes the legacy `File` from that arena for compatibility. `shuck check` builds its index and lint diagnostics through arena-aware entry points. The indexer, semantic model, and linter have arena entry points that materialize the current recursive compatibility shape internally while downstream logic migrates.

The arena is now beginning to replace command payloads with native data rather than only carrying graph links plus a legacy escape hatch. Simple commands and typed builtins (`break`, `continue`, `return`, and `exit`) have arena-native payloads and borrowed views. Compatibility materialization for those command families is rebuilt from the arena payloads; other command families still materialize from the legacy command stored on `CommandNode`.

Still remaining:

- pushing arena construction deeper into parser command/word builders instead of moving the completed root body at the end
- migrating the remaining command families off `CommandNode::legacy`
- replacing linter fact references with stable AST IDs
- native arena traversal in semantic analysis instead of compatibility materialization
- formatter-native arena support, intentionally deferred while the formatter is being reworked

## Design

### Goals

- Represent parsed syntax in contiguous typed stores owned by a parsed file.
- Use `Idx<T>` and `IdRange<T>` from `shuck-ast` for stable node identity and variable-length children.
- Preserve source-backed text via spans and existing `SourceText`/`LiteralText` semantics.
- Keep the initial public parser API source-compatible enough that consumers can migrate incrementally.
- Give linter facts and semantic analysis stable AST IDs to replace borrowed AST references over time.
- Keep formatter mutation possible during the transition.

### Non-Goals

- Do not introduce a global bump arena or lifetime-threaded AST.
- Do not require all AST consumers to become ID-native in the first patch.
- Do not change shell parsing behavior, diagnostics, lint output, or formatter policy.
- Do not intern every string immediately. String interning can be evaluated after the tree shape is compact.
- Do not make the parser depend on linter or semantic crates.

### Store Shape

`shuck-ast` gains an ID-backed parsed file representation next to the recursive tree:

```rust
pub type FileId = Idx<FileNode>;
pub type StmtSeqId = Idx<StmtSeqNode>;
pub type StmtId = Idx<StmtNode>;
pub type CommandId = Idx<CommandNode>;
pub type WordId = Idx<WordNode>;
pub type WordPartId = Idx<WordPartNodeData>;
pub type RedirectId = Idx<Redirect>;
pub type AssignmentId = Idx<AssignmentNode>;
pub type PatternId = Idx<PatternNode>;
pub type PatternPartId = Idx<PatternPartNodeData>;
pub type ArithmeticExprId = Idx<ArithmeticExprNodeData>;
pub type ConditionalExprId = Idx<ConditionalExprNodeData>;
pub type HeredocBodyId = Idx<HeredocBodyNode>;
pub type CommentId = Idx<Comment>;

pub struct ArenaFile {
    pub root: FileId,
    pub store: AstStore,
}

pub struct AstStore {
    files: Vec<FileNode>,
    stmt_seqs: Vec<StmtSeqNode>,
    stmts: Vec<StmtNode>,
    commands: Vec<CommandNode>,
    words: Vec<WordNode>,
    word_parts: Vec<WordPartNodeData>,
    redirects: Vec<Redirect>,
    assignments: Vec<AssignmentNode>,
    patterns: Vec<PatternNode>,
    pattern_parts: Vec<PatternPartNodeData>,
    arithmetic_exprs: Vec<ArithmeticExprNodeData>,
    conditional_exprs: Vec<ConditionalExprNodeData>,
    heredoc_bodies: Vec<HeredocBodyNode>,
    comments: Vec<Comment>,
    stmt_id_lists: ListArena<StmtId>,
    comment_id_lists: ListArena<CommentId>,
    word_id_lists: ListArena<WordId>,
    word_part_id_lists: ListArena<WordPartId>,
    redirect_lists: ListArena<Redirect>,
    assignment_lists: ListArena<AssignmentId>,
    pattern_id_lists: ListArena<PatternId>,
    pattern_part_id_lists: ListArena<PatternPartId>,
}
```

Node structs store scalar fields directly and child relationships as typed IDs or ranges:

```rust
pub struct FileNode {
    pub body: StmtSeqId,
    pub span: Span,
}

pub struct StmtSeqNode {
    pub leading_comments: IdRange<CommentId>,
    pub stmts: IdRange<StmtId>,
    pub trailing_comments: IdRange<CommentId>,
    pub span: Span,
}

pub struct StmtNode {
    pub leading_comments: IdRange<CommentId>,
    pub command: CommandId,
    pub negated: bool,
    pub redirects: IdRange<Redirect>,
    pub terminator: Option<StmtTerminator>,
    pub terminator_span: Option<Span>,
    pub inline_comment: Option<CommentId>,
    pub span: Span,
}

pub struct WordNode {
    pub parts: IdRange<WordPartId>,
    pub span: Span,
    pub brace_syntax: IdRange<BraceSyntax>,
}
```

The exact internal enum names can differ from the sketch, but the ownership rule should hold: parent nodes own child IDs/ranges, not child `Vec` or `Box` allocations. Ranges over syntax children should generally point into list arenas of child IDs, not assume the child node arrays are physically contiguous. Ranges over plain payloads such as statement-local redirects can point directly into a payload `ListArena<T>`.

### Parser Result During Migration

`ParseResult` should become a dual representation before consumers are forced to switch:

```rust
pub struct ParseResult {
    pub file: File,
    pub arena_file: Option<ArenaFile>,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub status: ParseStatus,
    pub terminal_error: Option<Error>,
    pub syntax_facts: SyntaxFacts,
}
```

The first milestone may build `arena_file` by lowering the recursive `File` after parsing. That does not save parser allocations yet, but it validates node coverage, ID stability, and adapter APIs with low parser risk. Once adapters and consumers are ready, the parser can write into `AstStore` directly and materialize `file` only for compatibility callers.

The end state removes duplicate construction from hot paths:

```rust
pub struct ParseResult {
    pub file: ParsedFile,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub status: ParseStatus,
    pub terminal_error: Option<Error>,
    pub syntax_facts: SyntaxFacts,
}

pub struct ParsedFile {
    pub ast: ArenaFile,
    pub legacy: Option<File>,
}
```

The final shape can be adjusted, but there should be a period where both forms exist so parser tests and downstream crates can move intentionally.

### Views and Compatibility

`AstStore` exposes borrowed views that are cheap to copy and preserve most read-only traversal ergonomics:

```rust
pub struct AstView<'a> {
    store: &'a AstStore,
}

pub struct StmtView<'a> {
    store: &'a AstStore,
    id: StmtId,
}

impl AstStore {
    pub fn file(&self, id: FileId) -> FileView<'_>;
    pub fn stmt_seq(&self, id: StmtSeqId) -> StmtSeqView<'_>;
    pub fn stmt(&self, id: StmtId) -> StmtView<'_>;
    pub fn command(&self, id: CommandId) -> CommandView<'_>;
    pub fn word(&self, id: WordId) -> WordView<'_>;
}
```

View methods should return IDs for identity-sensitive callers and iterators for traversal:

```rust
impl<'a> StmtSeqView<'a> {
    pub fn id(&self) -> StmtSeqId;
    pub fn stmts(&self) -> impl ExactSizeIterator<Item = StmtView<'a>>;
    pub fn stmt_ids(&self) -> &'a [StmtId];
    pub fn span(&self) -> Span;
}
```

The first consumer migrations should prefer views over exposing raw store vectors. That keeps the storage compact but avoids tying every caller to the exact field layout.

### Construction Strategy

The migration should land in four phases.

#### Phase 1: Lower Recursive AST Into `AstStore`

Add arena node types, a lowering pass from `File` to `ArenaFile`, and tests that compare traversal output between the recursive tree and arena views. Keep `ParseResult::file` authoritative.

This phase proves the representation can cover the existing AST, including comments, zsh syntax extras, command substitutions, heredocs, arithmetic expressions, conditional expressions, and pattern groups.

#### Phase 2: Make Read-Only Consumers ID-Capable

Move `shuck-indexer`, `shuck-semantic`, and linter fact construction onto `AstStore` views behind opt-in entry points:

```rust
impl Indexer {
    pub fn new_arena(source: &str, ast: &ArenaFile, syntax_facts: &SyntaxFacts) -> Self;
}

impl SemanticModel {
    pub fn build_arena(ast: &ArenaFile, source: &str, indexer: &Indexer) -> Self;
}
```

Existing `new` and `build` methods remain until the arena path has equivalent coverage. Linter facts can then replace borrowed AST node references with typed IDs one fact family at a time.

#### Phase 3: Parse Directly Into `AstStore`

Change parser builders to allocate nodes and child lists into `AstStore` as they parse. The parser may still materialize the recursive `File` for compatibility, but that should become an explicit compatibility cost rather than the primary representation.

This phase should start with statement sequences, statements, commands, redirects, and words before deeper expression families. Command substitutions are a useful boundary test because they currently construct nested `StmtSeq` values and rebase/materialize source backing.

#### Phase 4: Retire Hot-Path Recursive AST Use

Once indexer, semantic, linter, and formatter read paths are arena-native, stop materializing `ParseResult::file` in `shuck check` hot paths. Keep test helpers or compatibility constructors only where needed.

Formatter mutation should be handled deliberately before removing the recursive tree. The likely path is to keep the formatter operating on views plus a separate edit/simplification plan, rather than mutating arena nodes in place as a general-purpose AST rewrite API.

### Stable Node Identity

IDs are stable only within one parsed file. They are not cache keys across parse runs and should not be serialized as durable external identifiers. Diagnostics continue to use spans as their public source identity.

Facts and semantic records may store IDs such as `StmtId`, `CommandId`, and `WordId` to avoid borrowed AST lifetimes. When a fact needs a source location, it should store the span or recover it from the node view.

### Source Text

Source-backed text remains span-based. `SourceText`, `LiteralText`, and related cooked-text wrappers remain valid node payloads, but arena conversion should avoid creating owned strings unless the recursive parser already does so for cooked or synthetic values.

Direct parser-to-arena construction should preserve the existing source materialization behavior for nested parses and command substitutions. A command substitution parsed from a source fragment must still be able to report spans in the outer file's coordinate space.

### Formatter Considerations

The formatter is the riskiest consumer because it currently formats read-only ASTs but also has simplification code that mutates `StmtSeq`, `Stmt`, words, and redirects. The arena migration should not force a broad mutable arena API into `shuck-ast` just to preserve current simplification internals.

Preferred direction:

- Keep initial formatter support read-only through views.
- Convert simplification into a formatter-local rewrite/edit plan where practical.
- If mutable arena access is needed, keep it narrow and local to specific node families.
- Preserve `format_source(source, path, options)` and `format_file_ast(source, file, path, options)` until callers have an arena alternative.

### Testing Strategy

The main correctness risk is a silent mismatch between recursive and arena traversal. Phase 1 should add structural equivalence tests that parse representative fixtures and compare:

- statement sequence length, spans, comments, and terminators
- command variant and scalar fields
- redirects and assignment surfaces
- word spans, word-part variants, and brace syntax
- nested command substitutions and arithmetic/conditional expression trees
- heredoc body parts
- pattern groups and zsh-specific syntax facts

Once consumers gain arena entry points, tests should run both recursive and arena paths on the same inputs and assert equivalent indexes, semantic summaries, lint diagnostics, and formatter output.

## Alternatives Considered

### Direct Parser Rewrite First

The parser could be changed to emit `AstStore` directly before any compatibility lowering exists. That would save allocations sooner, but it makes failures harder to localize because representation, parser control flow, and consumer APIs all change at once. Lowering first creates an oracle: the arena view should match the existing tree before the parser's construction strategy changes.

### One Untyped Node Arena

A single `Vec<Node>` with enum-dispatched children would simplify generic traversal and make IDs uniform. It would also erase useful type boundaries, require more runtime matching, and make child ranges less precise. Shuck already has typed `Idx<T>`/`IdRange<T>` primitives, so the AST store should keep typed arrays.

### Keep Recursive AST And Only Pack Lists

Packing only the largest child lists inside the current recursive structs would reduce some allocations with a smaller API change. It would not give facts and semantic analysis stable syntax IDs, and it would still leave recursive `Box` ownership throughout commands and expressions. It is a useful fallback for especially risky node families, but not the target architecture.

### Global Borrowed Arena

A bump arena of borrowed AST nodes could reduce allocation overhead, but it would thread arena lifetimes through parser, indexer, semantic, linter, formatter, and cache code. It would also make formatter rewriting harder. This repeats the rejected alternative from spec 015 and remains out of scope.

## Verification

Each phase should keep behavior stable:

- `cargo test -p shuck-ast`
- `cargo test -p shuck-parser`
- `cargo test -p shuck-indexer`
- `cargo test -p shuck-semantic`
- `cargo test -p shuck-linter`
- `cargo test -p shuck-formatter`
- `make test`
- `cargo bench -p shuck-benchmark --bench large_corpus_hotspots`
- `cargo bench -p shuck-benchmark --bench check_command`
- `make bench-memory-compare`

Phase 1 should additionally include recursive-vs-arena structural equivalence tests. Phase 2 should compare recursive and arena diagnostics for a focused fixture set before switching default paths. Phase 3 and Phase 4 should include allocation and throughput measurements; the migration should not continue past a phase that regresses `shuck check --no-cache --output-format concise` without a known reason.
