# shuck-linter

`shuck-linter` contains the rule engine behind `shuck check`.

It combines parser output, positional indexes, semantic analysis, suppressions, fixes, and rule
selection into a diagnostics pipeline for shell scripts. The crate is public because it is part
of the published Shuck toolchain, but its Rust API is still pre-1.0 and actively evolving.

Use `LinterSettings` constructors plus `AnalysisRequest` as the supported embedding path. Settings
and result fields remain readable, while non-exhaustive types protect routine field and rule
growth. Match `Rule` with a fallback. The workspace documents the full boundary in
[`docs/rust-api-compatibility.md`](../../docs/rust-api-compatibility.md).
