# shuck

`shuck` is the command-line shell checker and formatter in the Shuck workspace.

This crate is published so the binary, integration tests, and benchmarks can share the same
argument parsing and command execution logic. Most users should install and run the `shuck`
binary instead of depending on this crate directly. The `check` command lints standalone shell
files plus supported embedded shell in GitHub Actions workflow and composite-action `run:` blocks.

The Rust API is still pre-1.0 and may change between `0.x` releases.
