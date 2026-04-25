#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path


PRIMARY_METRIC_GROUPS = ["facts_metrics", "check_metrics"]
PRIMARY_METRICS = [
    "total_allocated_bytes",
    "total_reallocated_bytes",
    "allocation_count",
    "reallocation_count",
    "peak_live_bytes",
    "final_live_bytes",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run linter memory benchmarks with explicit save/compare baseline modes."
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
        help="Save linter memory results into the named baseline.",
    )
    mode.add_argument(
        "--baseline",
        metavar="NAME",
        help="Compare linter memory results against the named baseline.",
    )

    parser.add_argument(
        "--package",
        default="shuck-benchmark",
        help="Cargo package that owns the linter memory example.",
    )
    parser.add_argument(
        "--example",
        default="linter_memory",
        help="Example target that emits linter memory JSON.",
    )
    parser.add_argument(
        "--release",
        action="store_true",
        help="Run the memory harness in release mode.",
    )
    return parser.parse_args()


def target_dir(repo_root: Path) -> Path:
    cargo_target_dir = os.environ.get("CARGO_TARGET_DIR")
    if cargo_target_dir:
        target = Path(cargo_target_dir)
        if not target.is_absolute():
            target = repo_root / target
        return target
    return repo_root / "target"


def run_example(repo_root: Path, package: str, example: str, release: bool) -> list[dict]:
    command = ["cargo", "run", "-p", package, "--example", example]
    if release:
        command.append("--release")
    command.append("--quiet")

    completed = subprocess.run(
        command,
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(completed.stdout)


def baseline_path(repo_root: Path, baseline_name: str) -> Path:
    return target_dir(repo_root) / "linter-memory-baselines" / f"{baseline_name}.json"


def metric_change(current: int, baseline: int) -> str:
    if baseline == 0:
        return "n/a" if current == 0 else "inf"
    change = ((current / baseline) - 1.0) * 100.0
    sign = "+" if change > 0 else ""
    return f"{sign}{change:.2f}%"


def index_cases(payload: list[dict]) -> dict[str, dict]:
    return {entry["case"]: entry for entry in payload}


def print_comparison(current: list[dict], baseline: list[dict]) -> None:
    current_cases = index_cases(current)
    baseline_cases = index_cases(baseline)

    missing_in_current = sorted(set(baseline_cases) - set(current_cases))
    missing_in_baseline = sorted(set(current_cases) - set(baseline_cases))
    if missing_in_current or missing_in_baseline:
        raise SystemExit(
            "case mismatch between current and baseline: "
            f"missing_in_current={missing_in_current}, missing_in_baseline={missing_in_baseline}"
        )

    for case_name in sorted(current_cases):
        current_case = current_cases[case_name]
        baseline_case = baseline_cases[case_name]
        for count_name in ["command_count", "fact_count", "diagnostic_count"]:
            if current_case[count_name] != baseline_case[count_name]:
                raise SystemExit(
                    f"{count_name} mismatch for {case_name}: "
                    f"{baseline_case[count_name]} != {current_case[count_name]}"
                )
        print(
            f"{case_name}: commands={baseline_case['command_count']} "
            f"facts={baseline_case['fact_count']} diagnostics={baseline_case['diagnostic_count']}"
        )
        for group_name in PRIMARY_METRIC_GROUPS:
            print(f"  {group_name}:")
            for metric_name in PRIMARY_METRICS:
                baseline_value = baseline_case[group_name][metric_name]
                current_value = current_case[group_name][metric_name]
                print(
                    f"    {metric_name}: {baseline_value} -> {current_value} "
                    f"({metric_change(current_value, baseline_value)})"
                )


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    current = run_example(repo_root, args.package, args.example, args.release)

    if args.save_baseline is not None:
        path = baseline_path(repo_root, args.save_baseline)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(current, indent=2) + "\n")
        print(f"saved linter memory baseline `{args.save_baseline}` to {path}")
        return 0

    path = baseline_path(repo_root, args.baseline)
    if not path.is_file():
        raise SystemExit(f"missing linter memory baseline: {path}")

    baseline = json.loads(path.read_text())
    print_comparison(current, baseline)
    return 0


if __name__ == "__main__":
    sys.exit(main())
