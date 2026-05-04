#!/usr/bin/env python3

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


SCRIPT_PATH = Path(__file__).resolve()
MANUAL_ROOT = SCRIPT_PATH.parent
REPO_ROOT = MANUAL_ROOT.parents[3]
FIXTURE_ROOT = MANUAL_ROOT / "fixtures" / "workspace"

SCENARIOS = [
    "diagnostics/open_edit",
    "hover/rule_directive",
    "code_actions/quick_fix",
    "code_actions/fix_all",
    "formatting/request_round_trip",
    "configuration/reload_workspace_config",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run headless Neovim black-box tests against shuck server."
    )
    parser.add_argument(
        "--case",
        dest="cases",
        action="append",
        choices=sorted(SCENARIOS),
        help="Run only the named scenario. May be passed multiple times.",
    )
    parser.add_argument(
        "--nvim",
        default="nvim",
        help="Path to the Neovim binary to launch.",
    )
    parser.add_argument(
        "--cargo",
        default="cargo",
        help="Path to the cargo binary used to build shuck.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Reuse the existing target/debug/shuck binary instead of rebuilding it.",
    )
    return parser.parse_args()


def build_server(cargo: str, skip_build: bool) -> Path:
    server_binary = REPO_ROOT / "target" / "debug" / "shuck"
    if skip_build:
        if not server_binary.exists():
            raise SystemExit(
                f"--skip-build was set, but {server_binary} does not exist yet"
            )
        return server_binary

    cmd = [cargo, "build", "-p", "shuck-cli"]
    print("+", " ".join(cmd))
    subprocess.run(cmd, cwd=REPO_ROOT, check=True)
    if not server_binary.exists():
        raise SystemExit(f"expected cargo to produce {server_binary}")
    return server_binary


def stage_fixture_workspace() -> tuple[tempfile.TemporaryDirectory[str], Path]:
    tempdir = tempfile.TemporaryDirectory(prefix="shuck-lsp-blackbox-")
    temp_root = Path(tempdir.name)
    staged_project = temp_root / "project"
    shutil.copytree(FIXTURE_ROOT, staged_project)
    return tempdir, staged_project


def run_case(case_name: str, server_binary: Path, nvim: str) -> None:
    tempdir, project_root = stage_fixture_workspace()
    try:
        env = os.environ.copy()
        env["SHUCK_LSP_CASE"] = case_name
        env["SHUCK_LSP_SERVER_BINARY"] = str(server_binary)
        env["SHUCK_LSP_WORKSPACE_ROOT"] = str(project_root)

        cmd = [
            nvim,
            "--headless",
            "-u",
            str(MANUAL_ROOT / "minimal_init.lua"),
            "-c",
            "lua require('shuck_server_blackbox').run()",
        ]
        print("+", " ".join(cmd))
        completed = subprocess.run(
            cmd,
            cwd=REPO_ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            sys.stderr.write(completed.stdout)
            sys.stderr.write(completed.stderr)
            raise SystemExit(f"scenario {case_name} failed with exit code {completed.returncode}")

        stdout = completed.stdout.strip()
        if stdout:
            print(stdout)
        print(f"[pass] {case_name}")
    finally:
        tempdir.cleanup()


def main() -> None:
    args = parse_args()
    selected_cases = args.cases or list(SCENARIOS)
    server_binary = build_server(args.cargo, args.skip_build)
    for case_name in selected_cases:
        run_case(case_name, server_binary, args.nvim)


if __name__ == "__main__":
    main()
