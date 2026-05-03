#!/usr/bin/env python3
"""Aggregate per-repo hyperfine results into a website-friendly JSON dataset.

Reads the directory produced by ``scripts/benchmarks/run_repo_corpus.sh`` plus
``manifest.yaml`` from the large corpus cache, and emits a single dataset JSON
that the website ``benchmarks.tsx`` page consumes.
"""

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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument(
        "--bench-dir",
        type=Path,
        required=True,
        help="Directory containing per-repo subdirectories with bench.json + meta.json.",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        required=True,
        help="Path to .cache/large-corpus/manifest.yaml",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dataset-id", required=True)
    parser.add_argument("--dataset-name", required=True)
    parser.add_argument("--dataset-description", required=True)
    parser.add_argument(
        "--environment-kind", choices=("local", "ci"), default="local"
    )
    parser.add_argument("--environment-label", default="")
    parser.add_argument(
        "--source-command", default="make bench-repo-corpus"
    )
    parser.add_argument("--notes", default="")
    parser.add_argument("--warmup-runs", type=int, default=1)
    parser.add_argument("--measured-runs", type=int, default=3)
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
        return run_capture(
            ["sysctl", "-n", "machdep.cpu.brand_string"], cwd=Path.cwd()
        ) or None
    if system == "Linux":
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.exists():
            for line in cpuinfo.read_text(errors="replace").splitlines():
                if line.lower().startswith("model name"):
                    return line.partition(":")[2].strip() or None
    return platform.processor() or None


def detect_os_label() -> str:
    system = platform.system()
    if system == "Darwin":
        version = platform.mac_ver()[0] or platform.release()
        return f"macOS {version}"
    if system == "Linux":
        os_release = Path("/etc/os-release")
        if os_release.exists():
            for line in os_release.read_text(errors="replace").splitlines():
                if line.startswith("PRETTY_NAME="):
                    return line.partition("=")[2].strip().strip('"')
        return f"Linux {platform.release()}"
    return platform.platform()


def parse_manifest(manifest_path: Path) -> dict[str, dict[str, str]]:
    """Parse the simple two-level YAML the corpus downloader emits.

    Returns a mapping ``{ "owner/name": { "commit": ..., "date": ... } }``.
    """
    text = manifest_path.read_text(errors="replace")
    repos: dict[str, dict[str, str]] = {}
    current: dict[str, str] | None = None
    for raw in text.splitlines():
        line = raw.rstrip()
        if not line or line.startswith("#"):
            continue
        match = re.match(r"^\s*-\s*repo:\s*(\S+)\s*$", line)
        if match:
            current = {"repo": match.group(1)}
            repos[match.group(1)] = current
            continue
        if current is None:
            continue
        kv = re.match(r"^\s+(\w+):\s*(\S.*)$", line)
        if kv:
            current[kv.group(1)] = kv.group(2).strip()
    return repos


def detect_shuck_version(repo_root: Path) -> str | None:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", repo_root / "target"))
    shuck_bin = Path(
        os.environ.get(
            "SHUCK_BENCHMARK_SHUCK_BIN", target_dir / "release" / "shuck"
        )
    )
    if shuck_bin.exists():
        out = run_capture([str(shuck_bin), "--version"], cwd=repo_root)
        if out:
            return out.splitlines()[0].strip()

    workspace_toml = repo_root / "Cargo.toml"
    if workspace_toml.exists():
        match = re.search(
            r"(?ms)^\[workspace\.package\]\s+version\s*=\s*\"([^\"]+)\"",
            workspace_toml.read_text(errors="replace"),
        )
        if match:
            return f"shuck {match.group(1)}"
    return None


def detect_repository_url(repo_root: Path) -> str | None:
    env_repo = os.environ.get("GITHUB_REPOSITORY")
    env_server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    if env_repo:
        return f"{env_server.rstrip('/')}/{env_repo}"
    remote = run_capture(["git", "config", "--get", "remote.origin.url"], cwd=repo_root)
    if not remote:
        return None
    if remote.startswith("git@github.com:"):
        remote = "https://github.com/" + remote.removeprefix("git@github.com:")
    elif remote.startswith("ssh://git@github.com/"):
        remote = "https://github.com/" + remote.removeprefix("ssh://git@github.com/")
    remote = re.sub(r"\.git$", "", remote)
    return remote if remote.startswith("https://github.com/") else None


def make_measurement(
    tool: str, result: dict[str, Any], shuck_mean: float | None
) -> dict[str, Any]:
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
        "meanMemoryBytes": int(sum(memory_usage) / len(memory_usage))
        if memory_usage
        else None,
        "maxMemoryBytes": max(memory_usage) if memory_usage else None,
        "runCount": len(times) or len(result.get("exit_codes", [])),
        "exitCodes": exit_codes,
        "hasFailures": any(code != 0 for code in exit_codes),
        "relativeToShuck": relative_to_shuck,
    }


