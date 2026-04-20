#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
shuck="$repo_root/target/release/shuck"
file=${1:?Usage: run_single.sh <path-to-script>}

hyperfine \
    --ignore-failure \
    --warmup 5 \
    --runs 20 \
    -n "shuck" "$shuck check --no-cache $file" \
    -n "shellcheck" "shellcheck --severity=style $file"
