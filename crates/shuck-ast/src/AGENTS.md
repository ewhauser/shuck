# AGENTS.md

These instructions apply to `crates/shuck-ast/src`. Follow the repo-level
`/Users/ewhauser/working/shuck-rs/AGENTS.md` first, then this file.

## AST allocation guardrails

- Keep AST text source-backed whenever possible. If text can be recovered from
  the original shell source, store a `Span`/`TextRange` or use the existing
  source-backed wrappers instead of introducing a new `String` allocation.
- Reuse the existing AST text types before adding anything new:
  `SourceText`, `LiteralText::Source`, `Name`, and `Word.part_spans`.
- Owned text is only for cooked or synthetic values that do not exist verbatim
  in the source. Do not add new owned string fields or parser paths for normal
  AST construction.
- Avoid duplicated storage. Do not keep both an owned string and the span/range
  that can reproduce it unless there is a documented, unavoidable reason.
- Do not add convenience helpers that allocate during ordinary parsing, AST
  traversal, or comparisons. If a caller needs a rendered string for diagnostics
  or tests, keep that work in an explicit rendering step.
- Before adding any new textual AST field to `ast.rs`, first check whether
  `Span`, `TextRange`, `SourceText`, `LiteralText`, or `Name` can represent the
  same information without allocating.
