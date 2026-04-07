---
name: bench-compare
description: >
  Compare benchmark performance between two git worktrees (or the current
  worktree vs main). Runs Criterion microbenchmarks and hyperfine macrobenchmarks,
  extracts deltas, and drills down into regressions. Use this skill whenever the
  user asks to compare benchmarks, check for performance regressions, benchmark
  their branch against main, run a perf comparison, or says things like "bench
  compare", "any regressions?", "compare perf", "how does this branch perform",
  "run benchmarks against main". Even if the user just says "benchmarks" in the
  context of a feature branch or worktree, this skill applies.
---

# Benchmark Comparison

Compare shuck's benchmark performance between two worktrees (typically the current
feature worktree and the main worktree). Produces a delta report covering both
Criterion microbenchmarks (lexer, parser, semantic, linter) and hyperfine
macrobenchmarks (wall-clock comparison against ShellCheck).

## Before you start

Identify the two worktrees to compare:

```bash
git worktree list
```

This gives you:
- **Main worktree** — the first entry, typically `/Users/.../shuck`
- **Current worktree** — the one you're running in (may be the same as main)

If both are the same directory, you'll need to stash or commit changes and use
`--save-baseline` / `--baseline` on the same tree. But the typical case is two
separate worktrees.

Also check what benchmark targets exist — the registered Criterion benches may
change over time:

```bash
grep -A1 '^\[\[bench\]\]' crates/shuck-benchmark/Cargo.toml | grep '^name'
```

## Step 1: Create a shared scratch directory

Both worktrees must write Criterion artifacts to the same `target/criterion/`
directory for baseline comparison to work. Create a temp directory and point
`CARGO_TARGET_DIR` at it.

```bash
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/shuck-bench-compare.XXXXXX")
export CARGO_TARGET_DIR="$SCRATCH/target"
echo "Scratch: $SCRATCH"
```

Also create subdirectories for macrobenchmark exports:

```bash
mkdir -p "$SCRATCH/main-macro" "$SCRATCH/current-macro"
```

## Step 2: Run Criterion baseline on main

`cd` into the main worktree and run Criterion with `--save-baseline=main`.

**Important:** Do NOT use bare `cargo bench -p shuck-benchmark -- --save-baseline=main`.
That trips over the crate lib harness and fails. You must enumerate each bench target
explicitly with `--bench`:

```bash
cd /path/to/main/worktree
env CARGO_TARGET_DIR="$SCRATCH/target" \
  cargo bench -p shuck-benchmark \
    --bench lexer --bench lexer_hot_path --bench parser --bench semantic --bench linter \
    -- --save-baseline=main --noplot \
  > "$SCRATCH/main-criterion.log" 2>&1
```

Monitor progress by tailing the log. This typically takes 3-8 minutes depending on
the machine.

If additional bench targets exist (check Cargo.toml), add them to the `--bench` list.

## Step 3: Run Criterion comparison on current worktree

`cd` into the current worktree and run Criterion with `--baseline=main` (note: not
`--save-baseline`). This compares against the baseline saved in Step 2.

```bash
cd /path/to/current/worktree
env CARGO_TARGET_DIR="$SCRATCH/target" \
  cargo bench -p shuck-benchmark \
    --bench lexer --bench lexer_hot_path --bench parser --bench semantic --bench linter \
    -- --baseline=main --noplot \
  > "$SCRATCH/current-criterion.log" 2>&1
```

## Step 4: Run macrobenchmarks on both worktrees

Macrobenchmarks use hyperfine via `scripts/benchmarks/run.sh`, which requires
`hyperfine` and `shellcheck` — tools only available inside the nix dev shell.

For each worktree (main first, then current):

```bash
cd /path/to/worktree

# Clear stale bench exports to avoid mixing results
rm -f .cache/bench-*.json .cache/bench-*.md 2>/dev/null || true

# Build and verify deps
nix --extra-experimental-features 'nix-command flakes' develop --command \
  ./scripts/benchmarks/setup.sh > "$SCRATCH/{side}-macro-setup.log" 2>&1

# Run hyperfine comparisons
nix --extra-experimental-features 'nix-command flakes' develop --command \
  ./scripts/benchmarks/run.sh > "$SCRATCH/{side}-macro.log" 2>&1

# Copy exports to scratch
cp .cache/bench-*.json "$SCRATCH/{side}-macro/"
```

Replace `{side}` with `main` or `current` as appropriate.

## Step 5: Extract and report deltas

### Criterion deltas

Criterion stores change estimates at:
`$CARGO_TARGET_DIR/criterion/{group}/{case}/change/estimates.json`

Extract them with a Python script:

