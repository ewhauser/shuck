# Plugin Manager Guidance

This directory contains zsh plugin-manager adapters for source-closure analysis.
Each adapter recognizes one family of plugin/framework behavior and translates it
into common source-closure outputs. Keep the adapters small and declarative: they
should identify plugin entrypoints, imported contracts, or deferred runtime reads,
not become a second shell interpreter.

## Adding a Manager

- Add a new module next to `oh_my_zsh.rs` and `generic_zsh_runtime.rs`.
- Implement `ZshPluginManager` from `mod.rs`.
- Register the manager in the manager list in `mod.rs`.
- Prefer narrow `is_active` checks based on resolver state, detected framework
  syntax, or shell dialect. Do not key behavior only on arbitrary path substrings
  when a semantic or resolver signal is available.
- Return common outputs such as `PluginRequest`s or deferred required reads.
  Resolution of logical plugin requests to files belongs in `PluginResolver`,
  not in the manager.

## Design Boundaries

- Core zsh semantics belong in the semantic builder/runtime model, not in a
  plugin-specific manager.
- Plugin managers may model framework contracts and bounded deferred behavior,
  such as static hook registrations or known generated wrapper templates.
- Keep symbolic execution intentionally bounded. Follow static names and literal
  arguments when Shuck has already extracted them; do not evaluate arbitrary shell
  code, run commands, or interpret unconstrained `eval` strings.
- Avoid hard-coded plugin allowlists in Rust. If behavior is plugin-specific and
  cannot be inferred from source shape, prefer resolver data or future manifest
  data over adding exact plugin names here.
- Preserve caller diagnostics. A manager should only expose reads that are true
  contract reads; declaration names, printed text, and unrelated string templates
  must not become synthetic reads.

## Tests

- Add focused semantic tests for each new manager behavior.
- If a review comment points out a false positive or false negative, add a
  regression test that would fail without the fix.
- Include a negative test for every new inference path, especially generated
  callbacks or dynamic-looking source patterns.
- Run at least `cargo test -p shuck-semantic` and
  `cargo clippy -p shuck-semantic --all-targets -- -D warnings` before pushing.
  For source-closure changes that can affect zsh corpus behavior, also run the
  targeted zsh large-corpus comparison.
