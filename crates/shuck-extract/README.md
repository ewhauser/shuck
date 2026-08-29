# shuck-extract

`shuck-extract` extracts shell scripts embedded in non-shell host files.

Today it provides the extraction layer behind embedded GitHub Actions support in `shuck check`.
It matches workflow files under `.github/workflows/` plus composite actions in `action.yml`,
extracts `run:` blocks, resolves the effective shell, substitutes `${{ ... }}` expressions with
an opaque shell-analysis projection, parses each expression into an owned `shuck-ast` tree, and
returns the original decoded source plus a total projection-to-source offset map. Malformed
expressions are retained as expression-local diagnostics so the surrounding shell can still be
analyzed. The public template AST preserves literal, delimiter, body, grouping, and operator
ranges; the projection never replaces that authoritative syntax model.

The crate is part of the published Shuck toolchain, but its Rust API is still pre-1.0 and may
grow as more embedded-shell formats are added.
