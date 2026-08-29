# shuck-server perf notes

## Criterion LSP suite

Run the end-to-end in-memory LSP benchmarks with:

```bash
make bench-lsp
```

The suite measures warm and invalidated pull diagnostics for a 5 KiB open
document, a one-character incremental edit followed by diagnostics, warm hover,
completion, and document-symbol requests, and warm versus invalidated workspace
diagnostics across 32 closed files. These measurements include server request
scheduling, snapshot/cache behavior, handler work, and response serialization.
The lower-level `editor` benchmark remains the source for isolated semantic
query costs.

Use Criterion baselines for regression comparisons rather than committing a
machine-specific latency threshold:

```bash
cargo bench -p shuck-benchmark --bench lsp -- --save-baseline=lsp-main
cargo bench -p shuck-benchmark --bench lsp -- --baseline=lsp-main
```

## Historical measurements

Recorded on 2026-05-03 from `/Users/ewhauser/working/shuck-lsp`.

## Benchmark gate

Command:

```bash
cargo bench -p shuck-benchmark --bench check_command
```

Result:

- Completed successfully.
- Representative `check_command_full/all` sample: `444.64 ms .. 446.03 ms`
- Representative `check_command_concise/all` sample: `444.40 ms .. 448.09 ms`

## Pull diagnostics latency

Command:

```bash
cargo test -p shuck-server --release --test latency measure_pull_diagnostics_round_trip -- --ignored --nocapture
```

Result:

- `pull diagnostics round-trip: 18.143 ms for 5120 bytes (1 diagnostics)`
