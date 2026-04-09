#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
shuck="$repo_root/target/release/shuck"
timeout_runner="$repo_root/scripts/benchmarks/with_timeout.sh"
timeout_secs=${SHUCK_FORMAT_BENCH_TIMEOUT_SECS:-3}
file=${1:?Usage: run_formatter_single.sh <path-to-script>}

hyperfine \
    --ignore-failure \
    --warmup 5 \
    --runs 20 \
    -n "shuck-format" "$timeout_runner $timeout_secs $shuck format --check --no-cache --dialect bash $file" \
    -n "shfmt" "$timeout_runner $timeout_secs shfmt -l -ln=bash $file"
