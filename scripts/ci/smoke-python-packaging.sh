#!/usr/bin/env bash
set -euo pipefail

: "${RUNNER_TEMP:?RUNNER_TEMP must be set}"
: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE must be set}"

expected_version="$(python3 -c 'import tomllib, pathlib; print(tomllib.loads(pathlib.Path("python/pyproject.toml").read_text())["project"]["version"])')"

python3 -m venv "$RUNNER_TEMP/wheel-smoke"
# shellcheck disable=SC1091
source "$RUNNER_TEMP/wheel-smoke/bin/activate"
python -m pip install --upgrade pip setuptools wheel
python -m pip install "$RUNNER_TEMP"/python-dist/shuck_cli-*manylinux_2_28_x86_64.whl
test "$(shuck --version)" = "shuck ${expected_version}"
deactivate

python3 -m venv "$RUNNER_TEMP/placeholder-smoke"
# shellcheck disable=SC1091
source "$RUNNER_TEMP/placeholder-smoke/bin/activate"
python -m pip install --upgrade pip setuptools wheel pre-commit
python -m pip install \
  --find-links "$RUNNER_TEMP/python-dist" \
  --no-build-isolation \
  --no-index \
  .
test "$(shuck --version)" = "shuck ${expected_version}"

smoke_repo="$RUNNER_TEMP/pre-commit-smoke"
rm -rf "$smoke_repo"
mkdir -p "$smoke_repo"
cd "$smoke_repo"
git init -q
git config user.email shuck@example.com
git config user.name shuck
printf '%s\n' "tmp=\$(mktemp)" 'echo ok' > bad.sh
git add bad.sh
hook_rev="$(git -C "$GITHUB_WORKSPACE" rev-parse HEAD)"
hook_repo="$RUNNER_TEMP/hook-repo"
rm -rf "$hook_repo"
git clone --no-local "$GITHUB_WORKSPACE" "$hook_repo" >/dev/null 2>&1
{
  echo 'repos:'
  echo "  - repo: $GITHUB_WORKSPACE"
  echo "    rev: $hook_rev"
  echo '    hooks:'
  echo '      - id: shuck'
} > .pre-commit-config.yaml

hook_status=0
PIP_FIND_LINKS="$RUNNER_TEMP/python-dist" \
  pre-commit run shuck --files bad.sh \
  > "$RUNNER_TEMP/pre-commit-smoke.log" 2>&1 || hook_status=$?

cat "$RUNNER_TEMP/pre-commit-smoke.log"
if [ "$hook_status" -eq 0 ]; then
  echo "expected shuck pre-commit hook to report a lint failure" >&2
  exit 1
fi

grep -F "warning[C001]" "$RUNNER_TEMP/pre-commit-smoke.log" >/dev/null

{
  echo 'repos:'
  echo "  - repo: $hook_repo"
  echo "    rev: $hook_rev"
  echo '    hooks:'
  echo '      - id: shuck-src'
} > .pre-commit-config.yaml

src_hook_status=0
pre-commit run shuck-src --files bad.sh \
  > "$RUNNER_TEMP/pre-commit-src-smoke.log" 2>&1 || src_hook_status=$?

cat "$RUNNER_TEMP/pre-commit-src-smoke.log"
if [ "$src_hook_status" -eq 0 ]; then
  echo "expected shuck-src pre-commit hook to report a lint failure" >&2
  exit 1
fi

grep -F "warning[C001]" "$RUNNER_TEMP/pre-commit-src-smoke.log" >/dev/null
