# AGENTS.md

These instructions apply to `crates/shuck-benchmark`. Follow the repo-level
`AGENTS.md` first, including the clean-room policy.

## Memory and shape estimation tools

Use these examples when estimating the payoff from arena allocation, AST layout
changes, or fact/semantic storage changes. They are measurement tools, not
Criterion timing benchmarks.

```bash
cargo run -q -p shuck-benchmark --example parser_memory -- --case all
cargo run -q -p shuck-benchmark --example semantic_memory -- --case all
cargo run -q -p shuck-benchmark --example linter_memory -- --case all
cargo run -q -p shuck-benchmark --example formatter_memory -- --case all
cargo run -q -p shuck-benchmark --example ast_shape -- --case all
```

Pass any benchmark case name after `--case`, such as `nvm`,
`pyenv-python-build`, or `all`. The tools print JSON so results can be saved
and diffed between branches.

### Reading the reports

- `allocation_count`, `reallocation_count`, `total_allocated_bytes`, and
  `peak_live_bytes` are allocator-pressure signals. They are useful for sizing
  possible wins, not for wall-clock timing.
- `parser_memory` measures parsing and AST construction only.
- `semantic_memory` measures parse + indexer + semantic model construction.
- `linter_memory` reports both a full measured region and phase totals for
  parse, index/suppression setup, and linting. Phase `final_live_bytes` can be
  nonzero because later phases intentionally keep earlier outputs alive.
- `formatter_memory` separates source formatting from AST-only formatting, which
  helps distinguish parser/layout costs from formatter traversal/output costs.
- `ast_shape` counts structural pressure in the parsed AST: recursive nodes,
  non-empty `Vec` fields, `Box` edges, owned text fields, and an
  `estimated_replaceable_heap_allocations` rough-order signal.

Use the shape report together with allocator reports. For example, a large
number of non-empty AST `Vec` fields means compact child-range storage may help;
a large linter phase in `linter_memory` means AST arenas alone will not explain
most end-to-end lint allocation.

## Timing benchmarks

Use Criterion for wall-clock estimates and comparisons:

```bash
cargo bench -p shuck-benchmark --bench parser -- --save-baseline before
cargo bench -p shuck-benchmark --bench parser -- --baseline before
cargo bench -p shuck-benchmark --bench semantic -- semantic/all
cargo bench -p shuck-benchmark --bench linter -- linter/all
cargo bench -p shuck-benchmark --bench formatter -- formatter_source/all
```

Parser operation counters are available behind the parser benchmarking feature:

```bash
cargo run -q -p shuck-benchmark --features parser-benchmarking --example parser_counts -- all
```

## Validation

After changing benchmark helpers or examples, run:

```bash
cargo check -p shuck-benchmark --examples
cargo test -p shuck-benchmark
```
