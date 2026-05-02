#!/usr/bin/env python3
"""Validate release-please version bumps for publishable workspace crates."""

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
        entry["jsonpath"]
        for entry in package_config.get("extra-files", [])
        if entry.get("path") == "Cargo.toml" and "jsonpath" in entry
    }


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    expected = {
        f"$.workspace.dependencies['{crate_name}'].version"
        for crate_name in publishable_workspace_crates(repo_root)
    }
    actual = configured_jsonpaths(repo_root)
    missing = sorted(expected - actual)

    if missing:
        for jsonpath in missing:
            print(f"missing release-please extra-files entry: {jsonpath}", file=sys.stderr)
        return 1

    print("release-please config covers all publishable workspace crates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
