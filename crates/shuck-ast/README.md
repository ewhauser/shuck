# shuck-ast

`shuck-ast` defines the abstract syntax tree, token kinds, and span types shared across the
Shuck workspace.

Use this crate when you need to inspect or transform parsed shell syntax. In most cases,
`shuck-parser` is the crate that produces these types, while `shuck-indexer`, `shuck-linter`,
and `shuck-formatter` consume them.

The API is pre-1.0 and may evolve between `0.x` releases.
