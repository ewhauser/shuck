#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
shuck="$repo_root/target/release/shuck"
file=${1:?Usage: run_formatter_single.sh <path-to-script>}

hyperfine \
    --ignore-failure \
    --warmup 5 \
    --runs 20 \
    -n "shuck-format" "$shuck format --check --no-cache --dialect bash $file" \
    -n "shfmt" "shfmt -l -ln=bash $file"
