# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is this project?

Shuck is a shell script linter/checker CLI tool, built on top of **shuck-parser**, a shell parser written in Rust. The repo is a Cargo workspace containing the linter and formatter CLI plus the parser, AST, semantic, and supporting library crates listed under [Workspace crates](#workspace-crates).

## Clean-Room Policy

This project is a clean-room reimplementation of ShellCheck. To preserve the integrity of the independent authoring process, the following rules **must** be followed at all times.

### Prohibited Inputs

- **Do not** read, reference, or import ShellCheck source code.
- **Do not** read, reference, or import ShellCheck wiki pages or documentation examples.
- **Do not** reuse diagnostic wording from ShellCheck materials.
- **Do not** copy raw ShellCheck output into committed repository files.
- **Do not** search the web for ShellCheck source, wiki content, or diagnostic text.

### Approved Inputs

- Shell language manuals, specifications (POSIX, Bash reference manual), and semantic notes.
- Files already authored inside this repository.
- The ShellCheck binary used only as a **black-box oracle** (run it, observe numeric exit/code, but do not copy its output text into committed files). You can reverse engineer behavior by running code through the ShellCheck binary.
- The companion `shell-checks` repository (`../shell-checks`) which contains independently authored rule specs, examples, and compatibility mappings.

### Authoring Rules

- Write all summaries, rationales, comments, and diagnostics from scratch in your own words.
- Compatibility codes may be referenced as bare numbers or `SC1234`-style identifiers.
- Do not copy ShellCheck text or third-party snippets verbatim into committed code or comments.

## Build and test commands

```bash
# Build / test the whole workspace
make build                    # cargo build
make test                     # cargo test

# Run the shuck CLI
make run ARGS="check ."       # cargo run -p shuck-cli -- check .

# Faster iteration on specific crates
cargo build -p shuck-cli
cargo test -p shuck-cli -p shuck-linter

# Run a single test
cargo test -p shuck-cli -- test_name
cargo test -p shuck-linter -- test_name
cargo test -p shuck-parser -- test_name

# Format and lint
cargo fmt
cargo clippy --all-targets -- -D warnings

# CI-style checks (fmt, clippy --workspace --all-features, cargo shear, script checks)
make check
```

The toolchain is pinned to stable in `rust-toolchain.toml`; no nightly is needed for normal development (let-chains are stable on edition 2024). Only the fuzz targets (`make fuzz-*`) use `cargo +nightly`.

## Large corpus tests

Always run the large corpus comparisons with the nix-provided `shellcheck`, not a globally installed one. The supported path is `make test-large-corpus`, which enters the repo's nix dev environment before running the ignored large-corpus test.

```bash
# Download/populate the corpus if needed
make setup-large-corpus

# Run the full compatibility comparison
make test-large-corpus

# Run a targeted comparison for one rule or a CSV list of selectors
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001,C006

# Run a smaller sample while iterating
make test-large-corpus SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=10

# Print only the 25 slowest Shuck fixtures and always exit successfully
make test-large-corpus SHUCK_LARGE_CORPUS_TIMING=1
```

Relevant environment variables for the large-corpus harness:

- `SHUCK_TEST_LARGE_CORPUS=1` — enables the ignored large-corpus test. `make test-large-corpus` sets this for you.
- `SHUCK_LARGE_CORPUS_ROOT=/path/to/corpus` — overrides corpus discovery. By default the test looks under `.cache/large-corpus` and then `../shell-checks`.
- `SHUCK_LARGE_CORPUS_TIMEOUT_SECS=300` — ShellCheck timeout budget per fixture.
- `SHUCK_LARGE_CORPUS_SHUCK_TIMEOUT_SECS=30` — Shuck timeout budget per fixture.
- `TEST_SHARD_INDEX=0` and `TEST_TOTAL_SHARDS=1` — split the corpus run across shards.
- `SHUCK_LARGE_CORPUS_RULES=C001,C006` — comma-separated rule selectors to compare. Accepts exact rule codes and the existing selector syntax such as category or prefix selectors.
- `SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=100` — deterministic fixture sampling percentage in `[1,100]`.
- `SHUCK_LARGE_CORPUS_MAPPED_ONLY=1` — limits ShellCheck diagnostics to codes that Shuck maps today.
- `SHUCK_LARGE_CORPUS_KEEP_GOING=1` — collects all fixture failures instead of stopping at the first one.
- `SHUCK_LARGE_CORPUS_TIMING=1` — runs a Shuck-only timing pass, prints the 25 slowest fixtures, and always exits successfully. This mode does not produce a compatibility log, so `make large-corpus-report` rejects it.

If you need to call `shellcheck` directly while debugging a large-corpus issue, do it through nix so the version matches the test harness. For example:

```bash
nix --extra-experimental-features 'nix-command flakes' develop --command shellcheck --version
```

## Architecture

### Workspace crates

- **`crates/shuck-cli`** — CLI binary. Discovers files, resolves config, coordinates caching, parses shell sources, runs lint/format commands, applies fixes, and renders reports. Project roots are resolved by walking up to find `.shuck.toml` or `shuck.toml`.
- **`crates/shuck-linter`** — Rule registry, checker dispatch, suppressions, generated rule metadata, fix application, and linter-owned facts built over parser, indexer, and semantic output.
- **`crates/shuck-semantic`** — Semantic model for bindings, references, scopes, declarations, source closure, call graph, CFG, and dataflow.
- **`crates/shuck-indexer`** — Positional and structural indexes over parsed scripts, including lines, comments, syntactic regions, heredocs, and continuation lines.
- **`crates/shuck-extract`** — Embedded shell extraction for supported host files such as GitHub Actions workflows and composite actions.
- **`crates/shuck-config`** — Configuration loading (`.shuck.toml` / `shuck.toml`), project-root discovery, and CLI override handling.
- **`crates/shuck-discover`** — Workspace file discovery: walks input paths, applies exclusions and gitignore rules, and detects shell scripts by extension or shebang.
- **`crates/shuck-run`** — Managed shell interpreter resolution, version selection, and installation behind `shuck run`, `shuck install`, and `shuck shell`.
- **`crates/shuck-server`** — Language Server Protocol server behind `shuck server`.
- **`crates/shuck-wasm`** — WebAssembly bindings for source-only linting and formatting (built with `make build-wasm`, tested with `make test-wasm`).
- **`crates/shuck-benchmark`** — Benchmark fixtures and harnesses (Criterion and macrobenchmarks, profiling helpers); see the `bench-*` and `profile-*` Makefile targets.
- **`crates/shuck-cache`** — File-level caching with SHA-256 keyed `PackageCache<T>`. Stores results in a shared cache root (from `--cache-dir`, `SHUCK_CACHE_DIR`, or the OS cache directory such as `~/Library/Caches/shuck` / `$XDG_CACHE_HOME/shuck`) using bincode serialization. Entries are keyed by file mtime+permissions and auto-pruned after 30 days.
- **`crates/shuck-parser`** — The shell parser library. Provides source-backed lexing, dialect/profile-aware parsing, AST construction, recovery diagnostics, and syntax facts.
- **`crates/shuck-ast`** — Shared AST node types, tokens, identifiers, and source span utilities.
- **`crates/shuck-formatter`** — Shell formatting built directly on the parser and AST.

### Data flow for `shuck check`

1. **Discover** (`shuck-discover`, re-exported by `crates/shuck-cli/src/discover.rs`) — Walk input paths, detect shell scripts by extension (`.sh`, `.bash`, `.zsh`, `.ksh`) or shebang, skip ignored dirs (`.git`, `node_modules`, etc.)
2. **Cache lookup** (`shuck-cache`) — Check if file has a valid cached result based on mtime/permissions
3. **Parse and index** (`shuck-parser`, `shuck-indexer`) — Infer the shell dialect, parse into an AST with recovery diagnostics, and build line/comment/region indexes
4. **Suppressions and analysis** (`shuck-linter`) — Parse shuck and ShellCheck-style directives, build the semantic model and linter facts, dispatch enabled rules, and apply per-file ignores
5. **Fix and report** — Optionally apply requested fixes, remap embedded diagnostics back to host files, render the selected output format, cache results, and return the appropriate exit status

### shuck-parser internals

The parser (`crates/shuck-parser/src/parser/`) is a recursive descent parser. Key modules (several are directories split by construct):
- `lexer/` — Tokenizer that handles shell quoting, expansions, heredocs
- `commands/` — Command and statement parsing (simple commands, lists, loops, `case`, `if`, functions, conditionals)
- `words/`, `word/` — Word, expansion, pattern, and substitution parsing
- `redirects.rs` — Redirection and heredoc delimiter parsing
- `arithmetic.rs` — Arithmetic expression parsing
- `heredocs.rs` — Heredoc body parsing

## Key conventions

- Rust edition 2024 on the stable toolchain pinned in `rust-toolchain.toml` (let-chains are used extensively)
- The parser crate warns on `clippy::unwrap_used` outside tests (`crates/shuck-parser/src/parser/mod.rs`); avoid new `unwrap()` calls there
- Rule codes: shuck's canonical codes are a category letter plus a number (`C` correctness, `S` style, `X` portability, `P` performance, `K` security; e.g., `C001`). ShellCheck-style `SCNNNN` codes (e.g., `SC2086`) are also accepted in suppression directives. The old `SH-NNN` form is only a legacy suppression alias; rule selectors reject it, so don't use it in new code or docs.
- Config files: `.shuck.toml` or `shuck.toml` at project root
- Cache directory: shared OS cache root by default; legacy `.shuck_cache/` directories are still ignored and `shuck clean` removes them during cleanup

## Commit messages

This repository uses [Conventional Commits](https://www.conventionalcommits.org/) because `release-please` parses commit history on `main` to generate `CHANGELOG.md` and pick the next version.

PRs are squash-merged, so the PR title is what lands on `main`. Write PR titles — and any commits on `main` — in this form:

```
<type>(<optional scope>): <short summary>
```

Common types (full list in `.release-please-config.json`):

- `feat:` — user-visible new behavior (minor bump once we hit 1.0)
- `fix:` — bug fix (patch bump)
- `perf:` — performance improvement
- `docs:` — docs-only changes
- `refactor:` — internal restructuring with no behavior change
- `test:` — test-only changes
- `chore:` — tooling, CI, dependency bumps, anything that doesn't fit above
- `ci:` — changes to workflows under `.github/`
- `build:` — build-system or packaging changes

Append `!` or a `BREAKING CHANGE:` footer for breaking changes (e.g., `feat!: drop C005`).

Optional scopes should match a crate or an area (e.g., `feat(linter): add C042`, `fix(parser): handle nested heredocs`, `chore(deps): bump clap`).

Only `feat`, `fix`, `perf`, `revert`, `docs`, and `refactor` appear in the published changelog; the rest are kept in git history but hidden from `CHANGELOG.md`. Release notes come from this history — write the summary for a downstream reader, not for yourself.

Release flow: release-please watches `main` and opens/maintains a release PR that bumps `workspace.package.version` in `Cargo.toml` and updates `CHANGELOG.md`. Merging that PR creates the `vX.Y.Z` tag, which triggers `release.yml` (cargo-dist) to build and publish artifacts. Do not bump versions or edit `CHANGELOG.md` by hand.

## Linter Rule Authoring

When working on `crates/shuck-linter`, treat the `facts/` module
(`crates/shuck-linter/src/facts/`) and `Checker::facts()` as the required extension point
for rule logic. See `crates/shuck-linter/AGENTS.md` for the full layering rules.

- New rules in any category under `crates/shuck-linter/src/rules/`
  (`correctness/`, `style/`, `performance/`, `portability/`, `security/`)
  **must not** add direct AST walks.
- Do not call traversal/query helpers from rule files. If a rule needs new structural data,
  add it to `LinterFacts` in `crates/shuck-linter/src/facts/` and consume it from there.
- Do not import from `crate::rules::common::*` inside rule files. Rule-facing shared types and
  helpers should come from the crate root or from rule-local helper modules.
- Keep repeated structural discovery in `facts/`, not in per-rule helper code.
- Rule files should mostly be cheap filters over facts plus rule-specific policy and wording.

If a rule feels like it needs `walk_commands`, `iter_commands`, command normalization, test
operand reconstruction, or repeated word/redirect/substitution classification, stop and move
that work into the fact builder instead.
