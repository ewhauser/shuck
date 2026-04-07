#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
fixtures_dir="$repo_root/crates/shuck-benchmark/resources/files"
shuck="$repo_root/target/release/shuck"
cache_dir="$repo_root/.cache"

mkdir -p "$cache_dir"

for file in "$fixtures_dir"/*.sh; do
    name=$(basename "$file" .sh)
    echo "==> Benchmarking formatter: $name"
    hyperfine \
        --ignore-failure \
        --warmup 3 \
        --runs 10 \
        --export-json "$cache_dir/bench-format-$name.json" \
        -n "shuck-format/$name" "$shuck format --check --no-cache --dialect bash $file" \
        -n "shfmt/$name" "shfmt -l -ln=bash $file"
done

set -- "$fixtures_dir"/*.sh

echo "==> Benchmarking formatter: all"
hyperfine \
    --ignore-failure \
    --warmup 3 \
    --runs 10 \
    --export-json "$cache_dir/bench-format-all.json" \
    --export-markdown "$cache_dir/bench-format-all.md" \
    -n "shuck-format/all" "$shuck format --check --no-cache --dialect bash $*" \
    -n "shfmt/all" "shfmt -l -ln=bash $*"
