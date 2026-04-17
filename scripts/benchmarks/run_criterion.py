#!/usr/bin/env python3

from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
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
        "--noplot",
        action="store_true",
        help="Disable Criterion plot generation.",
    )
    return parser.parse_args()


def load_bench_names(repo_root: Path) -> list[str]:
    manifest_path = repo_root / "crates" / "shuck-benchmark" / "Cargo.toml"
    data = tomllib.loads(manifest_path.read_text())
    benches = [
        bench["name"]
        for bench in data.get("bench", [])
        if isinstance(bench, dict) and isinstance(bench.get("name"), str)
    ]
    if not benches:
        raise SystemExit(f"no Criterion benches found in {manifest_path}")
    return benches


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    bench_names = load_bench_names(repo_root)

    command = ["cargo", "bench", "-p", args.package]
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
    print(f"Benches: {', '.join(bench_names)}")
    print(f"Command: {' '.join(command)}")
    completed = subprocess.run(command, cwd=repo_root)
    return completed.returncode


if __name__ == "__main__":
    sys.exit(main())
