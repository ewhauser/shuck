# `shuck-hir` Design

## Summary

- Add a new `crates/shuck-hir` crate that consumes `ParsedSyntax` plus source text and produces an owned, normalized shell HIR.
- Follow Ruff's architecture at the layer boundary, not literally at the data shape: typed IDs, arena-backed storage, query-oriented semantic APIs, and a clear split between syntax, semantics, and project analysis.
- Do not port Go's `AstIndex` / `FileFacts` design directly. Use the Go code only as a feature inventory and parity oracle.
- Default to a normalized HIR, not a CST and not a thin wrapper over `shuck-parser` AST, because shell rules need stronger domain types than `shuck-parser` currently exposes.

## Key Changes

- Create `crates/shuck-hir` with an entrypoint equivalent to `build(source: Arc<str>, parsed: ParsedSyntax) -> Result<HirFile, HirBuildError>`.
- `HirFile` should own:
  - `ParseView` and parse diagnostics from `ParsedSyntax`
  - source text handle, line index, comments, directives, suppression index
  - typed arenas for normalized nodes and root command IDs
- Use local typed-ID wrappers over `Vec` arenas, not string keys and not raw `usize`:
  - `CommandId`, `WordId`, `AssignmentId`, `RedirectId`, `FunctionId`, `ScopeId`, `SymbolId`, `SourceRefId`
  - `AnyNodeId` for heterogeneous parent links and ordered traversal
- Every stored node should carry:
  - `SourceSpan`
  - `parent: Option<AnyNodeId>`
  - stable source order / ordinal for deterministic iteration
- Normalize syntax into shell-specific node families:
  - commands: simple, pipeline, list, if, for, arithmetic-for, while, until, case, subshell, brace-group, function
  - words: literal plus typed expansion parts
  - assignments and redirects as first-class nodes
- Keep parser-only details out of the rule API. Raw `shuck-parser` AST stays internal to lowering.
- Add a `SemanticModel` on top of `HirFile`, in the same crate for the first milestone, with typed enums instead of Go's stringly facts:
  - `ScopeKind::{File, Function, Subshell, CommandSubstitution}`
  - `SymbolKind::{Variable, Function}`
  - `SourceRefKind::{Literal, Directive, DirectiveDevNull, Dynamic, SingleVariableStaticTail}`
  - variable events for definition, read, required-read, declaration/export-like definition, loop binding, etc.
- Recognize linter-relevant command shapes semantically instead of forcing them into syntax:
  - declaration builtins (`declare`, `typeset`, `local`, `export`, `readonly`)
  - `source` / `.`
  - `read`
  - other rule-critical command summaries as typed helpers
- Expose a query-first rule API modeled after Ruff's semantic model:
  - ordered command/word iterators
  - parent/ancestor lookups
  - function declarations and call sites
  - visible variable state at an offset
  - rooted same-file call graph
  - overwritten-function diagnostics inputs
  - source refs, imported variables, imported functions
- Keep project closure as a distinct layer above file semantics:
  - `ProjectClosure` built from `SemanticModel` outputs, not mixed into syntax lowering
  - dependency fingerprinting and `source` resolution remain project-layer responsibilities
- Keep parse-view selection above HIR:
  - one `HirFile` per `ParseView`
  - engine/runtime decides which view a rule sees
  - HIR and semantics do not own parse-policy selection

## Implementation Sequence

### Phase 1

- Scaffold `crates/shuck-hir`
- Define typed IDs, node enums, `HirFile`, lowering context, and source-order bookkeeping

### Phase 2

- Lower `ParsedSyntax` into owned HIR for commands, words, assignments, redirects, comments, directives, and suppressions

### Phase 3

- Add `SemanticModel` passes for scopes, variable defs/refs, function declarations, call sites, call graph, and overwritten-function detection

### Phase 4

- Add project analysis over semantic outputs for `source` refs, imported variables/functions, closure expansion, and dependency fingerprinting

### Phase 5

- Add a thin rule-facing facade in `shuck` so future rules depend on `HirFile` + `SemanticModel` + `ProjectClosure`, not `shuck-parser` AST

### Phase 6

- Prove the design by porting a small rule set that exercises the layers:
  - one directive-sensitive rule
  - one syntax-only rule
  - one semantic variable/scope rule
  - one project-closure/source rule

## Test Plan

- Unit tests for lowering:
  - parent/child relationships
  - source-order iteration
  - stable IDs
  - exact spans and line/column slices
  - parse-view propagation from `ParsedSyntax`
- Semantic tests for:
  - function scopes
  - subshell and command-substitution scope boundaries
  - variable definitions and reads
  - declaration builtin classification
  - call-site and rooted-call-graph construction
- Project tests for:
  - literal `source`
  - `# shellcheck source=...`
  - `/dev/null`
  - dynamic paths
  - single-variable static tail cases
  - imported variables/functions
- Parity tests against the Go frontend on representative fixtures for:
  - comments/directives/suppressions
  - scopes
  - variable references
  - function declarations/calls
  - source refs
  - project closure
- Acceptance criteria:
  - first rules can be written without touching raw `shuck-parser` AST
  - no new stringly typed fact APIs are introduced
  - recovered and permissive views lower through the same HIR API

## Assumptions

- Use a separate `shuck-hir` crate.
- Use owned lowered nodes, not borrowed AST wrappers.
- Use query-first rule ergonomics as the primary API; visitor helpers are secondary.
- "Full facts parity" means the first milestone includes syntax HIR, semantic model, and project closure, but not full rule-engine migration or parse-policy execution logic.
- Default to local typed-ID arena utilities instead of pulling in a new arena/index dependency unless implementation friction proves that unjustified.
