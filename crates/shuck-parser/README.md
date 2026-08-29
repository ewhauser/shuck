# shuck-parser

`shuck-parser` is the lexer and parser library used throughout the Shuck workspace.

It turns shell source into `shuck-ast` syntax trees, exposes source-backed lexical tokens, and
lets callers choose shell dialects and zsh option profiles. The crate is pre-1.0 and may change
between `0.x` releases.

`ParseResult` is a producer-owned, non-exhaustive result with public fields for inspection.
`ParseStatus` and shell dialects remain deliberately exhaustive. See
[`docs/rust-api-compatibility.md`](../../docs/rust-api-compatibility.md) for the supported boundary.
