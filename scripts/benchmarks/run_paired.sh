#!/bin/sh
# Time a baseline and a head shuck binary against the same fixtures on the
# same machine. Each fixture (and the combined "all" case) is benchmarked in
# several rounds; every round runs both binaries in one hyperfine invocation
# and the order flips between rounds, so slow drift on the host lands on both
# sides instead of biasing one of them.
#
# Results land in $SHUCK_BENCHMARK_OUTPUT_DIR/round-<n>/bench-<case>.json with
# commands named base/<case> and head/<case>.
set -eu

repo_root=${SHUCK_BENCHMARK_REPO_ROOT:-$(CDPATH="" cd -- "$(dirname "$0")/../.." && pwd)}
fixtures_dir=${SHUCK_BENCHMARK_FIXTURES_DIR:-"$repo_root/crates/shuck-benchmark/resources/files"}
output_dir=${SHUCK_BENCHMARK_OUTPUT_DIR:-"$repo_root/.cache/paired"}
base_shuck=${SHUCK_BENCHMARK_BASE_BIN:?SHUCK_BENCHMARK_BASE_BIN must point at the baseline shuck binary}
head_shuck=${SHUCK_BENCHMARK_HEAD_BIN:?SHUCK_BENCHMARK_HEAD_BIN must point at the head shuck binary}
rounds=${SHUCK_BENCHMARK_ROUNDS:-3}
runs=${SHUCK_BENCHMARK_RUNS:-10}
warmup=${SHUCK_BENCHMARK_WARMUP:-3}

for binary in "$base_shuck" "$head_shuck"; do
    if [ ! -x "$binary" ]; then
        echo "ERROR: shuck binary not found or not executable: $binary" >&2
        exit 1
    fi
done

quote_shell_arg() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

append_shell_arg() {
    if [ -n "$1" ]; then
        printf '%s ' "$1"
    fi
    quote_shell_arg "$2"
}

base_check="$(quote_shell_arg "$base_shuck") check --no-cache --select ALL"
head_check="$(quote_shell_arg "$head_shuck") check --no-cache --select ALL"

all_fixtures=
for fixture in "$fixtures_dir"/*.sh "$fixtures_dir"/*.zsh; do
    if [ -f "$fixture" ]; then
        all_fixtures=$(append_shell_arg "$all_fixtures" "$fixture")
    fi
done
if [ -z "$all_fixtures" ]; then
    echo "ERROR: no benchmark fixtures found in $fixtures_dir" >&2
    exit 1
fi

# run_pair <round> <case> <quoted fixture args>
run_pair() {
    round_dir="$output_dir/round-$1"
    mkdir -p "$round_dir"
    base_args="$base_check $3"
    head_args="$head_check $3"
    if [ $(($1 % 2)) -eq 1 ]; then
        hyperfine \
            --ignore-failure \
            --warmup "$warmup" \
            --runs "$runs" \
            --export-json "$round_dir/bench-$2.json" \
            -n "base/$2" "$base_args" \
            -n "head/$2" "$head_args"
    else
        hyperfine \
            --ignore-failure \
            --warmup "$warmup" \
            --runs "$runs" \
            --export-json "$round_dir/bench-$2.json" \
            -n "head/$2" "$head_args" \
            -n "base/$2" "$base_args"
    fi
}

mkdir -p "$output_dir"

round=1
while [ "$round" -le "$rounds" ]; do
    for file in "$fixtures_dir"/*.sh "$fixtures_dir"/*.zsh; do
        if [ ! -f "$file" ]; then
            continue
        fi
        name=$(basename "$file")
        name=${name%.sh}
        echo "==> Round $round/$rounds: $name"
        run_pair "$round" "$name" "$(quote_shell_arg "$file")"
    done
    echo "==> Round $round/$rounds: all"
    run_pair "$round" all "$all_fixtures"
    round=$((round + 1))
done
