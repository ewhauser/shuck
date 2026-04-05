#!/bin/sh
set -eu

target_file=${1:?Usage: profile_cli.sh <script-path> [output-dir] [rate] [iterations]}
profile_root=${2:-.cache/profiles}
rate=${3:-1000}
iterations=${4:-1}

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
output_dir="$repo_root/$profile_root/cli"

case "$target_file" in
    /*) script_path=$target_file ;;
    *) script_path="$repo_root/$target_file" ;;
esac

if [ ! -f "$script_path" ]; then
    echo "ERROR: script not found: $script_path"
    exit 1
fi

mkdir -p "$output_dir"

echo "Building shuck CLI with profiling profile..."
cargo build --profile profiling -p shuck --manifest-path="$repo_root/Cargo.toml"

binary="$repo_root/target/profiling/shuck"
if [ ! -x "$binary" ]; then
    echo "ERROR: compiled shuck binary not found at $binary"
    exit 1
fi

case_name=$(basename "$script_path")
output="$output_dir/$case_name.json.gz"

echo "Recording cli/$case_name to $output"
samply record \
    --save-only \
    --output "$output" \
    --rate "$rate" \
    --iteration-count "$iterations" \
    --profile-name "cli/$case_name" \
    -- \
    "$binary" check --no-cache "$script_path"

if [ "${SAMPLY_VIEW:-0}" = "1" ]; then
    echo "Opening profile in samply..."
    samply load "$output"
else
    echo "Saved profile: $output"
    echo "Open with: samply load $output"
fi
