#!/usr/bin/env bash
# Creates CLAUDE.md symlinks pointing to AGENTS.md wherever AGENTS.md exists.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

find "$repo_root" -name AGENTS.md -not -path '*/.git/*' | while read -r agents_file; do
    dir="$(dirname "$agents_file")"
    claude_file="$dir/CLAUDE.md"

    if [ -L "$claude_file" ]; then
        continue
    fi

    if [ -f "$claude_file" ]; then
        rm "$claude_file"
    fi

    ln -s AGENTS.md "$claude_file"
    relative_path="$claude_file"
    if [[ $claude_file == "$repo_root/"* ]]; then
        relative_path=${claude_file:$(( ${#repo_root} + 1 ))}
    fi
    echo "Created symlink: $relative_path"
done
