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

shuck_fixture_list=$(
    for fixture in "$fixtures_dir"/*.sh "$fixtures_dir"/*.zsh; do
        if [ -f "$fixture" ]; then
            printf '%s\n' "$fixture"
        fi
    done | sort
)
shellcheck_fixture_list=$(
    for fixture in "$fixtures_dir"/*.sh; do
        if [ -f "$fixture" ]; then
            printf '%s\n' "$fixture"
        fi
    done | sort
)
if [ -z "$shuck_fixture_list" ]; then
    echo "ERROR: no benchmark fixtures found in $fixtures_dir" >&2
    exit 1
fi

for file in $shuck_fixture_list; do
    name=$(basename "$file")
    name=${name%.*}
    echo "==> Benchmarking: $name"
    case "$benchmark_mode" in
        compare)
            case "$file" in
                *.zsh)
                    hyperfine \
                        --ignore-failure \
                        --warmup 3 \
                        --runs 10 \
                        --export-json "$cache_dir/bench-$name.json" \
                        -n "shuck/$name" "$shuck_check $file"
                    ;;
                *)
                    hyperfine \
                        --ignore-failure \
                        --warmup 3 \
                        --runs 10 \
                        --export-json "$cache_dir/bench-$name.json" \
                        -n "shuck/$name" "$shuck_check $file" \
                        -n "shellcheck/$name" "$shellcheck_check $file"
                    ;;
            esac
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

set -- $shuck_fixture_list
shuck_fixtures=$*

echo "==> Benchmarking: all"
case "$benchmark_mode" in
    compare)
        if [ -n "$shellcheck_fixture_list" ]; then
            set -- $shellcheck_fixture_list
            shellcheck_fixtures=$*
            hyperfine \
                --ignore-failure \
                --warmup 3 \
                --runs 10 \
                --export-json "$cache_dir/bench-all.json" \
                --export-markdown "$cache_dir/bench-all.md" \
                -n "shuck/all" "$shuck_check $shuck_fixtures" \
                -n "shellcheck/all" "$shellcheck_check $shellcheck_fixtures"
        else
            hyperfine \
                --ignore-failure \
                --warmup 3 \
                --runs 10 \
                --export-json "$cache_dir/bench-all.json" \
                --export-markdown "$cache_dir/bench-all.md" \
                -n "shuck/all" "$shuck_check $shuck_fixtures"
        fi
        ;;
    shuck-only)
        hyperfine \
            --ignore-failure \
            --warmup 3 \
            --runs 10 \
            --export-json "$cache_dir/bench-all.json" \
            --export-markdown "$cache_dir/bench-all.md" \
            -n "shuck/all" "$shuck_check $shuck_fixtures"
        ;;
    *)
        echo "ERROR: unsupported SHUCK_BENCHMARK_MODE=$benchmark_mode" >&2
        exit 1
        ;;
esac
