#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)

echo "Building shuck in release mode..."
cargo build --release -p shuck --manifest-path="$repo_root/Cargo.toml"

echo "Verifying benchmark dependencies..."
for binary in hyperfine shellcheck; do
    if ! command -v "$binary" >/dev/null 2>&1; then
        echo "ERROR: $binary not found. Install it first."
        exit 1
    fi
done

echo "Setup complete."
echo "  shuck:      $("$repo_root/target/release/shuck" --version 2>/dev/null || echo 'built')"
echo "  shellcheck: $(shellcheck --version | awk 'NR==2 { print; exit }')"
echo "  hyperfine:  $(hyperfine --version | head -1)"
