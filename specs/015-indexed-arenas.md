# 015: Indexed Arenas

## Status

Proposed

## Summary

Move Shuck toward compact indexed storage for both parsed syntax and derived facts, with two separate arenas: an AST arena owned by parsed files and a fact arena owned by analysis. The first implementation milestone packs linter fact child lists behind compatibility accessors while leaving the recursive AST intact. A later milestone migrates `shuck-ast::File` to an ID-backed `AstStore`.

## Motivation

The current AST is easy to consume but allocation-heavy: statements, commands, words, patterns, redirects, and several expression forms carry nested `Vec` and `Box` fields. Linter facts compound that cost by storing many per-command child lists as individual boxed slices. Large-corpus profiling has shown the linter phase can allocate heavily, so the fastest visible win is to pack fact lists first.

The migration must preserve source-backed text and avoid spreading arena lifetimes through the public APIs. Diagnostics, formatter mutation, cache keys, and semantic analysis all benefit from stable IDs and spans, but they should not depend on one global bump arena of borrowed AST nodes.

## Design

### Shared Arena Primitives

`shuck-ast` provides reusable typed arena helpers:

```rust
pub struct Idx<T>(u32);
pub struct IdRange<T> { start: u32, len: u32 }
pub struct ListArena<T> { items: Vec<T> }
```

These types are intentionally small and dependency-free. They panic if an index or range cannot fit in `u32`, which keeps stored IDs compact and makes overflow failures explicit during construction. `ListArena<T>` is append-only and returns `IdRange<T>` values for variable-length child lists.

### Fact Arena First

`LinterFacts` keeps the existing rule-facing surface while packing high-churn child lists:

- `CommandFact::redirect_facts()`
- `CommandFact::substitution_facts()`
- `CommandFact::scope_read_source_words()`
- command-local comparable name-use buckets
- `CommandFact::declaration_assignment_probes()`
- word-occurrence IDs by command

Each parent fact stores a typed range. The backing storage is a contiguous side array. During the compatibility phase, `CommandFact` exposes the same slice-returning methods by holding packed slice handles, so rules do not need broad edits.

Facts continue to reference AST nodes through existing borrowed AST views and spans for now. The AST migration will replace those references with AST IDs once `AstStore` exists.

### AST Arena Later

The later AST milestone introduces:

```rust
pub struct ParsedFile {
    pub source: Arc<str>,
    pub ast: AstStore,
    pub root: StmtSeqId,
    pub syntax_facts: SyntaxFacts,
}
```

`AstStore` owns typed node arrays for statements, commands, words, word parts, patterns, arithmetic expressions, redirects, comments, and syntax extras. Variable-length children use `IdRange<T>` rather than per-node vectors. Text remains source-backed through spans and existing source-text wrappers; owned strings remain reserved for cooked or synthetic text.

### Boundary Between Layers

The AST and fact arenas stay separate. Facts should eventually store AST IDs, not references to AST nodes. That keeps derived facts compact, avoids lifetime coupling, and gives diagnostics and formatter analysis a stable source-node identity.

## Alternatives Considered

### One Global Bump Arena

A single bump-allocated world of `&'arena` AST and fact references would reduce allocation overhead, but it would spread lifetimes through parser, semantic, linter, formatter, cache, and diagnostic APIs. It also makes formatter mutation awkward. This design rejects the global arena in favor of owned stores and typed IDs.

### Convert AST First

Converting the AST first attacks parser allocations directly, but it has a large blast radius across parser tests, semantic traversal, formatter logic, and rule internals. Packing facts first gives an incremental performance win while establishing the ID/range vocabulary the AST migration will reuse.

### Change Rule APIs Immediately

Rules could switch directly from `&CommandFact` slices to explicit fact views over `FactStore`. That is the end-state shape, but doing it in the first patch would mix storage migration with rule rewrites. Compatibility accessors keep the first milestone behavior-preserving.

## Verification

- `cargo test -p shuck-ast`
- `cargo test -p shuck-linter`
- `cargo test -p shuck-semantic`
- `cargo test -p shuck-parser`
- `cargo test -p shuck-formatter`
- `make test`
- `cargo bench -p shuck-benchmark --bench large_corpus_hotspots`
- `cargo bench -p shuck-benchmark --bench check_command`
- `make bench-memory-compare`

For the fact-packing milestone, rule snapshots should not change. Benchmark runs should show no throughput regression before continuing to the AST migration.
