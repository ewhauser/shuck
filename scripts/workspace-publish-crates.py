#!/usr/bin/env python3
"""Print publishable workspace crates in dependency order.

This keeps the crates.io publish workflow and the local bootstrap scripts aligned
with the current Cargo workspace metadata.
"""

from __future__ import annotations

import json
import subprocess
import sys
from graphlib import TopologicalSorter


def load_workspace_metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def is_publishable(package: dict) -> bool:
    publish = package.get("publish")
    return publish != []


def main() -> int:
    metadata = load_workspace_metadata()
    workspace_member_ids = set(metadata["workspace_members"])
    workspace_packages = [
        package for package in metadata["packages"] if package["id"] in workspace_member_ids
    ]

    workspace_names = {package["name"] for package in workspace_packages}
    publishable_packages = sorted(
        (package for package in workspace_packages if is_publishable(package)),
        key=lambda package: package["name"],
    )
    publishable_names = {package["name"] for package in publishable_packages}

    graph: dict[str, tuple[str, ...]] = {}
    unpublished_dependency_errors: list[str] = []

    for package in publishable_packages:
        internal_dependencies = set()
        for dependency in package.get("dependencies", []):
            if dependency.get("path") is None:
                continue
            dependency_name = dependency["name"]
            if dependency_name not in workspace_names:
                continue
            if dependency_name not in publishable_names:
                unpublished_dependency_errors.append(
                    f"{package['name']} depends on unpublished workspace crate {dependency_name}"
                )
                continue
            internal_dependencies.add(dependency_name)
        graph[package["name"]] = tuple(sorted(internal_dependencies))

    if unpublished_dependency_errors:
        for error in unpublished_dependency_errors:
            print(f"error: {error}", file=sys.stderr)
        return 1

    for crate_name in TopologicalSorter(graph).static_order():
        print(crate_name)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
