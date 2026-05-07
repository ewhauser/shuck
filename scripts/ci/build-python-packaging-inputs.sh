#!/usr/bin/env bash
set -euo pipefail

: "${RUNNER_TEMP:?RUNNER_TEMP must be set}"

cargo build -p shuck-cli --locked

python3 -m venv "$RUNNER_TEMP/python-packaging"
# shellcheck disable=SC1091
source "$RUNNER_TEMP/python-packaging/bin/activate"
python -m pip install --upgrade pip
python -m pip install build pre-commit setuptools wheel

python scripts/build-python-release.py build-wheel \
  --target x86_64-unknown-linux-gnu \
  --binary target/debug/shuck \
  --out-dir "$RUNNER_TEMP/python-dist"

pre-commit validate-manifest .pre-commit-hooks.yaml
