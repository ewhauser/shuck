#!/bin/sh
set -eu

fixture=${1:?Usage: profile_large_corpus.sh <fixture> [output-dir] [rate] [profile-iterations] [fixture-iterations]}
profile_root=${2:-.cache/profiles}
rate=${3:-1000}
profile_iterations=${4:-1}
fixture_iterations=${5:-1}

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
output_dir="$repo_root/$profile_root/large-corpus"

mkdir -p "$output_dir"

echo "Building large corpus profiling harness..."
cargo build \
    --profile profiling \
    -p shuck-benchmark \
    --features large-corpus-hotspots \
    --example large_corpus_profile \
    --manifest-path="$repo_root/Cargo.toml"

binary="$repo_root/target/profiling/examples/large_corpus_profile"
if [ ! -x "$binary" ]; then
    echo "ERROR: compiled harness not found at $binary"
    exit 1
fi

case_name=$(printf '%s' "$fixture" | tr '/:' '__')
output="$output_dir/$case_name.json.gz"
manifest="$output_dir/large-corpus-fixtures.tsv"

echo "Preparing large corpus fixture manifest outside the sampled process..."
"$binary" --write-fixture-manifest "$manifest"

echo "Recording large-corpus/$fixture to $output"
samply record \
    --save-only \
    --output "$output" \
    --rate "$rate" \
    --iteration-count "$profile_iterations" \
    --profile-name "large-corpus/$fixture" \
    -- \
    "$binary" "$fixture" \
        --iterations "$fixture_iterations" \
        --fixture-manifest "$manifest"

if [ "${SAMPLY_VIEW:-0}" = "1" ]; then
    echo "Opening profile in samply..."
    samply load "$output"
else
    echo "Saved profile: $output"
    echo "Open with: samply load $output"
fi
