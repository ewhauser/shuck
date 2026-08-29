# shuck-semantic

`shuck-semantic` builds semantic models for shell scripts on top of parsed syntax.

It tracks scopes, bindings, references, control-flow, source references, and selected dataflow
facts so higher-level tooling can reason about shell behavior. The crate is published as part of
the Shuck workspace and is still pre-1.0.

Semantic query APIs are available to embedders, but semantic facts are not blanket-marked as
stable or non-exhaustive. See
[`docs/rust-api-compatibility.md`](../../docs/rust-api-compatibility.md).
