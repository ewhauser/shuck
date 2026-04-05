# AGENTS.md

These instructions apply to `crates/shuck-parser`. Follow the repo-level
`/Users/ewhauser/working/shuck-rs/AGENTS.md` first, then this file.

## Performance guardrails

- Performance is part of correctness in this crate, especially under
  `src/parser`. Treat lexer and parser hot-path allocations, copies, and extra
  full-input scans as regressions unless they are clearly required.
- Prefer source positions, spans, and cursor-based parsing over building new
  owned strings. If text can stay borrowed or be recovered from the input later,
  do that instead of allocating eagerly.
- Owned text is still acceptable when the parser must normalize, unescape,
  synthesize, or buffer content that does not exist verbatim in the source.
  Typical examples are cooked strings, heredoc bodies, and error messages.
- Preserve the existing parser-to-AST pattern where the parser collapses back to
  source-backed AST text whenever the cooked text matches the original source.
  Reuse helpers like `literal_text()` and `source_text()` instead of adding
  parallel representations.
- Do not add new owned string fields, `to_string()` calls, `format!()` calls,
  or duplicate text buffers in `src/parser` hot paths without a specific reason
  documented in code or the PR description.
- Keep diagnostic rendering, debug formatting, and test-only convenience work
  out of the main parse path when possible.
- Before adding a new parser pass or rescanning large chunks of input, first
  check whether the existing cursor/span tracking can supply the same result in
  one pass.

## Scope notes

- `tests`, `examples`, and `fuzz` code do not need the same allocation
  discipline as `src/parser`, but avoid copying production patterns there in a
  way that would encourage slower parser changes later.
