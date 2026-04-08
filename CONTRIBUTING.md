# Contributing to Shuck

Thanks for your interest in contributing to Shuck! This guide covers how to build, test, and add lint rules.

## Prerequisites

- **Rust** stable toolchain (pinned in `rust-toolchain.toml`; includes `rustfmt` and `clippy`)
- **Nix** (optional) — required for large corpus tests, macrobenchmarks, and profiling. The `flake.nix` provides a dev shell with `shellcheck`, `shfmt`, `hyperfine`, `samply`, and `cargo-udeps`.

## Getting Started

```bash
git clone https://github.com/ewhauser/shuck.git
cd shuck

# Set up pre-commit hooks (runs cargo fmt and clippy before each commit)
make setup-hooks

# Build
make build

# Run tests
make test

# Run the CLI
make run ARGS="check ."
```

## Development Workflow

Before submitting changes, run the full check suite:

```bash
make check    # cargo fmt --check + cargo clippy -D warnings + cargo udeps
```

Or run the individual steps:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```

Pre-commit hooks enforce formatting and clippy automatically when you commit.

## Testing

**Run all tests:**

```bash
make test          # or: cargo test
```

**Run a single test:**

```bash
cargo test -p shuck-linter -- test_name
```

**Snapshot tests** — The linter uses [insta](https://insta.rs) for snapshot testing. When you add or change a rule, the test will fail with a diff. Review and accept with:

```bash
cargo insta accept --workspace
```

**Large corpus conformance** (requires Nix):

```bash
make setup-large-corpus       # download corpus (first time only)
make test-large-corpus        # run full comparison against ShellCheck
```

You can target specific rules or sample a subset:

```bash
make test-large-corpus SHUCK_LARGE_CORPUS_RULES=C001
make test-large-corpus SHUCK_LARGE_CORPUS_SAMPLE_PERCENT=10
```

## Project Structure

| Crate | Purpose |
|-------|---------|
| `shuck` | CLI binary — file discovery, parsing, reporting |
| `shuck-linter` | Lint rule registry, checker dispatch, violation types |
| `shuck-semantic` | Semantic model — bindings, scopes, CFG, dataflow |
| `shuck-indexer` | Positional and structural indexes over parsed scripts |
| `shuck-parser` | Recursive-descent Bash parser |
| `shuck-ast` | AST node types, tokens, spans |
| `shuck-syntax` | Dialect profiles, comment collection, suppression directives |
| `shuck-cache` | SHA-256 keyed file-level result caching |
| `shuck-formatter` | Shell script formatter |
| `shuck-format` | Low-level formatting document and printer primitives |

## Adding a Lint Rule

Rules are organized into five categories:

| Prefix | Category | Example |
|--------|----------|---------|
| `C` | Correctness | `C001` — unused assignment |
| `S` | Style | `S001` — unquoted expansion |
| `P` | Performance | `P001` — useless cat |
| `X` | Portability | `X001` — bashism in sh script |
| `K` | Security | `K001` — unquoted glob in rm |

### Step 1: Write the rule spec

Create `docs/rules/{CODE}.yaml` with the rule definition:

```yaml
legacy_code: SH-NNN
legacy_name: rule-name
new_category: Correctness
new_code: C042
runtime_kind: ast          # ast, semantic, or flow
shellcheck_code: SC2034    # ShellCheck compatibility code, if applicable
shells:
  - sh
  - bash
description: What the rule detects.
rationale: Why it matters and how to fix it.
examples:
  - kind: invalid
    code: |
      #!/bin/sh
      problematic_code
  - kind: valid
    code: |
      #!/bin/sh
      correct_code
```

### Step 2: Register the rule

In `crates/shuck-linter/src/registry.rs`, add an entry to the `declare_rules!` macro in code-sorted order:

```rust
declare_rules! {
    ("C001", Category::Correctness, Severity::Warning, UnusedAssignment),
    // ...
    ("C042", Category::Correctness, Severity::Warning, YourRuleName),
    // ...
}
```

If the rule has a legacy `SH-NNN` code, add an alias in the `code_to_rule()` function in the same file.

### Step 3: Add ShellCheck mapping

If the rule maps to a ShellCheck code, add the mapping in `crates/shuck-linter/src/suppression/shellcheck_map.rs`:

```rust
(2034, Rule::YourRuleName),  // SC2034
```

### Step 4: Implement the rule

Create `crates/shuck-linter/src/rules/{category}/{snake_case_name}.rs`:

```rust
use crate::{Checker, Rule, Violation};

