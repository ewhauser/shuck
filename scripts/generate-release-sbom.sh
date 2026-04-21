#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)

cd "$repo_root"

# cargo-cyclonedx emits one SBOM per workspace crate next to each Cargo.toml.
# Dist expects a single release artifact, so we copy the distributable crate's
# SBOM to the repository root and clean up the temporary workspace files.
find crates -name '*.cdx.xml' -delete
cargo cyclonedx --manifest-path Cargo.toml --format xml -q
cp crates/shuck/shuck.cdx.xml shuck.cdx.xml
find crates -name '*.cdx.xml' -delete
