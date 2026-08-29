# shuck-ast

`shuck-ast` defines the abstract syntax tree, token kinds, and span types shared across the
Shuck workspace.

Use this crate when you need to inspect or transform parsed shell syntax. In most cases,
`shuck-parser` is the crate that produces these types, while `shuck-indexer`, `shuck-linter`,
and `shuck-formatter` consume them.

AST node documentation is available from the crate root. Start with `File`, then traverse
`File::body` through `Stmt` and `Command`.

The API is pre-1.0 and may evolve between `0.x` releases. Public AST types are not blanket-marked
as stable or non-exhaustive; structural changes remain possible and will be handled explicitly.
See [`docs/rust-api-compatibility.md`](../../docs/rust-api-compatibility.md).
