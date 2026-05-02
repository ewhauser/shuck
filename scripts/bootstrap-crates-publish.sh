#!/usr/bin/env bash
# One-time bootstrap publish of every workspace crate to crates.io.
#
# Use this the first time the workspace is pushed to crates.io to claim the
# unpublished crate names. Once each name is claimed, configure Trusted
# Publishing (see setup-trusted-publishers.sh) and let the
# crates-publish.yml workflow handle subsequent releases.
#
# Usage:
#   CRATES_IO_TOKEN=<token> scripts/bootstrap-crates-publish.sh
#
# Token needs scopes: publish-new + publish-update.
#
# Env vars:
#   CRATES_IO_TOKEN   Required. Token value from crates.io → Account Settings.
#   DRY_RUN           Optional. If set to 1, runs `cargo publish --dry-run`.
#   SKIP_PUBLISHED    Optional. If set to 1, skips crates already on crates.io
#                     at the workspace version (instead of erroring on 409).

set -euo pipefail

if [ -z "${CRATES_IO_TOKEN:-}" ]; then
  echo "error: CRATES_IO_TOKEN must be set" >&2
  exit 1
fi

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

declare -a crates=()
while IFS= read -r crate; do
  crates+=("$crate")
done < <(python3 "$REPO_ROOT/scripts/workspace-publish-crates.py")

if [ "${#crates[@]}" -eq 0 ]; then
  echo "error: no publishable workspace crates found" >&2
  exit 1
fi

workspace_version="$(cargo metadata --format-version 1 --no-deps \
  | jq -r '
      .workspace_members as $members
      | [
          .packages[]
          | . as $package
          | select($members | index($package.id))
          | select(.publish != [])
          | .version
        ]
      | unique
      | if length == 1 then .[0] else empty end
    ')"
if [ -z "$workspace_version" ]; then
  echo "error: publishable workspace crates do not share one version" >&2
  exit 1
fi
echo "Workspace version: $workspace_version"

publish_flag=""
if [ "${DRY_RUN:-0}" = "1" ]; then
  # --no-verify skips the post-package rebuild, but Cargo still validates
  # registry dependency names during `publish --dry-run`. That means this
  # mode is only a partial packaging check when a new internal crate name
  # has not been uploaded to crates.io yet.
  publish_flag="--dry-run --no-verify"
  echo "DRY_RUN=1 → cargo publish --dry-run --no-verify (no upload; unpublished internal deps may still fail)"
fi

is_version_on_crates_io() {
  local crate="$1" ver="$2"
  curl -fsS "https://crates.io/api/v1/crates/${crate}/${ver}" \
    -o /dev/null 2>/dev/null
}

for crate in "${crates[@]}"; do
  echo
  echo "=== ${crate} ${workspace_version} ==="

  if [ "${SKIP_PUBLISHED:-0}" = "1" ] \
     && is_version_on_crates_io "$crate" "$workspace_version"; then
    echo "already on crates.io, skipping"
    continue
  fi

  CARGO_REGISTRY_TOKEN="$CRATES_IO_TOKEN" \
    cargo publish -p "$crate" --locked $publish_flag

  if [ -z "$publish_flag" ]; then
    # Index propagation is eventually consistent. Give it a moment so the
    # next crate (which depends on this one) resolves cleanly.
    sleep 20
  fi
done

echo
echo "Bootstrap publish complete."
echo "Next: scripts/setup-trusted-publishers.sh to wire up OIDC for CI."
