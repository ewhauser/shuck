#!/bin/sh
set -eu

repo_root=${SHUCK_BENCHMARK_REPO_ROOT:-$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)}
fixtures_dir=${SHUCK_BENCHMARK_FIXTURES_DIR:-"$repo_root/crates/shuck-benchmark/resources/files"}
target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
shuck=${SHUCK_BENCHMARK_SHUCK_BIN:-"$target_dir/release/shuck"}
cache_dir=${SHUCK_BENCHMARK_OUTPUT_DIR:-"$repo_root/.cache"}
benchmark_mode=${SHUCK_BENCHMARK_MODE:-compare}
shuck_check="$shuck check --no-cache --select ALL"
shellcheck_check="shellcheck --enable=all --severity=style"

mkdir -p "$cache_dir"

for file in "$fixtures_dir"/*.sh; do
    name=$(basename "$file" .sh)
    echo "==> Benchmarking: $name"
    case "$benchmark_mode" in
        compare)
            hyperfine \
                --ignore-failure \
                --warmup 3 \
                --runs 10 \
                --export-json "$cache_dir/bench-$name.json" \
                -n "shuck/$name" "$shuck_check $file" \
                -n "shellcheck/$name" "$shellcheck_check $file"
            ;;
        shuck-only)
            hyperfine \
                --ignore-failure \
                --warmup 3 \
                --runs 10 \
                --export-json "$cache_dir/bench-$name.json" \
                -n "shuck/$name" "$shuck_check $file"
            ;;
        *)
            echo "ERROR: unsupported SHUCK_BENCHMARK_MODE=$benchmark_mode" >&2
            exit 1
            ;;
    esac
done

set -- "$fixtures_dir"/*.sh

echo "==> Benchmarking: all"
case "$benchmark_mode" in
    compare)
        hyperfine \
            --ignore-failure \
            --warmup 3 \
            --runs 10 \
            --export-json "$cache_dir/bench-all.json" \
            --export-markdown "$cache_dir/bench-all.md" \
            -n "shuck/all" "$shuck_check $*" \
            -n "shellcheck/all" "$shellcheck_check $*"
        ;;
    shuck-only)
        hyperfine \
            --ignore-failure \
            --warmup 3 \
            --runs 10 \
            --export-json "$cache_dir/bench-all.json" \
            --export-markdown "$cache_dir/bench-all.md" \
            -n "shuck/all" "$shuck_check $*"
        ;;
    *)
        echo "ERROR: unsupported SHUCK_BENCHMARK_MODE=$benchmark_mode" >&2
        exit 1
        ;;
esac
