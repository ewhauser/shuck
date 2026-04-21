#!/bin/sh
set -eu

repo_root=${SHUCK_BENCHMARK_REPO_ROOT:-$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)}
target_dir=${CARGO_TARGET_DIR:-"$repo_root/target"}
shuck=${SHUCK_BENCHMARK_SHUCK_BIN:-"$target_dir/release/shuck"}

if [ "$#" -eq 0 ]; then
    set -- hyperfine shellcheck
fi

echo "Building shuck in release mode..."
cargo build --release -p shuck-cli --manifest-path="$repo_root/Cargo.toml"

echo "Verifying benchmark dependencies..."
for binary in "$@"; do
    if ! command -v "$binary" >/dev/null 2>&1; then
        echo "ERROR: $binary not found. Install it first."
        exit 1
    fi
done

echo "Setup complete."
echo "  shuck:      $("${shuck}" --version 2>/dev/null || echo 'built')"
for binary in "$@"; do
    case "$binary" in
        hyperfine)
            echo "  hyperfine:  $(hyperfine --version | head -1)"
            ;;
        shellcheck)
            echo "  shellcheck: $(shellcheck --version | awk 'NR==2 { print; exit }')"
            ;;
        shfmt)
            echo "  shfmt:      $(shfmt --version | head -1)"
            ;;
    esac
done
