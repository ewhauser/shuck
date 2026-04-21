#!/usr/bin/env bash
# Configure crates.io Trusted Publishing for every publishable workspace crate.
#
# Run once after bootstrap-crates-publish.sh has claimed the crate names.
# Re-run whenever a new publishable crate is added to the workspace.
#
# Usage:
#   CRATES_IO_TOKEN=<token> scripts/setup-trusted-publishers.sh
#
# Token needs the `trusted-publishing` scope (create it at crates.io →
# Account Settings → API Tokens). The caller must be a verified owner of
# each crate below.
#
# Env vars:
#   CRATES_IO_TOKEN      Required. API token with trusted-publishing scope.
#   REPOSITORY_OWNER     Default: ewhauser
#   REPOSITORY_NAME      Default: shuck
#   WORKFLOW_FILENAME    Default: crates-publish.yml
#   ENVIRONMENT          Default: release   (set empty to omit)

set -euo pipefail

if [ -z "${CRATES_IO_TOKEN:-}" ]; then
  echo "error: CRATES_IO_TOKEN must be set" >&2
  exit 1
fi

REPOSITORY_OWNER="${REPOSITORY_OWNER:-ewhauser}"
REPOSITORY_NAME="${REPOSITORY_NAME:-shuck}"
WORKFLOW_FILENAME="${WORKFLOW_FILENAME:-crates-publish.yml}"
ENVIRONMENT="${ENVIRONMENT-release}"

crates=(
  shuck-ast
  shuck-format
  shuck-parser
  shuck-indexer
  shuck-cache
  shuck-semantic
  shuck-linter
  shuck-formatter
  shuck-cli
)

configured=0
skipped=0
failed=0

for crate in "${crates[@]}"; do
  echo
  echo "=== ${crate} ==="

  payload="$(jq -n \
    --arg krate "$crate" \
    --arg owner "$REPOSITORY_OWNER" \
    --arg repo "$REPOSITORY_NAME" \
    --arg workflow "$WORKFLOW_FILENAME" \
    --arg env "$ENVIRONMENT" \
    '{github_config: ({
        krate: $krate,
        repository_owner: $owner,
        repository_name: $repo,
        workflow_filename: $workflow
      } + (if $env == "" then {} else {environment: $env} end))}')"

  response="$(mktemp)"
  status=$(curl -sS -o "$response" -w "%{http_code}" \
    -X POST "https://crates.io/api/v1/trusted_publishing/github_configs" \
    -H "Authorization: $CRATES_IO_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$payload")

  case "$status" in
    200|201)
      id=$(jq -r '.github_config.id // .id // "?"' "$response")
      echo "configured (id=$id)"
      configured=$((configured + 1))
      ;;
    409|422)
      msg=$(jq -r '.errors[0].detail // .error // empty' "$response" 2>/dev/null || true)
      if echo "$msg" | grep -qi 'already'; then
        echo "already configured, skipping"
        skipped=$((skipped + 1))
      else
        echo "HTTP $status: $msg"
        cat "$response"
        failed=$((failed + 1))
      fi
      ;;
    *)
      echo "HTTP $status"
      cat "$response"
      failed=$((failed + 1))
      ;;
  esac

  rm -f "$response"
done

echo
printf 'configured: %d  skipped: %d  failed: %d\n' "$configured" "$skipped" "$failed"
if [ "$failed" -gt 0 ]; then
  exit 1
fi
