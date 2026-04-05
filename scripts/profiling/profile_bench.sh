#!/bin/sh
set -eu

bench_name=${1:?Usage: profile_bench.sh <parser|linter> [case] [output-dir] [rate] [iterations]}
case_name=${2:-nvm}
profile_root=${3:-.cache/profiles}
rate=${4:-1000}
iterations=${5:-1}

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
output_dir="$repo_root/$profile_root/$bench_name"

mkdir -p "$output_dir"

echo "Building $bench_name benchmark with profiling profile..."
cargo build --profile profiling -p shuck-benchmark --bench "$bench_name" --manifest-path="$repo_root/Cargo.toml"

binary=$(
    find "$repo_root/target/profiling/deps" \
        -maxdepth 1 \
        -type f \
        -perm -111 \
        -name "$bench_name-*" \
        -exec ls -t {} + 2>/dev/null \
        | head -n 1
)

if [ -z "$binary" ]; then
    echo "ERROR: could not locate compiled $bench_name bench binary under target/profiling/deps"
    exit 1
fi

output="$output_dir/$case_name.json.gz"

echo "Recording $bench_name/$case_name to $output"
samply record \
    --save-only \
    --output "$output" \
    --rate "$rate" \
    --iteration-count "$iterations" \
    --profile-name "$bench_name/$case_name" \
    -- \
    "$binary" "$case_name" --noplot

if [ "${SAMPLY_VIEW:-0}" = "1" ]; then
    echo "Opening profile in samply..."
    samply load "$output"
else
    echo "Saved profile: $output"
    echo "Open with: samply load $output"
fi