pub struct YourRuleName {
    pub name: String,
}

impl Violation for YourRuleName {
    fn rule() -> Rule {
        Rule::YourRuleName
    }

    fn message(&self) -> String {
        format!("description of the problem for `{}`", self.name)
    }
}

pub fn your_rule_name(checker: &mut Checker) {
    // Query the semantic model, facts, or AST
    // Report violations with checker.report(violation, span)
}
```

Key APIs available on `Checker`:
- `checker.semantic()` — bindings, references, scopes, call graph
- `checker.facts()` — normalized commands, pipelines, conditionals
- `checker.ast()` — raw AST
- `checker.source()` — source text
- `checker.shell()` — detected shell dialect
- `checker.report(violation, span)` — emit a diagnostic
- `checker.report_dedup(violation, span)` — emit with deduplication

Look at existing rules for patterns:
- Simple semantic rule: `rules/correctness/unused_assignment.rs`
- Facts-based rule: `rules/style/read_without_raw.rs`
- Complex rule: `rules/correctness/find_output_to_xargs.rs`

### Step 5: Register the module

In `crates/shuck-linter/src/rules/{category}/mod.rs`, add:

```rust
pub mod your_rule_name;
```

Then add a `#[test_case]` entry in the test function at the bottom of the same file:

```rust
#[test_case(Rule::YourRuleName, Path::new("C042.sh"))]
```

### Step 6: Wire into checker dispatch

In `crates/shuck-linter/src/checker.rs`, add the rule to the appropriate checker phase:

| Phase | Use for |
|-------|---------|
| `check_bindings` | Variable assignments, unused variables |
| `check_references` | Variable uses, undefined variables |
| `check_declarations` | `declare`/`local`/`export` commands |
| `check_call_sites` | Function calls |
| `check_source_refs` | `source`/`.` commands |
| `check_commands` | Command structure, most rules go here |
| `check_flow` | Control flow, dead code |

```rust
if self.is_rule_enabled(Rule::YourRuleName) {
    rules::category::your_rule_name::your_rule_name(self);
}
```

### Step 7: Create a test fixture

Create `crates/shuck-linter/resources/test/fixtures/{category}/C042.sh` with both triggering and non-triggering cases:

```bash
#!/bin/sh

# Should trigger
problematic_code

# Should not trigger
correct_code
```

### Step 8: Run tests and accept snapshots

```bash
cargo test -p shuck-linter -- your_rule_name    # run the new tests
cargo insta accept --workspace                   # accept snapshot output
cargo test                                       # verify no regressions
```

## Clean-Room Policy

Shuck is a clean-room reimplementation. All contributors must follow these rules:

- **Do not** read, reference, or import ShellCheck source code or wiki pages
- **Do not** reuse diagnostic wording from ShellCheck materials
- **Do** write all descriptions, rationales, and messages from scratch in your own words
- **Do** reference shell language manuals and specifications (POSIX, Bash reference manual)
- **Do** use the ShellCheck binary as a black-box oracle (run it, observe behavior, but do not copy its output text)

See `CLAUDE.md` for the full policy.

## Code Style

- **Rust edition 2024**, stable toolchain
- Repo-pinned `rustfmt` settings via `rustfmt.toml`, plus default `clippy` settings
- **Error handling**: `anyhow` for error propagation with `.context()`, `thiserror` for domain-specific error enums
- **Suppression codes**: Shuck uses `SH-NNN` format; ShellCheck `SCNNNN` format is also accepted in suppression directives

## Benchmarking

```bash
make bench                    # Criterion microbenchmarks
make bench-parser             # parser benchmarks only
make bench-linter             # linter benchmarks only
make bench-macro              # Hyperfine comparison vs ShellCheck (requires Nix)
```

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
