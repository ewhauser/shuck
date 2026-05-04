# shuck-server perf notes

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