```python
import json, pathlib

ROOT = pathlib.Path("$SCRATCH")
crit = ROOT / "target" / "criterion"
rows = []

for group in sorted(p for p in crit.iterdir() if p.is_dir()):
    for case in sorted(p for p in group.iterdir() if p.is_dir()):
        try:
            base = json.loads((case / "main" / "estimates.json").read_text())["mean"]["point_estimate"]
            new = json.loads((case / "new" / "estimates.json").read_text())["mean"]["point_estimate"]
            change = json.loads((case / "change" / "estimates.json").read_text())["mean"]["point_estimate"] * 100
        except FileNotFoundError:
            continue
        rows.append((group.name, case.name, base, new, change))

print("=== Criterion Summary (group-level) ===")
for group, case, base, new, change in rows:
    if case == "all":
        sign = "+" if change > 0 else ""
        print(f"  {group:30s}  {base:12.0f} → {new:12.0f} ns  ({sign}{change:.2f}%)")

print("\n=== Top Regressions ===")
for group, case, base, new, change in sorted(rows, key=lambda r: r[4], reverse=True)[:10]:
    print(f"  {group}/{case:20s}  {change:+.2f}%")

print("\n=== Top Improvements ===")
for group, case, base, new, change in sorted(rows, key=lambda r: r[4])[:10]:
    print(f"  {group}/{case:20s}  {change:+.2f}%")
```

### Macrobenchmark deltas

Hyperfine exports JSON with a `results` array. Each result has `command` and `mean`
(in seconds). Compare the shuck entries between main and current:

```python
for p in sorted((ROOT / "main-macro").glob("bench-*.json")):
    name = p.stem.removeprefix("bench-")
    main_results = json.loads(p.read_text())["results"]
    cur_results = json.loads((ROOT / "current-macro" / p.name).read_text())["results"]
    m_shuck = next(r for r in main_results if r["command"].startswith("shuck/"))
    c_shuck = next(r for r in cur_results if r["command"].startswith("shuck/"))
    change = (c_shuck["mean"] / m_shuck["mean"] - 1) * 100
    print(f"  {name:30s}  {m_shuck['mean']*1000:.1f} → {c_shuck['mean']*1000:.1f} ms  ({change:+.2f}%)")
```

## Step 6: Interpret the results

Present a summary table to the user covering:
- Each Criterion benchmark group with % change
- Each macrobenchmark fixture with % change
- Highlight anything above +3% as a potential regression (Criterion has noise, so
  small changes are usually not meaningful)

If no regressions are found, report that and stop.

## Step 7: Drill down into regressions

If a significant regression is found (>3% in Criterion or >5% in macrobenchmarks),
investigate the root cause:

### 7a. Identify which component regressed

The Criterion benchmarks isolate stages: lexer, parser, semantic, linter. If only
`linter` regressed but `parser` and `semantic` are flat, the regression is in the
linting pipeline (checker, suppression, directives).

### 7b. Build a breakdown harness (if needed)

For finer-grained measurement, create a temporary
`crates/shuck-benchmark/examples/lint_breakdown.rs` that measures individual pipeline
stages:

1. Parse only
2. Indexer only
3. Directives/suppression only
4. Semantic model build only
5. Lint with empty rules
6. Lint with default rules

Run it on both worktrees with `cargo run -p shuck-benchmark --release --example lint_breakdown`
and compare the per-stage timings. This isolates which stage accounts for the
regression.

### 7c. Diff the suspected code

Once you've narrowed to a stage, diff the relevant source files between worktrees:

```bash
git diff --no-index /path/to/main/crates/... /path/to/current/crates/...
```

Look for:
- New `.clone()` calls on hot paths
- Recursive traversals that weren't there before
- Changed data structures (Vec → HashMap, etc.)
- New allocations in tight loops

### 7d. Report findings

Tell the user:
- Which benchmark regressed and by how much
- Which code change caused it (with file path and description)
- Whether you have a fix suggestion

Remove any temporary breakdown harness files after the investigation.

## Gotchas

- **Explicit --bench flags**: Always enumerate bench targets individually
  (`--bench lexer --bench parser ...`). The bare `cargo bench -p shuck-benchmark`
  form trips over the crate lib harness when passing Criterion flags like
  `--save-baseline`.

- **Macrobenchmarks need nix**: `hyperfine` and `shellcheck` are only available
  inside `nix develop`. Always run macro setup and benchmarks through
  `nix --extra-experimental-features 'nix-command flakes' develop --command ...`.

- **Shared CARGO_TARGET_DIR**: Both worktrees must use the same target directory
  for Criterion baseline comparison to work. This is the whole reason for the
  scratch directory.

- **Suppression-heavy fixtures**: `ruby-build.sh` and `nvm.sh` have many
  `shellcheck disable=` comments. Regressions in the directive/suppression parsing
  path show up disproportionately on these fixtures.

- **zsh glob errors**: When clearing `.cache/bench-*.json`, use
  `rm -f ... 2>/dev/null || true` because zsh complains when no files match a glob.

- **Benchmark noise**: Criterion microbenchmarks on a laptop can have 2-3% noise.
  Don't chase regressions under 3% unless they're consistent across all fixtures.
  Macrobenchmarks (hyperfine) are more stable since they measure full CLI invocations.
