# shuck-extract

`shuck-extract` extracts shell scripts embedded in non-shell host files.

Today it provides the extraction layer behind embedded GitHub Actions support in `shuck check`.
It matches workflow files under `.github/workflows/` plus composite actions in `action.yml`,
extracts `run:` blocks, resolves the effective shell, substitutes `${{ ... }}` expressions with
synthetic shell placeholders, and returns enough source-location metadata to remap diagnostics back
to the host YAML file.

The crate is part of the published Shuck toolchain, but its Rust API is still pre-1.0 and may
grow as more embedded-shell formats are added.
