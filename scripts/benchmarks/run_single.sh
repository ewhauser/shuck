#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
shuck="$repo_root/target/release/shuck"
file=${1:?Usage: run_single.sh <path-to-script>}
shuck_check="$shuck check --no-cache --select ALL"
shellcheck_check="shellcheck --enable=all --severity=style"

hyperfine \
    --ignore-failure \
    --warmup 5 \
    --runs 20 \
    -n "shuck" "$shuck_check $file" \
    -n "shellcheck" "$shellcheck_check $file"
