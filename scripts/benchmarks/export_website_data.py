#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any


DEFAULT_WARMUP_RUNS = 3
DEFAULT_MEASURED_RUNS = 10


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Export macro benchmark results into a website-friendly JSON payload."
    )
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--bench-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dataset-id", required=True)
    parser.add_argument("--dataset-name", required=True)
    parser.add_argument("--dataset-description", required=True)
    parser.add_argument(
        "--environment-kind",
        choices=("local", "ci"),
        required=True,
    )
    parser.add_argument("--environment-label", default="")
    parser.add_argument(
        "--source-command",
        default="make bench-macro",
        help="Human-readable command used to produce the benchmark exports.",
    )
    parser.add_argument(
        "--notes",
        default="",
        help="Optional explanatory note stored alongside the methodology metadata.",
    )
    parser.add_argument("--run-url", default="")
    parser.add_argument("--warmup-runs", type=int, default=DEFAULT_WARMUP_RUNS)
    parser.add_argument("--measured-runs", type=int, default=DEFAULT_MEASURED_RUNS)
    return parser.parse_args()


def run_capture(args: list[str], cwd: Path) -> str | None:
    try:
        completed = subprocess.run(
            args,
            cwd=cwd,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        return None
    return completed.stdout.strip()


def read_json(path: Path) -> Any:
    return json.loads(path.read_text())


def detect_cpu() -> str | None:
    system = platform.system()
    if system == "Darwin":
        cpu = run_capture(["sysctl", "-n", "machdep.cpu.brand_string"], cwd=Path.cwd())
        return cpu or None
    if system == "Linux":
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.exists():
            for line in cpuinfo.read_text(errors="replace").splitlines():
                if line.lower().startswith("model name"):
                    _, _, value = line.partition(":")
                    return value.strip() or None
    return platform.processor() or None


def detect_os_label() -> str:
    system = platform.system()
    if system == "Darwin":
        version = platform.mac_ver()[0] or platform.release()
        return f"macOS {version}"
    if system == "Linux":
        os_release = Path("/etc/os-release")
        if os_release.exists():
            pretty_name = None
            for line in os_release.read_text(errors="replace").splitlines():
                if line.startswith("PRETTY_NAME="):
                    pretty_name = line.partition("=")[2].strip().strip('"')
                    break
            if pretty_name:
                return pretty_name
        return f"Linux {platform.release()}"
    if system == "Windows":
        return f"Windows {platform.release()}"
    return platform.platform()


def parse_shellcheck_version(output: str | None) -> str | None:
    if not output:
        return None
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.lower().startswith("version:"):
            return stripped.partition(":")[2].strip()
    first_line = output.splitlines()[0].strip()
    return first_line or None


def parse_hyperfine_version(output: str | None) -> str | None:
    if not output:
        return None
    first_line = output.splitlines()[0].strip()
    return first_line or None


def parse_shuck_version(output: str | None) -> str | None:
    if not output:
        return None
    first_line = output.splitlines()[0].strip()
    return first_line or None


def detect_shuck_version(repo_root: Path, shuck_bin: Path) -> str | None:
    direct_version = parse_shuck_version(run_capture([str(shuck_bin), "--version"], cwd=repo_root))
    if direct_version:
        return direct_version

    cargo_toml = repo_root / "crates" / "shuck" / "Cargo.toml"
    package_match = re.search(
        r'^version\s*=\s*"([^"]+)"',
        cargo_toml.read_text(errors="replace"),
        re.MULTILINE,
    )
    if package_match:
        return f"shuck {package_match.group(1)}"

    workspace_toml = repo_root / "Cargo.toml"
    workspace_match = re.search(
        r"(?ms)^\[workspace\.package\]\s+version\s*=\s*\"([^\"]+)\"",
        workspace_toml.read_text(errors="replace"),
    )
    if workspace_match:
        return f"shuck {workspace_match.group(1)}"
    return None


def normalize_repository_url(url: str | None) -> str | None:
    if not url:
        return None
    normalized = url.strip()
    if normalized.startswith("git@github.com:"):
        normalized = "https://github.com/" + normalized.removeprefix("git@github.com:")
    elif normalized.startswith("ssh://git@github.com/"):
        normalized = "https://github.com/" + normalized.removeprefix("ssh://git@github.com/")

    normalized = re.sub(r"\.git$", "", normalized)
    if normalized.startswith("https://github.com/"):
        return normalized
    return None


def detect_repository_url(repo_root: Path) -> str | None:
    env_repo = os.environ.get("GITHUB_REPOSITORY")
    env_server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    if env_repo:
        return f"{env_server.rstrip('/')}/{env_repo}"

    remote = run_capture(["git", "config", "--get", "remote.origin.url"], cwd=repo_root)
    return normalize_repository_url(remote)


def load_fixture_metadata(repo_root: Path) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    manifest_path = repo_root / "crates" / "shuck-benchmark" / "resources" / "manifest.json"
    manifest = read_json(manifest_path)

    fixtures: list[dict[str, Any]] = []
    fixtures_by_slug: dict[str, dict[str, Any]] = {}
    for fixture in manifest["fixtures"]:
        local_filename = fixture["local_filename"]
        file_path = manifest_path.parent / local_filename
        source = file_path.read_text(errors="replace")
        slug = Path(local_filename).stem
        line_count = len(source.splitlines())
        entry = {
            "slug": slug,
            "name": slug,
            "fileName": Path(local_filename).name,
            "path": local_filename,
            "bytes": int(fixture["byte_size"]),
            "lines": line_count,
            "upstreamRepo": fixture["upstream_repo"],
            "upstreamPath": fixture["upstream_path"],
            "sourceUrl": fixture["source_url"],
            "license": fixture["spdx_license_id"],
            "commit": fixture["commit"],
            "commitShort": fixture["commit"][:7],
        }
        fixtures.append(entry)
        fixtures_by_slug[slug] = entry
    return fixtures, fixtures_by_slug


def make_measurement(tool: str, result: dict[str, Any], shuck_mean: float | None) -> dict[str, Any]:
    times = [float(value) for value in result.get("times", [])]
    memory_usage = [int(value) for value in result.get("memory_usage_byte", [])]
    exit_codes = sorted({int(code) for code in result.get("exit_codes", [])})
    mean_seconds = float(result["mean"])

    relative_to_shuck = None
    if shuck_mean and shuck_mean > 0.0:
        relative_to_shuck = mean_seconds / shuck_mean

    return {
        "tool": tool,
        "command": str(result.get("command", "")),
        "meanSeconds": mean_seconds,
        "stddevSeconds": float(result.get("stddev", 0.0)),
        "medianSeconds": float(result.get("median", mean_seconds)),
        "minSeconds": float(result.get("min", mean_seconds)),
        "maxSeconds": float(result.get("max", mean_seconds)),
        "userSeconds": float(result.get("user", 0.0)),
        "systemSeconds": float(result.get("system", 0.0)),
        "meanMemoryBytes": int(sum(memory_usage) / len(memory_usage)) if memory_usage else None,
        "maxMemoryBytes": max(memory_usage) if memory_usage else None,
        "runCount": len(times) or len(result.get("exit_codes", [])),
        "exitCodes": exit_codes,
        "hasFailures": any(code != 0 for code in exit_codes),
        "relativeToShuck": relative_to_shuck,
    }


def load_cases(
    bench_dir: Path,
    fixtures: list[dict[str, Any]],
    fixtures_by_slug: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    bench_paths = list(bench_dir.glob("bench-*.json"))
    raw_cases: dict[str, list[dict[str, Any]]] = {}
    for bench_path in bench_paths:
        payload = read_json(bench_path)
        case_slug = bench_path.stem.removeprefix("bench-")
        raw_cases[case_slug] = list(payload.get("results", []))

    fixture_order = {fixture["slug"]: index for index, fixture in enumerate(fixtures)}

    def sort_key(slug: str) -> tuple[int, int, str]:
        if slug == "all":
            return (0, -1, slug)
        return (1, fixture_order.get(slug, len(fixture_order)), slug)

    cases: list[dict[str, Any]] = []
    for slug in sorted(raw_cases, key=sort_key):
        results = raw_cases[slug]
        fixture = fixtures_by_slug.get(slug)
        shuck_result = None
        for result in results:
            tool = str(result.get("command", "")).split("/", 1)[0]
            if tool == "shuck":
                shuck_result = result
                break

        shuck_mean = float(shuck_result["mean"]) if shuck_result else None
        measurements: list[dict[str, Any]] = []
        for result in results:
            tool = str(result.get("command", "")).split("/", 1)[0]
            measurements.append(make_measurement(tool, result, shuck_mean))
        measurements.sort(key=lambda item: (item["tool"] != "shuck", item["tool"]))

        entry: dict[str, Any] = {
            "slug": slug,
            "name": slug,
            "kind": "aggregate" if slug == "all" else "fixture",
            "bytes": sum(item["bytes"] for item in fixtures) if slug == "all" else (fixture["bytes"] if fixture else None),
            "lines": sum(item["lines"] for item in fixtures) if slug == "all" else (fixture["lines"] if fixture else None),
            "fixtureCount": len(fixtures) if slug == "all" else 1,
            "measurements": measurements,
        }
        if fixture is not None:
            entry["fixture"] = fixture
        cases.append(entry)
    return cases


def build_summary(cases: list[dict[str, Any]]) -> dict[str, Any] | None:
    aggregate = next((case for case in cases if case["slug"] == "all"), None)
    if aggregate is None:
        return None

    shuck = next((item for item in aggregate["measurements"] if item["tool"] == "shuck"), None)
    comparison = next((item for item in aggregate["measurements"] if item["tool"] != "shuck"), None)
    if shuck is None:
        return None

    summary: dict[str, Any] = {
        "aggregateCase": "all",
        "primaryTool": "shuck",
        "comparisonTool": comparison["tool"] if comparison else None,
        "shuckMeanSeconds": shuck["meanSeconds"],
        "comparisonMeanSeconds": comparison["meanSeconds"] if comparison else None,
        "speedupRatio": None,
        "timeSavedPct": None,
    }

    if comparison and shuck["meanSeconds"] > 0.0:
        speedup_ratio = comparison["meanSeconds"] / shuck["meanSeconds"]
        summary["speedupRatio"] = speedup_ratio
        summary["timeSavedPct"] = (1.0 - (shuck["meanSeconds"] / comparison["meanSeconds"])) * 100.0

    return summary


def write_json_atomic(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile("w", encoding="utf-8", delete=False, dir=path.parent) as handle:
        json.dump(payload, handle, indent=2)
        handle.write("\n")
        temp_path = Path(handle.name)
    temp_path.replace(path)


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    bench_dir = args.bench_dir.resolve()

    fixtures, fixtures_by_slug = load_fixture_metadata(repo_root)
    cases = load_cases(bench_dir, fixtures, fixtures_by_slug)
    summary = build_summary(cases)
    cpu = detect_cpu()
    generated_at = (
        datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    )

    repository_url = detect_repository_url(repo_root)
    commit_sha = run_capture(["git", "rev-parse", "HEAD"], cwd=repo_root)
    commit_short = commit_sha[:7] if commit_sha else None
    commit_url = f"{repository_url}/commit/{commit_sha}" if repository_url and commit_sha else None

    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", repo_root / "target"))
    shuck_bin = Path(os.environ.get("SHUCK_BENCHMARK_SHUCK_BIN", target_dir / "release" / "shuck"))

    shuck_version = detect_shuck_version(repo_root, shuck_bin)
    hyperfine_version = parse_hyperfine_version(run_capture(["hyperfine", "--version"], cwd=repo_root))
    shellcheck_version = parse_shellcheck_version(run_capture(["shellcheck", "--version"], cwd=repo_root))

    comparison_tool = summary["comparisonTool"] if summary else None
    benchmark_mode = "compare" if comparison_tool else "shuck-only"
    environment_label = args.environment_label.strip()
    if not environment_label:
        if args.environment_kind == "local" and cpu:
            environment_label = f"{cpu} local snapshot"
        elif args.environment_kind == "ci":
            environment_label = "GitHub Actions CI snapshot"
        else:
            environment_label = "Macro benchmark snapshot"

    payload = {
        "schemaVersion": 1,
        "available": True,
        "id": args.dataset_id,
        "name": args.dataset_name,
        "description": args.dataset_description,
        "generatedAt": generated_at,
        "commit": {
            "sha": commit_sha,
            "shortSha": commit_short,
        },
        "links": {
            "repositoryUrl": repository_url,
            "commitUrl": commit_url,
            "runUrl": args.run_url or None,
        },
        "environment": {
            "kind": args.environment_kind,
            "label": environment_label,
            "os": detect_os_label(),
            "arch": platform.machine(),
            "cpu": cpu,
            "python": platform.python_version(),
        },
        "toolVersions": {
            "shuck": shuck_version,
            "hyperfine": hyperfine_version,
            "shellcheck": shellcheck_version,
        },
        "methodology": {
            "benchmarkCommand": args.source_command,
            "benchmarkMode": benchmark_mode,
            "warmupRuns": args.warmup_runs,
            "measuredRuns": args.measured_runs,
            "ignoreFailure": True,
            "shuckCommand": "shuck check --no-cache <fixture>",
            "comparisonCommand": (
                "shellcheck --severity=style <fixture>" if comparison_tool else None
            ),
            "notes": args.notes or None,
        },
        "corpus": {
            "fixtureCount": len(fixtures),
            "totalBytes": sum(item["bytes"] for item in fixtures),
            "totalLines": sum(item["lines"] for item in fixtures),
            "fixtures": fixtures,
        },
        "summary": summary,
        "cases": cases,
    }

    write_json_atomic(args.output.resolve(), payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
