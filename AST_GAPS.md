# AST Gaps for CFG Construction

This document tracks the remaining parser-to-HIR gaps that matter for CFG and dataflow work.

## Current parser state

The AST is no longer in the old "flatten everything into strings" shape:

- Flow-control builtins (`break`, `continue`, `return`, `exit`) are typed AST nodes.
- `[[ ... ]]` conditionals are parsed into a structured conditional expression tree.
- `(( ... ))` and `for (( ... ; ... ; ... ))` preserve exact source spans instead of rebuilding expression strings.
- `shuck-syntax` now exposes zero-copy arithmetic def/use analysis over arithmetic commands, arithmetic `for` headers, and `$(( ... ))` expansions.
- Identifier-like fields use compact owned `Name` values plus exact source spans where later indexing and diagnostics need them:
  - function names
  - `for` / `select` loop variables
  - coprocess names
  - assignment names and indices
  - fd-variable redirect names

That means HIR, semantic indexes, and CFG can lower from source slices and `Name` values without introducing new owned string copies.

## Remaining gaps

## 1. `trap` Is Still A Generic SimpleCommand

`trap` defines signal handlers that execute asynchronously. For CFG purposes, trap bodies are reachable from any point after the `trap` call.

```bash
trap 'cleanup' EXIT
```

### Recommended layer

HIR / semantics, not parser AST.

### Why

The interesting part is not just recognizing `trap`, but classifying:

- handler vs reset vs ignore
- signal set
- whether the handler body should be parsed separately

That is linter-facing semantics, not syntax.

## 2. `source` / `.` Are Still Generic SimpleCommands

`source` and `.` include other scripts and affect scope, imports, and project closure.

```bash
source ./lib.sh
. ./lib.sh
```

### Recommended layer

HIR / project analysis.

### Why

The parser should not resolve files or decide how project closure works. HIR can classify the command shape, and project analysis can resolve literal vs dynamic paths.

## 3. Arithmetic Def/Use Analysis Is Now Source-Backed

Arithmetic commands and arithmetic `for` headers now preserve exact spans:

```rust
CompoundCommand::Arithmetic(ArithmeticCommand {
    left_paren_span,
    expr_span,
    right_paren_span,
    ..
})

ArithmeticForCommand {
    init_span,
    condition_span,
    step_span,
    ..
}
```

### What this fixes

- No reconstructed arithmetic strings in the parser.
- HIR can slice the original source text exactly.
- `shuck-syntax` can now emit ordered variable read/write events directly from those spans, plus `$(( ... ))` expansion spans, without introducing new owned strings.

### Recommended layer

Keep using the dedicated arithmetic analysis pass until full HIR lowering exists, then thread those results through HIR/CFG.

## 4. Comments, Directives, And Suppressions Are Not Yet Unified Into HIR

`shuck-syntax` already collects comments, directives, and suppressions, but HIR does not exist yet as the single source of truth for rule execution.

### Recommended layer

HIR.

### Why

Rules, CFG, and semantic indexes need one consistent file model that includes:

- source text
- line index
- comments/directives
- suppression queries
- lowered commands/words/redirects/assignments

## 5. CFG/Dataflow Layers Do Not Exist Yet

The AST/parser now exposes the fidelity needed for zero-copy lowering, but the actual analysis layers still need to be built:

- HIR
- scope and symbol indexes
- CFG
- reaching-definitions / unset-variable / dead-code analyses

## Summary

| Area | Status | Recommended layer |
|-----|-----|-----|
| Flow-control builtins | Resolved in AST | Done |
| Structured `[[ ... ]]` | Resolved in AST | Done |
| Arithmetic source fidelity | Resolved in AST | Done |
| `trap` classification | Missing | HIR / semantics |
| `source` / `.` classification | Missing | HIR / project analysis |
| Arithmetic defs/uses semantics | Resolved in `shuck-syntax` arithmetic analysis | Done |
| Unified rule-facing file model | Missing | HIR |
| CFG and dataflow | Missing | CFG layer |

## Recommended order

1. Build HIR around the new source-backed AST.
2. Classify `source` / `.` and `trap` in HIR semantics.
3. Reuse arithmetic analysis results during HIR and semantic-model construction.
4. Build CFG on top of HIR.
5. Add dataflow passes.
