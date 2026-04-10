#!/usr/bin/env bash
set -euo pipefail

DRY_RUN=false
BUMP=""

usage() {
    echo "Usage: $0 [--dry-run] [major|minor|patch|VERSION]"
    echo
    echo "Bump the workspace version, commit, tag, and push."
    echo "Defaults to patch if no bump type is given."
    echo
    echo "Options:"
    echo "  --dry-run   Show what would happen without making changes"
    echo
    echo "Examples:"
    echo "  $0                # patch bump (default)"
    echo "  $0 minor          # 0.0.1 -> 0.1.0"
    echo "  $0 --dry-run      # preview a patch bump"
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            BUMP="$1"
            shift
            ;;
    esac
done

BUMP="${BUMP:-patch}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

# Read current version from workspace Cargo.toml
CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' "$CARGO_TOML" | head -1)
if [ -z "$CURRENT" ]; then
    echo "Error: could not read current version from $CARGO_TOML"
    exit 1
fi

IFS='.' read -r CUR_MAJOR CUR_MINOR CUR_PATCH <<< "$CURRENT"

case "$BUMP" in
    major)
        NEW_VERSION="$((CUR_MAJOR + 1)).0.0"
        ;;
    minor)
        NEW_VERSION="$CUR_MAJOR.$((CUR_MINOR + 1)).0"
        ;;
    patch)
        NEW_VERSION="$CUR_MAJOR.$CUR_MINOR.$((CUR_PATCH + 1))"
        ;;
    [0-9]*)
        if ! echo "$BUMP" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
            echo "Error: version must be in X.Y.Z format, got: $BUMP"
            exit 1
        fi
        NEW_VERSION="$BUMP"
        ;;
    *)
        usage
        ;;
esac

if [ "$NEW_VERSION" = "$CURRENT" ]; then
    echo "Error: new version ($NEW_VERSION) is the same as the current version"
    exit 1
fi

echo "Releasing: $CURRENT -> $NEW_VERSION"

if [ "$DRY_RUN" = true ]; then
    echo "(dry run — no changes made)"
    exit 0
fi

# Ensure working tree is clean
if ! git -C "$REPO_ROOT" diff --quiet || ! git -C "$REPO_ROOT" diff --cached --quiet; then
    echo "Error: working tree is dirty — commit or stash changes first"
    exit 1
fi

# Ensure we're on the main branch
BRANCH=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
    echo "Error: releases must be made from the main branch (currently on $BRANCH)"
    exit 1
fi

# Bump version in workspace Cargo.toml
sed -i '' "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"

# Commit and tag
git -C "$REPO_ROOT" add Cargo.toml
git -C "$REPO_ROOT" commit -m "release: v$NEW_VERSION"
git -C "$REPO_ROOT" tag "v$NEW_VERSION"

echo
echo "Pushing release v$NEW_VERSION..."
git -C "$REPO_ROOT" push
git -C "$REPO_ROOT" push --tags
