#!/usr/bin/env bash
# Fails if a crates/* directory is not mentioned in the root AGENTS.md.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
missing=0

for dir in "$repo_root"/crates/*/; do
    name="$(basename "$dir")"
    if ! grep -qF "crates/$name" "$repo_root/AGENTS.md"; then
        echo "AGENTS.md does not mention crates/$name" >&2
        missing=1
    fi
done

exit "$missing"
