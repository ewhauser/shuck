# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is this project?

Shuck is a shell script linter/checker CLI tool, built on top of **shuck-parser** (an in-process virtual bash interpreter written in Rust). The repo is a Cargo workspace containing both shuck (the linter) and shuck-parser (the underlying library).

## Build and test commands

```bash
# Build just the shuck crates (fast iteration)
make build                    # cargo build -p shuck -p shuck-cache

# Test just the shuck crates
make test                     # cargo test -p shuck -p shuck-cache

# Run the shuck CLI
make run ARGS="check ."       # cargo run -p shuck -- check .

# Build/test everything (including shuck-parser)
cargo build
cargo test --features http_client

# Run a single test
cargo test -p shuck -- test_name
cargo test -p shuck-syntax -- test_name
cargo test -p shuck-parser -- test_name

# Format and lint
cargo fmt
cargo clippy --all-targets -- -D warnings
```

## Architecture

### Workspace crates

- **`crates/shuck`** — CLI binary. Discovers shell files, parses them via shuck-syntax, reports parse errors. Subcommands: `check` (lint files) and `clean` (remove caches). Project root is resolved by walking up to find `.shuck.toml` or `shuck.toml`.
- **`crates/shuck-syntax`** — Linter-oriented syntax wrapper over shuck-parser's parser. Adds comment collection (including inside `$(...)` substitutions), suppression directive parsing (`# shuck: disable=SH-001` and `# shellcheck disable=SC2086`), a `SuppressionIndex` for line-level queries, and a dialect/profile parse-view layer. Today it supports native Bash `strict` and `strict-recovered` views plus Bash-backed `permissive` and `permissive-recovered` fallbacks for `sh`/`dash`/`ksh`/`mksh`.
- **`crates/shuck-cache`** — File-level caching with SHA-256 keyed `PackageCache<T>`. Stores results in `.shuck_cache/` under the project root using bincode serialization. Entries are keyed by file mtime+permissions and auto-pruned after 30 days.
- **`crates/shuck-parser`** — The bash parser library. Provides `Lexer`, `Parser`, AST types, and the full execution runtime. Shuck only uses the parser/lexer portion.

### Data flow for `shuck check`

1. **Discover** (`discover.rs`) — Walk input paths, detect shell scripts by extension (`.sh`, `.bash`, `.zsh`, `.ksh`) or shebang, skip ignored dirs (`.git`, `node_modules`, etc.)
2. **Cache lookup** (`shuck-cache`) — Check if file has a valid cached result based on mtime/permissions
3. **Parse** (`shuck-syntax::parse`) — Resolve a dialect profile into a concrete parse view, lex and collect comments, parse directives/suppressions, then parse AST via the selected backend grammar
4. **Report** — Print `path:line:col: parse error message` format, cache results, exit 0 (success) or 1 (failures found)

### shuck-parser internals

The parser (`crates/shuck-parser/src/parser/`) is a recursive descent parser with these modules:
- `lexer.rs` — Tokenizer that handles shell quoting, expansions, heredocs
- `ast.rs` — AST node types (`Script`, `Command`, `Pipeline`, etc.)
- `tokens.rs` — Token enum
- `span.rs` — Position/Span tracking
- `budget.rs` — Parser fuel/budget limits

## Key conventions

- Rust edition 2024, requires nightly features (let-chains used extensively)
- `#[allow(clippy::unwrap_used)]` is used in the parser module since unwraps follow validated bounds checks
- Suppression codes: shuck uses `SH-NNN` format (e.g., `SH-001`), shellcheck uses `SCNNNN` format (e.g., `SC2086`). Both are supported as suppression directives.
- Config files: `.shuck.toml` or `shuck.toml` at project root
- Cache directory: `.shuck_cache/` (add to `.gitignore`)
