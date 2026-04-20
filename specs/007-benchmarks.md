# 007: Benchmarks

## Status

Proposed

## Summary

A benchmarking setup for shuck that measures parse, semantic, and lint throughput on real-world shell scripts and compares end-to-end CLI performance against shellcheck. The design follows ruff's two-tier approach: **Criterion.rs micro-benchmarks** in a dedicated `shuck-benchmark` crate for measuring individual components (lexer, parser, semantic analysis, linter), and **hyperfine macro-benchmarks** via shell scripts for CLI-vs-shellcheck wall-time comparison. Both tiers operate on the same 5 vendored benchmark fixtures used by the Go frontend (`shuck`), copied directly from `shuck/testdata/benchmarks/`.

## Motivation

The CHECKLIST.md item 12.3 calls for "benchmark cases for parse throughput and memory use." Today we have correctness tests (large corpus conformance against shellcheck) but no performance measurement. Without benchmarks we can't:

- **Detect regressions** — A parser refactor could halve throughput and we wouldn't notice until users report it.
- **Quantify our advantage** — Shuck's value proposition includes speed. We need numbers to back this up, comparing against shellcheck on the same scripts.
- **Guide optimization** — Profiling without repeatable benchmarks is guesswork. Criterion's statistical analysis and baseline comparison give us actionable data.

Ruff's benchmark infrastructure (dedicated `ruff_benchmark` crate, Criterion for micro-benchmarks, hyperfine for macro-benchmarks) is battle-tested and maps cleanly to our problem. We adapt it for shell scripts rather than Python files.

## Design

### Benchmark Corpus

We use the same 5 vendored benchmark fixtures from the Go frontend (`shuck/testdata/benchmarks/`), copied into `crates/shuck-benchmark/resources/`. These files are committed to the repo so benchmarks run without network access or corpus download steps. Using the identical fixture set ensures benchmark numbers are directly comparable between the Go and Rust implementations.

Each fixture is pinned to a specific upstream commit and includes its license file, following the Go frontend's `manifest.json` convention:

| File | Source | Commit | Lines | Bytes | License |
|------|--------|--------|-------|-------|---------|
| `homebrew-install.sh` | Homebrew/install `install.sh` | `6d5e267` | 1,175 | 33,212 | BSD-2-Clause |
| `nvm.sh` | nvm-sh/nvm `nvm.sh` | `977563e` | 4,661 | 150,227 | MIT |
| `pyenv-python-build.sh` | pyenv/pyenv `plugins/python-build/bin/python-build` | `8397a19` | 2,740 | 81,725 | MIT |
| `ruby-build.sh` | rbenv/ruby-build `bin/ruby-build` | `d099da0` | 1,643 | 47,738 | MIT |
| `fzf-install.sh` | junegunn/fzf `install` | `55d5b15` | 452 | 12,760 | MIT |

Total: 10,671 lines, 325,662 bytes.

The Go frontend also runs an "all" benchmark case that passes all 5 files in a single invocation. We replicate this in both the Criterion and hyperfine benchmarks.

### Crate: `shuck-benchmark`

A new workspace member crate, following ruff's `ruff_benchmark` structure:

```
crates/shuck-benchmark/
├── Cargo.toml
├── benches/
│   ├── lexer.rs          # Criterion: lex throughput per file
│   ├── parser.rs         # Criterion: parse throughput per file
│   ├── semantic.rs       # Criterion: semantic-model throughput per file
│   ├── unused_assignment.rs # Criterion: exact variable-dataflow throughput per file
│   └── linter.rs         # Criterion: full lint pipeline per file
├── src/
│   └── lib.rs            # TestCase, TestFile, load helpers
└── resources/
    ├── README.md          # Source, commit, license for each fixture
    ├── manifest.json      # Fixture metadata (matches Go frontend format)
    ├── files/
    │   ├── homebrew-install.sh
    │   ├── nvm.sh
    │   ├── pyenv-python-build.sh
    │   ├── ruby-build.sh
    │   └── fzf-install.sh
    └── licenses/
        ├── homebrew-install-LICENSE.txt
        ├── nvm-LICENSE.md
        ├── pyenv-LICENSE
        ├── ruby-build-LICENSE
        └── fzf-LICENSE
```

