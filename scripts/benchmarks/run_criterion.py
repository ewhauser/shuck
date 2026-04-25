#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run all shuck Criterion benchmarks with an explicit baseline mode."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        required=True,
        help="Path to the shuck repository checkout to benchmark.",
    )

    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--save-baseline",
        metavar="NAME",
        help="Save benchmark results into the named Criterion baseline.",
    )
    mode.add_argument(
        "--baseline",
        metavar="NAME",
        help="Compare benchmark results against the named Criterion baseline.",
    )

    parser.add_argument(
        "--package",
        default="shuck-benchmark",
        help="Cargo package that owns the Criterion benches.",
    )
    parser.add_argument(
        "--features",
        default="",
        help="Comma-separated Cargo features to enable for optional benchmark targets.",
    )
    parser.add_argument(
        "--noplot",
        action="store_true",
        help="Disable Criterion plot generation.",
    )
    return parser.parse_args()


def parse_features(features: str) -> set[str]:
    return {feature.strip() for feature in features.split(",") if feature.strip()}


def load_bench_names(
    repo_root: Path, package_name: str, enabled_features: set[str]
) -> list[str]:
    metadata = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(repo_root / "Cargo.toml"),
            "--format-version",
            "1",
            "--no-deps",
        ],
        check=True,
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    payload = json.loads(metadata.stdout)
    benches: list[str] = []

    for package in payload.get("packages", []):
        if package.get("name") != package_name:
            continue
        for target in package.get("targets", []):
            if "bench" not in target.get("kind", []):
                continue
            name = target.get("name")
            if not isinstance(name, str):
                continue
            required_features = set(target.get("required-features") or [])
            if required_features and not required_features.issubset(enabled_features):
                continue
            benches.append(name)
        break

    if not benches:
        raise SystemExit(f"no Criterion benches found in cargo metadata for {package_name}")
    return benches


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    enabled_features = parse_features(args.features)
    bench_names = load_bench_names(repo_root, args.package, enabled_features)

    command = ["cargo", "bench", "-p", args.package]
    if enabled_features:
        command.extend(["--features", ",".join(sorted(enabled_features))])
    for bench_name in bench_names:
        command.extend(["--bench", bench_name])

    command.append("--")
    if args.save_baseline is not None:
        command.append(f"--save-baseline={args.save_baseline}")
    else:
        command.append(f"--baseline={args.baseline}")

    if args.noplot:
        command.append("--noplot")

    print(f"Repository: {repo_root}")
    if enabled_features:
        print(f"Features: {', '.join(sorted(enabled_features))}")
    print(f"Benches: {', '.join(bench_names)}")
    print(f"Command: {' '.join(command)}")
    completed = subprocess.run(command, cwd=repo_root)
    return completed.returncode


if __name__ == "__main__":
    sys.exit(main())
