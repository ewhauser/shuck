#!/usr/bin/env python3
"""Validate release-please version bumps for Rust crates and Python packaging files."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def publishable_workspace_crates(repo_root: Path) -> list[str]:
    result = subprocess.run(
        [sys.executable, str(repo_root / "scripts" / "workspace-publish-crates.py")],
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def configured_jsonpaths(repo_root: Path) -> set[str]:
    config = json.loads((repo_root / ".release-please-config.json").read_text())
    package_config = config["packages"]["."]
    return {
        f"{entry['path']}::{entry['jsonpath']}"
        for entry in package_config.get("extra-files", [])
        if "jsonpath" in entry and "path" in entry
    }


def configured_generic_paths(repo_root: Path) -> set[str]:
    config = json.loads((repo_root / ".release-please-config.json").read_text())
    package_config = config["packages"]["."]
    return {
        entry["path"]
        for entry in package_config.get("extra-files", [])
        if entry.get("type") == "generic" and "path" in entry
    }


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    expected = {
        f"Cargo.toml::$.workspace.dependencies['{crate_name}'].version"
        for crate_name in publishable_workspace_crates(repo_root)
    }
    expected.update(
        {
            "python/pyproject.toml::$.project.version",
        }
    )
    actual = configured_jsonpaths(repo_root)
    missing = sorted(expected - actual)
    generic_paths = configured_generic_paths(repo_root)

    if "pyproject.toml" not in generic_paths:
        missing.append("pyproject.toml::generic")

    if missing:
        for entry in missing:
            print(f"missing release-please extra-files entry: {entry}", file=sys.stderr)
        return 1

    pyproject_text = (repo_root / "pyproject.toml").read_text()
    marker = "x-release-please-version"
    marker_count = pyproject_text.count(marker)
    if marker_count != 2:
        print(
            f"expected pyproject.toml to contain 2 {marker!r} annotations, found {marker_count}",
            file=sys.stderr,
        )
        return 1

    print("release-please config covers Rust crates and Python packaging metadata")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
