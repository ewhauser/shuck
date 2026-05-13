#!/bin/sh
set -eu

repo_root=${SHUCK_BENCHMARK_REPO_ROOT:-$(CDPATH="" cd -- "$(dirname "$0")/../.." && pwd)}
fixtures_dir=${SHUCK_BENCHMARK_FIXTURES_DIR:-"$repo_root/crates/shuck-benchmark/resources/files"}
target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
shuck=${SHUCK_BENCHMARK_SHUCK_BIN:-"$target_dir/release/shuck"}
cache_dir=${SHUCK_BENCHMARK_OUTPUT_DIR:-"$repo_root/.cache"}
benchmark_mode=${SHUCK_BENCHMARK_MODE:-compare}
shuck_check="$shuck check --no-cache --select ALL"
shellcheck_check="shellcheck --enable=all --severity=style"

mkdir -p "$cache_dir"

quote_shell_arg() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

append_shell_arg() {
    if [ -n "$1" ]; then
        printf '%s ' "$1"
    fi
    quote_shell_arg "$2"
}

shuck_fixtures=
shellcheck_fixtures=

for fixture in "$fixtures_dir"/*.sh "$fixtures_dir"/*.zsh; do
    if [ -f "$fixture" ]; then
        shuck_fixtures=$(append_shell_arg "$shuck_fixtures" "$fixture")
    fi
done
for fixture in "$fixtures_dir"/*.sh; do
    if [ -f "$fixture" ]; then
        shellcheck_fixtures=$(append_shell_arg "$shellcheck_fixtures" "$fixture")
    fi
done
if [ -z "$shuck_fixtures" ]; then
    echo "ERROR: no benchmark fixtures found in $fixtures_dir" >&2
    exit 1
fi

for file in "$fixtures_dir"/*.sh "$fixtures_dir"/*.zsh; do
    if [ ! -f "$file" ]; then
        continue
    fi
    name=$(basename "$file")
    name=${name%.sh}
    quoted_file=$(quote_shell_arg "$file")
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
                        -n "shuck/$name" "$shuck_check $quoted_file"
                    ;;
                *)
                    hyperfine \
                        --ignore-failure \
                        --warmup 3 \
                        --runs 10 \
                        --export-json "$cache_dir/bench-$name.json" \
                        -n "shuck/$name" "$shuck_check $quoted_file" \
                        -n "shellcheck/$name" "$shellcheck_check $quoted_file"
                    ;;
            esac
            ;;
        shuck-only)
            hyperfine \
                --ignore-failure \
                --warmup 3 \
                --runs 10 \
                --export-json "$cache_dir/bench-$name.json" \
                -n "shuck/$name" "$shuck_check $quoted_file"
            ;;
        *)
            echo "ERROR: unsupported SHUCK_BENCHMARK_MODE=$benchmark_mode" >&2
            exit 1
            ;;
    esac
done

echo "==> Benchmarking: all"
case "$benchmark_mode" in
    compare)
        if [ -n "$shellcheck_fixtures" ]; then
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