The `resources/` directory structure mirrors `shuck/testdata/benchmarks/` exactly, so files can be copied directly without renaming.

#### Cargo.toml

```toml
[package]
name = "shuck-benchmark"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
shuck-syntax = { path = "../shuck-syntax" }

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "lexer"
harness = false

[[bench]]
name = "parser"
harness = false

[[bench]]
name = "semantic"
harness = false

[[bench]]
name = "unused_assignment"
harness = false

[[bench]]
name = "linter"
harness = false
```

`unused_assignment.rs` is the dedicated Criterion harness for exact variable-dataflow work. It keeps the isolated `unused_assignment` and `uninitialized_reference` groups and also provides a combined `variable_dataflow_combined` group that measures the real shared-cache access pattern on one `SemanticAnalysis`.

Before/after exact-dataflow comparisons should use explicit Criterion baseline commands against that bench target:

```bash
env CARGO_TARGET_DIR="$SCRATCH/target" \
  cargo bench -p shuck-benchmark --bench unused_assignment \
  -- --save-baseline=exact-dataflow-before --noplot

env CARGO_TARGET_DIR="$SCRATCH/target" \
  cargo bench -p shuck-benchmark --bench unused_assignment \
  -- --baseline=exact-dataflow-before --noplot
```

#### src/lib.rs — Test Case Infrastructure

```rust
use std::path::{Path, PathBuf};

pub struct TestFile {
    pub name: &'static str,
    pub source: &'static str,
    pub speed: TestCaseSpeed,
}

/// Categorize files by expected benchmark duration to tune
/// Criterion sample sizes.
pub enum TestCaseSpeed {
    /// < 1ms expected — use default sample size
    Fast,
    /// 1–50ms expected — reduce sample size
    Normal,
    /// > 50ms expected — minimal sample size
    Slow,
}

impl TestCaseSpeed {
    pub fn sample_size(&self) -> usize {
        match self {
            Self::Fast => 100,
            Self::Normal => 20,
            Self::Slow => 5,
        }
    }
}

/// All vendored benchmark files — same set as the Go frontend's
/// testdata/benchmarks/manifest.json, ordered smallest to largest.
pub static TEST_FILES: &[TestFile] = &[
    TestFile {
        name: "fzf-install",
        source: include_str!("../resources/files/fzf-install.sh"),
        speed: TestCaseSpeed::Fast,
    },
    TestFile {
        name: "homebrew-install",
        source: include_str!("../resources/files/homebrew-install.sh"),
        speed: TestCaseSpeed::Fast,
    },
    TestFile {
        name: "ruby-build",
        source: include_str!("../resources/files/ruby-build.sh"),
        speed: TestCaseSpeed::Normal,
    },
    TestFile {
        name: "pyenv-python-build",
        source: include_str!("../resources/files/pyenv-python-build.sh"),
        speed: TestCaseSpeed::Normal,
    },
    TestFile {
        name: "nvm",
        source: include_str!("../resources/files/nvm.sh"),
        speed: TestCaseSpeed::Slow,
    },
];

pub fn resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources")
}
```

#### benches/parser.rs — Parser Micro-Benchmark

```rust
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use shuck_benchmark::{TestFile, TEST_FILES};
use shuck_syntax::parse;

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    for file in TEST_FILES {
        group.sample_size(file.speed.sample_size());
        group.throughput(Throughput::Bytes(file.source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("parse", file.name),
            &file.source,
            |b, source| {
                b.iter(|| parse(source));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
```

The lexer, semantic, and linter benches follow the same structure, benchmarking `Lexer::new(source).collect::<Vec<_>>()`, `SemanticModel::build(...)`, and the full `lint_file()` pipeline respectively.

