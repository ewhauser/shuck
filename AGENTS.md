# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is this project?

Shuck is a shell script linter/checker CLI tool, built on top of **shuck-parser** (an in-process virtual bash interpreter written in Rust). The repo is a Cargo workspace containing both shuck (the linter) and shuck-parser (the underlying library).

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
# Build just the shuck crates (fast iteration)
make build                    # cargo build -p shuck-cli -p shuck-cache

# Test just the shuck crates
make test                     # cargo test -p shuck-cli -p shuck-cache

# Run the shuck CLI
make run ARGS="check ."       # cargo run -p shuck-cli -- check .

# Build/test everything (including shuck-parser)
cargo build
cargo test --features http_client

# Run a single test
cargo test -p shuck-cli -- test_name
cargo test -p shuck-syntax -- test_name
cargo test -p shuck-parser -- test_name

# Format and lint
cargo fmt
cargo clippy --all-targets -- -D warnings
```

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

- **`crates/shuck`** — CLI binary. Discovers shell files, parses them via shuck-syntax, reports parse errors. Subcommands: `check` (lint files) and `clean` (remove caches). Project root is resolved by walking up to find `.shuck.toml` or `shuck.toml`.
- **`crates/shuck-syntax`** — Linter-oriented syntax wrapper over shuck-parser's parser. Adds comment collection (including inside `$(...)` substitutions), suppression directive parsing (`# shuck: disable=SH-001` and `# shellcheck disable=SC2086`), a `SuppressionIndex` for line-level queries, and a dialect/profile parse-view layer. Today it supports native Bash `strict` and `strict-recovered` views plus Bash-backed `permissive` and `permissive-recovered` fallbacks for `sh`/`dash`/`ksh`/`mksh`.
- **`crates/shuck-cache`** — File-level caching with SHA-256 keyed `PackageCache<T>`. Stores results in a shared cache root (from `--cache-dir`, `SHUCK_CACHE_DIR`, or the OS cache directory such as `~/Library/Caches/shuck` / `$XDG_CACHE_HOME/shuck`) using bincode serialization. Entries are keyed by file mtime+permissions and auto-pruned after 30 days.
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

## Key conventions

- Rust edition 2024, requires nightly features (let-chains used extensively)
- `#[allow(clippy::unwrap_used)]` is used in the parser module since unwraps follow validated bounds checks
- Suppression codes: shuck uses `SH-NNN` format (e.g., `SH-001`), shellcheck uses `SCNNNN` format (e.g., `SC2086`). Both are supported as suppression directives.
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

When working on `crates/shuck-linter`, treat `facts.rs` and `Checker::facts()` as the
required extension point for rule logic.

- New rules in `crates/shuck-linter/src/rules/style/` and
  `crates/shuck-linter/src/rules/correctness/` **must not** add direct AST walks.
- Do not call traversal/query helpers from rule files. If a rule needs new structural data,
  add it to `LinterFacts` in `crates/shuck-linter/src/facts.rs` and consume it from there.
- Do not import from `crate::rules::common::*` inside rule files. Rule-facing shared types and
  helpers should come from the crate root or from rule-local helper modules.
- Keep repeated structural discovery in `facts.rs`, not in per-rule helper code.
- Rule files should mostly be cheap filters over facts plus rule-specific policy and wording.

If a rule feels like it needs `walk_commands`, `iter_commands`, command normalization, test
operand reconstruction, or repeated word/redirect/substitution classification, stop and move
that work into the fact builder instead.
