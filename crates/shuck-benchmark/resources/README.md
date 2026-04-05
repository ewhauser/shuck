# Benchmark Fixtures

This directory vendors the benchmark corpus from
`/Users/ewhauser/working/shuck/testdata/benchmarks` so the Rust workspace can
run repeatable benchmarks without any network or download step.

The fixtures, manifest, and license files are kept in the same layout as the Go
frontend:

- `manifest.json` pins each upstream source and commit
- `files/` contains the vendored shell scripts
- `licenses/` contains the corresponding upstream license texts