#### Memory Allocator

Following ruff, pin the allocator in benchmark binaries to reduce noise:

```rust
#[cfg(not(target_os = "windows"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
```

Add `tikv-jemallocator` as an optional dev-dependency, gated behind a `jemalloc` feature that defaults to on for non-Windows targets.

### Macro-Benchmarks: Hyperfine Scripts

Shell scripts in `scripts/benchmarks/` for CLI-level comparison against shellcheck. These use the same vendored fixture files — no corpus download required.

#### scripts/benchmarks/setup.sh

Builds shuck and verifies shellcheck is available:

```bash
#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)

echo "Building shuck in release mode..."
cargo build --release -p shuck --manifest-path="$repo_root/Cargo.toml"

echo "Verifying shellcheck is installed..."
if ! command -v shellcheck >/dev/null 2>&1; then
    echo "ERROR: shellcheck not found. Install it first."
    exit 1
fi

echo "Setup complete."
echo "  shuck:      $(cargo run --release -p shuck -- --version 2>/dev/null || echo 'built')"
echo "  shellcheck: $(shellcheck --version | head -2 | tail -1)"
```

#### scripts/benchmarks/run.sh

Runs shuck vs shellcheck on each fixture file individually and on all files together, mirroring the Go frontend's `BenchmarkCLI` structure:

```bash
#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
fixtures_dir="$repo_root/crates/shuck-benchmark/resources/files"
shuck="$repo_root/target/release/shuck"

# Per-file benchmarks
for file in "$fixtures_dir"/*.sh; do
    name=$(basename "$file" .sh)
    echo "==> Benchmarking: $name"
    hyperfine \
        --warmup 3 \
        --runs 10 \
        --export-json "$repo_root/.cache/bench-${name}.json" \
        -n "shuck/$name" "$shuck check --no-cache $file" \
        -n "shellcheck/$name" "shellcheck --severity=style $file"
done

# All-files benchmark
echo "==> Benchmarking: all"
all_files=$(echo "$fixtures_dir"/*.sh)
hyperfine \
    --warmup 3 \
    --runs 10 \
    --export-json "$repo_root/.cache/bench-all.json" \
    --export-markdown "$repo_root/.cache/bench-all.md" \
    -n "shuck/all" "$shuck check --no-cache $all_files" \
    -n "shellcheck/all" "shellcheck --severity=style $all_files"
```

Note: The macro benchmark compares shuck against the default shellcheck CLI invocation. The `--no-cache` flag ensures shuck measures cold parse performance.

#### scripts/benchmarks/run_single.sh

Benchmarks a single file for focused comparison:

```bash
#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
shuck="$repo_root/target/release/shuck"
file="${1:?Usage: run_single.sh <path-to-script>}"

hyperfine \
    --warmup 5 \
    --runs 20 \
    -n "shuck" "$shuck check --no-cache $file" \
    -n "shellcheck" "shellcheck --severity=style $file"
```

### Makefile Targets

```makefile
bench:
	cargo bench -p shuck-benchmark

bench-save:
	cargo bench -p shuck-benchmark -- --save-baseline=main

bench-compare:
	cargo bench -p shuck-benchmark -- --baseline=main

bench-parser:
	cargo bench -p shuck-benchmark --bench parser

bench-lexer:
	cargo bench -p shuck-benchmark --bench lexer

bench-macro:
	./scripts/benchmarks/setup.sh
	./scripts/benchmarks/run.sh
```

### Profiling Support

A Cargo profile for benchmark profiling (symbols preserved, no LTO):

```toml
# workspace Cargo.toml
[profile.profiling]
inherits = "release"
strip = false
debug = "full"
lto = false
```

This enables `cargo flamegraph` and `samply` on benchmark binaries without losing symbol resolution.

### What We Measure