def load_repo_cases(
    bench_dir: Path, manifest: dict[str, dict[str, str]]
) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    if not bench_dir.exists():
        return cases

    for entry in sorted(bench_dir.iterdir()):
        if not entry.is_dir():
            continue
        meta_path = entry / "meta.json"
        bench_path = entry / "bench.json"
        if not meta_path.exists() or not bench_path.exists():
            continue

        meta = read_json(meta_path)
        results = list(read_json(bench_path).get("results", []))

        shuck_result = next(
            (r for r in results if "shuck" in str(r.get("command", ""))
             and "shellcheck" not in str(r.get("command", "")).split()[0]),
            None,
        )
        comparison_result = next(
            (r for r in results if "shellcheck" in str(r.get("command", ""))),
            None,
        )

        shuck_mean = float(shuck_result["mean"]) if shuck_result else None
        measurements: list[dict[str, Any]] = []
        if shuck_result is not None:
            measurements.append(make_measurement("shuck", shuck_result, shuck_mean))
        if comparison_result is not None:
            measurements.append(
                make_measurement("shellcheck", comparison_result, shuck_mean)
            )

        repo_name = meta["repo"]
        manifest_entry = manifest.get(repo_name, {})
        # Prefer the SHA the bench script actually checked out; fall back to
        # the manifest pin when older meta files lack it.
        commit = meta.get("commit") or manifest_entry.get("commit")
        commit_short = (
            meta.get("commitShort")
            or (commit[:7] if commit else None)
        )
        repo_url = f"https://github.com/{repo_name}"
        commit_url = f"{repo_url}/commit/{commit}" if commit else None

        cases.append(
            {
                "slug": meta["repoKey"],
                "repo": repo_name,
                "fileCount": int(meta["fileCount"]),
                "availableFileCount": int(meta.get("availableFileCount", meta["fileCount"])),
                "truncated": bool(meta.get("truncated", False)),
                "truncateLimit": int(meta.get("truncateLimit", 0)) or None,
                "totalBytes": int(meta["totalBytes"]),
                "totalLines": int(meta["totalLines"]),
                "commit": commit,
                "commitShort": commit_short,
                "captureDate": manifest_entry.get("date"),
                "repoUrl": repo_url,
                "commitUrl": commit_url,
                "measurements": measurements,
            }
        )
    return cases


def build_summary(cases: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not cases:
        return None
    total_shuck = 0.0
    total_comparison = 0.0
    total_files = 0
    total_lines = 0
    for case in cases:
        shuck = next((m for m in case["measurements"] if m["tool"] == "shuck"), None)
        comparison = next(
            (m for m in case["measurements"] if m["tool"] != "shuck"), None
        )
        if not shuck or not comparison:
            continue
        total_shuck += shuck["meanSeconds"]
        total_comparison += comparison["meanSeconds"]
        total_files += case["fileCount"]
        total_lines += case["totalLines"]

    speedup = (
        total_comparison / total_shuck
        if total_shuck > 0
        else None
    )
    return {
        "repoCount": len(cases),
        "totalFiles": total_files,
        "totalLines": total_lines,
        "shuckTotalSeconds": total_shuck,
        "comparisonTotalSeconds": total_comparison,
        "speedupRatio": speedup,
    }


def write_json_atomic(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(
        "w", encoding="utf-8", delete=False, dir=path.parent
    ) as handle:
        json.dump(payload, handle, indent=2)
        handle.write("\n")
        temp_path = Path(handle.name)
    temp_path.replace(path)


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    bench_dir = args.bench_dir.resolve()

    manifest = parse_manifest(args.manifest.resolve())
    cases = load_repo_cases(bench_dir, manifest)
    summary = build_summary(cases)

    cpu = detect_cpu()
    generated_at = (
        datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    )

    repository_url = detect_repository_url(repo_root)
    commit_sha = run_capture(["git", "rev-parse", "HEAD"], cwd=repo_root)
    commit_short = commit_sha[:7] if commit_sha else None
    commit_url = (
        f"{repository_url}/commit/{commit_sha}" if repository_url and commit_sha else None
    )

    shuck_version = detect_shuck_version(repo_root)
    hyperfine_version = (
        run_capture(["hyperfine", "--version"], cwd=repo_root) or ""
    ).splitlines()[0:1]
    shellcheck_raw = run_capture(["shellcheck", "--version"], cwd=repo_root) or ""
    shellcheck_version = None
    for line in shellcheck_raw.splitlines():
        if line.lower().startswith("version:"):
            shellcheck_version = line.partition(":")[2].strip()
            break

    environment_label = args.environment_label.strip()
    if not environment_label:
        if args.environment_kind == "local" and cpu:
            environment_label = f"{cpu} local snapshot"
        else:
            environment_label = "Repo-corpus snapshot"

    payload = {
        "schemaVersion": 1,
        "available": bool(cases),
        "id": args.dataset_id,
        "name": args.dataset_name,
        "description": args.dataset_description,
        "generatedAt": generated_at,
        "commit": {"sha": commit_sha, "shortSha": commit_short},
        "links": {
            "repositoryUrl": repository_url,
            "commitUrl": commit_url,
            "runUrl": None,
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
            "hyperfine": hyperfine_version[0] if hyperfine_version else None,
            "shellcheck": shellcheck_version,
        },
        "methodology": {
            "benchmarkCommand": args.source_command,
            "warmupRuns": args.warmup_runs,
            "measuredRuns": args.measured_runs,
            "shuckCommand": "xargs shuck check --no-cache --select ALL <filelist>",
            "comparisonCommand": "xargs shellcheck --enable=all --severity=style <filelist>",
            "notes": args.notes or None,
        },
        "summary": summary,
        "cases": cases,
    }

    write_json_atomic(args.output.resolve(), payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