| Tier | Tool | What | Metric |
|------|------|------|--------|
| Micro | Criterion | Lex each fixture | throughput (bytes/s), time (mean ± σ) |
| Micro | Criterion | Parse each fixture | throughput (bytes/s), time (mean ± σ) |
| Micro | Criterion | Build semantics for each fixture | throughput (bytes/s), time (mean ± σ) |
| Micro | Criterion | Lint each fixture | throughput (bytes/s), time (mean ± σ) |
| Macro | Hyperfine | `shuck check` per fixture | wall time (mean ± σ), vs shellcheck |
| Macro | Hyperfine | `shuck check` all fixtures | wall time (mean ± σ), vs shellcheck |

### What We Don't Measure (Yet)

- **Memory usage** — Deferred until we have a clear memory regression concern. Can be added via `dhat` or jemalloc stats without changing the benchmark structure.
- **Incremental/cached performance** — The cache layer (shuck-cache) makes re-runs trivially fast. Benchmarking it is less useful than benchmarking cold parse performance.
- **CI integration** — No Codspeed or automated regression detection initially. Can be added once baselines are established and benchmark variance is understood.

## Alternatives Considered

### Alternative A: Criterion Only (No Hyperfine)

Run all benchmarks through Criterion, including CLI-level measurements.

**Rejected because:** Criterion measures library-level function calls, not CLI startup, file discovery, argument parsing, and I/O. Hyperfine captures the full user-facing experience and directly compares against shellcheck's CLI — the comparison users care about. The two tools measure different things and complement each other.

### Alternative B: Hyperfine Only (No Criterion)

Use only CLI-level benchmarks for simplicity.

**Rejected because:** CLI benchmarks conflate I/O, argument parsing, file discovery, and actual analysis. When a regression appears in hyperfine, you need Criterion-level data to know whether the lexer, parser, or linter is responsible. Ruff's experience shows that micro-benchmarks catch regressions that macro-benchmarks miss due to noise.

### Alternative C: Divan Instead of Criterion

Use the `divan` crate (simpler API, less boilerplate) for micro-benchmarks.

**Rejected because:** Criterion has broader ecosystem support, established baseline comparison workflows (`--save-baseline`/`--baseline`), HTML report generation, and better documentation. Divan is newer and doesn't yet support baseline comparison, which is the primary use case for catching regressions. If Codspeed CI integration is added later, criterion has a `codspeed-criterion-compat` drop-in.

### Alternative D: Use the Large Corpus (129 Repos) for Benchmarks

Use the `scripts/corpus-download.sh` output instead of the Go frontend's 5 fixtures.

**Rejected because:** The large corpus requires a ~10 minute download step, produces thousands of files of variable quality, and isn't deterministic across runs (repos change). The Go frontend's 5-fixture set is curated, pinned to exact commits, covers a good size range (12 KB to 150 KB), and — critically — lets us compare Rust vs Go benchmark numbers directly since both frontends measure the same files.

## Verification

Once implemented, verify with:

- **Micro-benchmarks run:** `cargo bench -p shuck-benchmark --bench parser` completes and produces Criterion output in `target/criterion/`.
- **All vendored files load:** Each `TestFile` in `TEST_FILES` has non-empty source (the `include_str!` calls compile).
- **Throughput reported:** Criterion output includes `throughput: X.XX MiB/s` for each benchmark.
- **Baseline comparison works:** Run `cargo bench -p shuck-benchmark -- --save-baseline=main`, make a change, then `cargo bench -p shuck-benchmark -- --baseline=main` shows a comparison.
- **Macro-benchmarks run:** `./scripts/benchmarks/run.sh` produces `bench-*.json` files with timing data for shuck and shellcheck.
- **Per-file and all-files:** The macro-benchmark output includes both individual fixture results and the combined "all" benchmark.
- **Single-file comparison:** `./scripts/benchmarks/run_single.sh crates/shuck-benchmark/resources/files/nvm.sh` produces hyperfine output comparing shuck vs shellcheck.
- **Make targets:** `make bench` and `make bench-macro` succeed.
